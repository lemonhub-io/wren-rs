// Port of wren_compiler.c: lexer, Pratt parser, and direct bytecode
// generation. State lives on the VM during compilation:
//
//   vm.parser    — the shared parser (Option<Parser>)
//   vm.compilers — the compiler stack; parent of compilers[i] is compilers[i-1]
//
// Because holding a `&mut Compiler` would conflict with `&mut Vm` calls
// (allocation can trigger GC), compile functions take `&mut self` plus a
// compiler index `ci` and access state through `self.compilers[ci]` /
// `self.parser` field paths sequentially.

use crate::opcodes::*;
use crate::utils::*;
use crate::value::*;
use crate::WrenVM;

pub const MAX_LOCALS: usize = 256;
pub const MAX_UPVALUES: usize = 256;
pub const MAX_CONSTANTS: usize = 1 << 16;
pub const MAX_JUMP: usize = 1 << 16;
pub const MAX_INTERPOLATION_NESTING: usize = 8;
pub const MAX_VARIABLE_NAME: usize = 64;
pub const MAX_METHOD_NAME: usize = 64;
pub const MAX_PARAMETERS: i32 = 16;
pub const MAX_METHOD_SIGNATURE: usize = MAX_METHOD_NAME + (MAX_PARAMETERS as usize * 2) + 6;
pub const MAX_FIELDS: usize = 255;
pub const MAX_METHODS: i32 = 65535;

// -- Token types --------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum TokenType {
    LeftParen = 0,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Colon,
    Dot,
    DotDot,
    DotDotDot,
    Comma,
    Star,
    Slash,
    Percent,
    Hash,
    Plus,
    Minus,
    LtLt,
    GtGt,
    Pipe,
    PipePipe,
    Caret,
    Amp,
    AmpAmp,
    Bang,
    Tilde,
    Question,
    Eq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    EqEq,
    BangEq,
    Break,
    Continue,
    Class,
    Construct,
    Else,
    False,
    For,
    Foreign,
    If,
    Import,
    As,
    In,
    Is,
    Null,
    Return,
    Static,
    Super,
    This,
    True,
    Var,
    While,
    Field,
    StaticField,
    Name,
    Number,
    Str,
    Interpolation,
    Line,
    Error,
    Eof,
}

pub const NUM_TOKEN_TYPES: usize = TokenType::Eof as usize + 1;

#[derive(Clone, Copy)]
pub struct Token {
    pub typ: TokenType,
    pub start: usize, // offset into parser.source
    pub length: usize,
    pub line: i32,
    pub value: Value,
}

impl Token {
    fn new() -> Token {
        Token {
            typ: TokenType::Error,
            start: 0,
            length: 0,
            line: 0,
            value: UNDEFINED_VAL,
        }
    }
}

pub struct Parser {
    pub module: ObjRef,
    pub source: Vec<u8>,
    pub token_start: usize,
    pub current_char: usize,
    pub current_line: i32,
    pub next: Token,
    pub current: Token,
    pub previous: Token,
    pub parens: [i32; MAX_INTERPOLATION_NESTING],
    pub num_parens: usize,
    pub print_errors: bool,
    pub has_error: bool,
}

/// A variable/signature name: either a slice of the source being compiled or
/// a synthetic name not present in the source ("this", "seq ", "iter ", "").
#[derive(Clone, Copy)]
pub enum NameRef {
    /// (start, length) into parser.source
    Source(usize, usize),
    Static(&'static [u8]),
}

#[derive(Clone, Copy)]
pub struct Local {
    pub name: NameRef,
    pub depth: i32,
    pub is_upvalue: bool,
}

#[derive(Clone, Copy)]
pub struct CompilerUpvalue {
    pub is_local: bool,
    pub index: i32,
}

#[derive(Clone, Copy)]
pub struct Loop {
    pub start: i32,
    pub exit_jump: i32,
    pub body: i32,
    pub scope_depth: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SigType {
    Method,
    Getter,
    Setter,
    Subscript,
    SubscriptSetter,
    Initializer,
}

#[derive(Clone, Copy)]
pub struct Signature {
    pub name: NameRef,
    pub typ: SigType,
    pub arity: i32,
}

pub struct ClassInfo {
    pub name: ObjRef, // ObjString
    pub class_attributes: ObjRef,
    pub method_attributes: ObjRef,
    pub fields: Vec<ObjRef>, // SymbolTable of ObjString
    pub methods: Vec<i32>,
    pub static_methods: Vec<i32>,
    pub is_foreign: bool,
    pub in_static: bool,
    pub signature: Option<Signature>,
}

pub struct Compiler {
    pub locals: Vec<Local>,
    pub upvalues: Vec<CompilerUpvalue>,
    pub scope_depth: i32,
    pub num_slots: i32,
    pub loops: Vec<Loop>,
    pub enclosing_class: Option<ClassInfo>,
    pub fn_ref: ObjRef,    // NULL_OBJ until allocated
    pub constants: ObjRef, // ObjMap or NULL_OBJ
    pub is_initializer: bool,
    pub num_attributes: i32,
    pub attributes: ObjRef, // ObjMap
}

impl Compiler {
    #[inline]
    pub fn num_locals(&self) -> usize {
        self.locals.len()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Local,
    Upvalue,
    Module,
}

#[derive(Clone, Copy)]
pub struct Variable {
    pub index: i32,
    pub scope: Scope,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Prec {
    None = 0,
    Lowest,
    Assignment,
    Conditional,
    LogicalOr,
    LogicalAnd,
    Equality,
    Is,
    Comparison,
    BitwiseOr,
    BitwiseXor,
    BitwiseAnd,
    BitwiseShift,
    Range,
    Term,
    Factor,
    Unary,
    Call,
    Primary,
}

type GrammarFn = fn(&mut WrenVM, usize, bool);
type SignatureFn = fn(&mut WrenVM, usize, &mut Signature);

struct GrammarRule {
    prefix: Option<GrammarFn>,
    infix: Option<GrammarFn>,
    method: Option<SignatureFn>,
    precedence: Prec,
    name: &'static str,
}

const UNUSED: GrammarRule = GrammarRule {
    prefix: None,
    infix: None,
    method: None,
    precedence: Prec::None,
    name: "",
};

macro_rules! prefix_rule {
    ($f:ident) => {
        GrammarRule {
            prefix: Some($f),
            infix: None,
            method: None,
            precedence: Prec::None,
            name: "",
        }
    };
}
macro_rules! infix_rule {
    ($p:expr, $f:ident) => {
        GrammarRule {
            prefix: None,
            infix: Some($f),
            method: None,
            precedence: $p,
            name: "",
        }
    };
}
macro_rules! infix_op_rule {
    ($p:expr, $n:literal) => {
        GrammarRule {
            prefix: None,
            infix: Some(infix_op),
            method: Some(infix_signature),
            precedence: $p,
            name: $n,
        }
    };
}
macro_rules! prefix_op_rule {
    ($n:literal) => {
        GrammarRule {
            prefix: Some(unary_op),
            infix: None,
            method: Some(unary_signature),
            precedence: Prec::None,
            name: $n,
        }
    };
}
macro_rules! operator_rule {
    ($n:literal) => {
        GrammarRule {
            prefix: Some(unary_op),
            infix: Some(infix_op),
            method: Some(mixed_signature),
            precedence: Prec::Term,
            name: $n,
        }
    };
}

use TokenType as T;

static RULES: [GrammarRule; NUM_TOKEN_TYPES] = [
    /* LeftParen    */ prefix_rule!(grouping),
    /* RightParen   */ UNUSED,
    /* LeftBracket  */
    GrammarRule {
        prefix: Some(list),
        infix: Some(subscript),
        method: Some(subscript_signature),
        precedence: Prec::Call,
        name: "",
    },
    /* RightBracket */ UNUSED,
    /* LeftBrace    */ prefix_rule!(map),
    /* RightBrace   */ UNUSED,
    /* Colon        */ UNUSED,
    /* Dot          */ infix_rule!(Prec::Call, call),
    /* DotDot       */ infix_op_rule!(Prec::Range, ".."),
    /* DotDotDot    */ infix_op_rule!(Prec::Range, "..."),
    /* Comma        */ UNUSED,
    /* Star         */ infix_op_rule!(Prec::Factor, "*"),
    /* Slash        */ infix_op_rule!(Prec::Factor, "/"),
    /* Percent      */ infix_op_rule!(Prec::Factor, "%"),
    /* Hash         */ UNUSED,
    /* Plus         */ infix_op_rule!(Prec::Term, "+"),
    /* Minus        */ operator_rule!("-"),
    /* LtLt         */ infix_op_rule!(Prec::BitwiseShift, "<<"),
    /* GtGt         */ infix_op_rule!(Prec::BitwiseShift, ">>"),
    /* Pipe         */ infix_op_rule!(Prec::BitwiseOr, "|"),
    /* PipePipe     */ infix_rule!(Prec::LogicalOr, or_),
    /* Caret        */ infix_op_rule!(Prec::BitwiseXor, "^"),
    /* Amp          */ infix_op_rule!(Prec::BitwiseAnd, "&"),
    /* AmpAmp       */ infix_rule!(Prec::LogicalAnd, and_),
    /* Bang         */ prefix_op_rule!("!"),
    /* Tilde        */ prefix_op_rule!("~"),
    /* Question     */ infix_rule!(Prec::Assignment, conditional),
    /* Eq           */ UNUSED,
    /* Lt           */ infix_op_rule!(Prec::Comparison, "<"),
    /* Gt           */ infix_op_rule!(Prec::Comparison, ">"),
    /* LtEq         */ infix_op_rule!(Prec::Comparison, "<="),
    /* GtEq         */ infix_op_rule!(Prec::Comparison, ">="),
    /* EqEq         */ infix_op_rule!(Prec::Equality, "=="),
    /* BangEq       */ infix_op_rule!(Prec::Equality, "!="),
    /* Break        */ UNUSED,
    /* Continue     */ UNUSED,
    /* Class        */ UNUSED,
    /* Construct    */
    GrammarRule {
        prefix: None,
        infix: None,
        method: Some(constructor_signature),
        precedence: Prec::None,
        name: "",
    },
    /* Else         */ UNUSED,
    /* False        */ prefix_rule!(boolean),
    /* For          */ UNUSED,
    /* Foreign      */ UNUSED,
    /* If           */ UNUSED,
    /* Import       */ UNUSED,
    /* As           */ UNUSED,
    /* In           */ UNUSED,
    /* Is           */ infix_op_rule!(Prec::Is, "is"),
    /* Null         */ prefix_rule!(null),
    /* Return       */ UNUSED,
    /* Static       */ UNUSED,
    /* Super        */ prefix_rule!(super_),
    /* This         */ prefix_rule!(this_),
    /* True         */ prefix_rule!(boolean),
    /* Var          */ UNUSED,
    /* While        */ UNUSED,
    /* Field        */ prefix_rule!(field),
    /* StaticField  */ prefix_rule!(static_field),
    /* Name         */
    GrammarRule {
        prefix: Some(name),
        infix: None,
        method: Some(named_signature),
        precedence: Prec::None,
        name: "",
    },
    /* Number       */ prefix_rule!(literal),
    /* String       */ prefix_rule!(literal),
    /* Interpolation*/ prefix_rule!(string_interpolation),
    /* Line         */ UNUSED,
    /* Error        */ UNUSED,
    /* Eof          */ UNUSED,
];

static KEYWORDS: &[(&str, TokenType)] = &[
    ("break", T::Break),
    ("continue", T::Continue),
    ("class", T::Class),
    ("construct", T::Construct),
    ("else", T::Else),
    ("false", T::False),
    ("for", T::For),
    ("foreign", T::Foreign),
    ("if", T::If),
    ("import", T::Import),
    ("as", T::As),
    ("in", T::In),
    ("is", T::Is),
    ("null", T::Null),
    ("return", T::Return),
    ("static", T::Static),
    ("super", T::Super),
    ("this", T::This),
    ("true", T::True),
    ("var", T::Var),
    ("while", T::While),
];

fn is_name(c: u8) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_uppercase() || c == b'_'
}
fn is_digit(c: u8) -> bool {
    c.is_ascii_digit()
}

impl WrenVM {
    // -- error reporting -------------------------------------------------------

    fn print_error(&mut self, line: i32, label: &str, message: &str) {
        {
            let p = self.parser.as_mut().unwrap();
            p.has_error = true;
            if !p.print_errors {
                return;
            }
        }
        if self.config.error_fn.is_none() {
            return;
        }
        let msg = format!("{}: {}", label, message);
        let module = self.heap.module(self.parser.as_ref().unwrap().module).name;
        let module_name: Vec<u8> = if module != NULL_OBJ {
            self.heap.string(module).bytes.clone()
        } else {
            b"<unknown>".to_vec()
        };
        let cb = self.config.error_fn.unwrap();
        cb(
            self,
            crate::WrenErrorType::Compile,
            Some(&module_name),
            line,
            msg.as_bytes(),
        );
    }

    fn lex_error(&mut self, message: &str) {
        let line = self.parser.as_ref().unwrap().current_line;
        self.print_error(line, "Error", message);
    }

    fn error(&mut self, message: &str) {
        let tok = self.parser.as_ref().unwrap().previous;
        if tok.typ == T::Error {
            return;
        }
        if tok.typ == T::Line {
            self.print_error(tok.line, "Error at newline", message);
        } else if tok.typ == T::Eof {
            self.print_error(tok.line, "Error at end of file", message);
        } else {
            let label = if tok.length <= MAX_VARIABLE_NAME {
                format!("Error at '{}'", self.token_text_string(&tok))
            } else {
                let t = &self.token_text(&tok)[..MAX_VARIABLE_NAME];
                format!("Error at '{}...'", String::from_utf8_lossy(t))
            };
            self.print_error(tok.line, &label, message);
        }
    }

    fn token_text(&self, tok: &Token) -> Vec<u8> {
        let src = &self.parser.as_ref().unwrap().source;
        src[tok.start..tok.start + tok.length].to_vec()
    }

    fn token_text_string(&self, tok: &Token) -> String {
        String::from_utf8_lossy(&self.token_text(tok)).to_string()
    }

    fn name_bytes(&self, name: NameRef) -> &[u8] {
        match name {
            NameRef::Source(s, l) => &self.parser.as_ref().unwrap().source[s..s + l],
            NameRef::Static(b) => b,
        }
    }

    // -- lexer ----------------------------------------------------------------

    #[inline]
    fn peek_char(&self) -> u8 {
        let p = self.parser.as_ref().unwrap();
        if p.current_char < p.source.len() {
            p.source[p.current_char]
        } else {
            0
        }
    }

    #[inline]
    fn peek_next_char(&self) -> u8 {
        let p = self.parser.as_ref().unwrap();
        if self.peek_char() == 0 {
            return 0;
        }
        let i = p.current_char + 1;
        if i < p.source.len() {
            p.source[i]
        } else {
            0
        }
    }

    fn next_char(&mut self) -> u8 {
        let c = self.peek_char();
        let p = self.parser.as_mut().unwrap();
        p.current_char += 1;
        if c == b'\n' {
            p.current_line += 1;
        }
        c
    }

    fn match_char(&mut self, c: u8) -> bool {
        if self.peek_char() != c {
            return false;
        }
        self.next_char();
        true
    }

    fn make_token(&mut self, typ: TokenType) {
        let p = self.parser.as_mut().unwrap();
        p.next.typ = typ;
        p.next.start = p.token_start;
        p.next.length = p.current_char - p.token_start;
        p.next.line = p.current_line;
        if typ == T::Line {
            p.next.line -= 1;
        }
    }

    fn two_char_token(&mut self, c: u8, two: TokenType, one: TokenType) {
        let t = if self.match_char(c) { two } else { one };
        self.make_token(t);
    }

    fn skip_line_comment(&mut self) {
        while self.peek_char() != b'\n' && self.peek_char() != 0 {
            self.next_char();
        }
    }

    fn skip_block_comment(&mut self) {
        let mut nesting = 1;
        while nesting > 0 {
            if self.peek_char() == 0 {
                self.lex_error("Unterminated block comment.");
                return;
            }
            if self.peek_char() == b'/' && self.peek_next_char() == b'*' {
                self.next_char();
                self.next_char();
                nesting += 1;
                continue;
            }
            if self.peek_char() == b'*' && self.peek_next_char() == b'/' {
                self.next_char();
                self.next_char();
                nesting -= 1;
                continue;
            }
            self.next_char();
        }
    }

    fn read_hex_digit(&mut self) -> i32 {
        let c = self.next_char();
        if c.is_ascii_digit() {
            return (c - b'0') as i32;
        }
        if (b'a'..=b'f').contains(&c) {
            return (c - b'a' + 10) as i32;
        }
        if (b'A'..=b'F').contains(&c) {
            return (c - b'A' + 10) as i32;
        }
        self.parser.as_mut().unwrap().current_char -= 1;
        -1
    }

    fn make_number(&mut self, is_hex: bool) {
        let p = self.parser.as_ref().unwrap();
        let start = p.token_start;
        let end = p.current_char;
        let text = p.source[start..end].to_vec();

        let (value, erange) = if is_hex {
            parse_hex_literal(&text)
        } else {
            let (v, _, erange) = parse_double(&text);
            (v, erange)
        };

        if erange {
            self.lex_error(&format!(
                "Number literal was too large ({}).",
                std::mem::size_of::<i64>()
            ));
            self.parser.as_mut().unwrap().next.value = Value::num(0.0);
        } else {
            self.parser.as_mut().unwrap().next.value = Value::num(value);
        }

        self.make_token(T::Number);
    }

    fn read_hex_number(&mut self) {
        self.next_char(); // skip 'x'
        while self.read_hex_digit() != -1 {}
        self.make_number(true);
    }

    fn read_number(&mut self) {
        while is_digit(self.peek_char()) {
            self.next_char();
        }
        if self.peek_char() == b'.' && is_digit(self.peek_next_char()) {
            self.next_char();
            while is_digit(self.peek_char()) {
                self.next_char();
            }
        }
        if self.match_char(b'e') || self.match_char(b'E') {
            if !self.match_char(b'+') {
                self.match_char(b'-');
            }
            if !is_digit(self.peek_char()) {
                self.lex_error("Unterminated scientific notation.");
            }
            while is_digit(self.peek_char()) {
                self.next_char();
            }
        }
        self.make_number(false);
    }

    fn read_name(&mut self, mut typ: TokenType, first: u8) {
        let mut string = vec![first];
        while is_name(self.peek_char()) || is_digit(self.peek_char()) {
            let c = self.next_char();
            string.push(c);
        }
        self.track_alloc(string.len() + 1);

        let typ2 = {
            let p = self.parser.as_ref().unwrap();
            let tok_text = &p.source[p.token_start..p.current_char];
            let mut t = typ;
            for (kw, kt) in KEYWORDS {
                if tok_text == kw.as_bytes() {
                    t = *kt;
                    break;
                }
            }
            t
        };
        typ = typ2;

        let v = self.new_string_len(&string);
        self.parser.as_mut().unwrap().next.value = v;
        self.make_token(typ);
    }

    fn read_hex_escape(&mut self, digits: usize, description: &str) -> i32 {
        let mut value = 0;
        for _ in 0..digits {
            if self.peek_char() == b'"' || self.peek_char() == 0 {
                self.lex_error(&format!("Incomplete {} escape sequence.", description));
                self.parser.as_mut().unwrap().current_char -= 1;
                break;
            }
            let digit = self.read_hex_digit();
            if digit == -1 {
                self.lex_error(&format!("Invalid {} escape sequence.", description));
                break;
            }
            value = (value * 16) | digit;
        }
        value
    }

    fn read_unicode_escape(&mut self, string: &mut Vec<u8>, length: usize) {
        let value = self.read_hex_escape(length, "Unicode");
        let num_bytes = utf8_encode_num_bytes(value);
        if num_bytes != 0 {
            utf8_encode(value, string);
        }
    }

    fn read_raw_string(&mut self) {
        let mut string: Vec<u8> = Vec::new();
        let typ = T::Str;

        // consume the second and third "
        self.next_char();
        self.next_char();

        let mut skip_start: i64 = 0;
        let mut first_newline: i64 = -1;
        let mut skip_end: i64 = -1;
        let mut last_newline: i64 = -1;

        loop {
            let c = self.next_char();
            let c1 = self.peek_char();
            let c2 = self.peek_next_char();

            if c == b'\r' {
                continue;
            }

            if c == b'\n' {
                last_newline = string.len() as i64;
                skip_end = last_newline;
                first_newline = if first_newline == -1 {
                    string.len() as i64
                } else {
                    first_newline
                };
            }

            if c == b'"' && c1 == b'"' && c2 == b'"' {
                break;
            }

            let is_whitespace = c == b' ' || c == b'\t';
            skip_end = if c == b'\n' || is_whitespace {
                skip_end
            } else {
                -1
            };

            let skippable = skip_start != -1 && is_whitespace && first_newline == -1;
            skip_start = if skippable {
                string.len() as i64 + 1
            } else {
                skip_start
            };

            if first_newline == -1 && !is_whitespace && c != b'\n' {
                skip_start = -1;
            }

            if c == 0 || c1 == 0 || c2 == 0 {
                self.lex_error("Unterminated raw string.");
                self.parser.as_mut().unwrap().current_char -= 1;
                break;
            }

            string.push(c);
        }

        // consume the second and third "
        self.next_char();
        self.next_char();
        self.track_alloc(string.len() + 1);

        let mut offset = 0usize;
        let mut count = string.len();

        if first_newline != -1 && skip_start == first_newline {
            offset = first_newline as usize + 1;
        }
        if last_newline != -1 && skip_end == last_newline {
            count = last_newline as usize;
        }
        count -= if offset > count { count } else { offset };

        let slice = string[offset..offset + count].to_vec();
        let v = self.new_string_len(&slice);
        self.parser.as_mut().unwrap().next.value = v;
        self.make_token(typ);
    }

    fn read_string(&mut self) {
        let mut string: Vec<u8> = Vec::new();
        let mut typ = T::Str;

        loop {
            let c = self.next_char();
            if c == b'"' {
                break;
            }
            if c == b'\r' {
                continue;
            }
            if c == 0 {
                self.lex_error("Unterminated string.");
                self.parser.as_mut().unwrap().current_char -= 1;
                break;
            }

            if c == b'%' {
                if self.parser.as_ref().unwrap().num_parens < MAX_INTERPOLATION_NESTING {
                    if self.next_char() != b'(' {
                        self.lex_error("Expect '(' after '%'.");
                    }
                    let p = self.parser.as_mut().unwrap();
                    p.parens[p.num_parens] = 1;
                    p.num_parens += 1;
                    typ = T::Interpolation;
                    break;
                }
                self.lex_error(&format!(
                    "Interpolation may only nest {} levels deep.",
                    MAX_INTERPOLATION_NESTING
                ));
            }

            if c == b'\\' {
                match self.next_char() {
                    b'"' => string.push(b'"'),
                    b'\\' => string.push(b'\\'),
                    b'%' => string.push(b'%'),
                    b'0' => string.push(0),
                    b'a' => string.push(7),
                    b'b' => string.push(8),
                    b'e' => string.push(27),
                    b'f' => string.push(12),
                    b'n' => string.push(b'\n'),
                    b'r' => string.push(b'\r'),
                    b't' => string.push(b'\t'),
                    b'u' => self.read_unicode_escape(&mut string, 4),
                    b'U' => self.read_unicode_escape(&mut string, 8),
                    b'v' => string.push(11),
                    b'x' => {
                        let v = self.read_hex_escape(2, "byte");
                        string.push(v as u8);
                    }
                    other => {
                        let msg = format!("Invalid escape character '{}'.", other as char);
                        self.lex_error(&msg);
                    }
                }
            } else {
                string.push(c);
            }
        }

        self.track_alloc(string.len() + 1);
        let v = self.new_string_len(&string);
        self.parser.as_mut().unwrap().next.value = v;
        self.make_token(typ);
    }

    pub(crate) fn next_token(&mut self) {
        {
            let p = self.parser.as_mut().unwrap();
            p.previous = p.current;
            p.current = p.next;
            if p.next.typ == T::Eof || p.current.typ == T::Eof {
                return;
            }
        }

        while self.peek_char() != 0 {
            self.parser.as_mut().unwrap().token_start = self.parser.as_ref().unwrap().current_char;

            let c = self.next_char();
            match c {
                b'(' => {
                    {
                        let p = self.parser.as_mut().unwrap();
                        if p.num_parens > 0 {
                            p.parens[p.num_parens - 1] += 1;
                        }
                    }
                    self.make_token(T::LeftParen);
                    return;
                }
                b')' => {
                    let mut do_read_string = false;
                    {
                        let p = self.parser.as_mut().unwrap();
                        if p.num_parens > 0 {
                            p.parens[p.num_parens - 1] -= 1;
                            if p.parens[p.num_parens - 1] == 0 {
                                p.num_parens -= 1;
                                do_read_string = true;
                            }
                        }
                    }
                    if do_read_string {
                        self.read_string();
                        return;
                    }
                    self.make_token(T::RightParen);
                    return;
                }
                b'[' => {
                    self.make_token(T::LeftBracket);
                    return;
                }
                b']' => {
                    self.make_token(T::RightBracket);
                    return;
                }
                b'{' => {
                    self.make_token(T::LeftBrace);
                    return;
                }
                b'}' => {
                    self.make_token(T::RightBrace);
                    return;
                }
                b':' => {
                    self.make_token(T::Colon);
                    return;
                }
                b',' => {
                    self.make_token(T::Comma);
                    return;
                }
                b'*' => {
                    self.make_token(T::Star);
                    return;
                }
                b'%' => {
                    self.make_token(T::Percent);
                    return;
                }
                b'#' => {
                    if self.parser.as_ref().unwrap().current_line == 1
                        && self.peek_char() == b'!'
                        && self.peek_next_char() == b'/'
                    {
                        self.skip_line_comment();
                    } else {
                        self.make_token(T::Hash);
                        return;
                    }
                }
                b'^' => {
                    self.make_token(T::Caret);
                    return;
                }
                b'+' => {
                    self.make_token(T::Plus);
                    return;
                }
                b'-' => {
                    self.make_token(T::Minus);
                    return;
                }
                b'~' => {
                    self.make_token(T::Tilde);
                    return;
                }
                b'?' => {
                    self.make_token(T::Question);
                    return;
                }
                b'|' => {
                    self.two_char_token(b'|', T::PipePipe, T::Pipe);
                    return;
                }
                b'&' => {
                    self.two_char_token(b'&', T::AmpAmp, T::Amp);
                    return;
                }
                b'=' => {
                    self.two_char_token(b'=', T::EqEq, T::Eq);
                    return;
                }
                b'!' => {
                    self.two_char_token(b'=', T::BangEq, T::Bang);
                    return;
                }
                b'.' => {
                    if self.match_char(b'.') {
                        self.two_char_token(b'.', T::DotDotDot, T::DotDot);
                        return;
                    }
                    self.make_token(T::Dot);
                    return;
                }
                b'/' => {
                    if self.match_char(b'/') {
                        self.skip_line_comment();
                    } else if self.match_char(b'*') {
                        self.skip_block_comment();
                    } else {
                        self.make_token(T::Slash);
                        return;
                    }
                }
                b'<' => {
                    if self.match_char(b'<') {
                        self.make_token(T::LtLt);
                    } else {
                        self.two_char_token(b'=', T::LtEq, T::Lt);
                    }
                    return;
                }
                b'>' => {
                    if self.match_char(b'>') {
                        self.make_token(T::GtGt);
                    } else {
                        self.two_char_token(b'=', T::GtEq, T::Gt);
                    }
                    return;
                }
                b'\n' => {
                    self.make_token(T::Line);
                    return;
                }
                b' ' | b'\r' | b'\t' => {
                    while matches!(self.peek_char(), b' ' | b'\r' | b'\t') {
                        self.next_char();
                    }
                }
                b'"' => {
                    if self.peek_char() == b'"' && self.peek_next_char() == b'"' {
                        self.read_raw_string();
                        return;
                    }
                    self.read_string();
                    return;
                }
                b'_' => {
                    let t = if self.peek_char() == b'_' {
                        T::StaticField
                    } else {
                        T::Field
                    };
                    self.read_name(t, c);
                    return;
                }
                b'0' => {
                    if self.peek_char() == b'x' {
                        self.read_hex_number();
                        return;
                    }
                    self.read_number();
                    return;
                }
                _ => {
                    if is_name(c) {
                        self.read_name(T::Name, c);
                    } else if is_digit(c) {
                        self.read_number();
                    } else {
                        if (32..=126).contains(&c) {
                            let msg = format!("Invalid character '{}'.", c as char);
                            self.lex_error(&msg);
                        } else {
                            let msg = format!("Invalid byte 0x{:x}.", c);
                            self.lex_error(&msg);
                        }
                        let p = self.parser.as_mut().unwrap();
                        p.next.typ = T::Error;
                        p.next.length = 0;
                    }
                    return;
                }
            }
        }

        {
            let p = self.parser.as_mut().unwrap();
            p.token_start = p.current_char;
        }
        self.make_token(T::Eof);
    }

    // -- parsing helpers ---------------------------------------------------------

    #[inline]
    fn peek(&self) -> TokenType {
        self.parser.as_ref().unwrap().current.typ
    }
    #[inline]
    fn peek_next(&self) -> TokenType {
        self.parser.as_ref().unwrap().next.typ
    }

    fn match_(&mut self, expected: TokenType) -> bool {
        if self.peek() != expected {
            return false;
        }
        self.next_token();
        true
    }

    fn consume(&mut self, expected: TokenType, message: &str) {
        self.next_token();
        let p = self.parser.as_ref().unwrap();
        if p.previous.typ != expected {
            let msg = message.to_string();
            self.error(&msg);
            if self.parser.as_ref().unwrap().current.typ == expected {
                self.next_token();
            }
        }
    }

    fn match_line(&mut self) -> bool {
        if !self.match_(T::Line) {
            return false;
        }
        while self.match_(T::Line) {}
        true
    }

    fn ignore_newlines(&mut self) {
        self.match_line();
    }

    fn consume_line(&mut self, message: &str) {
        self.consume(T::Line, message);
        self.ignore_newlines();
    }

    fn allow_line_before_dot(&mut self) {
        if self.peek() == T::Line && self.peek_next() == T::Dot {
            self.next_token();
        }
    }

    // -- emit helpers -------------------------------------------------------------

    /// Index of the current (top) compiler.
    #[inline]
    fn cur(&self) -> usize {
        self.compilers.len() - 1
    }

    fn emit_byte(&mut self, ci: usize, byte: i32) -> usize {
        let fn_ref = self.compilers[ci].fn_ref;
        let line = self.parser.as_ref().unwrap().previous.line;
        let f = self.heap.func_mut(fn_ref);
        f.code.push(byte as u8);
        if let Some(d) = f.debug.as_mut() {
            d.source_lines.push(line);
        }
        self.track_alloc(9); // byte + source line
        self.heap.func(fn_ref).code.len() - 1
    }

    fn emit_op(&mut self, ci: usize, op: u8) {
        self.emit_byte(ci, op as i32);
        let effect = STACK_EFFECTS[op as usize];
        let fn_ref = self.compilers[ci].fn_ref;
        let c = &mut self.compilers[ci];
        c.num_slots += effect;
        let max = self.heap.func(fn_ref).max_slots;
        if c.num_slots > max {
            self.heap.func_mut(fn_ref).max_slots = c.num_slots;
        }
    }

    fn emit_short(&mut self, ci: usize, arg: i32) {
        self.emit_byte(ci, (arg >> 8) & 0xff);
        self.emit_byte(ci, arg & 0xff);
    }

    fn emit_byte_arg(&mut self, ci: usize, op: u8, arg: i32) -> usize {
        self.emit_op(ci, op);
        self.emit_byte(ci, arg)
    }

    fn emit_short_arg(&mut self, ci: usize, op: u8, arg: i32) {
        self.emit_op(ci, op);
        self.emit_short(ci, arg);
    }

    fn emit_jump(&mut self, ci: usize, op: u8) -> usize {
        self.emit_op(ci, op);
        self.emit_byte(ci, 0xff);
        self.emit_byte(ci, 0xff) - 1
    }

    fn emit_constant(&mut self, ci: usize, value: Value) {
        let constant = self.add_constant(ci, value);
        self.emit_short_arg(ci, CONSTANT, constant);
    }

    pub(crate) fn add_constant(&mut self, ci: usize, constant: Value) -> i32 {
        if self.parser.as_ref().unwrap().has_error {
            return -1;
        }
        let map = self.compilers[ci].constants;
        if map != NULL_OBJ {
            let existing = self.map_get(map, constant);
            if existing.is_num() {
                return existing.as_num() as i32;
            }
        }

        let fn_ref = self.compilers[ci].fn_ref;
        let count = self.heap.func(fn_ref).constants.len();
        if count < MAX_CONSTANTS {
            if constant.is_obj() {
                self.push_root(constant.as_obj_unchecked());
            }
            self.heap.func_mut(fn_ref).constants.push(constant);
            self.track_alloc(8);
            if constant.is_obj() {
                self.pop_root();
            }
            if self.compilers[ci].constants == NULL_OBJ {
                let m = self.new_map();
                self.compilers[ci].constants = m;
            }
            let m = self.compilers[ci].constants;
            let idx = self.heap.func(fn_ref).constants.len() as i32 - 1;
            self.map_set(m, constant, Value::num(idx as f64));
        } else {
            self.error(&format!(
                "A function may only contain {} unique constants.",
                MAX_CONSTANTS
            ));
        }
        self.heap.func(fn_ref).constants.len() as i32 - 1
    }

    // -- variables & scopes -------------------------------------------------------

    fn add_local(&mut self, ci: usize, name: NameRef) -> usize {
        let depth = self.compilers[ci].scope_depth;
        self.compilers[ci].locals.push(Local {
            name,
            depth,
            is_upvalue: false,
        });
        self.compilers[ci].num_locals() - 1
    }

    fn declare_variable(&mut self, ci: usize, token: Option<Token>) -> i32 {
        let token = token.unwrap_or(self.parser.as_ref().unwrap().previous);

        if token.length > MAX_VARIABLE_NAME {
            self.error(&format!(
                "Variable name cannot be longer than {} characters.",
                MAX_VARIABLE_NAME
            ));
        }

        let scope_depth = self.compilers[ci].scope_depth;
        if scope_depth == -1 {
            let module = self.parser.as_ref().unwrap().module;
            let name = self.token_text(&token);
            let mut line = -1i32;
            let symbol = self.define_variable(module, &name, NULL_VAL, &mut line);
            match symbol {
                -1 => self.error("Module variable is already defined."),
                -2 => self.error("Too many module variables defined."),
                -3 => {
                    let name = String::from_utf8_lossy(&name).to_string();
                    self.error(&format!(
                        "Variable '{}' referenced before this definition (first use at line {}).",
                        name, line
                    ));
                }
                _ => {}
            }
            return symbol;
        }

        // Check for a duplicate in the current scope.
        let tok_bytes = self.token_text(&token);
        let c = &self.compilers[ci];
        for i in (0..c.num_locals()).rev() {
            let local = c.locals[i];
            if local.depth < scope_depth {
                break;
            }
            if self.name_bytes(local.name) == tok_bytes.as_slice() {
                let i2 = i as i32;
                self.error("Variable is already declared in this scope.");
                return i2;
            }
        }

        if self.compilers[ci].num_locals() == MAX_LOCALS {
            self.error(&format!(
                "Cannot declare more than {} variables in one scope.",
                MAX_LOCALS
            ));
            return -1;
        }

        self.add_local(ci, NameRef::Source(token.start, token.length)) as i32
    }

    fn declare_named_variable(&mut self, ci: usize) -> i32 {
        self.consume(T::Name, "Expect variable name.");
        self.declare_variable(ci, None)
    }

    fn define_variable_code(&mut self, ci: usize, symbol: i32) {
        if self.compilers[ci].scope_depth >= 0 {
            return;
        }
        self.emit_short_arg(ci, STORE_MODULE_VAR, symbol);
        self.emit_op(ci, POP);
    }

    fn push_scope(&mut self, ci: usize) {
        self.compilers[ci].scope_depth += 1;
    }

    fn discard_locals(&mut self, ci: usize, depth: i32) -> i32 {
        debug_assert!(self.compilers[ci].scope_depth > -1);
        let mut local = self.compilers[ci].num_locals() as i32 - 1;
        while local >= 0 && self.compilers[ci].locals[local as usize].depth >= depth {
            let is_up = self.compilers[ci].locals[local as usize].is_upvalue;
            if is_up {
                self.emit_byte(ci, CLOSE_UPVALUE as i32);
            } else {
                self.emit_byte(ci, POP as i32);
            }
            local -= 1;
        }
        self.compilers[ci].num_locals() as i32 - local - 1
    }

    fn pop_scope(&mut self, ci: usize) {
        let depth = self.compilers[ci].scope_depth;
        let popped = self.discard_locals(ci, depth);
        let c = &mut self.compilers[ci];
        c.locals.truncate(c.locals.len() - popped as usize);
        c.num_slots -= popped;
        c.scope_depth -= 1;
    }

    fn resolve_local(&self, ci: usize, name: &[u8]) -> i32 {
        let c = &self.compilers[ci];
        for i in (0..c.num_locals()).rev() {
            if self.name_bytes(c.locals[i].name) == name {
                return i as i32;
            }
        }
        -1
    }

    fn add_upvalue(&mut self, ci: usize, is_local: bool, index: i32) -> i32 {
        {
            let c = &self.compilers[ci];
            for (i, up) in c.upvalues.iter().enumerate() {
                if up.index == index && up.is_local == is_local {
                    return i as i32;
                }
            }
        }
        self.compilers[ci]
            .upvalues
            .push(CompilerUpvalue { is_local, index });
        let fn_ref = self.compilers[ci].fn_ref;
        let n = self.compilers[ci].upvalues.len();
        self.heap.func_mut(fn_ref).num_upvalues = n as i32;
        (n - 1) as i32
    }

    fn find_upvalue(&mut self, ci: usize, name: &[u8]) -> i32 {
        if ci == 0 {
            return -1;
        }
        let parent = ci - 1;
        // Method boundary: parent has an enclosingClass and name isn't _static.
        if name.first() != Some(&b'_') && self.compilers[parent].enclosing_class.is_some() {
            return -1;
        }
        let local = self.resolve_local(parent, name);
        if local != -1 {
            self.compilers[parent].locals[local as usize].is_upvalue = true;
            return self.add_upvalue(ci, true, local);
        }
        let upvalue = self.find_upvalue(parent, name);
        if upvalue != -1 {
            return self.add_upvalue(ci, false, upvalue);
        }
        -1
    }

    fn resolve_nonmodule(&mut self, ci: usize, name: &[u8]) -> Variable {
        let idx = self.resolve_local(ci, name);
        if idx != -1 {
            return Variable {
                index: idx,
                scope: Scope::Local,
            };
        }
        let idx = self.find_upvalue(ci, name);
        Variable {
            index: idx,
            scope: Scope::Upvalue,
        }
    }

    fn resolve_name(&mut self, ci: usize, name: &[u8]) -> Variable {
        let mut v = self.resolve_nonmodule(ci, name);
        if v.index != -1 {
            return v;
        }
        v.scope = Scope::Module;
        let module = self.parser.as_ref().unwrap().module;
        v.index = self.module_find_variable(module, name);
        v
    }

    fn load_local(&mut self, ci: usize, slot: i32) {
        if slot <= 8 {
            self.emit_op(ci, LOAD_LOCAL_0 + slot as u8);
            return;
        }
        self.emit_byte_arg(ci, LOAD_LOCAL, slot);
    }

    fn end_compiler(&mut self, ci: usize, debug_name: &[u8]) -> ObjRef {
        if self.parser.as_ref().unwrap().has_error {
            self.compilers.pop();
            return NULL_OBJ;
        }

        self.emit_op(ci, END);

        let fn_ref = self.compilers[ci].fn_ref;
        let name = String::from_utf8_lossy(debug_name).to_string();
        if let Some(d) = self.heap.func_mut(fn_ref).debug.as_mut() {
            d.name = name;
        }
        self.track_alloc(debug_name.len() + 1);

        if ci > 0 {
            let parent = ci - 1;
            let constant = self.add_constant(parent, Value::obj(fn_ref));
            self.emit_short_arg(parent, CLOSURE, constant);
            let n = self.compilers[ci].upvalues.len();
            for i in 0..n {
                let up = self.compilers[ci].upvalues[i];
                self.emit_byte(parent, if up.is_local { 1 } else { 0 });
                self.emit_byte(parent, up.index);
            }
        }

        self.compilers.pop();
        fn_ref
    }

    // -- grammar ----------------------------------------------------------------

    fn patch_jump(&mut self, ci: usize, offset: usize) {
        let fn_ref = self.compilers[ci].fn_ref;
        let jump = self.heap.func(fn_ref).code.len() - offset - 2;
        if jump > MAX_JUMP {
            self.error("Too much code to jump over.");
        }
        let f = self.heap.func_mut(fn_ref);
        f.code[offset] = ((jump >> 8) & 0xff) as u8;
        f.code[offset + 1] = (jump & 0xff) as u8;
    }

    fn finish_block(&mut self, ci: usize) -> bool {
        if self.match_(T::RightBrace) {
            return false;
        }
        if !self.match_line() {
            self.expression(ci);
            self.consume(T::RightBrace, "Expect '}' at end of block.");
            return true;
        }
        if self.match_(T::RightBrace) {
            return false;
        }
        loop {
            self.definition(ci);
            self.consume_line("Expect newline after statement.");
            if self.peek() == T::RightBrace || self.peek() == T::Eof {
                break;
            }
        }
        self.consume(T::RightBrace, "Expect '}' at end of block.");
        false
    }

    fn finish_body(&mut self, ci: usize) {
        let is_expression_body = self.finish_block(ci);
        if self.compilers[ci].is_initializer {
            if is_expression_body {
                self.emit_op(ci, POP);
            }
            self.emit_op(ci, LOAD_LOCAL_0);
        } else if !is_expression_body {
            self.emit_op(ci, NULL);
        }
        self.emit_op(ci, RETURN);
    }

    fn validate_num_parameters(&mut self, num_args: i32) {
        if num_args == MAX_PARAMETERS + 1 {
            self.error(&format!(
                "Methods cannot have more than {} parameters.",
                MAX_PARAMETERS
            ));
        }
    }

    fn finish_parameter_list(&mut self, ci: usize, signature: &mut Signature) {
        loop {
            self.ignore_newlines();
            signature.arity += 1;
            let a = signature.arity;
            self.validate_num_parameters(a);
            self.declare_named_variable(ci);
            if !self.match_(T::Comma) {
                break;
            }
        }
    }

    fn method_symbol(&mut self, name: &[u8]) -> i32 {
        let symbol = self.ensure_method_symbol(name);
        if symbol > MAX_METHODS {
            self.error(&format!("Method limit of {} reached.", MAX_METHODS));
        }
        symbol
    }

    fn signature_symbol(&mut self, signature: &Signature) -> i32 {
        let name = self.signature_to_string(signature);
        self.method_symbol(&name)
    }

    pub(crate) fn signature_to_string(&self, signature: &Signature) -> Vec<u8> {
        let mut name = Vec::with_capacity(MAX_METHOD_SIGNATURE);
        let name_bytes = self.name_bytes(signature.name);
        match signature.typ {
            SigType::Initializer => {
                name.extend_from_slice(b"init ");
                name.extend_from_slice(name_bytes);
                signature_parameter_list(&mut name, signature.arity, b'(', b')');
            }
            _ => {
                name.extend_from_slice(name_bytes);
                match signature.typ {
                    SigType::Method => {
                        signature_parameter_list(&mut name, signature.arity, b'(', b')');
                    }
                    SigType::Getter => {}
                    SigType::Setter => {
                        name.push(b'=');
                        signature_parameter_list(&mut name, 1, b'(', b')');
                    }
                    SigType::Subscript => {
                        signature_parameter_list(&mut name, signature.arity, b'[', b']');
                    }
                    SigType::SubscriptSetter => {
                        signature_parameter_list(&mut name, signature.arity - 1, b'[', b']');
                        name.push(b'=');
                        signature_parameter_list(&mut name, 1, b'(', b')');
                    }
                    SigType::Initializer => unreachable!(),
                }
            }
        }
        name
    }

    fn signature_from_token(&mut self, typ: SigType) -> Signature {
        let tok = self.parser.as_ref().unwrap().previous;
        let mut sig = Signature {
            name: NameRef::Source(tok.start, tok.length),
            typ,
            arity: 0,
        };
        if tok.length > MAX_METHOD_NAME {
            self.error(&format!(
                "Method names cannot be longer than {} characters.",
                MAX_METHOD_NAME
            ));
            sig.name = NameRef::Source(tok.start, MAX_METHOD_NAME);
        }
        sig
    }

    fn finish_argument_list(&mut self, ci: usize, signature: &mut Signature) {
        loop {
            self.ignore_newlines();
            signature.arity += 1;
            let a = signature.arity;
            self.validate_num_parameters(a);
            self.expression(ci);
            if !self.match_(T::Comma) {
                break;
            }
        }
        self.ignore_newlines();
    }

    fn call_signature(&mut self, ci: usize, instruction: u8, signature: &Signature) {
        let symbol = self.signature_symbol(signature);
        self.emit_short_arg(ci, instruction + signature.arity as u8, symbol);
        if instruction == SUPER_0 {
            let c = self.add_constant(ci, NULL_VAL);
            self.emit_short(ci, c);
        }
    }

    fn call_method(&mut self, ci: usize, num_args: i32, name: &[u8]) {
        let symbol = self.method_symbol(name);
        self.emit_short_arg(ci, CALL_0 + num_args as u8, symbol);
    }

    fn method_call(&mut self, ci: usize, instruction: u8, signature: &Signature) {
        let mut called = Signature {
            name: signature.name,
            typ: SigType::Getter,
            arity: 0,
        };

        if self.match_(T::LeftParen) {
            called.typ = SigType::Method;
            self.ignore_newlines();
            if self.peek() != T::RightParen {
                self.finish_argument_list(ci, &mut called);
            }
            self.consume(T::RightParen, "Expect ')' after arguments.");
        }

        if self.match_(T::LeftBrace) {
            called.typ = SigType::Method;
            called.arity += 1;

            self.init_compiler(false);
            let fn_ci = self.cur();

            let mut fn_signature = Signature {
                name: NameRef::Static(b""),
                typ: SigType::Method,
                arity: 0,
            };
            if self.match_(T::Pipe) {
                self.finish_parameter_list(fn_ci, &mut fn_signature);
                self.consume(T::Pipe, "Expect '|' after function parameters.");
            }
            let fn_ref = self.compilers[fn_ci].fn_ref;
            self.heap.func_mut(fn_ref).arity = fn_signature.arity;

            self.finish_body(fn_ci);

            let mut block_name = self.signature_to_string(&called);
            block_name.extend_from_slice(b" block argument");
            self.end_compiler(fn_ci, &block_name);
        }

        if signature.typ == SigType::Initializer {
            if called.typ != SigType::Method {
                self.error("A superclass constructor must have an argument list.");
            }
            called.typ = SigType::Initializer;
        }

        self.call_signature(ci, instruction, &called);
    }

    fn named_call(&mut self, ci: usize, can_assign: bool, instruction: u8) {
        let mut signature = self.signature_from_token(SigType::Getter);
        if can_assign && self.match_(T::Eq) {
            self.ignore_newlines();
            signature.typ = SigType::Setter;
            signature.arity = 1;
            self.expression(ci);
            self.call_signature(ci, instruction, &signature);
        } else {
            self.method_call(ci, instruction, &signature);
            self.allow_line_before_dot();
        }
    }

    fn load_variable(&mut self, ci: usize, variable: Variable) {
        match variable.scope {
            Scope::Local => self.load_local(ci, variable.index),
            Scope::Upvalue => {
                self.emit_byte_arg(ci, LOAD_UPVALUE, variable.index);
            }
            Scope::Module => {
                self.emit_short_arg(ci, LOAD_MODULE_VAR, variable.index);
            }
        }
    }

    fn load_this(&mut self, ci: usize) {
        let v = self.resolve_nonmodule(ci, b"this");
        self.load_variable(ci, v);
    }

    fn load_core_variable(&mut self, ci: usize, name: &str) {
        let module = self.parser.as_ref().unwrap().module;
        let symbol = self.module_find_variable(module, name.as_bytes());
        debug_assert!(symbol != -1, "Should have already defined core name.");
        self.emit_short_arg(ci, LOAD_MODULE_VAR, symbol);
    }

    // -- expressions -------------------------------------------------------------

    fn expression(&mut self, ci: usize) {
        self.parse_precedence(ci, Prec::Lowest);
    }

    fn parse_precedence(&mut self, ci: usize, precedence: Prec) {
        self.next_token();
        let prev_typ = self.parser.as_ref().unwrap().previous.typ;
        let prefix = RULES[prev_typ as usize].prefix;
        match prefix {
            None => {
                self.error("Expected expression.");
                return;
            }
            Some(f) => {
                let can_assign = precedence <= Prec::Conditional;
                f(self, ci, can_assign);
            }
        }
        let can_assign = precedence <= Prec::Conditional;
        while precedence <= RULES[self.parser.as_ref().unwrap().current.typ as usize].precedence {
            self.next_token();
            let prev = self.parser.as_ref().unwrap().previous.typ;
            if let Some(infix) = RULES[prev as usize].infix {
                infix(self, ci, can_assign);
            }
        }
    }

    // -- statements -----------------------------------------------------------------

    fn start_loop(&mut self, ci: usize) {
        let fn_ref = self.compilers[ci].fn_ref;
        let start = self.heap.func(fn_ref).code.len() as i32 - 1;
        let scope_depth = self.compilers[ci].scope_depth;
        self.compilers[ci].loops.push(Loop {
            start,
            exit_jump: 0,
            body: 0,
            scope_depth,
        });
    }

    fn test_exit_loop(&mut self, ci: usize) {
        let j = self.emit_jump(ci, JUMP_IF);
        self.compilers[ci].loops.last_mut().unwrap().exit_jump = j as i32;
    }

    fn loop_body(&mut self, ci: usize) {
        let fn_ref = self.compilers[ci].fn_ref;
        let body = self.heap.func(fn_ref).code.len() as i32;
        self.compilers[ci].loops.last_mut().unwrap().body = body;
        self.statement(ci);
    }

    fn end_loop(&mut self, ci: usize) {
        let (start, exit_jump, body) = {
            let l = self.compilers[ci].loops.last().unwrap();
            (l.start, l.exit_jump, l.body)
        };
        let fn_ref = self.compilers[ci].fn_ref;
        let loop_offset = self.heap.func(fn_ref).code.len() as i32 - start + 2;
        self.emit_short_arg(ci, LOOP, loop_offset);
        self.patch_jump(ci, exit_jump as usize);

        let mut i = body as usize;
        loop {
            let code_len = self.heap.func(fn_ref).code.len();
            if i >= code_len {
                break;
            }
            if self.heap.func(fn_ref).code[i] == END {
                self.heap.func_mut(fn_ref).code[i] = JUMP;
                self.patch_jump(ci, i + 1);
                i += 3;
            } else {
                i += 1 + self.get_byte_count_for_arguments(fn_ref, i);
            }
        }

        self.compilers[ci].loops.pop();
    }

    fn for_statement(&mut self, ci: usize) {
        self.push_scope(ci);
        self.consume(T::LeftParen, "Expect '(' after 'for'.");
        self.consume(T::Name, "Expect for loop variable name.");
        let name_tok = self.parser.as_ref().unwrap().previous;
        self.consume(T::In, "Expect 'in' after loop variable.");
        self.ignore_newlines();

        self.expression(ci);

        if self.compilers[ci].num_locals() + 2 > MAX_LOCALS {
            self.error(&format!(
                "Cannot declare more than {} variables in one scope. (Not enough space for for-loops internal variables)",
                MAX_LOCALS
            ));
            return;
        }
        let seq_slot = self.add_local(ci, NameRef::Static(b"seq ")) as i32;

        self.emit_op(ci, NULL);
        let iter_slot = self.add_local(ci, NameRef::Static(b"iter ")) as i32;

        self.consume(T::RightParen, "Expect ')' after loop expression.");
        self.start_loop(ci);

        self.load_local(ci, seq_slot);
        self.load_local(ci, iter_slot);
        self.call_method(ci, 1, b"iterate(_)");
        self.emit_byte_arg(ci, STORE_LOCAL, iter_slot);
        self.test_exit_loop(ci);

        self.load_local(ci, seq_slot);
        self.load_local(ci, iter_slot);
        self.call_method(ci, 1, b"iteratorValue(_)");

        self.push_scope(ci);
        self.add_local(ci, NameRef::Source(name_tok.start, name_tok.length));

        self.loop_body(ci);
        self.pop_scope(ci);
        self.end_loop(ci);
        self.pop_scope(ci);
    }

    fn if_statement(&mut self, ci: usize) {
        self.consume(T::LeftParen, "Expect '(' after 'if'.");
        self.expression(ci);
        self.consume(T::RightParen, "Expect ')' after if condition.");
        let if_jump = self.emit_jump(ci, JUMP_IF);
        self.statement(ci);
        if self.match_(T::Else) {
            let else_jump = self.emit_jump(ci, JUMP);
            self.patch_jump(ci, if_jump);
            self.statement(ci);
            self.patch_jump(ci, else_jump);
        } else {
            self.patch_jump(ci, if_jump);
        }
    }

    fn while_statement(&mut self, ci: usize) {
        self.start_loop(ci);
        self.consume(T::LeftParen, "Expect '(' after 'while'.");
        self.expression(ci);
        self.consume(T::RightParen, "Expect ')' after while condition.");
        self.test_exit_loop(ci);
        self.loop_body(ci);
        self.end_loop(ci);
    }

    fn statement(&mut self, ci: usize) {
        if self.match_(T::Break) {
            if self.compilers[ci].loops.is_empty() {
                self.error("Cannot use 'break' outside of a loop.");
                return;
            }
            let depth = self.compilers[ci].loops.last().unwrap().scope_depth + 1;
            self.discard_locals(ci, depth);
            self.emit_jump(ci, END);
        } else if self.match_(T::Continue) {
            if self.compilers[ci].loops.is_empty() {
                self.error("Cannot use 'continue' outside of a loop.");
                return;
            }
            let depth = self.compilers[ci].loops.last().unwrap().scope_depth + 1;
            self.discard_locals(ci, depth);
            let fn_ref = self.compilers[ci].fn_ref;
            let start = self.compilers[ci].loops.last().unwrap().start;
            let loop_offset = self.heap.func(fn_ref).code.len() as i32 - start + 2;
            self.emit_short_arg(ci, LOOP, loop_offset);
        } else if self.match_(T::For) {
            self.for_statement(ci);
        } else if self.match_(T::If) {
            self.if_statement(ci);
        } else if self.match_(T::Return) {
            if self.peek() == T::Line {
                let result = if self.compilers[ci].is_initializer {
                    LOAD_LOCAL_0
                } else {
                    NULL
                };
                self.emit_op(ci, result);
            } else {
                if self.compilers[ci].is_initializer {
                    self.error("A constructor cannot return a value.");
                }
                self.expression(ci);
            }
            self.emit_op(ci, RETURN);
        } else if self.match_(T::While) {
            self.while_statement(ci);
        } else if self.match_(T::LeftBrace) {
            self.push_scope(ci);
            if self.finish_block(ci) {
                self.emit_op(ci, POP);
            }
            self.pop_scope(ci);
        } else {
            self.expression(ci);
            self.emit_op(ci, POP);
        }
    }

    // -- classes & methods -----------------------------------------------------------

    fn create_constructor(&mut self, ci: usize, signature: &Signature, initializer_symbol: i32) {
        self.init_compiler(true);
        let mci = self.cur();

        let is_foreign = self.compilers[ci]
            .enclosing_class
            .as_ref()
            .map(|c| c.is_foreign)
            .unwrap_or(false);
        self.emit_op(
            mci,
            if is_foreign {
                FOREIGN_CONSTRUCT
            } else {
                CONSTRUCT
            },
        );
        self.emit_short_arg(mci, CALL_0 + signature.arity as u8, initializer_symbol);
        self.emit_op(mci, RETURN);
        self.end_compiler(mci, b"");
    }

    fn define_method(
        &mut self,
        ci: usize,
        class_variable: Variable,
        is_static: bool,
        method_symbol: i32,
    ) {
        self.load_variable(ci, class_variable);
        let op = if is_static {
            METHOD_STATIC
        } else {
            METHOD_INSTANCE
        };
        self.emit_short_arg(ci, op, method_symbol);
    }

    fn declare_method(&mut self, ci: usize, signature: &Signature, name: &[u8]) -> i32 {
        let symbol = self.signature_symbol(signature);

        let (already, in_static) = {
            let info = self.compilers[ci].enclosing_class.as_mut().unwrap();
            let methods = if info.in_static {
                &mut info.static_methods
            } else {
                &mut info.methods
            };
            (methods.contains(&symbol), info.in_static)
        };
        if already {
            let static_prefix = if in_static { "static " } else { "" };
            let class_name_ref = self.compilers[ci].enclosing_class.as_ref().unwrap().name;
            let class_name = self.heap.string(class_name_ref).bytes.clone();
            self.error(&format!(
                "Class {} already defines a {}method '{}'.",
                String::from_utf8_lossy(&class_name),
                static_prefix,
                String::from_utf8_lossy(name)
            ));
        }

        let info = self.compilers[ci].enclosing_class.as_mut().unwrap();
        let methods = if info.in_static {
            &mut info.static_methods
        } else {
            &mut info.methods
        };
        methods.push(symbol);
        symbol
    }

    fn consume_literal(&mut self, message: &str) -> Value {
        if self.match_(T::False) {
            return FALSE_VAL;
        }
        if self.match_(T::True) {
            return TRUE_VAL;
        }
        if self.match_(T::Number) {
            return self.parser.as_ref().unwrap().previous.value;
        }
        if self.match_(T::Str) {
            return self.parser.as_ref().unwrap().previous.value;
        }
        if self.match_(T::Name) {
            return self.parser.as_ref().unwrap().previous.value;
        }
        self.error(message);
        self.next_token();
        NULL_VAL
    }

    fn match_attribute(&mut self, ci: usize) -> bool {
        if !self.match_(T::Hash) {
            return false;
        }
        self.compilers[ci].num_attributes += 1;
        let runtime_access = self.match_(T::Bang);
        if self.match_(T::Name) {
            let group = self.parser.as_ref().unwrap().previous.value;
            let ahead = self.peek();
            if ahead == T::Eq || ahead == T::Line {
                let key = group;
                let mut value = NULL_VAL;
                if self.match_(T::Eq) {
                    value = self.consume_literal(
                        "Expect a Bool, Num, String or Identifier literal for an attribute value.",
                    );
                }
                if runtime_access {
                    self.add_to_attribute_group(ci, NULL_VAL, key, value);
                }
            } else if self.match_(T::LeftParen) {
                self.ignore_newlines();
                if self.match_(T::RightParen) {
                    self.error("Expected attributes in group, group cannot be empty.");
                } else {
                    while self.peek() != T::RightParen {
                        self.consume(T::Name, "Expect name for attribute key.");
                        let key = self.parser.as_ref().unwrap().previous.value;
                        let mut value = NULL_VAL;
                        if self.match_(T::Eq) {
                            value = self.consume_literal(
                                "Expect a Bool, Num, String or Identifier literal for an attribute value.",
                            );
                        }
                        if runtime_access {
                            self.add_to_attribute_group(ci, group, key, value);
                        }
                        self.ignore_newlines();
                        if !self.match_(T::Comma) {
                            break;
                        }
                        self.ignore_newlines();
                    }
                    self.ignore_newlines();
                    self.consume(T::RightParen, "Expected ')' after grouped attributes.");
                }
            } else {
                self.error("Expect an equal, newline or grouping after an attribute key.");
            }
        } else {
            self.error("Expect an attribute definition after #.");
        }

        self.consume_line("Expect newline after attribute.");
        true
    }

    fn method(&mut self, ci: usize, class_variable: Variable) -> bool {
        if self.match_attribute(ci) {
            return self.method(ci, class_variable);
        }

        let is_foreign = self.match_(T::Foreign);
        let is_static = self.match_(T::Static);
        if let Some(info) = self.compilers[ci].enclosing_class.as_mut() {
            info.in_static = is_static;
        }

        let signature_fn = RULES[self.parser.as_ref().unwrap().current.typ as usize].method;
        self.next_token();

        let signature_fn = match signature_fn {
            None => {
                self.error("Expect method definition.");
                return false;
            }
            Some(f) => f,
        };

        let mut signature = self.signature_from_token(SigType::Getter);
        if let Some(info) = self.compilers[ci].enclosing_class.as_mut() {
            info.signature = Some(signature);
        }

        self.init_compiler(true);
        let mci = self.cur();

        signature_fn(self, mci, &mut signature);
        if let Some(info) = self.compilers[ci].enclosing_class.as_mut() {
            info.signature = Some(signature);
        }

        let is_init = signature.typ == SigType::Initializer;
        self.compilers[mci].is_initializer = is_init;

        if is_static && is_init {
            self.error("A constructor cannot be static.");
        }

        let full_signature = self.signature_to_string(&signature);

        self.copy_method_attributes(ci, is_foreign, is_static, &full_signature);

        let method_symbol = self.declare_method(ci, &signature, &full_signature);

        if is_foreign {
            let sig_str = self.new_string_len(&full_signature);
            self.emit_constant(ci, sig_str);
            // Pop the method compiler manually (no body).
            self.compilers.pop();
        } else {
            self.consume(T::LeftBrace, "Expect '{' to begin method body.");
            self.finish_body(mci);
            self.end_compiler(mci, &full_signature);
        }

        self.define_method(ci, class_variable, is_static, method_symbol);

        if signature.typ == SigType::Initializer {
            let mut sig2 = signature;
            sig2.typ = SigType::Method;
            let constructor_symbol = self.signature_symbol(&sig2);
            self.create_constructor(ci, &sig2, method_symbol);
            self.define_method(ci, class_variable, true, constructor_symbol);
        }

        true
    }

    fn class_definition(&mut self, ci: usize, is_foreign: bool) {
        let scope = if self.compilers[ci].scope_depth == -1 {
            Scope::Module
        } else {
            Scope::Local
        };
        let index = self.declare_named_variable(ci);
        let class_variable = Variable { index, scope };

        let name_tok = self.parser.as_ref().unwrap().previous;
        let class_name_bytes = self.token_text(&name_tok);
        let class_name_val = self.new_string_len(&class_name_bytes);
        let class_name = class_name_val.as_obj();

        self.emit_constant(ci, class_name_val);

        if self.match_(T::Is) {
            self.parse_precedence(ci, Prec::Call);
        } else {
            self.load_core_variable(ci, "Object");
        }

        let mut num_fields_instruction: usize = usize::MAX;
        if is_foreign {
            self.emit_op(ci, FOREIGN_CLASS);
        } else {
            num_fields_instruction = self.emit_byte_arg(ci, CLASS, 255);
        }

        self.define_variable_code(ci, class_variable.index);
        self.push_scope(ci);

        // Allocate class attribute map if the compiler has pending attributes.
        let attr_map = self.compilers[ci].attributes;
        let class_attributes = if attr_map != NULL_OBJ && self.heap.map(attr_map).count > 0 {
            self.new_map()
        } else {
            NULL_OBJ
        };

        self.compilers[ci].enclosing_class = Some(ClassInfo {
            is_foreign,
            name: class_name,
            class_attributes,
            method_attributes: NULL_OBJ,
            fields: Vec::new(),
            methods: Vec::new(),
            static_methods: Vec::new(),
            in_static: false,
            signature: None,
        });

        if class_attributes != NULL_OBJ {
            self.copy_attributes_into(ci, class_attributes);
        } else {
            // Still resets numAttributes like copyAttributes does.
            self.compilers[ci].num_attributes = 0;
        }

        self.consume(T::LeftBrace, "Expect '{' after class declaration.");
        self.match_line();

        while !self.match_(T::RightBrace) {
            if !self.method(ci, class_variable) {
                break;
            }
            if self.match_(T::RightBrace) {
                break;
            }
            self.consume_line("Expect newline after definition in class.");
        }

        let has_attr = {
            let info = self.compilers[ci].enclosing_class.as_ref().unwrap();
            info.class_attributes != NULL_OBJ || info.method_attributes != NULL_OBJ
        };
        if has_attr {
            self.emit_class_attributes(ci);
            self.load_variable(ci, class_variable);
            self.emit_op(ci, END_CLASS);
        }

        if !is_foreign {
            let n = self.compilers[ci]
                .enclosing_class
                .as_ref()
                .unwrap()
                .fields
                .len();
            let fn_ref = self.compilers[ci].fn_ref;
            self.heap.func_mut(fn_ref).code[num_fields_instruction] = n as u8;
        }

        self.compilers[ci].enclosing_class = None;
        self.pop_scope(ci);
    }

    fn import(&mut self, ci: usize) {
        self.ignore_newlines();
        self.consume(T::Str, "Expect a string after 'import'.");
        let module_value = self.parser.as_ref().unwrap().previous.value;
        let module_constant = self.add_constant(ci, module_value);
        self.emit_short_arg(ci, IMPORT_MODULE, module_constant);
        self.emit_op(ci, POP);

        if !self.match_(T::For) {
            return;
        }

        loop {
            self.ignore_newlines();
            self.consume(T::Name, "Expect variable name.");
            let source_variable_token = self.parser.as_ref().unwrap().previous;
            let name_bytes = self.token_text(&source_variable_token);
            let sv = self.new_string_len(&name_bytes);
            let source_variable_constant = self.add_constant(ci, sv);

            let slot = if self.match_(T::As) {
                self.declare_named_variable(ci)
            } else {
                self.declare_variable(ci, Some(source_variable_token))
            };

            self.emit_short_arg(ci, IMPORT_VARIABLE, source_variable_constant);
            self.define_variable_code(ci, slot);
            if !self.match_(T::Comma) {
                break;
            }
        }
    }

    fn variable_definition(&mut self, ci: usize) {
        self.consume(T::Name, "Expect variable name.");
        let name_token = self.parser.as_ref().unwrap().previous;
        if self.match_(T::Eq) {
            self.ignore_newlines();
            self.expression(ci);
        } else {
            self.emit_op(ci, NULL);
        }
        let symbol = self.declare_variable(ci, Some(name_token));
        self.define_variable_code(ci, symbol);
    }

    fn definition(&mut self, ci: usize) {
        if self.match_attribute(ci) {
            self.definition(ci);
            return;
        }
        if self.match_(T::Class) {
            self.class_definition(ci, false);
            return;
        } else if self.match_(T::Foreign) {
            self.consume(T::Class, "Expect 'class' after 'foreign'.");
            self.class_definition(ci, true);
            return;
        }

        self.disallow_attributes(ci);

        if self.match_(T::Import) {
            self.import(ci);
        } else if self.match_(T::Var) {
            self.variable_definition(ci);
        } else {
            self.statement(ci);
        }
    }

    // -- attributes -------------------------------------------------------------------

    fn disallow_attributes(&mut self, ci: usize) {
        if self.compilers[ci].num_attributes > 0 {
            self.error("Attributes can only specified before a class or a method");
            let attrs = self.compilers[ci].attributes;
            if attrs != NULL_OBJ {
                self.map_clear(attrs);
            }
            self.compilers[ci].num_attributes = 0;
        }
    }

    fn add_to_attribute_group(&mut self, ci: usize, group: Value, key: Value, value: Value) {
        // Attributes stored as: group -> { key -> [value, ...] }
        if group.is_obj() {
            self.push_root(group.as_obj_unchecked());
        }
        if key.is_obj() {
            self.push_root(key.as_obj_unchecked());
        }
        if value.is_obj() {
            self.push_root(value.as_obj_unchecked());
        }

        let attrs = self.compilers[ci].attributes;

        let mut group_map_value = self.map_get(attrs, group);
        if group_map_value.is_undefined() {
            let m = self.new_map();
            group_map_value = Value::obj(m);
            self.map_set(attrs, group, group_map_value);
        }
        let group_map = group_map_value.as_obj();

        let mut key_items_value = self.map_get(group_map, key);
        if key_items_value.is_undefined() {
            let l = self.new_list(0);
            key_items_value = Value::obj(l);
            self.map_set(group_map, key, key_items_value);
        }
        let key_items = key_items_value.as_obj();
        self.heap.list_mut(key_items).elements.push(value);
        self.track_alloc(8);

        if group.is_obj() {
            self.pop_root();
        }
        if key.is_obj() {
            self.pop_root();
        }
        if value.is_obj() {
            self.pop_root();
        }
    }

    fn emit_attributes(&mut self, ci: usize, attributes: ObjRef) {
        // Instantiate a new map for the attributes.
        self.load_core_variable(ci, "Map");
        self.call_method(ci, 0, b"new()");

        // attributes is group -> { key -> [values] }
        let group_entries: Vec<MapEntry> = self.heap.map(attributes).entries.clone();
        for ge in &group_entries {
            if ge.key.is_undefined() {
                continue;
            }
            self.emit_constant(ci, ge.key);

            // The group's map.
            self.load_core_variable(ci, "Map");
            self.call_method(ci, 0, b"new()");

            let group_items = ge.value.as_obj();
            let item_entries: Vec<MapEntry> = self.heap.map(group_items).entries.clone();
            for ie in &item_entries {
                if ie.key.is_undefined() {
                    continue;
                }
                self.emit_constant(ci, ie.key);
                // The list of values.
                self.load_core_variable(ci, "List");
                self.call_method(ci, 0, b"new()");
                let items: Vec<Value> = self.heap.list(ie.value.as_obj()).elements.clone();
                for item in items {
                    self.emit_constant(ci, item);
                    self.call_method(ci, 1, b"addCore_(_)");
                }
                self.call_method(ci, 2, b"addCore_(_,_)");
            }

            self.call_method(ci, 2, b"addCore_(_,_)");
        }
    }

    fn emit_attribute_methods(&mut self, ci: usize, attributes: ObjRef) {
        self.load_core_variable(ci, "Map");
        self.call_method(ci, 0, b"new()");

        let entries: Vec<MapEntry> = self.heap.map(attributes).entries.clone();
        for e in &entries {
            if e.key.is_undefined() {
                continue;
            }
            self.emit_constant(ci, e.key);
            self.emit_attributes(ci, e.value.as_obj());
            self.call_method(ci, 2, b"addCore_(_,_)");
        }
    }

    fn emit_class_attributes(&mut self, ci: usize) {
        let (class_attrs, method_attrs) = {
            let info = self.compilers[ci].enclosing_class.as_ref().unwrap();
            (info.class_attributes, info.method_attributes)
        };

        self.load_core_variable(ci, "ClassAttributes");

        if class_attrs != NULL_OBJ {
            self.emit_attributes(ci, class_attrs);
        } else {
            self.emit_op(ci, NULL);
        }

        if method_attrs != NULL_OBJ {
            self.emit_attribute_methods(ci, method_attrs);
        } else {
            self.emit_op(ci, NULL);
        }

        self.call_method(ci, 2, b"new(_,_)");
    }

    fn copy_attributes_into(&mut self, ci: usize, into: ObjRef) {
        self.compilers[ci].num_attributes = 0;
        let attrs = self.compilers[ci].attributes;
        if attrs == NULL_OBJ || self.heap.map(attrs).count == 0 {
            return;
        }
        if into == NULL_OBJ {
            return;
        }
        let entries: Vec<MapEntry> = self.heap.map(attrs).entries.clone();
        for e in &entries {
            if e.key.is_undefined() {
                continue;
            }
            self.map_set(into, e.key, e.value);
        }
        self.map_clear(attrs);
    }

    fn copy_method_attributes(
        &mut self,
        ci: usize,
        is_foreign: bool,
        is_static: bool,
        full_signature: &[u8],
    ) {
        self.compilers[ci].num_attributes = 0;
        let attrs = self.compilers[ci].attributes;
        if attrs == NULL_OBJ || self.heap.map(attrs).count == 0 {
            return;
        }

        // Make a map for this method to copy into.
        let method_attr = self.new_map();
        self.push_root(method_attr);
        self.copy_attributes_into(ci, method_attr);

        // "foreign static " prefix order is foreign then static.
        let mut key = Vec::new();
        if is_foreign {
            key.extend_from_slice(b"foreign ");
        }
        if is_static {
            key.extend_from_slice(b"static ");
        }
        key.extend_from_slice(full_signature);

        if self.compilers[ci]
            .enclosing_class
            .as_ref()
            .unwrap()
            .method_attributes
            == NULL_OBJ
        {
            let m = self.new_map();
            self.compilers[ci]
                .enclosing_class
                .as_mut()
                .unwrap()
                .method_attributes = m;
        }
        let method_attr_map = self.compilers[ci]
            .enclosing_class
            .as_ref()
            .unwrap()
            .method_attributes;

        let key = self.new_string_len(&key);
        self.map_set(method_attr_map, key, Value::obj(method_attr));
        self.pop_root();
    }

    // -- compiler lifecycle -----------------------------------------------------------

    /// Pushes a new compiler. Mirrors initCompiler; the new top compiler's
    /// parent is implicitly the previous top (`ci - 1`).
    pub(crate) fn init_compiler(&mut self, is_method: bool) {
        let parent_exists = !self.compilers.is_empty();
        self.compilers.push(Compiler {
            locals: Vec::with_capacity(16),
            upvalues: Vec::new(),
            scope_depth: if parent_exists { 0 } else { -1 },
            num_slots: 1,
            loops: Vec::new(),
            enclosing_class: None,
            fn_ref: NULL_OBJ,
            constants: NULL_OBJ,
            is_initializer: false,
            num_attributes: 0,
            attributes: NULL_OBJ,
        });
        let ci = self.cur();
        // locals[0]: "this" for methods, "" for functions.
        self.compilers[ci].locals.push(Local {
            name: NameRef::Static(if is_method { b"this" } else { b"" }),
            depth: -1,
            is_upvalue: false,
        });

        let attrs = self.new_map();
        self.compilers[ci].attributes = attrs;
        let module = self.parser.as_ref().unwrap().module;
        let f = self.new_function(module, 1);
        self.compilers[ci].fn_ref = f;
    }

    fn get_byte_count_for_arguments(&self, fn_ref: ObjRef, ip: usize) -> usize {
        let f = self.heap.func(fn_ref);
        get_byte_count_for_arguments(&f.code, &f.constants, self, ip)
    }

    // -- entry point ------------------------------------------------------------

    pub fn wren_compile(
        &mut self,
        module: ObjRef,
        source: &[u8],
        is_expression: bool,
        print_errors: bool,
    ) -> ObjRef {
        // Skip UTF-8 BOM.
        let source = if source.starts_with(b"\xEF\xBB\xBF") {
            &source[3..]
        } else {
            source
        };

        let old_parser = self.parser.take();
        let old_compilers = std::mem::take(&mut self.compilers);

        self.parser = Some(Parser {
            module,
            source: source.to_vec(),
            token_start: 0,
            current_char: 0,
            current_line: 1,
            next: Token::new(),
            current: Token::new(),
            previous: Token::new(),
            parens: [0; MAX_INTERPOLATION_NESTING],
            num_parens: 0,
            print_errors,
            has_error: false,
        });

        self.next_token();
        self.next_token();

        let num_existing_variables = self.heap.module(module).variables.len();

        self.init_compiler(false);
        let ci = self.cur();
        self.ignore_newlines();

        if is_expression {
            self.expression(ci);
            self.consume(T::Eof, "Expect end of expression.");
        } else {
            while !self.match_(T::Eof) {
                self.definition(ci);
                if !self.match_line() {
                    self.consume(T::Eof, "Expect end of file.");
                    break;
                }
            }
            self.emit_op(ci, END_MODULE);
        }

        self.emit_op(ci, RETURN);

        // Check for implicitly declared module vars that were never defined.
        let var_count = self.heap.module(module).variables.len();
        for i in num_existing_variables..var_count {
            let v = self.heap.module(module).variables[i];
            if v.is_num() {
                let name_ref = self.heap.module(module).variable_names[i];
                let name = self.heap.string(name_ref).bytes.clone();
                let line = v.as_num() as i32;
                let label = format!(
                    "Error at '{}'",
                    String::from_utf8_lossy(&name[..name.len().min(MAX_VARIABLE_NAME)])
                );
                self.print_error(line, &label, "Variable is used but not defined.");
            }
        }

        let f = self.end_compiler(ci, b"(script)");

        self.parser = old_parser;
        self.compilers = old_compilers;
        f
    }

    /// wrenBindMethodCode: patch field ops and super constants in a bound method.
    pub fn bind_method_code(&mut self, class_ref: ObjRef, fn_ref: ObjRef) {
        let superclass = self.heap.class(class_ref).superclass;
        let num_fields = self.heap.class(superclass).num_fields;
        let mut ip = 0usize;
        loop {
            let instruction = self.heap.func(fn_ref).code[ip];
            match instruction {
                LOAD_FIELD | STORE_FIELD | LOAD_FIELD_THIS | STORE_FIELD_THIS => {
                    let patched =
                        self.heap.func(fn_ref).code[ip + 1].wrapping_add(num_fields as u8);
                    self.heap.func_mut(fn_ref).code[ip + 1] = patched;
                }
                SUPER_0..=SUPER_16 => {
                    let constant = ((self.heap.func(fn_ref).code[ip + 3] as usize) << 8)
                        | self.heap.func(fn_ref).code[ip + 4] as usize;
                    self.heap.func_mut(fn_ref).constants[constant] = Value::obj(superclass);
                }
                CLOSURE => {
                    let constant = ((self.heap.func(fn_ref).code[ip + 1] as usize) << 8)
                        | self.heap.func(fn_ref).code[ip + 2] as usize;
                    let sub = self.heap.func(fn_ref).constants[constant].as_obj();
                    self.bind_method_code(class_ref, sub);
                }
                END => return,
                _ => {}
            }
            ip += 1 + self.get_byte_count_for_arguments(fn_ref, ip);
        }
    }

    /// GC marking of the compiler state (wrenMarkCompiler).
    pub(crate) fn mark_compiler(&mut self) {
        let mut to_gray: Vec<ObjRef> = Vec::new();
        if let Some(p) = &self.parser {
            for v in [p.current.value, p.previous.value, p.next.value] {
                if v.is_obj() {
                    to_gray.push(v.as_obj_unchecked());
                }
            }
        }
        for ci in 0..self.compilers.len() {
            let c = &self.compilers[ci];
            for r in [c.fn_ref, c.constants, c.attributes] {
                if r != NULL_OBJ {
                    to_gray.push(r);
                }
            }
            if let Some(info) = &c.enclosing_class {
                to_gray.extend_from_slice(&info.fields);
                if info.method_attributes != NULL_OBJ {
                    to_gray.push(info.method_attributes);
                }
                if info.class_attributes != NULL_OBJ {
                    to_gray.push(info.class_attributes);
                }
            }
        }
        for r in to_gray {
            self.gray_obj(r);
        }
    }
}

fn signature_parameter_list(name: &mut Vec<u8>, num_params: i32, left: u8, right: u8) {
    name.push(left);
    let n = num_params.min(MAX_PARAMETERS);
    for i in 0..n {
        if i > 0 {
            name.push(b',');
        }
        name.push(b'_');
    }
    name.push(right);
}

// -- grammar prefix/infix/signature functions --------------------------------

fn grouping(vm: &mut WrenVM, ci: usize, _can_assign: bool) {
    vm.expression(ci);
    vm.consume(T::RightParen, "Expect ')' after expression.");
}

fn list(vm: &mut WrenVM, ci: usize, _can_assign: bool) {
    vm.load_core_variable(ci, "List");
    vm.call_method(ci, 0, b"new()");
    loop {
        vm.ignore_newlines();
        if vm.peek() == T::RightBracket {
            break;
        }
        vm.expression(ci);
        vm.call_method(ci, 1, b"addCore_(_)");
        if !vm.match_(T::Comma) {
            break;
        }
    }
    vm.ignore_newlines();
    vm.consume(T::RightBracket, "Expect ']' after list elements.");
}

fn map(vm: &mut WrenVM, ci: usize, _can_assign: bool) {
    vm.load_core_variable(ci, "Map");
    vm.call_method(ci, 0, b"new()");
    loop {
        vm.ignore_newlines();
        if vm.peek() == T::RightBrace {
            break;
        }
        vm.parse_precedence(ci, Prec::Unary);
        vm.consume(T::Colon, "Expect ':' after map key.");
        vm.ignore_newlines();
        vm.expression(ci);
        vm.call_method(ci, 2, b"addCore_(_,_)");
        if !vm.match_(T::Comma) {
            break;
        }
    }
    vm.ignore_newlines();
    vm.consume(T::RightBrace, "Expect '}' after map entries.");
}

fn unary_op(vm: &mut WrenVM, ci: usize, _can_assign: bool) {
    let typ = vm.parser.as_ref().unwrap().previous.typ;
    let name = RULES[typ as usize].name.as_bytes().to_vec();
    vm.ignore_newlines();
    vm.parse_precedence(ci, Prec::Unary.next());
    vm.call_method(ci, 0, &name);
}

fn boolean(vm: &mut WrenVM, ci: usize, _can_assign: bool) {
    let typ = vm.parser.as_ref().unwrap().previous.typ;
    vm.emit_op(ci, if typ == T::False { FALSE } else { TRUE });
}

fn get_enclosing_class_compiler(vm: &WrenVM, ci: usize) -> Option<usize> {
    (0..=ci)
        .rev()
        .find(|&i| vm.compilers[i].enclosing_class.is_some())
}

fn field(vm: &mut WrenVM, ci: usize, can_assign: bool) {
    let mut field_idx = MAX_FIELDS as i32;
    let enclosing_ci = get_enclosing_class_compiler(vm, ci);

    if enclosing_ci.is_none() {
        vm.error("Cannot reference a field outside of a class definition.");
    } else {
        let eci = enclosing_ci.unwrap();
        let (is_foreign, in_static) = {
            let info = vm.compilers[eci].enclosing_class.as_ref().unwrap();
            (info.is_foreign, info.in_static)
        };
        if is_foreign {
            vm.error("Cannot define fields in a foreign class.");
        } else if in_static {
            vm.error("Cannot use an instance field in a static method.");
        } else {
            let tok = vm.parser.as_ref().unwrap().previous;
            let name = vm.token_text(&tok);
            // wrenSymbolTableEnsure on the class's fields.
            let mut found = -1i32;
            {
                let info = vm.compilers[eci].enclosing_class.as_ref().unwrap();
                for (i, &s) in info.fields.iter().enumerate() {
                    if vm.heap.string(s).bytes == name {
                        found = i as i32;
                        break;
                    }
                }
            }
            if found == -1 {
                let s = vm.new_string_len(&name).as_obj();
                vm.compilers[eci]
                    .enclosing_class
                    .as_mut()
                    .unwrap()
                    .fields
                    .push(s);
                found = vm.compilers[eci]
                    .enclosing_class
                    .as_ref()
                    .unwrap()
                    .fields
                    .len() as i32
                    - 1;
            }
            field_idx = found;
            if field_idx >= MAX_FIELDS as i32 {
                vm.error(&format!("A class can only have {} fields.", MAX_FIELDS));
            }
        }
    }

    let mut is_load = true;
    if can_assign && vm.match_(T::Eq) {
        vm.expression(ci);
        is_load = false;
    }

    // Directly inside a method of the enclosing class compiler?
    let use_this = ci > 0 && enclosing_ci == Some(ci - 1);
    if use_this {
        vm.emit_byte_arg(
            ci,
            if is_load {
                LOAD_FIELD_THIS
            } else {
                STORE_FIELD_THIS
            },
            field_idx,
        );
    } else {
        vm.load_this(ci);
        vm.emit_byte_arg(
            ci,
            if is_load { LOAD_FIELD } else { STORE_FIELD },
            field_idx,
        );
    }
    vm.allow_line_before_dot();
}

fn bare_name(vm: &mut WrenVM, ci: usize, can_assign: bool, variable: Variable) {
    if can_assign && vm.match_(T::Eq) {
        vm.expression(ci);
        match variable.scope {
            Scope::Local => {
                vm.emit_byte_arg(ci, STORE_LOCAL, variable.index);
            }
            Scope::Upvalue => {
                vm.emit_byte_arg(ci, STORE_UPVALUE, variable.index);
            }
            Scope::Module => {
                vm.emit_short_arg(ci, STORE_MODULE_VAR, variable.index);
            }
        }
        return;
    }
    vm.load_variable(ci, variable);
    vm.allow_line_before_dot();
}

fn static_field(vm: &mut WrenVM, ci: usize, can_assign: bool) {
    let class_ci = match get_enclosing_class_compiler(vm, ci) {
        None => {
            vm.error("Cannot use a static field outside of a class definition.");
            return;
        }
        Some(c) => c,
    };

    let token = vm.parser.as_ref().unwrap().previous;
    let name_bytes = vm.token_text(&token);

    if vm.resolve_local(class_ci, &name_bytes) == -1 {
        let symbol = vm.declare_variable(class_ci, Some(token));
        vm.emit_op(class_ci, NULL);
        vm.define_variable_code(class_ci, symbol);
    }

    let variable = vm.resolve_name(ci, &name_bytes);
    bare_name(vm, ci, can_assign, variable);
}

fn name(vm: &mut WrenVM, ci: usize, can_assign: bool) {
    let token = vm.parser.as_ref().unwrap().previous;
    let name_bytes = vm.token_text(&token);
    let variable = vm.resolve_nonmodule(ci, &name_bytes);
    if variable.index != -1 {
        bare_name(vm, ci, can_assign, variable);
        return;
    }

    // If inside a method and the name is lowercase, treat as method on this.
    let first = name_bytes.first().copied().unwrap_or(0);
    if first.is_ascii_lowercase() && get_enclosing_class_compiler(vm, ci).is_some() {
        vm.load_this(ci);
        vm.named_call(ci, can_assign, CALL_0);
        return;
    }

    let module = vm.parser.as_ref().unwrap().module;
    let mut variable = Variable {
        scope: Scope::Module,
        index: vm.module_find_variable(module, &name_bytes),
    };
    if variable.index == -1 {
        let line = token.line;
        variable.index = vm.declare_module_variable(module, &name_bytes, line);
        if variable.index == -2 {
            vm.error("Too many module variables defined.");
        }
    }
    bare_name(vm, ci, can_assign, variable);
}

fn null(vm: &mut WrenVM, ci: usize, _can_assign: bool) {
    vm.emit_op(ci, NULL);
}

fn literal(vm: &mut WrenVM, ci: usize, _can_assign: bool) {
    let v = vm.parser.as_ref().unwrap().previous.value;
    vm.emit_constant(ci, v);
}

fn string_interpolation(vm: &mut WrenVM, ci: usize, _can_assign: bool) {
    vm.load_core_variable(ci, "List");
    vm.call_method(ci, 0, b"new()");
    loop {
        literal(vm, ci, false);
        vm.call_method(ci, 1, b"addCore_(_)");
        vm.ignore_newlines();
        vm.expression(ci);
        vm.call_method(ci, 1, b"addCore_(_)");
        vm.ignore_newlines();
        if !vm.match_(T::Interpolation) {
            break;
        }
    }
    vm.consume(T::Str, "Expect end of string interpolation.");
    literal(vm, ci, false);
    vm.call_method(ci, 1, b"addCore_(_)");
    vm.call_method(ci, 0, b"join()");
}

fn super_(vm: &mut WrenVM, ci: usize, can_assign: bool) {
    let enclosing = get_enclosing_class_compiler(vm, ci);
    if enclosing.is_none() {
        vm.error("Cannot use 'super' outside of a method.");
    }
    vm.load_this(ci);

    if vm.match_(T::Dot) {
        vm.consume(T::Name, "Expect method name after 'super.'.");
        vm.named_call(ci, can_assign, SUPER_0);
    } else if let Some(eci) = enclosing {
        let sig = vm.compilers[eci]
            .enclosing_class
            .as_ref()
            .unwrap()
            .signature
            .unwrap();
        vm.method_call(ci, SUPER_0, &sig);
    }
}

fn this_(vm: &mut WrenVM, ci: usize, _can_assign: bool) {
    if get_enclosing_class_compiler(vm, ci).is_none() {
        vm.error("Cannot use 'this' outside of a method.");
        return;
    }
    vm.load_this(ci);
}

fn subscript(vm: &mut WrenVM, ci: usize, can_assign: bool) {
    let mut signature = Signature {
        name: NameRef::Static(b""),
        typ: SigType::Subscript,
        arity: 0,
    };
    vm.finish_argument_list(ci, &mut signature);
    vm.consume(T::RightBracket, "Expect ']' after arguments.");
    vm.allow_line_before_dot();

    if can_assign && vm.match_(T::Eq) {
        signature.typ = SigType::SubscriptSetter;
        signature.arity += 1;
        let a = signature.arity;
        vm.validate_num_parameters(a);
        vm.expression(ci);
    }
    vm.call_signature(ci, CALL_0, &signature);
}

fn call(vm: &mut WrenVM, ci: usize, can_assign: bool) {
    vm.ignore_newlines();
    vm.consume(T::Name, "Expect method name after '.'.");
    vm.named_call(ci, can_assign, CALL_0);
}

fn and_(vm: &mut WrenVM, ci: usize, _can_assign: bool) {
    vm.ignore_newlines();
    let jump = vm.emit_jump(ci, AND);
    vm.parse_precedence(ci, Prec::LogicalAnd);
    vm.patch_jump(ci, jump);
}

fn or_(vm: &mut WrenVM, ci: usize, _can_assign: bool) {
    vm.ignore_newlines();
    let jump = vm.emit_jump(ci, OR);
    vm.parse_precedence(ci, Prec::LogicalOr);
    vm.patch_jump(ci, jump);
}

fn conditional(vm: &mut WrenVM, ci: usize, _can_assign: bool) {
    vm.ignore_newlines();
    let if_jump = vm.emit_jump(ci, JUMP_IF);
    vm.parse_precedence(ci, Prec::Conditional);
    vm.consume(
        T::Colon,
        "Expect ':' after then branch of conditional operator.",
    );
    vm.ignore_newlines();
    let else_jump = vm.emit_jump(ci, JUMP);
    vm.patch_jump(ci, if_jump);
    vm.parse_precedence(ci, Prec::Assignment);
    vm.patch_jump(ci, else_jump);
}

fn infix_op(vm: &mut WrenVM, ci: usize, _can_assign: bool) {
    let typ = vm.parser.as_ref().unwrap().previous.typ;
    let prec = RULES[typ as usize].precedence.next();
    let name: &'static str = RULES[typ as usize].name;
    vm.ignore_newlines();
    vm.parse_precedence(ci, prec);
    let signature = Signature {
        name: NameRef::Static(name.as_bytes()),
        typ: SigType::Method,
        arity: 1,
    };
    vm.call_signature(ci, CALL_0, &signature);
}

fn infix_signature(vm: &mut WrenVM, ci: usize, signature: &mut Signature) {
    signature.typ = SigType::Method;
    signature.arity = 1;
    vm.consume(T::LeftParen, "Expect '(' after operator name.");
    vm.declare_named_variable(ci);
    vm.consume(T::RightParen, "Expect ')' after parameter name.");
}

fn unary_signature(_vm: &mut WrenVM, _ci: usize, signature: &mut Signature) {
    signature.typ = SigType::Getter;
}

fn mixed_signature(vm: &mut WrenVM, ci: usize, signature: &mut Signature) {
    signature.typ = SigType::Getter;
    if vm.match_(T::LeftParen) {
        signature.typ = SigType::Method;
        signature.arity = 1;
        vm.declare_named_variable(ci);
        vm.consume(T::RightParen, "Expect ')' after parameter name.");
    }
}

fn maybe_setter(vm: &mut WrenVM, ci: usize, signature: &mut Signature) -> bool {
    if !vm.match_(T::Eq) {
        return false;
    }
    signature.typ = if signature.typ == SigType::Subscript {
        SigType::SubscriptSetter
    } else {
        SigType::Setter
    };
    vm.consume(T::LeftParen, "Expect '(' after '='.");
    vm.declare_named_variable(ci);
    vm.consume(T::RightParen, "Expect ')' after parameter name.");
    signature.arity += 1;
    true
}

fn subscript_signature(vm: &mut WrenVM, ci: usize, signature: &mut Signature) {
    signature.typ = SigType::Subscript;
    signature.name = NameRef::Static(b"");
    vm.finish_parameter_list(ci, signature);
    vm.consume(T::RightBracket, "Expect ']' after parameters.");
    maybe_setter(vm, ci, signature);
}

fn parameter_list(vm: &mut WrenVM, ci: usize, signature: &mut Signature) {
    if !vm.match_(T::LeftParen) {
        return;
    }
    signature.typ = SigType::Method;
    vm.ignore_newlines();
    if vm.match_(T::RightParen) {
        return;
    }
    vm.finish_parameter_list(ci, signature);
    vm.consume(T::RightParen, "Expect ')' after parameters.");
}

fn named_signature(vm: &mut WrenVM, ci: usize, signature: &mut Signature) {
    signature.typ = SigType::Getter;
    if maybe_setter(vm, ci, signature) {
        return;
    }
    parameter_list(vm, ci, signature);
}

fn constructor_signature(vm: &mut WrenVM, ci: usize, signature: &mut Signature) {
    vm.consume(T::Name, "Expect constructor name after 'construct'.");
    *signature = vm.signature_from_token(SigType::Initializer);
    if vm.match_(T::Eq) {
        vm.error("A constructor cannot be a setter.");
    }
    if !vm.match_(T::LeftParen) {
        vm.error("A constructor cannot be a getter.");
        return;
    }
    if vm.match_(T::RightParen) {
        return;
    }
    vm.finish_parameter_list(ci, signature);
    vm.consume(T::RightParen, "Expect ')' after parameters.");
}

impl Prec {
    /// precedence + 1
    fn next(self) -> Prec {
        let v = self as u8 + 1;
        // Prec is a contiguous enum.
        unsafe { std::mem::transmute(v) }
    }
}

// -- number parsing ------------------------------------------------------------

/// Parses a decimal/hex floating literal like C's strtod. Returns
/// (value, bytes_consumed, erange).
pub fn parse_double(text: &[u8]) -> (f64, usize, bool) {
    let mut i = 0usize;
    let n = text.len();
    // Skip leading whitespace (isspace).
    while i < n && matches!(text[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let start = i;
    let mut negative = false;
    if i < n && (text[i] == b'+' || text[i] == b'-') {
        negative = text[i] == b'-';
        i += 1;
    }

    // inf / infinity / nan (case-insensitive)
    let rest = &text[i..];
    if rest.len() >= 3 && rest[..3].eq_ignore_ascii_case(b"inf") {
        let mut end = i + 3;
        if rest.len() >= 8 && rest[..8].eq_ignore_ascii_case(b"infinity") {
            end = i + 8;
        }
        let v = if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
        return (v, end, false);
    }
    if rest.len() >= 3 && rest[..3].eq_ignore_ascii_case(b"nan") {
        let v = if negative { -f64::NAN } else { f64::NAN };
        return (v, i + 3, false);
    }

    // Hex float: 0x...
    if rest.len() >= 2 && rest[0] == b'0' && (rest[1] == b'x' || rest[1] == b'X') {
        let mut j = i + 2;
        let mut mantissa: f64 = 0.0;
        let mut any = false;
        while j < n && text[j].is_ascii_hexdigit() {
            mantissa = mantissa * 16.0 + hex_val(text[j]) as f64;
            j += 1;
            any = true;
        }
        let mut frac_scale = 1.0;
        if j < n && text[j] == b'.' {
            j += 1;
            while j < n && text[j].is_ascii_hexdigit() {
                frac_scale /= 16.0;
                mantissa += hex_val(text[j]) as f64 * frac_scale;
                j += 1;
                any = true;
            }
        }
        if !any {
            // "0x" with no digits: strtod stops at 'x', returns 0.
            return (0.0, i + 1, false);
        }
        let mut exp = 0i32;
        if j < n && (text[j] == b'p' || text[j] == b'P') {
            let mut k = j + 1;
            let mut eneg = false;
            if k < n && (text[k] == b'+' || text[k] == b'-') {
                eneg = text[k] == b'-';
                k += 1;
            }
            let mut eval = 0i64;
            let mut eany = false;
            while k < n && text[k].is_ascii_digit() {
                eval = (eval * 10 + (text[k] - b'0') as i64).min(1 << 30);
                k += 1;
                eany = true;
            }
            if eany {
                exp = if eneg { -(eval as i32) } else { eval as i32 };
                j = k;
            }
        }
        let mut v = mantissa * 2f64.powi(exp);
        if negative {
            v = -v;
        }
        let erange = v.is_infinite();
        (v, j, erange)
    } else {
        // Decimal.
        let mut j = i;
        let mut any = false;
        while j < n && text[j].is_ascii_digit() {
            j += 1;
            any = true;
        }
        if j < n && text[j] == b'.' {
            j += 1;
            while j < n && text[j].is_ascii_digit() {
                j += 1;
                any = true;
            }
        }
        if !any {
            return (0.0, start, false); // strtod: no digits → 0, endptr = start
        }
        if j < n && (text[j] == b'e' || text[j] == b'E') {
            let mut k = j + 1;
            if k < n && (text[k] == b'+' || text[k] == b'-') {
                k += 1;
            }
            if k < n && text[k].is_ascii_digit() {
                k += 1;
                while k < n && text[k].is_ascii_digit() {
                    k += 1;
                }
                j = k;
            }
        }
        let s = std::str::from_utf8(&text[i..j]).unwrap_or("0");
        let mut v: f64 = s.parse().unwrap_or(0.0);
        if negative {
            v = -v;
        }
        // ERANGE: overflow to inf, or a nonzero literal underflowing to
        // 0/subnormal.
        let erange = v.is_infinite()
            || (v != 0.0 && v.abs() < f64::MIN_POSITIVE)
            || (v == 0.0 && s.bytes().any(|b| (b'1'..=b'9').contains(&b)));
        (v, j, erange)
    }
}

fn hex_val(b: u8) -> u32 {
    match b {
        b'0'..=b'9' => (b - b'0') as u32,
        b'a'..=b'f' => (b - b'a' + 10) as u32,
        b'A'..=b'F' => (b - b'A' + 10) as u32,
        _ => 0,
    }
}

/// strtoll(text, 16) for hex literals. Returns (value, erange).
fn parse_hex_literal(text: &[u8]) -> (f64, bool) {
    // text is like "0xABC...".
    let mut v: i128 = 0;
    let mut i = 2; // skip 0x
    while i < text.len() && text[i].is_ascii_hexdigit() {
        v = v * 16 + hex_val(text[i]) as i128;
        if v > i64::MAX as i128 {
            return (i64::MAX as f64, true);
        }
        i += 1;
    }
    (v as i64 as f64, false)
}

/// getByteCountForArguments.
pub fn get_byte_count_for_arguments(
    bytecode: &[u8],
    constants: &[Value],
    vm: &WrenVM,
    ip: usize,
) -> usize {
    let instruction = bytecode[ip];
    match instruction {
        NULL | FALSE | TRUE | POP | CLOSE_UPVALUE | RETURN | END | LOAD_LOCAL_0 | LOAD_LOCAL_1
        | LOAD_LOCAL_2 | LOAD_LOCAL_3 | LOAD_LOCAL_4 | LOAD_LOCAL_5 | LOAD_LOCAL_6
        | LOAD_LOCAL_7 | LOAD_LOCAL_8 | CONSTRUCT | FOREIGN_CONSTRUCT | FOREIGN_CLASS
        | END_MODULE | END_CLASS => 0,

        LOAD_LOCAL | STORE_LOCAL | LOAD_UPVALUE | STORE_UPVALUE | LOAD_FIELD_THIS
        | STORE_FIELD_THIS | LOAD_FIELD | STORE_FIELD | CLASS => 1,

        CONSTANT | LOAD_MODULE_VAR | STORE_MODULE_VAR | CALL_0 | CALL_1 | CALL_2 | CALL_3
        | CALL_4 | CALL_5 | CALL_6 | CALL_7 | CALL_8 | CALL_9 | CALL_10 | CALL_11 | CALL_12
        | CALL_13 | CALL_14 | CALL_15 | CALL_16 | JUMP | LOOP | JUMP_IF | AND | OR
        | METHOD_INSTANCE | METHOD_STATIC | IMPORT_MODULE | IMPORT_VARIABLE => 2,

        SUPER_0..=SUPER_16 => 4,

        CLOSURE => {
            let constant = ((bytecode[ip + 1] as usize) << 8) | bytecode[ip + 2] as usize;
            let loaded_fn = vm.heap.func(constants[constant].as_obj());
            2 + (loaded_fn.num_upvalues as usize * 2)
        }
        _ => unreachable!(),
    }
}
