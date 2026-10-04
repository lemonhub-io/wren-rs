// Port of wren_opcodes.h: opcode constants in the exact C ordering (the
// compiler emits `CODE_CALL_0 + numArgs` so relative order matters), plus
// the per-opcode stack-effect table used by emitOp().

#![allow(dead_code)]

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Code {
    Constant = 0,
    Null,
    False,
    True,
    LoadLocal0,
    LoadLocal1,
    LoadLocal2,
    LoadLocal3,
    LoadLocal4,
    LoadLocal5,
    LoadLocal6,
    LoadLocal7,
    LoadLocal8,
    LoadLocal,
    StoreLocal,
    LoadUpvalue,
    StoreUpvalue,
    LoadModuleVar,
    StoreModuleVar,
    LoadFieldThis,
    StoreFieldThis,
    LoadField,
    StoreField,
    Pop,
    Call0,
    Call1,
    Call2,
    Call3,
    Call4,
    Call5,
    Call6,
    Call7,
    Call8,
    Call9,
    Call10,
    Call11,
    Call12,
    Call13,
    Call14,
    Call15,
    Call16,
    Super0,
    Super1,
    Super2,
    Super3,
    Super4,
    Super5,
    Super6,
    Super7,
    Super8,
    Super9,
    Super10,
    Super11,
    Super12,
    Super13,
    Super14,
    Super15,
    Super16,
    Jump,
    Loop,
    JumpIf,
    And,
    Or,
    CloseUpvalue,
    Return,
    Closure,
    Construct,
    ForeignConstruct,
    Class,
    EndClass,
    ForeignClass,
    MethodInstance,
    MethodStatic,
    EndModule,
    ImportModule,
    ImportVariable,
    End,
}

impl Code {
    pub const CALL_0: u8 = Code::Call0 as u8;
    pub const SUPER_0: u8 = Code::Super0 as u8;
    pub const END: u8 = Code::End as u8;
}

pub const CONSTANT: u8 = Code::Constant as u8;
pub const NULL: u8 = Code::Null as u8;
pub const FALSE: u8 = Code::False as u8;
pub const TRUE: u8 = Code::True as u8;
pub const LOAD_LOCAL_0: u8 = Code::LoadLocal0 as u8;
pub const LOAD_LOCAL_1: u8 = Code::LoadLocal1 as u8;
pub const LOAD_LOCAL_2: u8 = Code::LoadLocal2 as u8;
pub const LOAD_LOCAL_3: u8 = Code::LoadLocal3 as u8;
pub const LOAD_LOCAL_4: u8 = Code::LoadLocal4 as u8;
pub const LOAD_LOCAL_5: u8 = Code::LoadLocal5 as u8;
pub const LOAD_LOCAL_6: u8 = Code::LoadLocal6 as u8;
pub const LOAD_LOCAL_7: u8 = Code::LoadLocal7 as u8;
pub const LOAD_LOCAL_8: u8 = Code::LoadLocal8 as u8;
pub const LOAD_LOCAL: u8 = Code::LoadLocal as u8;
pub const STORE_LOCAL: u8 = Code::StoreLocal as u8;
pub const LOAD_UPVALUE: u8 = Code::LoadUpvalue as u8;
pub const STORE_UPVALUE: u8 = Code::StoreUpvalue as u8;
pub const LOAD_MODULE_VAR: u8 = Code::LoadModuleVar as u8;
pub const STORE_MODULE_VAR: u8 = Code::StoreModuleVar as u8;
pub const LOAD_FIELD_THIS: u8 = Code::LoadFieldThis as u8;
pub const STORE_FIELD_THIS: u8 = Code::StoreFieldThis as u8;
pub const LOAD_FIELD: u8 = Code::LoadField as u8;
pub const STORE_FIELD: u8 = Code::StoreField as u8;
pub const POP: u8 = Code::Pop as u8;
pub const CALL_0: u8 = Code::Call0 as u8;
pub const CALL_1: u8 = Code::Call1 as u8;
pub const CALL_2: u8 = Code::Call2 as u8;
pub const CALL_3: u8 = Code::Call3 as u8;
pub const CALL_4: u8 = Code::Call4 as u8;
pub const CALL_5: u8 = Code::Call5 as u8;
pub const CALL_6: u8 = Code::Call6 as u8;
pub const CALL_7: u8 = Code::Call7 as u8;
pub const CALL_8: u8 = Code::Call8 as u8;
pub const CALL_9: u8 = Code::Call9 as u8;
pub const CALL_10: u8 = Code::Call10 as u8;
pub const CALL_11: u8 = Code::Call11 as u8;
pub const CALL_12: u8 = Code::Call12 as u8;
pub const CALL_13: u8 = Code::Call13 as u8;
pub const CALL_14: u8 = Code::Call14 as u8;
pub const CALL_15: u8 = Code::Call15 as u8;
pub const CALL_16: u8 = Code::Call16 as u8;
pub const SUPER_0: u8 = Code::Super0 as u8;
pub const SUPER_1: u8 = Code::Super1 as u8;
pub const SUPER_2: u8 = Code::Super2 as u8;
pub const SUPER_3: u8 = Code::Super3 as u8;
pub const SUPER_4: u8 = Code::Super4 as u8;
pub const SUPER_5: u8 = Code::Super5 as u8;
pub const SUPER_6: u8 = Code::Super6 as u8;
pub const SUPER_7: u8 = Code::Super7 as u8;
pub const SUPER_8: u8 = Code::Super8 as u8;
pub const SUPER_9: u8 = Code::Super9 as u8;
pub const SUPER_10: u8 = Code::Super10 as u8;
pub const SUPER_11: u8 = Code::Super11 as u8;
pub const SUPER_12: u8 = Code::Super12 as u8;
pub const SUPER_13: u8 = Code::Super13 as u8;
pub const SUPER_14: u8 = Code::Super14 as u8;
pub const SUPER_15: u8 = Code::Super15 as u8;
pub const SUPER_16: u8 = Code::Super16 as u8;
pub const JUMP: u8 = Code::Jump as u8;
pub const LOOP: u8 = Code::Loop as u8;
pub const JUMP_IF: u8 = Code::JumpIf as u8;
pub const AND: u8 = Code::And as u8;
pub const OR: u8 = Code::Or as u8;
pub const CLOSE_UPVALUE: u8 = Code::CloseUpvalue as u8;
pub const RETURN: u8 = Code::Return as u8;
pub const CLOSURE: u8 = Code::Closure as u8;
pub const CONSTRUCT: u8 = Code::Construct as u8;
pub const FOREIGN_CONSTRUCT: u8 = Code::ForeignConstruct as u8;
pub const CLASS: u8 = Code::Class as u8;
pub const END_CLASS: u8 = Code::EndClass as u8;
pub const FOREIGN_CLASS: u8 = Code::ForeignClass as u8;
pub const METHOD_INSTANCE: u8 = Code::MethodInstance as u8;
pub const METHOD_STATIC: u8 = Code::MethodStatic as u8;
pub const END_MODULE: u8 = Code::EndModule as u8;
pub const IMPORT_MODULE: u8 = Code::ImportModule as u8;
pub const IMPORT_VARIABLE: u8 = Code::ImportVariable as u8;
pub const END: u8 = Code::End as u8;

/// Stack effect of each opcode, indexed by opcode value (mirrors
/// stackEffects[] in wren_compiler.c).
pub const STACK_EFFECTS: [i32; 77] = [
    1,   // CONSTANT
    1,   // NULL
    1,   // FALSE
    1,   // TRUE
    1,   // LOAD_LOCAL_0
    1,   // LOAD_LOCAL_1
    1,   // LOAD_LOCAL_2
    1,   // LOAD_LOCAL_3
    1,   // LOAD_LOCAL_4
    1,   // LOAD_LOCAL_5
    1,   // LOAD_LOCAL_6
    1,   // LOAD_LOCAL_7
    1,   // LOAD_LOCAL_8
    1,   // LOAD_LOCAL
    0,   // STORE_LOCAL
    1,   // LOAD_UPVALUE
    0,   // STORE_UPVALUE
    1,   // LOAD_MODULE_VAR
    0,   // STORE_MODULE_VAR
    1,   // LOAD_FIELD_THIS
    0,   // STORE_FIELD_THIS
    0,   // LOAD_FIELD
    -1,  // STORE_FIELD
    -1,  // POP
    0,   // CALL_0
    -1,  // CALL_1
    -2,  // CALL_2
    -3,  // CALL_3
    -4,  // CALL_4
    -5,  // CALL_5
    -6,  // CALL_6
    -7,  // CALL_7
    -8,  // CALL_8
    -9,  // CALL_9
    -10, // CALL_10
    -11, // CALL_11
    -12, // CALL_12
    -13, // CALL_13
    -14, // CALL_14
    -15, // CALL_15
    -16, // CALL_16
    0,   // SUPER_0
    -1,  // SUPER_1
    -2,  // SUPER_2
    -3,  // SUPER_3
    -4,  // SUPER_4
    -5,  // SUPER_5
    -6,  // SUPER_6
    -7,  // SUPER_7
    -8,  // SUPER_8
    -9,  // SUPER_9
    -10, // SUPER_10
    -11, // SUPER_11
    -12, // SUPER_12
    -13, // SUPER_13
    -14, // SUPER_14
    -15, // SUPER_15
    -16, // SUPER_16
    0,   // JUMP
    0,   // LOOP
    -1,  // JUMP_IF
    -1,  // AND
    -1,  // OR
    -1,  // CLOSE_UPVALUE
    0,   // RETURN
    1,   // CLOSURE
    0,   // CONSTRUCT
    0,   // FOREIGN_CONSTRUCT
    -1,  // CLASS
    -2,  // END_CLASS
    -1,  // FOREIGN_CLASS
    -2,  // METHOD_INSTANCE
    -2,  // METHOD_STATIC
    1,   // END_MODULE
    1,   // IMPORT_MODULE
    1,   // IMPORT_VARIABLE
    0,   // END
];
