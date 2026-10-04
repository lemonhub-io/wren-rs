// The dynamically-typed value representation and heap object model, ported
// from wren_value.h/.c. Values are NaN-tagged u64s exactly as in the C
// implementation; object pointers are replaced by 32-bit indices into the
// VM's heap array (see [Heap]).

use std::mem;

pub type ObjRef = u32;

/// Sentinel for "no object" where the C code would use NULL.
pub const NULL_OBJ: ObjRef = u32::MAX;

pub const SIGN_BIT: u64 = 1 << 63;
pub const QNAN: u64 = 0x7ffc000000000000;

const TAG_NULL: u64 = 1;
const TAG_FALSE: u64 = 2;
const TAG_TRUE: u64 = 3;
const TAG_UNDEFINED: u64 = 4;

pub const NULL_VAL: Value = Value(QNAN | TAG_NULL);
pub const FALSE_VAL: Value = Value(QNAN | TAG_FALSE);
pub const TRUE_VAL: Value = Value(QNAN | TAG_TRUE);
pub const UNDEFINED_VAL: Value = Value(QNAN | TAG_UNDEFINED);

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Value(pub u64);

impl Value {
    #[inline]
    pub fn null() -> Value {
        NULL_VAL
    }
    #[inline]
    pub fn undefined() -> Value {
        UNDEFINED_VAL
    }
    #[inline]
    pub fn bool(b: bool) -> Value {
        if b {
            TRUE_VAL
        } else {
            FALSE_VAL
        }
    }
    #[inline]
    pub fn num(n: f64) -> Value {
        Value(n.to_bits())
    }
    #[inline]
    pub fn obj(r: ObjRef) -> Value {
        Value(SIGN_BIT | QNAN | r as u64)
    }

    #[inline]
    pub fn is_num(self) -> bool {
        (self.0 & QNAN) != QNAN
    }
    #[inline]
    pub fn is_obj(self) -> bool {
        (self.0 & (QNAN | SIGN_BIT)) == (QNAN | SIGN_BIT)
    }
    #[inline]
    pub fn is_false(self) -> bool {
        self.0 == FALSE_VAL.0
    }
    #[inline]
    pub fn is_null(self) -> bool {
        self.0 == NULL_VAL.0
    }
    #[inline]
    pub fn is_undefined(self) -> bool {
        self.0 == UNDEFINED_VAL.0
    }
    #[inline]
    pub fn is_bool(self) -> bool {
        self.0 == TRUE_VAL.0 || self.0 == FALSE_VAL.0
    }

    #[inline]
    pub fn as_num(self) -> f64 {
        f64::from_bits(self.0)
    }
    #[inline]
    pub fn as_bool(self) -> bool {
        self.0 == TRUE_VAL.0
    }
    #[inline]
    pub fn as_obj(self) -> ObjRef {
        debug_assert!(self.is_obj());
        (self.0 & !(QNAN | SIGN_BIT)) as ObjRef
    }
    /// Same as as_obj but without the debug assertion (used on paths where the
    /// value is known to be an object by construction).
    #[inline]
    pub fn as_obj_unchecked(self) -> ObjRef {
        (self.0 & !(QNAN | SIGN_BIT)) as ObjRef
    }
}

// -- ObjType enum ------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObjType {
    Class,
    Closure,
    Fiber,
    Fn,
    Foreign,
    Instance,
    List,
    Map,
    Module,
    Range,
    String,
    Upvalue,
}

// -- Object payloads ---------------------------------------------------------

#[derive(Clone)]
pub struct ObjString {
    pub hash: u32,
    pub bytes: Vec<u8>, // NUL terminator NOT stored; helpers append as needed
}

impl ObjString {
    /// The NUL-terminated view used by C-style consumers. Callers must ensure
    /// `out` has room; here we build on the fly in helpers instead.
    pub fn value(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Copy)]
pub enum UpvalueLoc {
    /// The upvalue points at `stack[slot]` of `fiber`.
    Open {
        fiber: ObjRef,
        slot: usize,
    },
    Closed,
}

#[derive(Clone)]
pub struct ObjUpvalue {
    pub loc: UpvalueLoc,
    pub closed: Value,
    pub next: ObjRef, // next open upvalue in the fiber's list, NULL_OBJ if end
}

pub type PrimitiveFn = fn(&mut crate::WrenVM, usize) -> bool;
pub type ForeignMethodFn = fn(&mut crate::WrenVM);

#[derive(Clone)]
pub struct FnDebug {
    pub name: String,
    pub source_lines: Vec<i32>,
}

#[derive(Clone)]
pub struct ObjFn {
    pub code: Vec<u8>,
    pub constants: Vec<Value>,
    pub module: ObjRef, // ObjModule
    pub max_slots: i32,
    pub num_upvalues: i32,
    pub arity: i32,
    pub debug: Option<Box<FnDebug>>,
}

#[derive(Clone)]
pub struct ObjClosure {
    pub fn_ref: ObjRef,
    pub upvalues: Vec<ObjRef>, // ObjUpvalue refs
}

#[derive(Clone, Copy)]
pub struct CallFrame {
    /// Byte offset into the fn's code of the next instruction to execute.
    pub ip: usize,
    pub closure: ObjRef,
    /// Index into fiber.stack of the first slot used by this frame.
    pub stack_start: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FiberState {
    Try,
    Root,
    Other,
}

#[derive(Clone)]
pub struct ObjFiber {
    /// Fixed-capacity stack. `stack.len()` is the allocated capacity.
    pub stack: Vec<Value>,
    /// Index one past the top-most slot in `stack`.
    pub stack_top: usize,
    pub frames: Vec<CallFrame>,
    /// Head of open-upvalue list (closest to stack top), NULL_OBJ if none.
    pub open_upvalues: ObjRef,
    pub caller: ObjRef, // NULL_OBJ if none
    pub error: Value,
    pub state: FiberState,
}

impl ObjFiber {
    #[inline]
    pub fn stack_capacity(&self) -> usize {
        self.stack.len()
    }
    #[inline]
    pub fn num_frames(&self) -> usize {
        self.frames.len()
    }
}

#[derive(Clone)]
pub enum Method {
    Primitive(PrimitiveFn),
    FunctionCall(PrimitiveFn),
    Foreign(ForeignMethodFn),
    Block(ObjRef), // ObjClosure
    /// The "<finalize>" pseudo-method: a WrenFinalizerFn stored in the method
    /// table. Never callable from code.
    Finalizer(fn(&mut [u8])),
    None,
}

#[derive(Clone)]
pub struct ObjClass {
    pub superclass: ObjRef, // NULL_OBJ if none
    pub num_fields: i32,    // -1 for foreign classes
    pub methods: Vec<Method>,
    pub name: ObjRef, // ObjString
    pub attributes: Value,
}

#[derive(Clone)]
pub struct ObjForeign {
    pub data: Vec<u8>,
}

#[derive(Clone)]
pub struct ObjInstance {
    pub fields: Vec<Value>,
}

#[derive(Clone)]
pub struct ObjList {
    pub elements: Vec<Value>,
}

#[derive(Clone, Copy)]
pub struct MapEntry {
    /// UNDEFINED_VAL if the entry is not in use.
    pub key: Value,
    /// If key is UNDEFINED_VAL: FALSE = open entry, TRUE = tombstone.
    pub value: Value,
}

#[derive(Clone)]
pub struct ObjMap {
    pub entries: Vec<MapEntry>,
    pub count: usize,
}

#[derive(Clone)]
pub struct ObjRange {
    pub from: f64,
    pub to: f64,
    pub is_inclusive: bool,
}

#[derive(Clone)]
pub struct ObjModule {
    pub variables: Vec<Value>,
    /// Names of module variables; indexes correspond to `variables`.
    pub variable_names: Vec<ObjRef>, // ObjString refs
    pub name: ObjRef, // ObjString or NULL_OBJ
}

// -- Heap --------------------------------------------------------------------

#[derive(Clone)]
pub enum ObjData {
    Class(ObjClass),
    Closure(ObjClosure),
    Fiber(ObjFiber),
    Fn(ObjFn),
    Foreign(ObjForeign),
    Instance(ObjInstance),
    List(ObjList),
    Map(ObjMap),
    Module(ObjModule),
    Range(ObjRange),
    Str(ObjString),
    Upvalue(ObjUpvalue),
}

pub struct Obj {
    pub is_dark: bool,
    /// The object's class. NULL_OBJ during bootstrap for objects created
    /// before the core classes exist.
    pub class: ObjRef,
    pub data: ObjData,
}

impl Obj {
    pub fn obj_type(&self) -> ObjType {
        match &self.data {
            ObjData::Class(_) => ObjType::Class,
            ObjData::Closure(_) => ObjType::Closure,
            ObjData::Fiber(_) => ObjType::Fiber,
            ObjData::Fn(_) => ObjType::Fn,
            ObjData::Foreign(_) => ObjType::Foreign,
            ObjData::Instance(_) => ObjType::Instance,
            ObjData::List(_) => ObjType::List,
            ObjData::Map(_) => ObjType::Map,
            ObjData::Module(_) => ObjType::Module,
            ObjData::Range(_) => ObjType::Range,
            ObjData::Str(_) => ObjType::String,
            ObjData::Upvalue(_) => ObjType::Upvalue,
        }
    }
}

pub struct Heap {
    pub objs: Vec<Option<Obj>>,
    pub free_list: Vec<ObjRef>,
}

impl Heap {
    pub fn new() -> Heap {
        Heap {
            objs: Vec::with_capacity(1024),
            free_list: Vec::new(),
        }
    }

    pub fn alloc(&mut self, data: ObjData, class: ObjRef) -> ObjRef {
        let obj = Obj {
            is_dark: false,
            class,
            data,
        };
        if let Some(idx) = self.free_list.pop() {
            self.objs[idx as usize] = Some(obj);
            idx
        } else {
            self.objs.push(Some(obj));
            (self.objs.len() - 1) as ObjRef
        }
    }

    /// Frees the slot. Callers must have already run finalizers etc.
    pub fn free(&mut self, idx: ObjRef) {
        self.objs[idx as usize] = None;
        self.free_list.push(idx);
    }

    #[inline]
    pub fn get(&self, r: ObjRef) -> &Obj {
        self.objs[r as usize].as_ref().expect("freed object")
    }
    #[inline]
    pub fn get_mut(&mut self, r: ObjRef) -> &mut Obj {
        self.objs[r as usize].as_mut().expect("freed object")
    }
    #[inline]
    pub fn contains(&self, r: ObjRef) -> bool {
        r != NULL_OBJ && (r as usize) < self.objs.len() && self.objs[r as usize].is_some()
    }
}

// -- typed accessors on the heap (mirror the AS_* macros) ---------------------

macro_rules! obj_accessors {
    ($get_name:ident, $get_mut_name:ident, $variant:ident, $ty:ty) => {
        impl Heap {
            #[inline]
            pub fn $get_name(&self, r: ObjRef) -> &$ty {
                match &self.get(r).data {
                    ObjData::$variant(v) => v,
                    _ => panic!("bad object access: wrong type"),
                }
            }
            #[inline]
            pub fn $get_mut_name(&mut self, r: ObjRef) -> &mut $ty {
                match &mut self.get_mut(r).data {
                    ObjData::$variant(v) => v,
                    _ => panic!("bad object access: wrong type"),
                }
            }
        }
    };
}

obj_accessors!(class, class_mut, Class, ObjClass);
obj_accessors!(closure, closure_mut, Closure, ObjClosure);
obj_accessors!(fiber, fiber_mut, Fiber, ObjFiber);
obj_accessors!(func, func_mut, Fn, ObjFn);
obj_accessors!(foreign, foreign_mut, Foreign, ObjForeign);
obj_accessors!(instance, instance_mut, Instance, ObjInstance);
obj_accessors!(list, list_mut, List, ObjList);
obj_accessors!(map, map_mut, Map, ObjMap);
obj_accessors!(module, module_mut, Module, ObjModule);
obj_accessors!(range, range_mut, Range, ObjRange);
obj_accessors!(string, string_mut, Str, ObjString);
obj_accessors!(upvalue, upvalue_mut, Upvalue, ObjUpvalue);

// -- hashing & equality --------------------------------------------------------

pub fn hash_bits(hash: u64) -> u32 {
    // From wren_value.c: hashBits (Thomas Wang integer hash).
    let mut hash = hash;
    hash = (!hash).wrapping_add(hash << 18);
    hash ^= hash >> 31;
    hash = hash.wrapping_mul(21);
    hash ^= hash >> 11;
    hash = hash.wrapping_add(hash << 6);
    hash ^= hash >> 22;
    (hash & 0x3fffffff) as u32
}

fn hash_number(num: f64) -> u32 {
    hash_bits(num.to_bits())
}

pub fn hash_string(bytes: &[u8]) -> u32 {
    // FNV-1a
    let mut hash: u32 = 2166136261;
    for &b in bytes {
        hash ^= b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    hash
}

fn hash_object(heap: &Heap, r: ObjRef) -> u32 {
    let obj = heap.get(r);
    match &obj.data {
        ObjData::Class(c) => {
            // Classes just use their name.
            hash_object(heap, c.name)
        }
        ObjData::Fn(f) => hash_number(f.arity as f64) ^ hash_number(f.code.len() as f64),
        ObjData::Str(s) => s.hash,
        ObjData::Range(range) => hash_number(range.from) ^ hash_number(range.to),
        _ => {
            // Objects that are compared only by identity use their index.
            // (C uses the pointer address; index serves the same role.)
            hash_number(r as f64)
        }
    }
}

pub fn hash_value(heap: &Heap, value: Value) -> u32 {
    if value.is_obj() {
        hash_object(heap, value.as_obj_unchecked())
    } else {
        hash_bits(value.0)
    }
}

/// wrenValuesSame: bitwise equality.
#[inline]
pub fn values_same(a: Value, b: Value) -> bool {
    a.0 == b.0
}

/// wrenValuesEqual: deep equality for ranges and strings.
pub fn values_equal(heap: &Heap, a: Value, b: Value) -> bool {
    if values_same(a, b) {
        return true;
    }
    if !a.is_obj() || !b.is_obj() {
        return false;
    }
    let (oa, ob) = (a.as_obj_unchecked(), b.as_obj_unchecked());
    let (ta, tb) = (&heap.get(oa).data, &heap.get(ob).data);
    match (ta, tb) {
        (ObjData::Range(ra), ObjData::Range(rb)) => {
            ra.from == rb.from && ra.to == rb.to && ra.is_inclusive == rb.is_inclusive
        }
        (ObjData::Str(sa), ObjData::Str(sb)) => sa.bytes == sb.bytes,
        _ => false,
    }
}

/// wrenMapIsValidKey.
pub fn map_is_valid_key(heap: &Heap, key: Value) -> bool {
    if key.is_bool() || key.is_null() || key.is_num() {
        return true;
    }
    if key.is_obj() {
        matches!(
            heap.get(key.as_obj_unchecked()).data,
            ObjData::Class(_) | ObjData::Range(_) | ObjData::Str(_)
        )
    } else {
        false
    }
}

// -- memory size accounting (approximates the C accounting for GC pacing) ----

pub fn obj_size(obj: &Obj) -> usize {
    let base = mem::size_of::<Obj>();
    base + match &obj.data {
        ObjData::Class(c) => c.methods.len() * mem::size_of::<Method>(),
        ObjData::Closure(c) => c.upvalues.len() * mem::size_of::<ObjRef>(),
        ObjData::Fiber(f) => {
            f.stack.len() * mem::size_of::<Value>() + f.frames.len() * mem::size_of::<CallFrame>()
        }
        ObjData::Fn(f) => {
            f.code.len()
                + f.constants.len() * mem::size_of::<Value>()
                + f.debug
                    .as_ref()
                    .map(|d| d.source_lines.len() * 4 + d.name.len())
                    .unwrap_or(0)
        }
        ObjData::Foreign(fo) => fo.data.len(),
        ObjData::Instance(i) => i.fields.len() * mem::size_of::<Value>(),
        ObjData::List(l) => l.elements.len() * mem::size_of::<Value>(),
        ObjData::Map(m) => m.entries.len() * mem::size_of::<MapEntry>(),
        ObjData::Module(m) => {
            m.variables.len() * mem::size_of::<Value>()
                + m.variable_names.len() * mem::size_of::<ObjRef>()
        }
        ObjData::Range(_) => 0,
        ObjData::Str(s) => s.bytes.len(),
        ObjData::Upvalue(_) => 0,
    }
}
