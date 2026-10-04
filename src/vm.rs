// Port of wren_vm.c + wren_value.c + wren.h: the WrenVM type, configuration,
// garbage collector, object constructors, module machinery, the bytecode
// interpreter, and the embedding API.

use std::time::Instant;

use crate::compiler::{Compiler, Parser};
use crate::opcodes::*;
use crate::utils::*;
use crate::value::*;

pub const WREN_VERSION_NUMBER: i32 = 4000;
pub const WREN_VERSION_STRING: &str = "0.4.0";
const MAX_MODULE_VARS: usize = 65536;
const MAX_FIELDS: i32 = 255;
const MAX_METHODS: i32 = 65535;
const MAP_LOAD_PERCENT: usize = 75;
const GROW_FACTOR: usize = 2;
const MIN_CAPACITY: usize = 16;
const INITIAL_CALL_FRAMES: usize = 4;

pub use crate::value::ObjType;

// -- API enums -----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WrenInterpretResult {
    Success,
    CompileError,
    RuntimeError,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WrenType {
    Bool,
    Num,
    Foreign,
    List,
    Map,
    Null,
    Str,
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WrenErrorType {
    Compile,
    Runtime,
    StackTrace,
}

// -- Configuration callbacks -----------------------------------------------------

pub type WrenReallocateFn = fn(*mut u8, usize, usize) -> *mut u8;
pub type WrenForeignMethodFn = fn(&mut WrenVM);
pub type WrenFinalizerFn = fn(&mut [u8]);
pub type WrenResolveModuleFn = fn(&mut WrenVM, &[u8], &[u8]) -> Option<Vec<u8>>;
pub type WrenLoadModuleCompleteFn = fn(&mut WrenVM, &[u8]);
pub type WrenLoadModuleFn = fn(&mut WrenVM, &[u8]) -> WrenLoadModuleResult;
pub type WrenBindForeignMethodFn =
    fn(&mut WrenVM, &[u8], &[u8], bool, &[u8]) -> Option<WrenForeignMethodFn>;
pub type WrenWriteFn = fn(&mut WrenVM, &[u8]);
pub type WrenErrorFn = fn(&mut WrenVM, WrenErrorType, Option<&[u8]>, i32, &[u8]);
pub type WrenBindForeignClassFn = fn(&mut WrenVM, &[u8], &[u8]) -> WrenForeignClassMethods;

#[derive(Clone, Copy, Default)]
pub struct WrenForeignClassMethods {
    pub allocate: Option<WrenForeignMethodFn>,
    pub finalize: Option<WrenFinalizerFn>,
}

pub struct WrenLoadModuleResult {
    pub source: Option<Vec<u8>>,
    pub on_complete: Option<WrenLoadModuleCompleteFn>,
}

impl WrenLoadModuleResult {
    pub fn none() -> WrenLoadModuleResult {
        WrenLoadModuleResult {
            source: None,
            on_complete: None,
        }
    }
}

pub struct WrenConfiguration {
    /// Mirrors WrenReallocateFn. Called once for the VM's own bookkeeping
    /// block (the Rust port manages internal memory with Vec/Box; the hook is
    /// invoked so embedders who install a custom allocator still see traffic
    /// and can vet userData).
    pub reallocate_fn: WrenReallocateFn,
    pub resolve_module_fn: Option<WrenResolveModuleFn>,
    pub load_module_fn: Option<WrenLoadModuleFn>,
    pub bind_foreign_method_fn: Option<WrenBindForeignMethodFn>,
    pub bind_foreign_class_fn: Option<WrenBindForeignClassFn>,
    pub write_fn: Option<WrenWriteFn>,
    pub error_fn: Option<WrenErrorFn>,
    pub initial_heap_size: usize,
    pub min_heap_size: usize,
    pub heap_growth_percent: i64,
    pub user_data: usize, // opaque host pointer
}

fn default_reallocate(ptr: *mut u8, new_size: usize, _user: usize) -> *mut u8 {
    if new_size == 0 {
        if !ptr.is_null() {
            unsafe {
                drop(Vec::from_raw_parts(ptr, 0, 0));
            }
        }
        return std::ptr::null_mut();
    }
    unsafe {
        let layout = std::alloc::Layout::from_size_align(new_size, 8).unwrap();
        if ptr.is_null() {
            std::alloc::alloc(layout)
        } else {
            std::alloc::realloc(ptr, layout, new_size)
        }
    }
}

impl Default for WrenConfiguration {
    fn default() -> Self {
        Self::new()
    }
}

impl WrenConfiguration {
    /// wrenInitConfiguration.
    pub fn new() -> Self {
        WrenConfiguration {
            reallocate_fn: default_reallocate,
            resolve_module_fn: None,
            load_module_fn: None,
            bind_foreign_method_fn: None,
            bind_foreign_class_fn: None,
            write_fn: None,
            error_fn: None,
            initial_heap_size: 1024 * 1024 * 10,
            min_heap_size: 1024 * 1024,
            heap_growth_percent: 50,
            user_data: 0,
        }
    }
}

// -- WrenHandle -------------------------------------------------------------------

/// A handle is an opaque id into vm.handles.
pub type WrenHandle = u32;

// -- WrenVM ------------------------------------------------------------------------

pub struct WrenVM {
    pub heap: Heap,

    pub bool_class: ObjRef,
    pub class_class: ObjRef,
    pub fiber_class: ObjRef,
    pub fn_class: ObjRef,
    pub list_class: ObjRef,
    pub map_class: ObjRef,
    pub null_class: ObjRef,
    pub num_class: ObjRef,
    pub object_class: ObjRef,
    pub range_class: ObjRef,
    pub string_class: ObjRef,

    /// The currently running fiber.
    pub fiber: ObjRef, // NULL_OBJ if none
    /// Loaded modules map (ObjMap).
    pub modules: ObjRef,
    /// Most recently imported module.
    pub last_module: ObjRef,

    pub bytes_allocated: usize,
    pub next_gc: usize,

    /// Gray stack during collection.
    pub gray: Vec<ObjRef>,
    /// Temporary roots.
    pub temp_roots: Vec<ObjRef>,
    /// Open handles: id -> index in this Vec (None = released).
    pub handles: Vec<Option<Value>>,
    free_handles: Vec<u32>,

    /// API stack: (fiber, base index into that fiber's stack).
    pub api_stack: Option<(ObjRef, usize)>,

    pub config: WrenConfiguration,

    /// Compiler state during wrenCompile (see compiler.rs).
    pub(crate) compilers: Vec<Compiler>,
    pub(crate) parser: Option<Parser>,

    /// Global method-name symbol table (Vec<ObjString ref>).
    pub method_names: Vec<ObjRef>,

    /// For tracking allocation of non-heap buffers (approximates the C
    /// wrenReallocate accounting for GC pacing).
    pub start_time: Instant,

    /// The pointer returned by the host's reallocateFn for our bookkeeping
    /// block, freed in wrenFreeVM.
    alloc_block: *mut u8,
}

// -- helpers for C-style semantics -------------------------------------------------

/// (uint32_t)(double) — mirrors x86 cvttsd2si semantics for out-of-range/NaN.
#[inline]
pub fn num_to_u32(d: f64) -> u32 {
    num_to_i64_c(d) as u32
}

#[inline]
pub fn num_to_i64_c(d: f64) -> i64 {
    if d.is_nan() || d >= 9.223372036854776e18 || d <= -9.223372036854776e18 {
        i64::MIN // x86 cvttsd2si "integer indefinite"
    } else {
        d as i64
    }
}

#[inline]
pub fn num_to_i32_c(d: f64) -> i32 {
    num_to_i64_c(d) as i32
}

/// %.Ng formatting (default %g). Mirrors C printf %g semantics.
pub fn format_g(value: f64, precision: usize) -> String {
    let p = if precision == 0 { 1 } else { precision };
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "inf".to_string()
        } else {
            "-inf".to_string()
        };
    }
    if value == 0.0 {
        // Covers +0 and -0: C prints "0" or "-0".
        if value.is_sign_negative() {
            return "-0".to_string();
        }
        return "0".to_string();
    }

    // Get the decimal exponent X via scientific formatting with P-1 digits.
    let sci = format!("{:.*e}", p - 1, value.abs());
    // Rust gives "d.ddddde±X" — exponent without leading zeros.
    let epos = sci.rfind('e').unwrap();
    let exp: i32 = sci[epos + 1..].parse().unwrap();

    let mut out = String::new();
    if value < 0.0 {
        out.push('-');
    }
    if exp < -4 || exp >= p as i32 {
        // Scientific style: P-1 fractional digits, strip trailing zeros.
        let mantissa = &sci[..epos];
        let mut m = mantissa.to_string();
        if m.contains('.') {
            while m.ends_with('0') {
                m.pop();
            }
            if m.ends_with('.') {
                m.pop();
            }
        }
        out.push_str(&m);
        out.push('e');
        if exp >= 0 {
            out.push('+');
        } else {
            out.push('-');
        }
        let ae = exp.abs();
        if ae < 10 {
            out.push('0');
        }
        out.push_str(&ae.to_string());
    } else {
        // Fixed style: precision P-1-X.
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        let mut m = format!("{:.*}", decimals, value.abs());
        if m.contains('.') {
            while m.ends_with('0') {
                m.pop();
            }
            if m.ends_with('.') {
                m.pop();
            }
        }
        out.push_str(&m);
    }
    out
}

/// %.14g — used by wrenNumToString.
pub fn format_num(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "infinity".to_string()
        } else {
            "-infinity".to_string()
        };
    }
    format_g(value, 14)
}

/// Arguments for string_format: `$` = byte string, `@` = ObjString Value.
pub enum Fmt<'a> {
    S(&'a [u8]),
    V(Value),
}

impl WrenVM {
    pub fn new_vm(config: Option<WrenConfiguration>) -> Box<WrenVM> {
        let config = config.unwrap_or_else(WrenConfiguration::new);
        let mut vm = Box::new(WrenVM {
            heap: Heap::new(),
            bool_class: NULL_OBJ,
            class_class: NULL_OBJ,
            fiber_class: NULL_OBJ,
            fn_class: NULL_OBJ,
            list_class: NULL_OBJ,
            map_class: NULL_OBJ,
            null_class: NULL_OBJ,
            num_class: NULL_OBJ,
            object_class: NULL_OBJ,
            range_class: NULL_OBJ,
            string_class: NULL_OBJ,
            fiber: NULL_OBJ,
            modules: NULL_OBJ,
            last_module: NULL_OBJ,
            bytes_allocated: 0,
            next_gc: config.initial_heap_size,
            gray: Vec::with_capacity(4),
            temp_roots: Vec::with_capacity(8),
            handles: Vec::new(),
            free_handles: Vec::new(),
            api_stack: None,
            config,
            compilers: Vec::new(),
            parser: None,
            method_names: Vec::new(),
            start_time: Instant::now(),
            alloc_block: std::ptr::null_mut(),
        });
        // Exercise the host allocator for the VM bookkeeping block.
        let reallocate = vm.config.reallocate_fn;
        let ud = vm.config.user_data;
        vm.alloc_block = reallocate(std::ptr::null_mut(), 16, ud);

        vm.modules = vm.new_map();
        crate::core::initialize_core(&mut vm);
        vm
    }

    /// wrenFreeVM.
    pub fn free_vm(&mut self) {
        // Finalize foreign objects first — finalizers read method names and
        // class tables that must not be freed yet (in C this ordering hazard
        // is a benign use-after-free; our index heap panics instead).
        for i in 0..self.heap.objs.len() {
            if let Some(o) = &self.heap.objs[i] {
                if matches!(o.data, ObjData::Foreign(_)) {
                    self.finalize_foreign(i as ObjRef);
                }
            }
        }
        for i in 0..self.heap.objs.len() {
            if self.heap.objs[i].is_some() {
                self.heap.free(i as ObjRef);
            }
        }
        if !self.alloc_block.is_null() {
            let reallocate = self.config.reallocate_fn;
            let ud = self.config.user_data;
            reallocate(self.alloc_block, 0, ud);
            self.alloc_block = std::ptr::null_mut();
        }
    }

    // -- allocation tracking / GC pacing --------------------------------------------

    /// Count `bytes` of new memory and collect if we passed the GC threshold.
    /// Mirrors wrenReallocate's accounting.
    #[inline]
    pub(crate) fn track_alloc(&mut self, bytes: usize) {
        self.bytes_allocated += bytes;
        if self.bytes_allocated > self.next_gc {
            self.collect_garbage();
        }
    }

    /// Allocates an object. Mirrors ALLOCATE: may trigger GC.
    pub(crate) fn alloc_obj(&mut self, data: ObjData, class: ObjRef) -> ObjRef {
        self.bytes_allocated += 1;
        if self.bytes_allocated > self.next_gc {
            self.collect_garbage();
        }
        let r = self.heap.alloc(data, class);
        r
    }

    pub fn collect_garbage(&mut self) {
        self.bytes_allocated = 0;

        // Roots.
        self.gray_obj(self.modules);
        let roots: Vec<ObjRef> = self.temp_roots.clone();
        for r in roots {
            self.gray_obj(r);
        }
        if self.fiber != NULL_OBJ {
            self.gray_obj(self.fiber);
        }
        for h in self.handles.clone() {
            if let Some(v) = h {
                self.gray_value(v);
            }
        }
        self.mark_compiler();
        // Method names.
        let names = self.method_names.clone();
        for r in names {
            self.gray_obj(r);
        }

        self.blacken_objects();

        // Sweep: finalize doomed foreign objects before freeing anything so
        // finalizers can still read method names and class tables.
        for i in 0..self.heap.objs.len() {
            if let Some(o) = &self.heap.objs[i] {
                if !o.is_dark && matches!(o.data, ObjData::Foreign(_)) {
                    self.finalize_foreign(i as ObjRef);
                }
            }
        }
        for i in 0..self.heap.objs.len() {
            if let Some(o) = &mut self.heap.objs[i] {
                if !o.is_dark {
                    self.heap.free(i as ObjRef);
                } else {
                    o.is_dark = false;
                }
            }
        }

        self.next_gc = self.bytes_allocated
            + (self.bytes_allocated * self.config.heap_growth_percent as usize) / 100;
        if self.next_gc < self.config.min_heap_size {
            self.next_gc = self.config.min_heap_size;
        }
    }

    fn finalize_foreign(&mut self, foreign: ObjRef) {
        let symbol = self.find_method_symbol(b"<finalize>");
        if symbol == -1 {
            return;
        }
        let class = self.heap.get(foreign).class;
        if symbol as usize >= self.heap.class(class).methods.len() {
            return;
        }
        let method = self.heap.class(class).methods[symbol as usize].clone();
        // The <finalize> slot stores a Method::Finalizer (bound in
        // bind_foreign_class); finalizers receive only the data, not the VM.
        if let Method::Finalizer(fin) = method {
            let mut data = std::mem::take(&mut self.heap.foreign_mut(foreign).data);
            fin(&mut data);
        }
    }

    // -- marking ---------------------------------------------------------------------

    pub(crate) fn gray_obj(&mut self, r: ObjRef) {
        if r == NULL_OBJ {
            return;
        }
        if !self.heap.contains(r) {
            return;
        }
        if self.heap.get(r).is_dark {
            return;
        }
        self.heap.get_mut(r).is_dark = true;
        self.gray.push(r);
    }

    pub(crate) fn gray_value(&mut self, v: Value) {
        if v.is_obj() {
            self.gray_obj(v.as_obj_unchecked());
        }
    }

    fn blacken_objects(&mut self) {
        while let Some(r) = self.gray.pop() {
            self.blacken_object(r);
        }
    }

    fn blacken_object(&mut self, r: ObjRef) {
        // Snapshot object type + class then mark children.
        let (class, data_tag) = {
            let o = self.heap.get(r);
            (o.class, o.obj_type())
        };
        if class != NULL_OBJ {
            self.gray_obj(class);
        }
        match data_tag {
            ObjType::Class => {
                let (superclass, name, attributes, methods) = {
                    let c = self.heap.class(r);
                    (c.superclass, c.name, c.attributes, c.methods.clone())
                };
                if superclass != NULL_OBJ {
                    self.gray_obj(superclass);
                }
                for m in &methods {
                    if let Method::Block(cl) = m {
                        self.gray_obj(*cl);
                    }
                }
                self.gray_obj(name);
                if !attributes.is_null() && attributes.is_obj() {
                    self.gray_obj(attributes.as_obj_unchecked());
                }
                self.bytes_allocated +=
                    std::mem::size_of::<ObjClass>() + methods.len() * std::mem::size_of::<Method>();
            }
            ObjType::Closure => {
                let (fn_ref, upvalues) = {
                    let c = self.heap.closure(r);
                    (c.fn_ref, c.upvalues.clone())
                };
                self.gray_obj(fn_ref);
                for u in upvalues {
                    self.gray_obj(u);
                }
                self.bytes_allocated += std::mem::size_of::<ObjClosure>();
            }
            ObjType::Fiber => {
                let (frames, stack_top, stack, open_ups, caller, error) = {
                    let f = self.heap.fiber(r);
                    (
                        f.frames.clone(),
                        f.stack_top,
                        f.stack[..f.stack_top.min(f.stack.len())].to_vec(),
                        f.open_upvalues,
                        f.caller,
                        f.error,
                    )
                };
                for fr in frames {
                    self.gray_obj(fr.closure);
                }
                for v in stack {
                    self.gray_value(v);
                }
                let _ = stack_top;
                let mut up = open_ups;
                while up != NULL_OBJ {
                    self.gray_obj(up);
                    up = self.heap.upvalue(up).next;
                }
                if caller != NULL_OBJ {
                    self.gray_obj(caller);
                }
                self.gray_value(error);
                self.bytes_allocated += std::mem::size_of::<ObjFiber>();
            }
            ObjType::Fn => {
                let (constants, module) = {
                    let f = self.heap.func(r);
                    (f.constants.clone(), f.module)
                };
                for v in constants {
                    self.gray_value(v);
                }
                if module != NULL_OBJ {
                    self.gray_obj(module);
                }
                self.bytes_allocated += std::mem::size_of::<ObjFn>();
            }
            ObjType::Foreign => {}
            ObjType::Instance => {
                let num_fields = {
                    let o = self.heap.get(r);
                    let c = self.heap.class(o.class);
                    c.num_fields
                };
                let fields: Vec<Value> = {
                    let i = self.heap.instance(r);
                    i.fields[..num_fields.max(0) as usize].to_vec()
                };
                for v in fields {
                    self.gray_value(v);
                }
                self.bytes_allocated += std::mem::size_of::<ObjInstance>();
            }
            ObjType::List => {
                let elems = self.heap.list(r).elements.clone();
                for v in elems {
                    self.gray_value(v);
                }
                self.bytes_allocated += std::mem::size_of::<ObjList>();
            }
            ObjType::Map => {
                let entries = self.heap.map(r).entries.clone();
                for e in entries {
                    if e.key.is_undefined() {
                        continue;
                    }
                    self.gray_value(e.key);
                    self.gray_value(e.value);
                }
                self.bytes_allocated += std::mem::size_of::<ObjMap>();
            }
            ObjType::Module => {
                let (vars, names, name) = {
                    let m = self.heap.module(r);
                    (m.variables.clone(), m.variable_names.clone(), m.name)
                };
                for v in vars {
                    self.gray_value(v);
                }
                for n in names {
                    self.gray_obj(n);
                }
                if name != NULL_OBJ {
                    self.gray_obj(name);
                }
                self.bytes_allocated += std::mem::size_of::<ObjModule>();
            }
            ObjType::Range => {
                self.bytes_allocated += std::mem::size_of::<ObjRange>();
            }
            ObjType::String => {
                self.bytes_allocated +=
                    std::mem::size_of::<ObjString>() + self.heap.string(r).bytes.len() + 1;
            }
            ObjType::Upvalue => {
                let closed = self.heap.upvalue(r).closed;
                self.gray_value(closed);
                self.bytes_allocated += std::mem::size_of::<ObjUpvalue>();
            }
        }
    }

    // -- temp roots --------------------------------------------------------------------

    pub fn push_root(&mut self, r: ObjRef) {
        self.temp_roots.push(r);
    }
    pub fn pop_root(&mut self) {
        self.temp_roots.pop();
    }

    // -- object constructors -------------------------------------------------------------

    pub fn new_single_class(&mut self, num_fields: i32, name: ObjRef) -> ObjRef {
        self.alloc_obj(
            ObjData::Class(ObjClass {
                superclass: NULL_OBJ,
                num_fields,
                methods: Vec::new(),
                name,
                attributes: NULL_VAL,
            }),
            NULL_OBJ,
        )
    }

    pub fn bind_superclass(&mut self, subclass: ObjRef, superclass: ObjRef) {
        debug_assert!(superclass != NULL_OBJ);
        self.heap.class_mut(subclass).superclass = superclass;
        let num_fields = self.heap.class(subclass).num_fields;
        let super_fields = self.heap.class(superclass).num_fields;
        if num_fields != -1 {
            self.heap.class_mut(subclass).num_fields += super_fields;
        } else {
            debug_assert!(super_fields == 0);
        }
        // Inherit methods.
        let methods = self.heap.class(superclass).methods.clone();
        for (i, m) in methods.into_iter().enumerate() {
            self.bind_method(subclass, i as i32, m);
        }
    }

    pub fn new_class(&mut self, superclass: ObjRef, num_fields: i32, name: ObjRef) -> ObjRef {
        // Create the metaclass.
        let metaclass_name = self.string_format("@ metaclass", &[Fmt::V(Value::obj(name))]);
        let metaclass_name = metaclass_name.as_obj();
        self.push_root(metaclass_name);

        let metaclass = self.new_single_class(0, metaclass_name);
        self.heap.get_mut(metaclass).class = self.class_class;
        self.pop_root();

        self.push_root(metaclass);
        let class_class = self.class_class;
        self.bind_superclass(metaclass, class_class);

        let class_obj = self.new_single_class(num_fields, name);
        self.push_root(class_obj);

        self.heap.get_mut(class_obj).class = metaclass;
        self.bind_superclass(class_obj, superclass);

        self.pop_root();
        self.pop_root();
        class_obj
    }

    pub fn bind_method(&mut self, class_ref: ObjRef, symbol: i32, method: Method) {
        let cls = self.heap.class_mut(class_ref);
        if symbol as usize >= cls.methods.len() {
            cls.methods.resize(symbol as usize + 1, Method::None);
        }
        cls.methods[symbol as usize] = method;
    }

    pub fn new_closure(&mut self, fn_ref: ObjRef) -> ObjRef {
        let n = self.heap.func(fn_ref).num_upvalues;
        self.alloc_obj(
            ObjData::Closure(ObjClosure {
                fn_ref,
                upvalues: vec![NULL_OBJ; n as usize],
            }),
            self.fn_class,
        )
    }

    pub fn new_fiber(&mut self, closure: ObjRef) -> ObjRef {
        let stack_capacity = if closure == NULL_OBJ {
            1
        } else {
            let max_slots = self.heap.func(self.heap.closure(closure).fn_ref).max_slots;
            power_of_2_ceil(max_slots + 1) as usize
        };
        let fiber = self.alloc_obj(
            ObjData::Fiber(ObjFiber {
                stack: vec![NULL_VAL; stack_capacity],
                stack_top: 0,
                frames: Vec::with_capacity(INITIAL_CALL_FRAMES),
                open_upvalues: NULL_OBJ,
                caller: NULL_OBJ,
                error: NULL_VAL,
                state: FiberState::Other,
            }),
            self.fiber_class,
        );
        if closure != NULL_OBJ {
            // Initialize the first call frame.
            self.append_call_frame(fiber, closure, 0);
            let f = self.heap.fiber_mut(fiber);
            f.stack[0] = Value::obj(closure);
            f.stack_top += 1;
        }
        fiber
    }

    pub fn append_call_frame(&mut self, fiber: ObjRef, closure: ObjRef, stack_start: usize) {
        let f = self.heap.fiber_mut(fiber);
        f.frames.push(CallFrame {
            ip: 0,
            closure,
            stack_start,
        });
    }

    pub fn ensure_stack(&mut self, fiber: ObjRef, needed: usize) {
        let cur = self.heap.fiber_mut(fiber).stack.len();
        if cur >= needed {
            return;
        }
        let capacity = power_of_2_ceil(needed as i32) as usize;
        self.track_alloc((capacity - cur) * 8);
        self.heap.fiber_mut(fiber).stack.resize(capacity, NULL_VAL);
    }

    pub fn new_foreign(&mut self, class_ref: ObjRef, size: usize) -> ObjRef {
        self.alloc_obj(
            ObjData::Foreign(ObjForeign {
                data: vec![0u8; size],
            }),
            class_ref,
        )
    }

    pub fn new_function(&mut self, module: ObjRef, max_slots: i32) -> ObjRef {
        self.alloc_obj(
            ObjData::Fn(ObjFn {
                code: Vec::new(),
                constants: Vec::new(),
                module,
                max_slots,
                num_upvalues: 0,
                arity: 0,
                debug: Some(Box::new(FnDebug {
                    name: String::new(),
                    source_lines: Vec::new(),
                })),
            }),
            self.fn_class,
        )
    }

    pub fn new_instance(&mut self, class_ref: ObjRef) -> Value {
        let num_fields = self.heap.class(class_ref).num_fields.max(0) as usize;
        let r = self.alloc_obj(
            ObjData::Instance(ObjInstance {
                fields: vec![NULL_VAL; num_fields],
            }),
            class_ref,
        );
        Value::obj(r)
    }

    pub fn new_list(&mut self, num_elements: usize) -> ObjRef {
        self.alloc_obj(
            ObjData::List(ObjList {
                elements: vec![NULL_VAL; num_elements],
            }),
            self.list_class,
        )
    }

    pub fn new_map(&mut self) -> ObjRef {
        self.alloc_obj(
            ObjData::Map(ObjMap {
                entries: Vec::new(),
                count: 0,
            }),
            self.map_class,
        )
    }

    pub fn new_module(&mut self, name: ObjRef) -> ObjRef {
        self.alloc_obj(
            ObjData::Module(ObjModule {
                variables: Vec::new(),
                variable_names: Vec::new(),
                name,
            }),
            NULL_OBJ,
        )
    }

    pub fn new_range(&mut self, from: f64, to: f64, is_inclusive: bool) -> Value {
        let r = self.alloc_obj(
            ObjData::Range(ObjRange {
                from,
                to,
                is_inclusive,
            }),
            self.range_class,
        );
        Value::obj(r)
    }

    pub fn new_string(&mut self, text: &[u8]) -> Value {
        self.new_string_len(text)
    }

    pub fn new_string_len(&mut self, text: &[u8]) -> Value {
        let hash = hash_string(text);
        let r = self.alloc_obj(
            ObjData::Str(ObjString {
                hash,
                bytes: text.to_vec(),
            }),
            self.string_class,
        );
        self.bytes_allocated += text.len() + 1;
        Value::obj(r)
    }

    pub fn new_string_from_range(
        &mut self,
        source: ObjRef,
        start: i32,
        count: u32,
        step: i32,
    ) -> Value {
        let bytes = self.heap.string(source).bytes.clone();
        let mut length = 0usize;
        for i in 0..count {
            let idx = (start + i as i32 * step) as usize;
            if idx < bytes.len() {
                length += utf8_decode_num_bytes(bytes[idx]);
            }
        }
        let mut out: Vec<u8> = Vec::with_capacity(length);
        for i in 0..count {
            let index = (start + i as i32 * step) as usize;
            if index >= bytes.len() {
                break;
            }
            let code_point = utf8_decode(&bytes[index..]);
            if code_point != -1 {
                utf8_encode(code_point, &mut out);
            }
        }
        self.new_string_len(&out)
    }

    pub fn num_to_string(&mut self, value: f64) -> Value {
        let s = format_num(value);
        self.new_string_len(s.as_bytes())
    }

    pub fn string_from_code_point(&mut self, value: i32) -> Value {
        let mut b = Vec::new();
        utf8_encode(value, &mut b);
        self.new_string_len(&b)
    }

    pub fn string_from_byte(&mut self, value: u8) -> Value {
        self.new_string_len(&[value])
    }

    /// wrenStringFormat: `$` = C string bytes, `@` = ObjString value.
    pub fn string_format(&mut self, format: &str, args: &[Fmt]) -> Value {
        let mut out: Vec<u8> = Vec::new();
        let mut argi = 0usize;
        for c in format.bytes() {
            match c {
                b'$' => {
                    if let Fmt::S(s) = &args[argi] {
                        out.extend_from_slice(s);
                    }
                    argi += 1;
                }
                b'@' => {
                    if let Fmt::V(v) = &args[argi] {
                        let s = self.heap.string(v.as_obj()).bytes.clone();
                        out.extend_from_slice(&s);
                    }
                    argi += 1;
                }
                other => out.push(other),
            }
        }
        self.new_string_len(&out)
    }

    pub fn string_code_point_at(&mut self, string: ObjRef, index: u32) -> Value {
        let bytes = self.heap.string(string).bytes.clone();
        let code_point = utf8_decode(&bytes[index as usize..]);
        if code_point == -1 {
            let b = bytes[index as usize];
            return self.new_string_len(&[b]);
        }
        self.string_from_code_point(code_point)
    }

    /// wrenStringFind (Boyer-Moore-Horspool).
    pub fn string_find(&mut self, haystack: &[u8], needle: &[u8], start: u32) -> u32 {
        if needle.is_empty() {
            return start;
        }
        if start as usize + needle.len() > haystack.len() {
            return u32::MAX;
        }
        if start as usize >= haystack.len() {
            return u32::MAX;
        }
        let mut shift = [needle.len() as u32; 256];
        let needle_end = needle.len() - 1;
        for i in 0..needle_end {
            shift[needle[i] as usize] = (needle_end - i) as u32;
        }
        let last_char = needle[needle_end];
        let range = haystack.len() - needle.len();
        let mut index = start as usize;
        while index <= range {
            let c = haystack[index + needle_end];
            if last_char == c && haystack[index..index + needle_end] == needle[..needle_end] {
                return index as u32;
            }
            index += shift[c as usize] as usize;
        }
        u32::MAX
    }

    pub fn new_upvalue(&mut self, fiber: ObjRef, slot: usize) -> ObjRef {
        self.alloc_obj(
            ObjData::Upvalue(ObjUpvalue {
                loc: UpvalueLoc::Open { fiber, slot },
                closed: NULL_VAL,
                next: NULL_OBJ,
            }),
            NULL_OBJ,
        )
    }

    // -- class-of / equality / truthiness ---------------------------------------------

    /// wrenGetClassInline.
    #[inline]
    pub fn class_of(&self, value: Value) -> ObjRef {
        if value.is_num() {
            return self.num_class;
        }
        if value.is_obj() {
            return self.heap.get(value.as_obj_unchecked()).class;
        }
        match value.0 & 7 {
            t if t == (NULL_VAL.0 & 7) => self.null_class,
            t if t == (TRUE_VAL.0 & 7) || t == (FALSE_VAL.0 & 7) => self.bool_class,
            _ => self.num_class, // TAG_NAN
        }
    }

    #[inline]
    pub fn is_falsy(&self, v: Value) -> bool {
        v.is_false() || v.is_null()
    }

    pub fn values_same(&self, a: Value, b: Value) -> bool {
        values_same(a, b)
    }

    pub fn values_equal(&self, a: Value, b: Value) -> bool {
        values_equal(&self.heap, a, b)
    }

    pub fn has_error(&self, fiber: ObjRef) -> bool {
        !self.heap.fiber(fiber).error.is_null()
    }

    // -- symbol tables -----------------------------------------------------------------

    /// wrenSymbolTableEnsure on vm.method_names.
    pub fn ensure_method_symbol(&mut self, name: &[u8]) -> i32 {
        if let Some(i) = self
            .method_names
            .iter()
            .position(|&r| self.heap.string(r).bytes == name)
        {
            return i as i32;
        }
        let s = self.new_string_len(name).as_obj();
        self.push_root(s);
        self.method_names.push(s);
        self.pop_root();
        (self.method_names.len() - 1) as i32
    }

    pub fn find_method_symbol(&self, name: &[u8]) -> i32 {
        self.method_names
            .iter()
            .position(|&r| self.heap.string(r).bytes == name)
            .map(|i| i as i32)
            .unwrap_or(-1)
    }

    /// wrenSymbolTableFind on a module's variableNames.
    pub fn module_find_variable(&self, module: ObjRef, name: &[u8]) -> i32 {
        let m = self.heap.module(module);
        for (i, &r) in m.variable_names.iter().enumerate() {
            if self.heap.string(r).bytes == name {
                return i as i32;
            }
        }
        -1
    }

    /// wrenSymbolTableAdd on a module's variableNames.
    fn module_add_variable_name(&mut self, module: ObjRef, name: &[u8]) -> i32 {
        let s = self.new_string_len(name).as_obj();
        self.push_root(s);
        self.heap.module_mut(module).variable_names.push(s);
        self.pop_root();
        (self.heap.module(module).variable_names.len() - 1) as i32
    }

    /// wrenDeclareVariable.
    pub fn declare_module_variable(&mut self, module: ObjRef, name: &[u8], line: i32) -> i32 {
        if self.heap.module(module).variables.len() == MAX_MODULE_VARS {
            return -2;
        }
        self.heap
            .module_mut(module)
            .variables
            .push(Value::num(line as f64));
        self.module_add_variable_name(module, name)
    }

    /// wrenDefineVariable. `line` receives the implicit-use line when returning -3.
    pub fn define_variable(
        &mut self,
        module: ObjRef,
        name: &[u8],
        value: Value,
        line: &mut i32,
    ) -> i32 {
        if self.heap.module(module).variables.len() == MAX_MODULE_VARS {
            return -2;
        }
        if value.is_obj() {
            self.push_root(value.as_obj_unchecked());
        }
        let symbol = self.module_find_variable(module, name);
        let symbol = if symbol == -1 {
            let s = self.module_add_variable_name(module, name);
            self.heap.module_mut(module).variables.push(value);
            s
        } else if self
            .heap
            .module(module)
            .variables
            .get(symbol as usize)
            .map(|v| v.is_num())
            .unwrap_or(false)
        {
            // Implicitly declared: value was the first-use line.
            let old = self.heap.module(module).variables[symbol as usize];
            *line = old.as_num() as i32;
            self.heap.module_mut(module).variables[symbol as usize] = value;
            if !name.is_empty() && name[0].is_ascii_lowercase() {
                -3
            } else {
                symbol
            }
        } else {
            -1
        };
        if value.is_obj() {
            self.pop_root();
        }
        symbol
    }

    // -- map operations ----------------------------------------------------------------

    pub fn map_get(&self, map: ObjRef, key: Value) -> Value {
        let m = self.heap.map(map);
        if m.entries.is_empty() {
            return UNDEFINED_VAL;
        }
        let (found, idx) = find_entry(&self.heap, &m.entries, key);
        if found {
            m.entries[idx].value
        } else {
            UNDEFINED_VAL
        }
    }

    pub fn map_set(&mut self, map: ObjRef, key: Value, value: Value) {
        let (count, cap) = {
            let m = self.heap.map(map);
            (m.count, m.entries.len())
        };
        if count + 1 > cap * MAP_LOAD_PERCENT / 100 {
            let mut capacity = cap * GROW_FACTOR;
            if capacity < MIN_CAPACITY {
                capacity = MIN_CAPACITY;
            }
            self.resize_map(map, capacity);
        }
        // insert
        let (found, idx) = {
            let m = self.heap.map(map);
            find_entry(&self.heap, &m.entries, key)
        };
        let inserted = {
            let m = self.heap.map_mut(map);
            if found {
                m.entries[idx].value = value;
                false
            } else {
                m.entries[idx].key = key;
                m.entries[idx].value = value;
                true
            }
        };
        if inserted {
            self.heap.map_mut(map).count += 1;
        }
        self.track_alloc(1);
    }

    fn resize_map(&mut self, map: ObjRef, capacity: usize) {
        let old = std::mem::take(&mut self.heap.map_mut(map).entries);
        let mut entries = vec![
            MapEntry {
                key: UNDEFINED_VAL,
                value: FALSE_VAL,
            };
            capacity
        ];
        for e in &old {
            if e.key.is_undefined() {
                continue;
            }
            let (found, idx) = find_entry(&self.heap, &entries, e.key);
            debug_assert!(!found);
            entries[idx].key = e.key;
            entries[idx].value = e.value;
        }
        self.heap.map_mut(map).entries = entries;
        self.track_alloc(capacity * std::mem::size_of::<MapEntry>());
    }

    pub fn map_clear(&mut self, map: ObjRef) {
        let m = self.heap.map_mut(map);
        m.entries.clear();
        m.count = 0;
    }

    pub fn map_remove_key(&mut self, map: ObjRef, key: Value) -> Value {
        let (found, idx) = {
            let m = self.heap.map(map);
            find_entry(&self.heap, &m.entries, key)
        };
        if !found {
            return NULL_VAL;
        }
        let value = self.heap.map(map).entries[idx].value;
        {
            let m = self.heap.map_mut(map);
            m.entries[idx].key = UNDEFINED_VAL;
            m.entries[idx].value = TRUE_VAL;
            m.count -= 1;
        }
        if value.is_obj() {
            self.push_root(value.as_obj_unchecked());
        }
        let (count, cap) = {
            let m = self.heap.map(map);
            (m.count, m.entries.len())
        };
        if count == 0 {
            self.map_clear(map);
        } else if cap > MIN_CAPACITY && count < cap / GROW_FACTOR * MAP_LOAD_PERCENT / 100 {
            let mut capacity = cap / GROW_FACTOR;
            if capacity < MIN_CAPACITY {
                capacity = MIN_CAPACITY;
            }
            self.resize_map(map, capacity);
        }
        if value.is_obj() {
            self.pop_root();
        }
        value
    }

    // -- list helpers -------------------------------------------------------------------

    pub fn list_insert(&mut self, list: ObjRef, value: Value, index: usize) {
        if value.is_obj() {
            self.push_root(value.as_obj_unchecked());
        }
        self.heap.list_mut(list).elements.push(NULL_VAL);
        if value.is_obj() {
            self.pop_root();
        }
        let l = self.heap.list_mut(list);
        for i in (index + 1..l.elements.len()).rev() {
            l.elements[i] = l.elements[i - 1];
        }
        l.elements[index] = value;
    }

    pub fn list_index_of(&self, list: ObjRef, value: Value) -> i32 {
        let l = self.heap.list(list);
        for i in 0..l.elements.len() {
            if self.values_equal(l.elements[i], value) {
                return i as i32;
            }
        }
        -1
    }

    pub fn list_remove_at(&mut self, list: ObjRef, index: usize) -> Value {
        let removed = self.heap.list(list).elements[index];
        if removed.is_obj() {
            self.push_root(removed.as_obj_unchecked());
        }
        self.heap.list_mut(list).elements.remove(index);
        self.track_alloc(1);
        if removed.is_obj() {
            self.pop_root();
        }
        removed
    }

    // -- modules -------------------------------------------------------------------------

    fn get_module(&self, name: Value) -> ObjRef {
        let v = self.map_get(self.modules, name);
        if v.is_undefined() {
            NULL_OBJ
        } else {
            v.as_obj()
        }
    }

    pub fn compile_in_module(
        &mut self,
        name: Value,
        source: &[u8],
        is_expression: bool,
        print_errors: bool,
    ) -> ObjRef {
        let mut module = self.get_module(name);
        if module == NULL_OBJ {
            let name_str = if name.is_obj() {
                name.as_obj()
            } else {
                NULL_OBJ
            };
            module = self.new_module(name_str);
            self.push_root(module);
            let modules = self.modules;
            self.map_set(modules, name, Value::obj(module));
            self.pop_root();

            // Implicitly import the core module.
            let core_module = self.get_module(NULL_VAL);
            let (names, values) = {
                let c = self.heap.module(core_module);
                (c.variable_names.clone(), c.variables.clone())
            };
            for (i, (n, v)) in names.iter().zip(values.iter()).enumerate() {
                let _ = i;
                let nb = self.heap.string(*n).bytes.clone();
                let mut line = 0;
                self.define_variable(module, &nb, *v, &mut line);
            }
        }

        let fn_ref = self.wren_compile(module, source, is_expression, print_errors);
        if fn_ref == NULL_OBJ {
            return NULL_OBJ;
        }
        self.push_root(fn_ref);
        let closure = self.new_closure(fn_ref);
        self.pop_root();
        closure
    }

    /// wrenCompileSource.
    pub fn compile_source(
        &mut self,
        module: Option<&[u8]>,
        source: &[u8],
        is_expression: bool,
        print_errors: bool,
    ) -> ObjRef {
        let name_value = match module {
            Some(m) => {
                let v = self.new_string(m);
                self.push_root(v.as_obj());
                v
            }
            None => NULL_VAL,
        };
        let closure = self.compile_in_module(name_value, source, is_expression, print_errors);
        if module.is_some() {
            self.pop_root();
        }
        closure
    }

    /// wrenInterpret.
    pub fn interpret(&mut self, module: Option<&[u8]>, source: &[u8]) -> WrenInterpretResult {
        let closure = self.compile_source(module, source, false, true);
        if closure == NULL_OBJ {
            return WrenInterpretResult::CompileError;
        }
        self.push_root(closure);
        let fiber = self.new_fiber(closure);
        self.pop_root();
        self.api_stack = None;
        self.run_interpreter(fiber)
    }

    fn get_module_variable(&mut self, module: ObjRef, variable_name: Value) -> Value {
        let var_name_bytes = self.heap.string(variable_name.as_obj()).bytes.clone();
        let entry = self.module_find_variable(module, &var_name_bytes);
        if entry != -1 {
            return self.heap.module(module).variables[entry as usize];
        }
        let module_name = self.heap.module(module).name;
        self.heap.fiber_mut(self.fiber).error = self.string_format(
            "Could not find a variable named '@' in module '@'.",
            &[Fmt::V(variable_name), Fmt::V(Value::obj(module_name))],
        );
        NULL_VAL
    }

    pub fn get_module_variable_by_name(
        &mut self,
        module_name: Value,
        variable_name: Value,
    ) -> Value {
        let module = self.get_module(module_name);
        if module == NULL_OBJ {
            self.heap.fiber_mut(self.fiber).error =
                self.string_format("Module '@' is not loaded.", &[Fmt::V(module_name)]);
            return NULL_VAL;
        }
        self.get_module_variable(module, variable_name)
    }

    /// wrenFindVariable.
    pub fn find_variable(&self, module: ObjRef, name: &[u8]) -> Value {
        let symbol = self.module_find_variable(module, name);
        self.heap.module(module).variables[symbol as usize]
    }

    // -- upvalues -------------------------------------------------------------------------

    fn capture_upvalue(&mut self, fiber: ObjRef, slot: usize) -> ObjRef {
        let mut prev = NULL_OBJ;
        let mut upvalue = self.heap.fiber(fiber).open_upvalues;

        // Walk towards the bottom of the stack until we find an existing
        // upvalue or pass where it should be (list sorted top-down by slot).
        while upvalue != NULL_OBJ {
            let u = self.heap.upvalue(upvalue);
            match u.loc {
                UpvalueLoc::Open { slot: s, .. } if s > slot => {
                    prev = upvalue;
                    upvalue = u.next;
                }
                UpvalueLoc::Open { slot: s, .. } if s == slot => {
                    return upvalue; // existing
                }
                _ => break,
            }
        }

        let created = self.new_upvalue(fiber, slot);
        if prev == NULL_OBJ {
            self.heap.fiber_mut(fiber).open_upvalues = created;
        } else {
            self.heap.upvalue_mut(prev).next = created;
        }
        self.heap.upvalue_mut(created).next = upvalue;
        created
    }

    /// Closes open upvalues for stack slots >= `last` (index).
    fn close_upvalues(&mut self, fiber: ObjRef, last: usize) {
        loop {
            let head = self.heap.fiber(fiber).open_upvalues;
            if head == NULL_OBJ {
                break;
            }
            let (slot, next) = {
                let u = self.heap.upvalue(head);
                match u.loc {
                    UpvalueLoc::Open { slot, .. } => (slot, u.next),
                    UpvalueLoc::Closed => break,
                }
            };
            if slot < last {
                break;
            }
            let value = self.heap.fiber(fiber).stack[slot];
            {
                let u = self.heap.upvalue_mut(head);
                u.closed = value;
                u.loc = UpvalueLoc::Closed;
            }
            self.heap.fiber_mut(fiber).open_upvalues = next;
        }
    }

    /// Reads the current value of an upvalue (open or closed).
    fn upvalue_get(&self, up: ObjRef) -> Value {
        let u = self.heap.upvalue(up);
        match u.loc {
            UpvalueLoc::Open { fiber, slot } => {
                let f = self.heap.fiber(fiber);
                if slot < f.stack.len() {
                    f.stack[slot]
                } else {
                    u.closed
                }
            }
            UpvalueLoc::Closed => u.closed,
        }
    }

    fn upvalue_set(&mut self, up: ObjRef, v: Value) {
        let u = self.heap.upvalue(up);
        match u.loc {
            UpvalueLoc::Open { fiber, slot } => {
                if slot < self.heap.fiber(fiber).stack.len() {
                    self.heap.fiber_mut(fiber).stack[slot] = v;
                    return;
                }
                self.heap.upvalue_mut(up).closed = v;
            }
            UpvalueLoc::Closed => {
                self.heap.upvalue_mut(up).closed = v;
            }
        }
    }

    // -- calls ------------------------------------------------------------------------------

    /// wrenCallFunction: push a call frame invoking `closure` on `fiber`.
    /// Expects `num_args` (incl. receiver) already on top of the stack.
    pub fn call_function(&mut self, fiber: ObjRef, closure: ObjRef, num_args: usize) {
        let max_slots = self.heap.func(self.heap.closure(closure).fn_ref).max_slots;
        let stack_size = self.heap.fiber(fiber).stack_top;
        let needed = stack_size + max_slots as usize;
        self.ensure_stack(fiber, needed);
        let stack_start = self.heap.fiber(fiber).stack_top - num_args;
        self.append_call_frame(fiber, closure, stack_start);
    }

    // -- the interpreter ---------------------------------------------------------------------

    /// Runs `fiber`. The caller must have set vm.fiber appropriately.
    pub fn run_interpreter(&mut self, mut fiber: ObjRef) -> WrenInterpretResult {
        self.fiber = fiber;
        self.heap.fiber_mut(fiber).state = FiberState::Root;

        let mut stack_start: usize;
        let mut ip: usize;
        let mut fn_ref: ObjRef;

        macro_rules! load_frame {
            () => {{
                let f = self.heap.fiber(fiber);
                let fr = f.frames[f.frames.len() - 1];
                stack_start = fr.stack_start;
                ip = fr.ip;
                fn_ref = self.heap.closure(fr.closure).fn_ref;
            }};
        }
        macro_rules! store_frame {
            () => {{
                let f = self.heap.fiber_mut(fiber);
                f.frames.last_mut().unwrap().ip = ip;
            }};
        }
        macro_rules! push {
            ($v:expr) => {{
                let v = $v;
                let f = self.heap.fiber_mut(fiber);
                f.stack[f.stack_top] = v;
                f.stack_top += 1;
            }};
        }
        macro_rules! pop {
            () => {{
                let f = self.heap.fiber_mut(fiber);
                f.stack_top -= 1;
                f.stack[f.stack_top]
            }};
        }
        macro_rules! drop_ {
            () => {{
                self.heap.fiber_mut(fiber).stack_top -= 1;
            }};
        }
        macro_rules! peek {
            () => {{
                let f = self.heap.fiber(fiber);
                f.stack[f.stack_top - 1]
            }};
        }
        macro_rules! peek2 {
            () => {{
                let f = self.heap.fiber(fiber);
                f.stack[f.stack_top - 2]
            }};
        }
        macro_rules! read_byte {
            () => {{
                let b = self.heap.func(fn_ref).code[ip];
                ip += 1;
                b
            }};
        }
        macro_rules! read_short {
            () => {{
                let c = &self.heap.func(fn_ref).code;
                let v = ((c[ip] as usize) << 8) | c[ip + 1] as usize;
                ip += 2;
                v
            }};
        }
        macro_rules! stack_get {
            ($i:expr) => {{
                let f = self.heap.fiber(fiber);
                f.stack[$i]
            }};
        }
        macro_rules! stack_set {
            ($i:expr, $v:expr) => {{
                let f = self.heap.fiber_mut(fiber);
                f.stack[$i] = $v;
            }};
        }
        macro_rules! runtime_error {
            () => {{
                store_frame!();
                self.runtime_error();
                if self.fiber == NULL_OBJ {
                    return WrenInterpretResult::RuntimeError;
                }
                fiber = self.fiber;
                load_frame!();
                continue;
            }};
        }

        load_frame!();

        loop {
            let instruction = read_byte!();
            match instruction {
                LOAD_LOCAL_0..=LOAD_LOCAL_8 => {
                    let v = stack_get!(stack_start + (instruction - LOAD_LOCAL_0) as usize);
                    push!(v);
                }
                LOAD_LOCAL => {
                    let i = read_byte!() as usize;
                    let v = stack_get!(stack_start + i);
                    push!(v);
                }
                LOAD_FIELD_THIS => {
                    let field = read_byte!() as usize;
                    let receiver = stack_get!(stack_start);
                    let inst = receiver.as_obj();
                    let v = self.heap.instance(inst).fields[field];
                    push!(v);
                }
                POP => drop_!(),
                NULL => push!(NULL_VAL),
                FALSE => push!(FALSE_VAL),
                TRUE => push!(TRUE_VAL),
                STORE_LOCAL => {
                    let i = read_byte!() as usize;
                    let v = peek!();
                    stack_set!(stack_start + i, v);
                }
                CONSTANT => {
                    let i = read_short!();
                    let v = self.heap.func(fn_ref).constants[i];
                    push!(v);
                }
                op if (CALL_0..=CALL_16).contains(&op) || (SUPER_0..=SUPER_16).contains(&op) => {
                    let (num_args, symbol, class_obj) = if op <= CALL_16 {
                        let n = (op - CALL_0 + 1) as usize;
                        let s = read_short!();
                        let base = self.heap.fiber(fiber).stack_top - n;
                        let recv = stack_get!(base);
                        let cls = self.class_of(recv);
                        (n, s, cls)
                    } else {
                        let n = (op - SUPER_0 + 1) as usize;
                        let s = read_short!();
                        let ci = read_short!();
                        let cls = self.heap.func(fn_ref).constants[ci].as_obj();
                        (n, s, cls)
                    };

                    let args_base = self.heap.fiber(fiber).stack_top - num_args;
                    let method = {
                        let cls = self.heap.class(class_obj);
                        if symbol < cls.methods.len() {
                            cls.methods[symbol].clone()
                        } else {
                            Method::None
                        }
                    };
                    if let Method::None = method {
                        self.method_not_found(class_obj, symbol as i32);
                        runtime_error!();
                    }
                    match method {
                        Method::Primitive(prim) => {
                            if prim(self, args_base) {
                                self.heap.fiber_mut(fiber).stack_top -= num_args - 1;
                            } else {
                                store_frame!();
                                fiber = self.fiber;
                                if fiber == NULL_OBJ {
                                    return WrenInterpretResult::Success;
                                }
                                if self.has_error(fiber) {
                                    runtime_error!();
                                }
                                load_frame!();
                            }
                        }
                        Method::FunctionCall(prim) => {
                            let recv = stack_get!(args_base);
                            if !self.check_arity(recv, num_args) {
                                runtime_error!();
                            }
                            store_frame!();
                            prim(self, args_base);
                            load_frame!();
                        }
                        Method::Foreign(foreign) => {
                            self.call_foreign(fiber, foreign, num_args);
                            if self.has_error(fiber) {
                                runtime_error!();
                            }
                        }
                        Method::Block(closure) => {
                            store_frame!();
                            self.call_function(fiber, closure, num_args);
                            load_frame!();
                        }
                        Method::None => unreachable!(),
                        Method::Finalizer(_) => unreachable!(),
                    }
                }
                LOAD_UPVALUE => {
                    let i = read_byte!() as usize;
                    let closure = {
                        let f = self.heap.fiber(fiber);
                        f.frames[f.frames.len() - 1].closure
                    };
                    let up = self.heap.closure(closure).upvalues[i];
                    let v = self.upvalue_get(up);
                    push!(v);
                }
                STORE_UPVALUE => {
                    let i = read_byte!() as usize;
                    let closure = {
                        let f = self.heap.fiber(fiber);
                        f.frames[f.frames.len() - 1].closure
                    };
                    let up = self.heap.closure(closure).upvalues[i];
                    let v = peek!();
                    self.upvalue_set(up, v);
                }
                LOAD_MODULE_VAR => {
                    let i = read_short!();
                    let module = self.heap.func(fn_ref).module;
                    let v = self.heap.module(module).variables[i];
                    push!(v);
                }
                STORE_MODULE_VAR => {
                    let i = read_short!();
                    let module = self.heap.func(fn_ref).module;
                    let v = peek!();
                    self.heap.module_mut(module).variables[i] = v;
                }
                STORE_FIELD_THIS => {
                    let field = read_byte!() as usize;
                    let receiver = stack_get!(stack_start);
                    let inst = receiver.as_obj();
                    let v = peek!();
                    self.heap.instance_mut(inst).fields[field] = v;
                }
                LOAD_FIELD => {
                    let field = read_byte!() as usize;
                    let receiver = pop!();
                    let inst = receiver.as_obj();
                    let v = self.heap.instance(inst).fields[field];
                    push!(v);
                }
                STORE_FIELD => {
                    let field = read_byte!() as usize;
                    let receiver = pop!();
                    let inst = receiver.as_obj();
                    let v = peek!();
                    self.heap.instance_mut(inst).fields[field] = v;
                }
                JUMP => {
                    let offset = read_short!();
                    ip += offset;
                }
                LOOP => {
                    let offset = read_short!();
                    ip -= offset;
                }
                JUMP_IF => {
                    let offset = read_short!();
                    let condition = pop!();
                    if self.is_falsy(condition) {
                        ip += offset;
                    }
                }
                AND => {
                    let offset = read_short!();
                    let condition = peek!();
                    if self.is_falsy(condition) {
                        ip += offset;
                    } else {
                        drop_!();
                    }
                }
                OR => {
                    let offset = read_short!();
                    let condition = peek!();
                    if self.is_falsy(condition) {
                        drop_!();
                    } else {
                        ip += offset;
                    }
                }
                CLOSE_UPVALUE => {
                    let last = self.heap.fiber(fiber).stack_top - 1;
                    self.close_upvalues(fiber, last);
                    drop_!();
                }
                RETURN => {
                    let result = pop!();
                    self.heap.fiber_mut(fiber).frames.pop();
                    self.close_upvalues(fiber, stack_start);

                    if self.heap.fiber(fiber).frames.is_empty() {
                        let caller = self.heap.fiber(fiber).caller;
                        if caller == NULL_OBJ {
                            let f = self.heap.fiber_mut(fiber);
                            f.stack[0] = result;
                            f.stack_top = 1;
                            return WrenInterpretResult::Success;
                        }
                        let resuming = caller;
                        self.heap.fiber_mut(fiber).caller = NULL_OBJ;
                        fiber = resuming;
                        self.fiber = resuming;
                        let top = self.heap.fiber(fiber).stack_top - 1;
                        stack_set!(top, result);
                    } else {
                        stack_set!(stack_start, result);
                        self.heap.fiber_mut(fiber).stack_top = stack_start + 1;
                    }
                    load_frame!();
                }
                CONSTRUCT => {
                    let cls_val = stack_get!(stack_start);
                    let inst = self.new_instance(cls_val.as_obj());
                    stack_set!(stack_start, inst);
                }
                FOREIGN_CONSTRUCT => {
                    let cls_val = stack_get!(stack_start);
                    self.create_foreign(fiber, stack_start);
                    let _ = cls_val;
                    if self.has_error(fiber) {
                        runtime_error!();
                    }
                }
                CLOSURE => {
                    let ci = read_short!();
                    let function = self.heap.func(fn_ref).constants[ci].as_obj();
                    let closure = self.new_closure(function);
                    push!(Value::obj(closure));

                    let n = self.heap.func(function).num_upvalues;
                    for i in 0..n {
                        let is_local = read_byte!();
                        let index = read_byte!() as usize;
                        if is_local != 0 {
                            let u = self.capture_upvalue(fiber, stack_start + index);
                            self.heap.closure_mut(closure).upvalues[i as usize] = u;
                        } else {
                            let parent_closure = {
                                let f = self.heap.fiber(fiber);
                                f.frames[f.frames.len() - 1].closure
                            };
                            let u = self.heap.closure(parent_closure).upvalues[index];
                            self.heap.closure_mut(closure).upvalues[i as usize] = u;
                        }
                    }
                }
                END_CLASS => {
                    self.end_class();
                    if self.has_error(fiber) {
                        runtime_error!();
                    }
                }
                CLASS => {
                    let n = read_byte!() as i32;
                    self.create_class(n, NULL_OBJ);
                    if self.has_error(fiber) {
                        runtime_error!();
                    }
                }
                FOREIGN_CLASS => {
                    let module = self.heap.func(fn_ref).module;
                    self.create_class(-1, module);
                    if self.has_error(fiber) {
                        runtime_error!();
                    }
                }
                METHOD_INSTANCE | METHOD_STATIC => {
                    let symbol = read_short!();
                    let class_val = peek!();
                    let method_val = peek2!();
                    let module = self.heap.func(fn_ref).module;
                    self.bind_method_op(instruction, symbol as i32, module, class_val, method_val);
                    if self.has_error(fiber) {
                        runtime_error!();
                    }
                    drop_!();
                    drop_!();
                }
                END_MODULE => {
                    let module = self.heap.func(fn_ref).module;
                    self.last_module = module;
                    push!(NULL_VAL);
                }
                IMPORT_MODULE => {
                    let ci = read_short!();
                    let name = self.heap.func(fn_ref).constants[ci];
                    let imported = self.import_module(name);
                    push!(imported);
                    if self.has_error(fiber) {
                        runtime_error!();
                    }
                    if peek!().is_obj()
                        && matches!(self.heap.get(peek!().as_obj()).data, ObjData::Closure(_))
                    {
                        store_frame!();
                        let closure = peek!().as_obj();
                        self.call_function(fiber, closure, 1);
                        load_frame!();
                    } else {
                        self.last_module = peek!().as_obj();
                    }
                }
                IMPORT_VARIABLE => {
                    let ci = read_short!();
                    let variable = self.heap.func(fn_ref).constants[ci];
                    let last = self.last_module;
                    let result = self.get_module_variable(last, variable);
                    if self.has_error(fiber) {
                        runtime_error!();
                    }
                    push!(result);
                }
                END => unreachable!(),
                _ => unreachable!(),
            }
        }
    }

    fn check_arity(&mut self, value: Value, num_args: usize) -> bool {
        let fn_ref = self.heap.closure(value.as_obj()).fn_ref;
        let arity = self.heap.func(fn_ref).arity;
        if num_args as i32 - 1 >= arity {
            return true;
        }
        let s = self.new_string_len(b"Function expects more arguments.");
        self.heap.fiber_mut(self.fiber).error = s;
        false
    }

    fn method_not_found(&mut self, class_obj: ObjRef, symbol: i32) {
        let name = self.heap.class(class_obj).name;
        let method_name = self
            .heap
            .string(self.method_names[symbol as usize])
            .bytes
            .clone();
        let msg = self.string_format(
            "@ does not implement '$'.",
            &[Fmt::V(Value::obj(name)), Fmt::S(&method_name)],
        );
        self.heap.fiber_mut(self.fiber).error = msg;
    }

    fn runtime_error(&mut self) {
        let mut current = self.fiber;
        let error = self.heap.fiber(current).error;
        while current != NULL_OBJ {
            self.heap.fiber_mut(current).error = error;
            if self.heap.fiber(current).state == FiberState::Try {
                let caller = self.heap.fiber(current).caller;
                let top = self.heap.fiber(caller).stack_top - 1;
                let e = self.heap.fiber(self.fiber).error;
                self.heap.fiber_mut(caller).stack[top] = e;
                self.fiber = caller;
                return;
            }
            let caller = self.heap.fiber(current).caller;
            self.heap.fiber_mut(current).caller = NULL_OBJ;
            current = caller;
        }
        self.debug_print_stack_trace();
        self.fiber = NULL_OBJ;
        self.api_stack = None;
    }

    fn find_foreign_method(
        &mut self,
        module_name: &[u8],
        class_name: &[u8],
        is_static: bool,
        signature: &[u8],
    ) -> Option<WrenForeignMethodFn> {
        if let Some(f) = self.config.bind_foreign_method_fn {
            if let Some(m) = f(self, module_name, class_name, is_static, signature) {
                return Some(m);
            }
        }
        if module_name == b"meta" {
            if let Some(m) =
                crate::opt_meta::bind_foreign_method(self, class_name, is_static, signature)
            {
                return Some(m);
            }
        }
        if module_name == b"random" {
            if let Some(m) =
                crate::opt_random::bind_foreign_method(self, class_name, is_static, signature)
            {
                return Some(m);
            }
        }
        None
    }

    fn bind_method_op(
        &mut self,
        method_type: u8,
        symbol: i32,
        module: ObjRef,
        class_val: Value,
        method_value: Value,
    ) {
        let mut class_obj = class_val.as_obj();
        let class_name = self.heap.class(class_obj).name;
        if method_type == METHOD_STATIC {
            class_obj = self.heap.get(class_obj).class;
        }
        let method;
        if method_value.is_obj()
            && matches!(self.heap.get(method_value.as_obj()).data, ObjData::Str(_))
        {
            let name = self.heap.string(method_value.as_obj()).bytes.clone();
            let module_name = self
                .heap
                .string(self.heap.module(module).name)
                .bytes
                .clone();
            let class_name_bytes = self.heap.string(class_name).bytes.clone();
            let foreign = self.find_foreign_method(
                &module_name,
                &class_name_bytes,
                method_type == METHOD_STATIC,
                &name,
            );
            match foreign {
                Some(f) => method = Method::Foreign(f),
                None => {
                    let cls_name = self.heap.class(class_obj).name;
                    let msg = self.string_format(
                        "Could not find foreign method '@' for class $ in module '$'.",
                        &[
                            Fmt::V(method_value),
                            Fmt::S(&self.heap.string(cls_name).bytes.clone()),
                            Fmt::S(
                                &self
                                    .heap
                                    .string(self.heap.module(module).name)
                                    .bytes
                                    .clone(),
                            ),
                        ],
                    );
                    self.heap.fiber_mut(self.fiber).error = msg;
                    return;
                }
            }
        } else {
            let closure = method_value.as_obj();
            method = Method::Block(closure);
            let fn_ref = self.heap.closure(closure).fn_ref;
            self.bind_method_code(class_obj, fn_ref);
        }
        self.bind_method(class_obj, symbol, method);
    }

    fn call_foreign(&mut self, fiber: ObjRef, foreign: WrenForeignMethodFn, num_args: usize) {
        let base = self.heap.fiber(fiber).stack_top - num_args;
        self.api_stack = Some((fiber, base));
        foreign(self);
        self.heap.fiber_mut(fiber).stack_top = base + 1;
        self.api_stack = None;
    }

    fn create_foreign(&mut self, _fiber: ObjRef, stack_start: usize) {
        let class_val = self.heap.fiber(self.fiber).stack[stack_start];
        let class_obj = class_val.as_obj();
        let symbol = self.find_method_symbol(b"<allocate>");
        debug_assert!(symbol != -1);
        let method = self.heap.class(class_obj).methods[symbol as usize].clone();
        debug_assert!(matches!(method, Method::Foreign(_)));

        self.api_stack = Some((self.fiber, stack_start));
        if let Method::Foreign(f) = method {
            f(self);
        }
        self.api_stack = None;
    }

    fn end_class(&mut self) {
        let fiber = self.fiber;
        let top = self.heap.fiber(fiber).stack_top;
        let attributes = self.heap.fiber(fiber).stack[top - 2];
        let class_val = self.heap.fiber(fiber).stack[top - 1];
        self.heap.fiber_mut(fiber).stack_top -= 2;
        let class_obj = class_val.as_obj();
        self.heap.class_mut(class_obj).attributes = attributes;
    }

    fn validate_superclass(
        &mut self,
        name: Value,
        superclass_value: Value,
        num_fields: i32,
    ) -> Value {
        if !superclass_value.is_obj()
            || !matches!(
                self.heap.get(superclass_value.as_obj()).data,
                ObjData::Class(_)
            )
        {
            return self.string_format(
                "Class '@' cannot inherit from a non-class object.",
                &[Fmt::V(name)],
            );
        }
        let superclass = superclass_value.as_obj();
        let built_ins = [
            self.class_class,
            self.fiber_class,
            self.fn_class,
            self.list_class,
            self.map_class,
            self.range_class,
            self.string_class,
            self.bool_class,
            self.null_class,
            self.num_class,
        ];
        if built_ins.contains(&superclass) {
            let sname = self.heap.class(superclass).name;
            return self.string_format(
                "Class '@' cannot inherit from built-in class '@'.",
                &[Fmt::V(name), Fmt::V(Value::obj(sname))],
            );
        }
        let super_fields = self.heap.class(superclass).num_fields;
        if super_fields == -1 {
            let sname = self.heap.class(superclass).name;
            return self.string_format(
                "Class '@' cannot inherit from foreign class '@'.",
                &[Fmt::V(name), Fmt::V(Value::obj(sname))],
            );
        }
        if num_fields == -1 && super_fields > 0 {
            return self.string_format(
                "Foreign class '@' may not inherit from a class with fields.",
                &[Fmt::V(name)],
            );
        }
        if super_fields + num_fields > MAX_FIELDS {
            return self.string_format(
                "Class '@' may not have more than 255 fields, including inherited ones.",
                &[Fmt::V(name)],
            );
        }
        NULL_VAL
    }

    fn bind_foreign_class(&mut self, class_obj: ObjRef, module: ObjRef) {
        let mut methods = WrenForeignClassMethods::default();
        let module_name = self
            .heap
            .string(self.heap.module(module).name)
            .bytes
            .clone();
        let class_name = self
            .heap
            .string(self.heap.class(class_obj).name)
            .bytes
            .clone();

        if let Some(f) = self.config.bind_foreign_class_fn {
            methods = f(self, &module_name, &class_name);
        }
        if methods.allocate.is_none() && methods.finalize.is_none() {
            if module_name == b"random" {
                methods = crate::opt_random::bind_foreign_class(self, &module_name, &class_name);
            }
        }

        let symbol = self.ensure_method_symbol(b"<allocate>");
        if let Some(alloc) = methods.allocate {
            self.bind_method(class_obj, symbol, Method::Foreign(alloc));
        }

        let symbol = self.ensure_method_symbol(b"<finalize>");
        if let Some(fin) = methods.finalize {
            self.bind_method(class_obj, symbol, Method::Finalizer(fin));
        }
    }

    fn create_class(&mut self, num_fields: i32, module: ObjRef) {
        let fiber = self.fiber;
        let top = self.heap.fiber(fiber).stack_top;
        let name = self.heap.fiber(fiber).stack[top - 2];
        let superclass = self.heap.fiber(fiber).stack[top - 1];
        self.heap.fiber_mut(fiber).stack_top -= 1;

        let err = self.validate_superclass(name, superclass, num_fields);
        if !err.is_null() {
            self.heap.fiber_mut(fiber).error = err;
            return;
        }
        let class_obj = self.new_class(superclass.as_obj(), num_fields, name.as_obj());
        let top = self.heap.fiber(fiber).stack_top;
        self.heap.fiber_mut(fiber).stack[top - 1] = Value::obj(class_obj);

        if num_fields == -1 {
            self.bind_foreign_class(class_obj, module);
        }
    }

    fn resolve_module(&mut self, name: Value) -> Value {
        if self.config.resolve_module_fn.is_none() {
            return name;
        }
        let fiber = self.fiber;
        let fn_ref = {
            let f = self.heap.fiber(fiber);
            self.heap
                .closure(f.frames[f.frames.len() - 1].closure)
                .fn_ref
        };
        let importer = self.heap.func(fn_ref).module;
        let importer_name = self.heap.module(importer).name;
        let importer_bytes = self.heap.string(importer_name).bytes.clone();
        let name_bytes = self.heap.string(name.as_obj()).bytes.clone();

        let resolved = (self.config.resolve_module_fn.unwrap())(self, &importer_bytes, &name_bytes);
        match resolved {
            None => {
                self.heap.fiber_mut(fiber).error = self.string_format(
                    "Could not resolve module '@' imported from '@'.",
                    &[Fmt::V(name), Fmt::V(Value::obj(importer_name))],
                );
                NULL_VAL
            }
            Some(r) => {
                if r == name_bytes {
                    name
                } else {
                    self.new_string(&r)
                }
            }
        }
    }

    fn import_module(&mut self, name: Value) -> Value {
        let name = self.resolve_module(name);
        if self.fiber != NULL_OBJ && self.has_error(self.fiber) {
            return NULL_VAL;
        }
        let existing = self.map_get(self.modules, name);
        if !existing.is_undefined() {
            return existing;
        }

        if name.is_obj() {
            self.push_root(name.as_obj_unchecked());
        }

        let mut source: Option<Vec<u8>> = None;
        let mut on_complete: Option<WrenLoadModuleCompleteFn> = None;

        if let Some(f) = self.config.load_module_fn {
            let nb = self.heap.string(name.as_obj()).bytes.clone();
            let result = f(self, &nb);
            source = result.source;
            on_complete = result.on_complete;
        }

        if source.is_none() {
            let nb = self.heap.string(name.as_obj()).bytes.clone();
            if nb == b"meta" {
                source = Some(crate::opt_meta::META_SOURCE.to_vec());
            } else if nb == b"random" {
                source = Some(crate::opt_random::RANDOM_SOURCE.to_vec());
            }
        }

        let name_bytes = self.heap.string(name.as_obj()).bytes.clone();

        if source.is_none() {
            self.heap.fiber_mut(self.fiber).error =
                self.string_format("Could not load module '@'.", &[Fmt::V(name)]);
            if name.is_obj() {
                self.pop_root();
            }
            return NULL_VAL;
        }

        let module_closure = self.compile_in_module(name, &source.unwrap(), false, true);

        if let Some(cb) = on_complete {
            cb(self, &name_bytes);
        }

        if module_closure == NULL_OBJ {
            self.heap.fiber_mut(self.fiber).error =
                self.string_format("Could not compile module '@'.", &[Fmt::V(name)]);
            if name.is_obj() {
                self.pop_root();
            }
            return NULL_VAL;
        }

        if name.is_obj() {
            self.pop_root();
        }
        Value::obj(module_closure)
    }

    // -- debug ---------------------------------------------------------------------------

    fn debug_print_stack_trace(&mut self) {
        // Bail if the host doesn't enable printing errors.
        if self.config.error_fn.is_none() {
            return;
        }
        let fiber = self.fiber;
        if fiber == NULL_OBJ {
            return;
        }
        // Print the error itself.
        let error = self.heap.fiber(fiber).error;
        self.print_error_obj(error);
        let frames = self.heap.fiber(fiber).frames.clone();
        for frame in frames.iter().rev() {
            let fn_ref = self.heap.closure(frame.closure).fn_ref;
            let f = self.heap.func(fn_ref);
            // Skip over stub functions for calling methods from the API.
            if f.module == NULL_OBJ {
                continue;
            }
            // The built-in core module has no name; omit it from traces.
            let module_name = self.heap.module(f.module).name;
            if module_name == NULL_OBJ {
                continue;
            }
            let module_name_bytes = self.heap.string(module_name).bytes.clone();
            // -1 because ip has advanced past the instruction just executed.
            let line =
                if frame.ip > 0 && frame.ip - 1 < f.debug.as_ref().unwrap().source_lines.len() {
                    f.debug.as_ref().unwrap().source_lines[frame.ip - 1]
                } else {
                    0
                };
            let fn_name = f.debug.as_ref().map(|d| d.name.clone()).unwrap_or_default();
            if let Some(cb) = self.config.error_fn {
                cb(
                    self,
                    WrenErrorType::StackTrace,
                    Some(&module_name_bytes),
                    line,
                    fn_name.as_bytes(),
                );
            }
        }
    }

    fn print_error_obj(&mut self, error: Value) {
        let msg = if error.is_obj() && matches!(self.heap.get(error.as_obj()).data, ObjData::Str(_))
        {
            self.heap.string(error.as_obj()).bytes.clone()
        } else {
            b"[error object]".to_vec()
        };
        if let Some(cb) = self.config.error_fn {
            cb(self, WrenErrorType::Runtime, None, -1, &msg);
        }
    }

    // -- wrenCall / handles / API -------------------------------------------------------------

    /// wrenMakeCallHandle.
    pub fn make_call_handle(&mut self, signature: &[u8]) -> WrenHandle {
        let mut num_params = 0usize;
        let n = signature.len();
        if signature[n - 1] == b')' {
            let mut i = n - 1;
            while i > 0 && signature[i] != b'(' {
                if signature[i] == b'_' {
                    num_params += 1;
                }
                i -= 1;
            }
        }
        if signature[0] == b'[' {
            let mut i = 0;
            while i < n && signature[i] != b']' {
                if signature[i] == b'_' {
                    num_params += 1;
                }
                i += 1;
            }
        }

        let method = self.ensure_method_symbol(signature);
        debug_assert!(method <= MAX_METHODS);

        let fn_ref = self.new_function(NULL_OBJ, num_params as i32 + 1);

        let value = self.make_handle(Value::obj(fn_ref));
        let closure = self.new_closure(fn_ref);
        self.handles[value as usize] = Some(Value::obj(closure));

        {
            let f = self.heap.func_mut(fn_ref);
            f.code.push(CALL_0 + num_params as u8);
            f.code.push(((method >> 8) & 0xff) as u8);
            f.code.push((method & 0xff) as u8);
            f.code.push(RETURN);
            f.code.push(END);
            f.debug.as_mut().unwrap().source_lines.resize(5, 0);
        }
        self.track_alloc(5 * 9);
        let name = String::from_utf8_lossy(signature).to_string();
        self.heap.func_mut(fn_ref).debug.as_mut().unwrap().name = name;

        value
    }

    /// wrenCall.
    pub fn call(&mut self, method: WrenHandle) -> WrenInterpretResult {
        let closure_val = self.handles[method as usize].unwrap();
        let closure = closure_val.as_obj();

        self.api_stack = None;

        let max_slots = self.heap.func(self.heap.closure(closure).fn_ref).max_slots;
        let fiber = self.fiber;
        self.heap.fiber_mut(fiber).stack_top = max_slots as usize;

        self.call_function(fiber, closure, 0);
        let result = self.run_interpreter(fiber);

        if self.fiber != NULL_OBJ {
            self.api_stack = Some((self.fiber, 0));
        }
        result
    }

    /// wrenMakeHandle.
    pub fn make_handle(&mut self, value: Value) -> WrenHandle {
        if value.is_obj() {
            self.push_root(value.as_obj_unchecked());
        }
        let id = if let Some(i) = self.free_handles.pop() {
            self.handles[i as usize] = Some(value);
            i
        } else {
            self.handles.push(Some(value));
            (self.handles.len() - 1) as u32
        };
        if value.is_obj() {
            self.pop_root();
        }
        id
    }

    /// wrenReleaseHandle.
    pub fn release_handle(&mut self, handle: WrenHandle) {
        self.handles[handle as usize] = None;
        self.free_handles.push(handle);
    }

    // -- API stack access ----------------------------------------------------------------

    fn api(&self) -> (ObjRef, usize) {
        self.api_stack.expect("apiStack is null")
    }

    pub fn get_slot_count(&self) -> usize {
        match self.api_stack {
            None => 0,
            Some((fiber, base)) => self.heap.fiber(fiber).stack_top - base,
        }
    }

    pub fn ensure_slots(&mut self, num_slots: usize) {
        if self.api_stack.is_none() {
            let fiber = self.new_fiber(NULL_OBJ);
            self.fiber = fiber;
            self.api_stack = Some((fiber, 0));
        }
        let (fiber, base) = self.api();
        let current = self.heap.fiber(fiber).stack_top - base;
        if current >= num_slots {
            return;
        }
        self.ensure_stack(fiber, base + num_slots);
        self.heap.fiber_mut(fiber).stack_top = base + num_slots;
    }

    pub fn get_slot_type(&self, slot: i32) -> WrenType {
        let (fiber, base) = self.api();
        let v = self.heap.fiber(fiber).stack[base + slot as usize];
        if v.is_bool() {
            return WrenType::Bool;
        }
        if v.is_num() {
            return WrenType::Num;
        }
        if v.is_null() {
            return WrenType::Null;
        }
        if v.is_obj() {
            match self.heap.get(v.as_obj_unchecked()).data {
                ObjData::Foreign(_) => return WrenType::Foreign,
                ObjData::List(_) => return WrenType::List,
                ObjData::Map(_) => return WrenType::Map,
                ObjData::Str(_) => return WrenType::Str,
                _ => {}
            }
        }
        WrenType::Unknown
    }

    fn api_get(&self, slot: i32) -> Value {
        let (fiber, base) = self.api();
        self.heap.fiber(fiber).stack[base + slot as usize]
    }

    fn api_set(&mut self, slot: i32, value: Value) {
        let (fiber, base) = self.api();
        self.heap.fiber_mut(fiber).stack[base + slot as usize] = value;
    }

    pub fn get_slot_bool(&self, slot: i32) -> bool {
        self.api_get(slot).as_bool()
    }

    pub fn get_slot_bytes(&self, slot: i32) -> Vec<u8> {
        let v = self.api_get(slot);
        self.heap.string(v.as_obj()).bytes.clone()
    }

    pub fn get_slot_double(&self, slot: i32) -> f64 {
        self.api_get(slot).as_num()
    }

    pub fn get_slot_foreign(&self, slot: i32) -> ObjRef {
        self.api_get(slot).as_obj()
    }

    pub fn get_slot_string(&self, slot: i32) -> Vec<u8> {
        self.get_slot_bytes(slot)
    }

    pub fn get_slot_handle(&mut self, slot: i32) -> WrenHandle {
        let v = self.api_get(slot);
        self.make_handle(v)
    }

    pub fn set_slot_bool(&mut self, slot: i32, value: bool) {
        self.api_set(slot, Value::bool(value));
    }

    pub fn set_slot_bytes(&mut self, slot: i32, bytes: &[u8]) {
        let v = self.new_string_len(bytes);
        self.api_set(slot, v);
    }

    pub fn set_slot_double(&mut self, slot: i32, value: f64) {
        self.api_set(slot, Value::num(value));
    }

    /// wrenSetSlotNewForeign: creates a foreign instance of the class in
    /// `class_slot` with `size` data bytes. Returns the object's ref (access
    /// data via `foreign_data*`).
    pub fn set_slot_new_foreign(&mut self, slot: i32, class_slot: i32, size: usize) -> ObjRef {
        let class_val = self.api_get(class_slot);
        let class_obj = class_val.as_obj();
        let foreign = self.new_foreign(class_obj, size);
        self.api_set(slot, Value::obj(foreign));
        foreign
    }

    pub fn foreign_data(&self, foreign: ObjRef) -> &[u8] {
        &self.heap.foreign(foreign).data
    }
    pub fn foreign_data_mut(&mut self, foreign: ObjRef) -> &mut [u8] {
        &mut self.heap.foreign_mut(foreign).data
    }

    pub fn set_slot_new_list(&mut self, slot: i32) {
        let l = self.new_list(0);
        self.api_set(slot, Value::obj(l));
    }

    pub fn set_slot_new_map(&mut self, slot: i32) {
        let m = self.new_map();
        self.api_set(slot, Value::obj(m));
    }

    pub fn set_slot_null(&mut self, slot: i32) {
        self.api_set(slot, NULL_VAL);
    }

    pub fn set_slot_string(&mut self, slot: i32, text: &[u8]) {
        let v = self.new_string_len(text);
        self.api_set(slot, v);
    }

    pub fn set_slot_handle(&mut self, slot: i32, handle: WrenHandle) {
        let v = self.handles[handle as usize].unwrap();
        self.api_set(slot, v);
    }

    pub fn get_list_count(&self, slot: i32) -> usize {
        let v = self.api_get(slot);
        self.heap.list(v.as_obj()).elements.len()
    }

    pub fn get_list_element(&mut self, list_slot: i32, index: i64, element_slot: i32) {
        let v = self.api_get(list_slot);
        let count = self.heap.list(v.as_obj()).elements.len();
        let used = crate::utils::validate_index(count as u32, index);
        debug_assert!(used != u32::MAX);
        let elem = self.heap.list(v.as_obj()).elements[used as usize];
        self.api_set(element_slot, elem);
    }

    pub fn set_list_element(&mut self, list_slot: i32, index: i64, element_slot: i32) {
        let v = self.api_get(list_slot);
        let elem = self.api_get(element_slot);
        let count = self.heap.list(v.as_obj()).elements.len();
        let used = crate::utils::validate_index(count as u32, index);
        debug_assert!(used != u32::MAX);
        self.heap.list_mut(v.as_obj()).elements[used as usize] = elem;
    }

    pub fn insert_in_list(&mut self, list_slot: i32, mut index: i64, element_slot: i32) {
        let v = self.api_get(list_slot);
        let elem = self.api_get(element_slot);
        let count = self.heap.list(v.as_obj()).elements.len();
        if index < 0 {
            index = count as i64 + 1 + index;
        }
        debug_assert!(index <= count as i64);
        self.list_insert(v.as_obj(), elem, index as usize);
    }

    pub fn get_map_count(&self, slot: i32) -> usize {
        let v = self.api_get(slot);
        self.heap.map(v.as_obj()).count
    }

    pub fn get_map_contains_key(&mut self, map_slot: i32, key_slot: i32) -> bool {
        let key = self.api_get(key_slot);
        if !self.validate_key(key) {
            return false;
        }
        let m = self.api_get(map_slot);
        let value = self.map_get(m.as_obj(), key);
        !value.is_undefined()
    }

    pub fn get_map_value(&mut self, map_slot: i32, key_slot: i32, value_slot: i32) {
        let m = self.api_get(map_slot);
        let key = self.api_get(key_slot);
        let mut value = self.map_get(m.as_obj(), key);
        if value.is_undefined() {
            value = NULL_VAL;
        }
        self.api_set(value_slot, value);
    }

    pub fn set_map_value(&mut self, map_slot: i32, key_slot: i32, value_slot: i32) {
        let key = self.api_get(key_slot);
        if !self.validate_key(key) {
            return;
        }
        let value = self.api_get(value_slot);
        let m = self.api_get(map_slot);
        self.map_set(m.as_obj(), key, value);
    }

    pub fn remove_map_value(&mut self, map_slot: i32, key_slot: i32, removed_value_slot: i32) {
        let key = self.api_get(key_slot);
        if !self.validate_key(key) {
            return;
        }
        let m = self.api_get(map_slot);
        let removed = self.map_remove_key(m.as_obj(), key);
        self.api_set(removed_value_slot, removed);
    }

    fn validate_key(&mut self, arg: Value) -> bool {
        if arg.is_bool() || arg.is_null() || arg.is_num() {
            return true;
        }
        if arg.is_obj()
            && matches!(
                self.heap.get(arg.as_obj_unchecked()).data,
                ObjData::Class(_) | ObjData::Range(_) | ObjData::Str(_)
            )
        {
            return true;
        }
        self.heap.fiber_mut(self.fiber).error = self.new_string_len(b"Key must be a value type.");
        false
    }

    pub fn get_variable(&mut self, module: &[u8], name: &[u8], slot: i32) {
        let module_name = self.new_string_len(module);
        self.push_root(module_name.as_obj());
        let module_obj = self.get_module(module_name);
        debug_assert!(module_obj != NULL_OBJ, "Could not find module.");
        self.pop_root();
        let variable_slot = self.module_find_variable(module_obj, name);
        debug_assert!(variable_slot != -1, "Could not find variable.");
        let v = self.heap.module(module_obj).variables[variable_slot as usize];
        self.api_set(slot, v);
    }

    pub fn has_variable(&mut self, module: &[u8], name: &[u8]) -> bool {
        let module_name = self.new_string_len(module);
        self.push_root(module_name.as_obj());
        let module_obj = self.get_module(module_name);
        debug_assert!(module_obj != NULL_OBJ, "Could not find module.");
        self.pop_root();
        self.module_find_variable(module_obj, name) != -1
    }

    pub fn has_module(&mut self, module: &[u8]) -> bool {
        let module_name = self.new_string_len(module);
        self.push_root(module_name.as_obj());
        let module_obj = self.get_module(module_name);
        self.pop_root();
        module_obj != NULL_OBJ
    }

    pub fn abort_fiber(&mut self, slot: i32) {
        let v = self.api_get(slot);
        let fiber = self.fiber;
        self.heap.fiber_mut(fiber).error = v;
    }

    pub fn get_user_data(&self) -> usize {
        self.config.user_data
    }
    pub fn set_user_data(&mut self, data: usize) {
        self.config.user_data = data;
    }

    /// Direct fiber stack access for primitives (args base + index).
    #[inline]
    pub fn arg(&self, args: usize, i: usize) -> Value {
        let f = self.heap.fiber(self.fiber);
        f.stack[args + i]
    }
    #[inline]
    pub fn set_arg(&mut self, args: usize, i: usize, v: Value) {
        let f = self.heap.fiber_mut(self.fiber);
        f.stack[args + i] = v;
    }
}

/// findEntry over an entries slice. Returns (found, index).
fn find_entry(heap: &Heap, entries: &[MapEntry], key: Value) -> (bool, usize) {
    if entries.is_empty() {
        return (false, 0);
    }
    let capacity = entries.len();
    let start_index = (hash_value(heap, key) as usize) % capacity;
    let mut index = start_index;
    let mut tombstone: Option<usize> = None;
    loop {
        let entry = &entries[index];
        if entry.key.is_undefined() {
            if entry.value.is_false() {
                // Empty slot: end of probe.
                return (false, tombstone.unwrap_or(index));
            } else if tombstone.is_none() {
                tombstone = Some(index);
            }
        } else if values_equal(heap, entry.key, key) {
            return (true, index);
        }
        index = (index + 1) % capacity;
        if index == start_index {
            break;
        }
    }
    (
        false,
        tombstone.expect("map should have tombstones or empty entries"),
    )
}
