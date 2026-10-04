// Port of wren_core.c + wren_primitive.c: primitive methods for the core
// classes and the core bootstrap sequence. wren_core.wren is embedded and
// interpreted during initialize_core(), exactly like the C version.

use crate::utils::*;
use crate::value::*;
use crate::vm::{num_to_i32_c, num_to_u32, Fmt, WrenVM};

pub const CORE_MODULE_SOURCE: &[u8] = include_bytes!("wren_core.wren");

type Prim = fn(&mut WrenVM, usize) -> bool;

// -- RETURN_* helpers ------------------------------------------------------------

#[inline]
fn aget(vm: &WrenVM, args: usize, i: usize) -> Value {
    vm.heap.fiber(vm.fiber).stack[args + i]
}

#[inline]
fn aset(vm: &mut WrenVM, args: usize, i: usize, v: Value) {
    vm.heap.fiber_mut(vm.fiber).stack[args + i] = v;
}

/// RETURN_VAL — evaluates `$v` first (avoids borrow conflicts), then stores
/// it in the result slot and returns true.
macro_rules! ret {
    ($vm:ident, $args:ident, $v:expr) => {{
        let __v = $v;
        aset($vm, $args, 0, __v);
        return true;
    }};
}

/// RETURN_ERROR: set the fiber's error to a new string, return false.
fn rerr(vm: &mut WrenVM, msg: &str) -> bool {
    let e = vm.new_string_len(msg.as_bytes());
    vm.heap.fiber_mut(vm.fiber).error = e;
    false
}

/// RETURN_ERROR_FMT with one argument substituted at '$' or '@'.
fn rerr_fmt1(vm: &mut WrenVM, fmt: &str, arg: Fmt) -> bool {
    let e = vm.string_format(fmt, &[arg]);
    vm.heap.fiber_mut(vm.fiber).error = e;
    false
}

fn is_string(vm: &WrenVM, v: Value) -> bool {
    v.is_obj() && matches!(vm.heap.get(v.as_obj_unchecked()).data, ObjData::Str(_))
}
fn is_closure(vm: &WrenVM, v: Value) -> bool {
    v.is_obj() && matches!(vm.heap.get(v.as_obj_unchecked()).data, ObjData::Closure(_))
}
fn is_range(vm: &WrenVM, v: Value) -> bool {
    v.is_obj() && matches!(vm.heap.get(v.as_obj_unchecked()).data, ObjData::Range(_))
}
fn is_class(vm: &WrenVM, v: Value) -> bool {
    v.is_obj() && matches!(vm.heap.get(v.as_obj_unchecked()).data, ObjData::Class(_))
}

// -- validators (wren_primitive.c) -------------------------------------------------

fn validate_fn(vm: &mut WrenVM, arg: Value, arg_name: &str) -> bool {
    if is_closure(vm, arg) {
        return true;
    }
    rerr_fmt1(vm, "$ must be a function.", Fmt::S(arg_name.as_bytes()))
}

fn validate_num(vm: &mut WrenVM, arg: Value, arg_name: &str) -> bool {
    if arg.is_num() {
        return true;
    }
    rerr_fmt1(vm, "$ must be a number.", Fmt::S(arg_name.as_bytes()))
}

fn validate_int_value(vm: &mut WrenVM, value: f64, arg_name: &str) -> bool {
    if value.trunc() == value {
        return true;
    }
    rerr_fmt1(vm, "$ must be an integer.", Fmt::S(arg_name.as_bytes()))
}

fn validate_int(vm: &mut WrenVM, arg: Value, arg_name: &str) -> bool {
    if !validate_num(vm, arg, arg_name) {
        return false;
    }
    validate_int_value(vm, arg.as_num(), arg_name)
}

fn validate_key(vm: &mut WrenVM, arg: Value) -> bool {
    if map_is_valid_key(&vm.heap, arg) {
        return true;
    }
    rerr(vm, "Key must be a value type.")
}

fn validate_index_value(vm: &mut WrenVM, count: u32, value: f64, arg_name: &str) -> u32 {
    if !validate_int_value(vm, value, arg_name) {
        return u32::MAX;
    }
    let mut value = value;
    if value < 0.0 {
        value = count as f64 + value;
    }
    if value >= 0.0 && value < count as f64 {
        return value as u32;
    }
    rerr_fmt1(vm, "$ out of bounds.", Fmt::S(arg_name.as_bytes()));
    u32::MAX
}

fn validate_index(vm: &mut WrenVM, arg: Value, count: u32, arg_name: &str) -> u32 {
    if !validate_num(vm, arg, arg_name) {
        return u32::MAX;
    }
    validate_index_value(vm, count, arg.as_num(), arg_name)
}

fn validate_string(vm: &mut WrenVM, arg: Value, arg_name: &str) -> bool {
    if is_string(vm, arg) {
        return true;
    }
    rerr_fmt1(vm, "$ must be a string.", Fmt::S(arg_name.as_bytes()))
}

/// calculateRange — returns (start, length, step) or None on error.
fn calculate_range(
    vm: &mut WrenVM,
    range: ObjRef,
    length: &mut u32,
    step: &mut i32,
) -> Option<u32> {
    *step = 0;
    let (from_d, to_d, inclusive) = {
        let r = vm.heap.range(range);
        (r.from, r.to, r.is_inclusive)
    };
    let len = *length;

    // Edge case: an empty range at the end of a sequence.
    if from_d == len as f64 && to_d == if inclusive { -1.0 } else { len as f64 } {
        *length = 0;
        return Some(0);
    }

    let from = validate_index_value(vm, len, from_d, "Range start");
    if from == u32::MAX {
        return None;
    }

    let mut value = to_d;
    if !validate_int_value(vm, value, "Range end") {
        return None;
    }
    if value < 0.0 {
        value += len as f64;
    }
    if !inclusive {
        if value == from as f64 {
            *length = 0;
            return Some(from);
        }
        value += if value >= from as f64 { -1.0 } else { 1.0 };
    }
    if value < 0.0 || value >= len as f64 {
        rerr(vm, "Range end out of bounds.");
        return None;
    }
    let to = value as u32;
    *length = (from as i32 - to as i32).unsigned_abs() + 1;
    *step = if from < to { 1 } else { -1 };
    Some(from)
}

// -- Bool ---------------------------------------------------------------------------

fn bool_not(vm: &mut WrenVM, args: usize) -> bool {
    ret!(vm, args, Value::bool(!aget(vm, args, 0).as_bool()));
}

fn bool_to_string(vm: &mut WrenVM, args: usize) -> bool {
    let s = vm.new_string_len(if aget(vm, args, 0).as_bool() {
        b"true"
    } else {
        b"false"
    });
    ret!(vm, args, s);
}

// -- Class --------------------------------------------------------------------------

fn class_name(vm: &mut WrenVM, args: usize) -> bool {
    let name = vm.heap.class(aget(vm, args, 0).as_obj()).name;
    ret!(vm, args, Value::obj(name));
}

fn class_supertype(vm: &mut WrenVM, args: usize) -> bool {
    let sup = vm.heap.class(aget(vm, args, 0).as_obj()).superclass;
    if sup == NULL_OBJ {
        ret!(vm, args, NULL_VAL);
    }
    ret!(vm, args, Value::obj(sup));
}

fn class_to_string(vm: &mut WrenVM, args: usize) -> bool {
    class_name(vm, args)
}

fn class_attributes(vm: &mut WrenVM, args: usize) -> bool {
    let a = vm.heap.class(aget(vm, args, 0).as_obj()).attributes;
    ret!(vm, args, a);
}

// -- Fiber --------------------------------------------------------------------------

fn fiber_new(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_fn(vm, aget(vm, args, 1), "Argument") {
        return false;
    }
    let closure = aget(vm, args, 1).as_obj();
    if vm.heap.func(vm.heap.closure(closure).fn_ref).arity > 1 {
        return rerr(vm, "Function cannot take more than one parameter.");
    }
    let f = vm.new_fiber(closure);
    ret!(vm, args, Value::obj(f));
}

fn fiber_abort(vm: &mut WrenVM, args: usize) -> bool {
    let v = aget(vm, args, 1);
    vm.heap.fiber_mut(vm.fiber).error = v;
    v.is_null()
}

/// runFiber.
fn run_fiber(
    vm: &mut WrenVM,
    fiber: ObjRef,
    args: usize,
    is_call: bool,
    has_value: bool,
    verb: &str,
) -> bool {
    if vm.has_error(fiber) {
        return rerr_fmt1(vm, "Cannot $ an aborted fiber.", Fmt::S(verb.as_bytes()));
    }
    if is_call {
        if vm.heap.fiber(fiber).caller != NULL_OBJ {
            return rerr(vm, "Fiber has already been called.");
        }
        if vm.heap.fiber(fiber).state == FiberState::Root {
            return rerr(vm, "Cannot call root fiber.");
        }
        let caller = vm.fiber;
        vm.heap.fiber_mut(fiber).caller = caller;
    }
    if vm.heap.fiber(fiber).frames.is_empty() {
        return rerr_fmt1(vm, "Cannot $ a finished fiber.", Fmt::S(verb.as_bytes()));
    }

    if has_value {
        let cur = vm.fiber;
        vm.heap.fiber_mut(cur).stack_top -= 1;
    }

    let arg_value = if has_value {
        aget(vm, args, 1)
    } else {
        NULL_VAL
    };

    let started = {
        let f = vm.heap.fiber(fiber);
        f.frames.len() == 1 && f.frames[0].ip == 0
    };
    if started {
        let arity = {
            let f = vm.heap.fiber(fiber);
            vm.heap
                .func(vm.heap.closure(f.frames[0].closure).fn_ref)
                .arity
        };
        if arity == 1 {
            let f = vm.heap.fiber_mut(fiber);
            f.stack[f.stack_top] = arg_value;
            f.stack_top += 1;
        }
    } else {
        let f = vm.heap.fiber_mut(fiber);
        f.stack[f.stack_top - 1] = arg_value;
    }

    vm.fiber = fiber;
    false
}

fn fiber_call(vm: &mut WrenVM, args: usize) -> bool {
    let f = aget(vm, args, 0).as_obj();
    run_fiber(vm, f, args, true, false, "call")
}
fn fiber_call1(vm: &mut WrenVM, args: usize) -> bool {
    let f = aget(vm, args, 0).as_obj();
    run_fiber(vm, f, args, true, true, "call")
}
fn fiber_current(vm: &mut WrenVM, args: usize) -> bool {
    ret!(vm, args, Value::obj(vm.fiber));
}
fn fiber_error(vm: &mut WrenVM, args: usize) -> bool {
    let e = vm.heap.fiber(aget(vm, args, 0).as_obj()).error;
    ret!(vm, args, e);
}
fn fiber_is_done(vm: &mut WrenVM, args: usize) -> bool {
    let f = aget(vm, args, 0).as_obj();
    let done = vm.heap.fiber(f).frames.is_empty() || vm.has_error(f);
    ret!(vm, args, Value::bool(done));
}
fn fiber_suspend(vm: &mut WrenVM, _args: usize) -> bool {
    vm.fiber = NULL_OBJ;
    vm.api_stack = None;
    false
}
fn fiber_transfer(vm: &mut WrenVM, args: usize) -> bool {
    let f = aget(vm, args, 0).as_obj();
    run_fiber(vm, f, args, false, false, "transfer to")
}
fn fiber_transfer1(vm: &mut WrenVM, args: usize) -> bool {
    let f = aget(vm, args, 0).as_obj();
    run_fiber(vm, f, args, false, true, "transfer to")
}
fn fiber_transfer_error(vm: &mut WrenVM, args: usize) -> bool {
    let f = aget(vm, args, 0).as_obj();
    // Capture args[1] before run_fiber switches vm.fiber.
    let e = aget(vm, args, 1);
    run_fiber(vm, f, args, false, true, "transfer to");
    vm.heap.fiber_mut(vm.fiber).error = e;
    false
}
fn fiber_try(vm: &mut WrenVM, args: usize) -> bool {
    let f = aget(vm, args, 0).as_obj();
    run_fiber(vm, f, args, true, false, "try");
    if !vm.has_error(vm.fiber) {
        let cur = vm.fiber;
        vm.heap.fiber_mut(cur).state = FiberState::Try;
    }
    false
}
fn fiber_try1(vm: &mut WrenVM, args: usize) -> bool {
    let f = aget(vm, args, 0).as_obj();
    run_fiber(vm, f, args, true, true, "try");
    if !vm.has_error(vm.fiber) {
        let cur = vm.fiber;
        vm.heap.fiber_mut(cur).state = FiberState::Try;
    }
    false
}
fn fiber_yield(vm: &mut WrenVM, _args: usize) -> bool {
    let current = vm.fiber;
    let caller = vm.heap.fiber(current).caller;
    vm.fiber = caller;
    {
        let f = vm.heap.fiber_mut(current);
        f.caller = NULL_OBJ;
        f.state = FiberState::Other;
    }
    if vm.fiber != NULL_OBJ {
        let f = vm.heap.fiber_mut(vm.fiber);
        f.stack[f.stack_top - 1] = NULL_VAL;
    }
    false
}
fn fiber_yield1(vm: &mut WrenVM, args: usize) -> bool {
    let current = vm.fiber;
    // Capture args[1] before switching vm.fiber.
    let v = aget(vm, args, 1);
    let caller = vm.heap.fiber(current).caller;
    vm.fiber = caller;
    {
        let f = vm.heap.fiber_mut(current);
        f.caller = NULL_OBJ;
        f.state = FiberState::Other;
    }
    if vm.fiber != NULL_OBJ {
        let f = vm.heap.fiber_mut(vm.fiber);
        f.stack[f.stack_top - 1] = v;
        vm.heap.fiber_mut(current).stack_top -= 1;
    }
    false
}

// -- Fn ------------------------------------------------------------------------------

fn fn_new(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_fn(vm, aget(vm, args, 1), "Argument") {
        return false;
    }
    ret!(vm, args, aget(vm, args, 1));
}
fn fn_arity(vm: &mut WrenVM, args: usize) -> bool {
    let c = aget(vm, args, 0).as_obj();
    let a = vm.heap.func(vm.heap.closure(c).fn_ref).arity;
    ret!(vm, args, Value::num(a as f64));
}

macro_rules! fn_call {
    ($name:ident, $n:expr) => {
        fn $name(vm: &mut WrenVM, args: usize) -> bool {
            let closure = aget(vm, args, 0).as_obj();
            let fiber = vm.fiber;
            vm.call_function(fiber, closure, $n + 1);
            false
        }
    };
}
fn_call!(fn_call0, 0);
fn_call!(fn_call1, 1);
fn_call!(fn_call2, 2);
fn_call!(fn_call3, 3);
fn_call!(fn_call4, 4);
fn_call!(fn_call5, 5);
fn_call!(fn_call6, 6);
fn_call!(fn_call7, 7);
fn_call!(fn_call8, 8);
fn_call!(fn_call9, 9);
fn_call!(fn_call10, 10);
fn_call!(fn_call11, 11);
fn_call!(fn_call12, 12);
fn_call!(fn_call13, 13);
fn_call!(fn_call14, 14);
fn_call!(fn_call15, 15);
fn_call!(fn_call16, 16);

fn fn_to_string(vm: &mut WrenVM, args: usize) -> bool {
    let s = vm.new_string_len(b"<fn>");
    ret!(vm, args, s);
}

// -- List -----------------------------------------------------------------------------

fn list_filled(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_int(vm, aget(vm, args, 1), "Size") {
        return false;
    }
    if aget(vm, args, 1).as_num() < 0.0 {
        return rerr(vm, "Size cannot be negative.");
    }
    let size = aget(vm, args, 1).as_num() as u32;
    let list = vm.new_list(size as usize);
    let fill = aget(vm, args, 2);
    for i in 0..size as usize {
        vm.heap.list_mut(list).elements[i] = fill;
    }
    ret!(vm, args, Value::obj(list));
}
fn list_new(vm: &mut WrenVM, args: usize) -> bool {
    let l = vm.new_list(0);
    ret!(vm, args, Value::obj(l));
}
fn list_add(vm: &mut WrenVM, args: usize) -> bool {
    let l = aget(vm, args, 0).as_obj();
    let v = aget(vm, args, 1);
    vm.heap.list_mut(l).elements.push(v);
    ret!(vm, args, v);
}
fn list_add_core(vm: &mut WrenVM, args: usize) -> bool {
    let l = aget(vm, args, 0).as_obj();
    let v = aget(vm, args, 1);
    vm.heap.list_mut(l).elements.push(v);
    ret!(vm, args, aget(vm, args, 0));
}
fn list_clear(vm: &mut WrenVM, args: usize) -> bool {
    let l = aget(vm, args, 0).as_obj();
    vm.heap.list_mut(l).elements.clear();
    ret!(vm, args, NULL_VAL);
}
fn list_count(vm: &mut WrenVM, args: usize) -> bool {
    let l = aget(vm, args, 0).as_obj();
    let n = vm.heap.list(l).elements.len();
    ret!(vm, args, Value::num(n as f64));
}
fn list_insert(vm: &mut WrenVM, args: usize) -> bool {
    let l = aget(vm, args, 0).as_obj();
    let count = vm.heap.list(l).elements.len() as u32;
    let index = validate_index(vm, aget(vm, args, 1), count + 1, "Index");
    if index == u32::MAX {
        return false;
    }
    let v = aget(vm, args, 2);
    vm.list_insert(l, v, index as usize);
    ret!(vm, args, v);
}
fn list_iterate(vm: &mut WrenVM, args: usize) -> bool {
    let l = aget(vm, args, 0).as_obj();
    let count = vm.heap.list(l).elements.len();
    if aget(vm, args, 1).is_null() {
        if count == 0 {
            ret!(vm, args, FALSE_VAL);
        }
        ret!(vm, args, Value::num(0.0));
    }
    if !validate_int(vm, aget(vm, args, 1), "Iterator") {
        return false;
    }
    let index = aget(vm, args, 1).as_num();
    if index < 0.0 || index >= count as f64 - 1.0 {
        ret!(vm, args, FALSE_VAL);
    }
    ret!(vm, args, Value::num(index + 1.0));
}
fn list_iterator_value(vm: &mut WrenVM, args: usize) -> bool {
    let l = aget(vm, args, 0).as_obj();
    let count = vm.heap.list(l).elements.len() as u32;
    let index = validate_index(vm, aget(vm, args, 1), count, "Iterator");
    if index == u32::MAX {
        return false;
    }
    ret!(vm, args, vm.heap.list(l).elements[index as usize]);
}
fn list_remove_at(vm: &mut WrenVM, args: usize) -> bool {
    let l = aget(vm, args, 0).as_obj();
    let count = vm.heap.list(l).elements.len() as u32;
    let index = validate_index(vm, aget(vm, args, 1), count, "Index");
    if index == u32::MAX {
        return false;
    }
    let v = vm.list_remove_at(l, index as usize);
    ret!(vm, args, v);
}
fn list_remove_value(vm: &mut WrenVM, args: usize) -> bool {
    let l = aget(vm, args, 0).as_obj();
    let idx = vm.list_index_of(l, aget(vm, args, 1));
    if idx == -1 {
        ret!(vm, args, NULL_VAL);
    }
    let v = vm.list_remove_at(l, idx as usize);
    ret!(vm, args, v);
}
fn list_index_of(vm: &mut WrenVM, args: usize) -> bool {
    let l = aget(vm, args, 0).as_obj();
    let i = vm.list_index_of(l, aget(vm, args, 1));
    ret!(vm, args, Value::num(i as f64));
}
fn list_swap(vm: &mut WrenVM, args: usize) -> bool {
    let l = aget(vm, args, 0).as_obj();
    let count = vm.heap.list(l).elements.len() as u32;
    let ia = validate_index(vm, aget(vm, args, 1), count, "Index 0");
    if ia == u32::MAX {
        return false;
    }
    let ib = validate_index(vm, aget(vm, args, 2), count, "Index 1");
    if ib == u32::MAX {
        return false;
    }
    let lref = vm.heap.list_mut(l);
    lref.elements.swap(ia as usize, ib as usize);
    ret!(vm, args, NULL_VAL);
}
fn list_subscript(vm: &mut WrenVM, args: usize) -> bool {
    let l = aget(vm, args, 0).as_obj();
    let count = vm.heap.list(l).elements.len() as u32;
    if aget(vm, args, 1).is_num() {
        let index = validate_index(vm, aget(vm, args, 1), count, "Subscript");
        if index == u32::MAX {
            return false;
        }
        ret!(vm, args, vm.heap.list(l).elements[index as usize]);
    }
    if !is_range(vm, aget(vm, args, 1)) {
        return rerr(vm, "Subscript must be a number or a range.");
    }
    let range = aget(vm, args, 1).as_obj();
    let mut cnt = count;
    let mut step = 0;
    let start = match calculate_range(vm, range, &mut cnt, &mut step) {
        Some(s) => s,
        None => return false,
    };
    let result = vm.new_list(cnt as usize);
    let elems = vm.heap.list(l).elements.clone();
    for i in 0..cnt as usize {
        let idx = (start as i32 + i as i32 * step) as usize;
        vm.heap.list_mut(result).elements[i] = elems[idx];
    }
    ret!(vm, args, Value::obj(result));
}
fn list_subscript_setter(vm: &mut WrenVM, args: usize) -> bool {
    let l = aget(vm, args, 0).as_obj();
    let count = vm.heap.list(l).elements.len() as u32;
    let index = validate_index(vm, aget(vm, args, 1), count, "Subscript");
    if index == u32::MAX {
        return false;
    }
    let v = aget(vm, args, 2);
    vm.heap.list_mut(l).elements[index as usize] = v;
    ret!(vm, args, v);
}

// -- Map -------------------------------------------------------------------------------

fn map_new(vm: &mut WrenVM, args: usize) -> bool {
    let m = vm.new_map();
    ret!(vm, args, Value::obj(m));
}
fn map_subscript(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_key(vm, aget(vm, args, 1)) {
        return false;
    }
    let m = aget(vm, args, 0).as_obj();
    let v = vm.map_get(m, aget(vm, args, 1));
    if v.is_undefined() {
        ret!(vm, args, NULL_VAL);
    }
    ret!(vm, args, v);
}
fn map_subscript_setter(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_key(vm, aget(vm, args, 1)) {
        return false;
    }
    let m = aget(vm, args, 0).as_obj();
    let v = aget(vm, args, 2);
    let k = aget(vm, args, 1);
    vm.map_set(m, k, v);
    ret!(vm, args, v);
}
fn map_add_core(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_key(vm, aget(vm, args, 1)) {
        return false;
    }
    let m = aget(vm, args, 0).as_obj();
    vm.map_set(m, aget(vm, args, 1), aget(vm, args, 2));
    ret!(vm, args, aget(vm, args, 0));
}
fn map_clear(vm: &mut WrenVM, args: usize) -> bool {
    let m = aget(vm, args, 0).as_obj();
    vm.map_clear(m);
    ret!(vm, args, NULL_VAL);
}
fn map_contains_key(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_key(vm, aget(vm, args, 1)) {
        return false;
    }
    let m = aget(vm, args, 0).as_obj();
    let v = vm.map_get(m, aget(vm, args, 1));
    ret!(vm, args, Value::bool(!v.is_undefined()));
}
fn map_count(vm: &mut WrenVM, args: usize) -> bool {
    let m = aget(vm, args, 0).as_obj();
    ret!(vm, args, Value::num(vm.heap.map(m).count as f64));
}
fn map_iterate(vm: &mut WrenVM, args: usize) -> bool {
    let m = aget(vm, args, 0).as_obj();
    let (count, cap) = {
        let mm = vm.heap.map(m);
        (mm.count, mm.entries.len())
    };
    if count == 0 {
        ret!(vm, args, FALSE_VAL);
    }
    let mut index: u32 = 0;
    if !aget(vm, args, 1).is_null() {
        if !validate_int(vm, aget(vm, args, 1), "Iterator") {
            return false;
        }
        if aget(vm, args, 1).as_num() < 0.0 {
            ret!(vm, args, FALSE_VAL);
        }
        index = num_to_u32(aget(vm, args, 1).as_num());
        if index as usize >= cap {
            ret!(vm, args, FALSE_VAL);
        }
        index += 1;
    }
    while (index as usize) < cap {
        if !vm.heap.map(m).entries[index as usize].key.is_undefined() {
            ret!(vm, args, Value::num(index as f64));
        }
        index += 1;
    }
    ret!(vm, args, FALSE_VAL);
}
fn map_remove(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_key(vm, aget(vm, args, 1)) {
        return false;
    }
    let m = aget(vm, args, 0).as_obj();
    let v = vm.map_remove_key(m, aget(vm, args, 1));
    ret!(vm, args, v);
}
fn map_key_iterator_value(vm: &mut WrenVM, args: usize) -> bool {
    let m = aget(vm, args, 0).as_obj();
    let cap = vm.heap.map(m).entries.len() as u32;
    let index = validate_index(vm, aget(vm, args, 1), cap, "Iterator");
    if index == u32::MAX {
        return false;
    }
    let e = vm.heap.map(m).entries[index as usize];
    if e.key.is_undefined() {
        return rerr(vm, "Invalid map iterator.");
    }
    ret!(vm, args, e.key);
}
fn map_value_iterator_value(vm: &mut WrenVM, args: usize) -> bool {
    let m = aget(vm, args, 0).as_obj();
    let cap = vm.heap.map(m).entries.len() as u32;
    let index = validate_index(vm, aget(vm, args, 1), cap, "Iterator");
    if index == u32::MAX {
        return false;
    }
    let e = vm.heap.map(m).entries[index as usize];
    if e.key.is_undefined() {
        return rerr(vm, "Invalid map iterator.");
    }
    ret!(vm, args, e.value);
}

// -- Null -------------------------------------------------------------------------------

fn null_not(vm: &mut WrenVM, args: usize) -> bool {
    ret!(vm, args, TRUE_VAL);
}
fn null_to_string(vm: &mut WrenVM, args: usize) -> bool {
    let s = vm.new_string_len(b"null");
    ret!(vm, args, s);
}

// -- Num --------------------------------------------------------------------------------

fn num_from_string(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_string(vm, aget(vm, args, 1), "Argument") {
        return false;
    }
    let bytes = vm.heap.string(aget(vm, args, 1).as_obj()).bytes.clone();
    if bytes.is_empty() {
        ret!(vm, args, NULL_VAL);
    }
    let (number, consumed, erange) = crate::compiler::parse_double(&bytes);
    let mut end = consumed;
    while end < bytes.len() && bytes[end].is_ascii_whitespace() {
        end += 1;
    }
    if erange {
        return rerr(vm, "Number literal is too large.");
    }
    if end < bytes.len() {
        ret!(vm, args, NULL_VAL);
    }
    ret!(vm, args, Value::num(number));
}

macro_rules! num_const {
    ($name:ident, $v:expr) => {
        fn $name(vm: &mut WrenVM, args: usize) -> bool {
            ret!(vm, args, Value::num($v));
        }
    };
}
num_const!(num_infinity, f64::INFINITY);
num_const!(num_nan, f64::NAN);
num_const!(num_pi, std::f64::consts::PI);
num_const!(num_tau, std::f64::consts::TAU);
num_const!(num_largest, f64::MAX);
num_const!(num_smallest, f64::MIN_POSITIVE);
num_const!(num_max_safe_integer, 9007199254740991.0);
num_const!(num_min_safe_integer, -9007199254740991.0);

macro_rules! num_infix {
    ($name:ident, $op:tt, bool_out) => {
        fn $name(vm: &mut WrenVM, args: usize) -> bool {
            if !validate_num(vm, aget(vm, args, 1), "Right operand") { return false; }
            ret!(vm, args, Value::bool(aget(vm, args, 0).as_num() $op aget(vm, args, 1).as_num()));
        }
    };
    ($name:ident, $op:tt) => {
        fn $name(vm: &mut WrenVM, args: usize) -> bool {
            if !validate_num(vm, aget(vm, args, 1), "Right operand") { return false; }
            ret!(vm, args, Value::num(aget(vm, args, 0).as_num() $op aget(vm, args, 1).as_num()));
        }
    };
}
num_infix!(num_minus, -);
num_infix!(num_plus, +);
num_infix!(num_multiply, *);
num_infix!(num_divide, /);
num_infix!(num_lt, <, bool_out);
num_infix!(num_gt, >, bool_out);
num_infix!(num_lte, <=, bool_out);
num_infix!(num_gte, >=, bool_out);

macro_rules! num_bitwise {
    ($name:ident, $op:tt) => {
        fn $name(vm: &mut WrenVM, args: usize) -> bool {
            if !validate_num(vm, aget(vm, args, 1), "Right operand") { return false; }
            let l = num_to_u32(aget(vm, args, 0).as_num());
            let r = num_to_u32(aget(vm, args, 1).as_num());
            ret!(vm, args, Value::num((l $op r) as f64));
        }
    };
}
num_bitwise!(num_bitwise_and, &);
num_bitwise!(num_bitwise_or, |);
num_bitwise!(num_bitwise_xor, ^);
num_bitwise!(num_bitwise_left_shift, <<);
num_bitwise!(num_bitwise_right_shift, >>);

macro_rules! num_fn {
    ($name:ident, $f:expr) => {
        fn $name(vm: &mut WrenVM, args: usize) -> bool {
            ret!(vm, args, Value::num($f(aget(vm, args, 0).as_num())));
        }
    };
}
num_fn!(num_abs, f64::abs);
num_fn!(num_acos, f64::acos);
num_fn!(num_asin, f64::asin);
num_fn!(num_atan, f64::atan);
num_fn!(num_cbrt, f64::cbrt);
num_fn!(num_ceil, f64::ceil);
num_fn!(num_cos, f64::cos);
num_fn!(num_floor, f64::floor);
num_fn!(num_negate, |x: f64| -x);
num_fn!(num_round, f64::round);
num_fn!(num_sin, f64::sin);
num_fn!(num_sqrt, f64::sqrt);
num_fn!(num_tan, f64::tan);
num_fn!(num_log, f64::ln);
num_fn!(num_log2, f64::log2);
num_fn!(num_exp, f64::exp);

fn num_mod(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_num(vm, aget(vm, args, 1), "Right operand") {
        return false;
    }
    ret!(
        vm,
        args,
        Value::num(aget(vm, args, 0).as_num() % aget(vm, args, 1).as_num())
    );
}
fn num_eqeq(vm: &mut WrenVM, args: usize) -> bool {
    if !aget(vm, args, 1).is_num() {
        ret!(vm, args, FALSE_VAL);
    }
    ret!(
        vm,
        args,
        Value::bool(aget(vm, args, 0).as_num() == aget(vm, args, 1).as_num())
    );
}
fn num_bangeq(vm: &mut WrenVM, args: usize) -> bool {
    if !aget(vm, args, 1).is_num() {
        ret!(vm, args, TRUE_VAL);
    }
    ret!(
        vm,
        args,
        Value::bool(aget(vm, args, 0).as_num() != aget(vm, args, 1).as_num())
    );
}
fn num_bitwise_not(vm: &mut WrenVM, args: usize) -> bool {
    ret!(
        vm,
        args,
        Value::num(!num_to_u32(aget(vm, args, 0).as_num()) as f64)
    );
}
fn num_dot_dot(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_num(vm, aget(vm, args, 1), "Right hand side of range") {
        return false;
    }
    let (from, to) = (aget(vm, args, 0).as_num(), aget(vm, args, 1).as_num());
    let r = vm.new_range(from, to, true);
    ret!(vm, args, r);
}
fn num_dot_dot_dot(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_num(vm, aget(vm, args, 1), "Right hand side of range") {
        return false;
    }
    let (from, to) = (aget(vm, args, 0).as_num(), aget(vm, args, 1).as_num());
    let r = vm.new_range(from, to, false);
    ret!(vm, args, r);
}
fn num_atan2(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_num(vm, aget(vm, args, 1), "x value") {
        return false;
    }
    ret!(
        vm,
        args,
        Value::num(aget(vm, args, 0).as_num().atan2(aget(vm, args, 1).as_num()))
    );
}
fn num_min(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_num(vm, aget(vm, args, 1), "Other value") {
        return false;
    }
    let (v, o) = (aget(vm, args, 0).as_num(), aget(vm, args, 1).as_num());
    ret!(vm, args, Value::num(if v <= o { v } else { o }));
}
fn num_max(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_num(vm, aget(vm, args, 1), "Other value") {
        return false;
    }
    let (v, o) = (aget(vm, args, 0).as_num(), aget(vm, args, 1).as_num());
    ret!(vm, args, Value::num(if v > o { v } else { o }));
}
fn num_clamp(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_num(vm, aget(vm, args, 1), "Min value") {
        return false;
    }
    if !validate_num(vm, aget(vm, args, 2), "Max value") {
        return false;
    }
    let (v, mn, mx) = (
        aget(vm, args, 0).as_num(),
        aget(vm, args, 1).as_num(),
        aget(vm, args, 2).as_num(),
    );
    ret!(
        vm,
        args,
        Value::num(if v < mn {
            mn
        } else if v > mx {
            mx
        } else {
            v
        })
    );
}
fn num_pow(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_num(vm, aget(vm, args, 1), "Power value") {
        return false;
    }
    ret!(
        vm,
        args,
        Value::num(aget(vm, args, 0).as_num().powf(aget(vm, args, 1).as_num()))
    );
}
fn num_fraction(vm: &mut WrenVM, args: usize) -> bool {
    // C uses modf(), whose fractional part keeps the sign of x even when it
    // is zero; Rust's fract() yields +0 there.
    let v = aget(vm, args, 0).as_num();
    let mut f = v.fract();
    if f == 0.0 {
        f = 0.0f64.copysign(v);
    }
    ret!(vm, args, Value::num(f));
}
fn num_is_infinity(vm: &mut WrenVM, args: usize) -> bool {
    ret!(
        vm,
        args,
        Value::bool(aget(vm, args, 0).as_num().is_infinite())
    );
}
fn num_is_integer(vm: &mut WrenVM, args: usize) -> bool {
    let v = aget(vm, args, 0).as_num();
    if v.is_nan() || v.is_infinite() {
        ret!(vm, args, FALSE_VAL);
    }
    ret!(vm, args, Value::bool(v.trunc() == v));
}
fn num_is_nan(vm: &mut WrenVM, args: usize) -> bool {
    ret!(vm, args, Value::bool(aget(vm, args, 0).as_num().is_nan()));
}
fn num_sign(vm: &mut WrenVM, args: usize) -> bool {
    let v = aget(vm, args, 0).as_num();
    ret!(
        vm,
        args,
        Value::num(if v > 0.0 {
            1.0
        } else if v < 0.0 {
            -1.0
        } else {
            0.0
        })
    );
}
fn num_to_string(vm: &mut WrenVM, args: usize) -> bool {
    let v = vm.num_to_string(aget(vm, args, 0).as_num());
    ret!(vm, args, v);
}
fn num_truncate(vm: &mut WrenVM, args: usize) -> bool {
    ret!(vm, args, Value::num(aget(vm, args, 0).as_num().trunc()));
}

// -- Object -----------------------------------------------------------------------------

fn object_same(vm: &mut WrenVM, args: usize) -> bool {
    ret!(
        vm,
        args,
        Value::bool(vm.values_equal(aget(vm, args, 1), aget(vm, args, 2)))
    );
}
fn object_not(vm: &mut WrenVM, args: usize) -> bool {
    ret!(vm, args, FALSE_VAL);
}
fn object_eqeq(vm: &mut WrenVM, args: usize) -> bool {
    ret!(
        vm,
        args,
        Value::bool(vm.values_equal(aget(vm, args, 0), aget(vm, args, 1)))
    );
}
fn object_bangeq(vm: &mut WrenVM, args: usize) -> bool {
    ret!(
        vm,
        args,
        Value::bool(!vm.values_equal(aget(vm, args, 0), aget(vm, args, 1)))
    );
}
fn object_is(vm: &mut WrenVM, args: usize) -> bool {
    if !is_class(vm, aget(vm, args, 1)) {
        return rerr(vm, "Right operand must be a class.");
    }
    let mut class_obj = vm.class_of(aget(vm, args, 0));
    let base = aget(vm, args, 1).as_obj();
    loop {
        if class_obj == base {
            ret!(vm, args, TRUE_VAL);
        }
        class_obj = vm.heap.class(class_obj).superclass;
        if class_obj == NULL_OBJ {
            break;
        }
    }
    ret!(vm, args, FALSE_VAL);
}
fn object_to_string(vm: &mut WrenVM, args: usize) -> bool {
    let obj = aget(vm, args, 0).as_obj();
    let name = vm.heap.class(vm.heap.get(obj).class).name;
    let s = vm.string_format("instance of @", &[Fmt::V(Value::obj(name))]);
    ret!(vm, args, s);
}
fn object_type(vm: &mut WrenVM, args: usize) -> bool {
    let c = vm.class_of(aget(vm, args, 0));
    ret!(vm, args, Value::obj(c));
}

// -- Range --------------------------------------------------------------------------------

fn range_from(vm: &mut WrenVM, args: usize) -> bool {
    ret!(
        vm,
        args,
        Value::num(vm.heap.range(aget(vm, args, 0).as_obj()).from)
    );
}
fn range_to(vm: &mut WrenVM, args: usize) -> bool {
    ret!(
        vm,
        args,
        Value::num(vm.heap.range(aget(vm, args, 0).as_obj()).to)
    );
}
fn range_min(vm: &mut WrenVM, args: usize) -> bool {
    let r = vm.heap.range(aget(vm, args, 0).as_obj());
    ret!(vm, args, Value::num(r.from.min(r.to)));
}
fn range_max(vm: &mut WrenVM, args: usize) -> bool {
    let r = vm.heap.range(aget(vm, args, 0).as_obj());
    ret!(vm, args, Value::num(r.from.max(r.to)));
}
fn range_is_inclusive(vm: &mut WrenVM, args: usize) -> bool {
    ret!(
        vm,
        args,
        Value::bool(vm.heap.range(aget(vm, args, 0).as_obj()).is_inclusive)
    );
}
fn range_iterate(vm: &mut WrenVM, args: usize) -> bool {
    let (from, to, inclusive) = {
        let r = vm.heap.range(aget(vm, args, 0).as_obj());
        (r.from, r.to, r.is_inclusive)
    };
    if from == to && !inclusive {
        ret!(vm, args, FALSE_VAL);
    }
    if aget(vm, args, 1).is_null() {
        ret!(vm, args, Value::num(from));
    }
    if !validate_num(vm, aget(vm, args, 1), "Iterator") {
        return false;
    }
    let mut it = aget(vm, args, 1).as_num();
    if from < to {
        it += 1.0;
        if it > to {
            ret!(vm, args, FALSE_VAL);
        }
    } else {
        it -= 1.0;
        if it < to {
            ret!(vm, args, FALSE_VAL);
        }
    }
    if !inclusive && it == to {
        ret!(vm, args, FALSE_VAL);
    }
    ret!(vm, args, Value::num(it));
}
fn range_iterator_value(vm: &mut WrenVM, args: usize) -> bool {
    ret!(vm, args, aget(vm, args, 1));
}
fn range_to_string(vm: &mut WrenVM, args: usize) -> bool {
    let (from, to, inclusive) = {
        let r = vm.heap.range(aget(vm, args, 0).as_obj());
        (r.from, r.to, r.is_inclusive)
    };
    let from_v = vm.num_to_string(from);
    let fobj = from_v.as_obj();
    vm.push_root(fobj);
    let to_v = vm.num_to_string(to);
    vm.push_root(to_v.as_obj());
    let result = vm.string_format(
        if inclusive { "@..@" } else { "@...@" },
        &[Fmt::V(from_v), Fmt::V(to_v)],
    );
    vm.pop_root();
    vm.pop_root();
    ret!(vm, args, result);
}

// -- String -------------------------------------------------------------------------------

fn string_from_code_point(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_int(vm, aget(vm, args, 1), "Code point") {
        return false;
    }
    let cp = num_to_i32_c(aget(vm, args, 1).as_num());
    if cp < 0 {
        return rerr(vm, "Code point cannot be negative.");
    }
    if cp > 0x10ffff {
        return rerr(vm, "Code point cannot be greater than 0x10ffff.");
    }
    let s = vm.string_from_code_point(cp);
    ret!(vm, args, s);
}
fn string_from_byte(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_int(vm, aget(vm, args, 1), "Byte") {
        return false;
    }
    let b = num_to_i32_c(aget(vm, args, 1).as_num());
    if b < 0 {
        return rerr(vm, "Byte cannot be negative.");
    }
    if b > 0xff {
        return rerr(vm, "Byte cannot be greater than 0xff.");
    }
    let s = vm.string_from_byte(b as u8);
    ret!(vm, args, s);
}
fn string_byte_at(vm: &mut WrenVM, args: usize) -> bool {
    let s = aget(vm, args, 0).as_obj();
    let len = vm.heap.string(s).bytes.len() as u32;
    let index = validate_index(vm, aget(vm, args, 1), len, "Index");
    if index == u32::MAX {
        return false;
    }
    let b = vm.heap.string(s).bytes[index as usize];
    ret!(vm, args, Value::num(b as f64));
}
fn string_byte_count(vm: &mut WrenVM, args: usize) -> bool {
    let s = aget(vm, args, 0).as_obj();
    let n = vm.heap.string(s).bytes.len();
    ret!(vm, args, Value::num(n as f64));
}
fn string_code_point_at(vm: &mut WrenVM, args: usize) -> bool {
    let s = aget(vm, args, 0).as_obj();
    let len = vm.heap.string(s).bytes.len() as u32;
    let index = validate_index(vm, aget(vm, args, 1), len, "Index");
    if index == u32::MAX {
        return false;
    }
    let bytes = vm.heap.string(s).bytes.clone();
    let i = index as usize;
    if (bytes[i] & 0xc0) == 0x80 {
        ret!(vm, args, Value::num(-1.0));
    }
    ret!(vm, args, Value::num(utf8_decode(&bytes[i..]) as f64));
}
fn string_contains(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_string(vm, aget(vm, args, 1), "Argument") {
        return false;
    }
    let (a, b) = (aget(vm, args, 0).as_obj(), aget(vm, args, 1).as_obj());
    let hs = vm.heap.string(a).bytes.clone();
    let nd = vm.heap.string(b).bytes.clone();
    let found = vm.string_find(&hs, &nd, 0) != u32::MAX;
    ret!(vm, args, Value::bool(found));
}
fn string_ends_with(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_string(vm, aget(vm, args, 1), "Argument") {
        return false;
    }
    let (a, b) = (aget(vm, args, 0).as_obj(), aget(vm, args, 1).as_obj());
    let (hs, nd) = (&vm.heap.string(a).bytes, &vm.heap.string(b).bytes);
    if nd.len() > hs.len() {
        ret!(vm, args, FALSE_VAL);
    }
    ret!(vm, args, Value::bool(hs[hs.len() - nd.len()..] == nd[..]));
}
fn string_index_of1(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_string(vm, aget(vm, args, 1), "Argument") {
        return false;
    }
    let (a, b) = (aget(vm, args, 0).as_obj(), aget(vm, args, 1).as_obj());
    let hs = vm.heap.string(a).bytes.clone();
    let nd = vm.heap.string(b).bytes.clone();
    let i = vm.string_find(&hs, &nd, 0);
    ret!(
        vm,
        args,
        Value::num(if i == u32::MAX { -1.0 } else { i as f64 })
    );
}
fn string_index_of2(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_string(vm, aget(vm, args, 1), "Argument") {
        return false;
    }
    let (a, b) = (aget(vm, args, 0).as_obj(), aget(vm, args, 1).as_obj());
    let len = vm.heap.string(a).bytes.len() as u32;
    let start = validate_index(vm, aget(vm, args, 2), len, "Start");
    if start == u32::MAX {
        return false;
    }
    let hs = vm.heap.string(a).bytes.clone();
    let nd = vm.heap.string(b).bytes.clone();
    let i = vm.string_find(&hs, &nd, start);
    ret!(
        vm,
        args,
        Value::num(if i == u32::MAX { -1.0 } else { i as f64 })
    );
}
fn string_iterate(vm: &mut WrenVM, args: usize) -> bool {
    let s = aget(vm, args, 0).as_obj();
    let len = vm.heap.string(s).bytes.len() as u32;
    if aget(vm, args, 1).is_null() {
        if len == 0 {
            ret!(vm, args, FALSE_VAL);
        }
        ret!(vm, args, Value::num(0.0));
    }
    if !validate_int(vm, aget(vm, args, 1), "Iterator") {
        return false;
    }
    if aget(vm, args, 1).as_num() < 0.0 {
        ret!(vm, args, FALSE_VAL);
    }
    let mut index = num_to_u32(aget(vm, args, 1).as_num());
    loop {
        index += 1;
        if index >= len {
            ret!(vm, args, FALSE_VAL);
        }
        if (vm.heap.string(s).bytes[index as usize] & 0xc0) != 0x80 {
            break;
        }
    }
    ret!(vm, args, Value::num(index as f64));
}
fn string_iterate_byte(vm: &mut WrenVM, args: usize) -> bool {
    let s = aget(vm, args, 0).as_obj();
    let len = vm.heap.string(s).bytes.len() as u32;
    if aget(vm, args, 1).is_null() {
        if len == 0 {
            ret!(vm, args, FALSE_VAL);
        }
        ret!(vm, args, Value::num(0.0));
    }
    if !validate_int(vm, aget(vm, args, 1), "Iterator") {
        return false;
    }
    if aget(vm, args, 1).as_num() < 0.0 {
        ret!(vm, args, FALSE_VAL);
    }
    let mut index = num_to_u32(aget(vm, args, 1).as_num());
    index += 1;
    if index >= len {
        ret!(vm, args, FALSE_VAL);
    }
    ret!(vm, args, Value::num(index as f64));
}
fn string_iterator_value(vm: &mut WrenVM, args: usize) -> bool {
    let s = aget(vm, args, 0).as_obj();
    let len = vm.heap.string(s).bytes.len() as u32;
    let index = validate_index(vm, aget(vm, args, 1), len, "Iterator");
    if index == u32::MAX {
        return false;
    }
    let v = vm.string_code_point_at(s, index);
    ret!(vm, args, v);
}
fn string_starts_with(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_string(vm, aget(vm, args, 1), "Argument") {
        return false;
    }
    let (a, b) = (aget(vm, args, 0).as_obj(), aget(vm, args, 1).as_obj());
    let (hs, nd) = (&vm.heap.string(a).bytes, &vm.heap.string(b).bytes);
    if nd.len() > hs.len() {
        ret!(vm, args, FALSE_VAL);
    }
    ret!(vm, args, Value::bool(hs[..nd.len()] == nd[..]));
}
fn string_plus(vm: &mut WrenVM, args: usize) -> bool {
    if !validate_string(vm, aget(vm, args, 1), "Right operand") {
        return false;
    }
    let s = vm.string_format(
        "@@",
        &[Fmt::V(aget(vm, args, 0)), Fmt::V(aget(vm, args, 1))],
    );
    ret!(vm, args, s);
}
fn string_subscript(vm: &mut WrenVM, args: usize) -> bool {
    let s = aget(vm, args, 0).as_obj();
    let len = vm.heap.string(s).bytes.len() as u32;
    if aget(vm, args, 1).is_num() {
        let index = validate_index(vm, aget(vm, args, 1), len, "Subscript");
        if index == u32::MAX {
            return false;
        }
        let v = vm.string_code_point_at(s, index);
        ret!(vm, args, v);
    }
    if !is_range(vm, aget(vm, args, 1)) {
        return rerr(vm, "Subscript must be a number or a range.");
    }
    let range = aget(vm, args, 1).as_obj();
    let mut cnt = len;
    let mut step = 0;
    let start = match calculate_range(vm, range, &mut cnt, &mut step) {
        Some(s) => s,
        None => return false,
    };
    let v = vm.new_string_from_range(s, start as i32, cnt, step);
    ret!(vm, args, v);
}
fn string_to_string(vm: &mut WrenVM, args: usize) -> bool {
    ret!(vm, args, aget(vm, args, 0));
}

// -- System --------------------------------------------------------------------------------

fn system_clock(vm: &mut WrenVM, args: usize) -> bool {
    let t = vm.start_time.elapsed().as_secs_f64();
    ret!(vm, args, Value::num(t));
}
fn system_gc(vm: &mut WrenVM, args: usize) -> bool {
    vm.collect_garbage();
    ret!(vm, args, NULL_VAL);
}
fn system_write_string(vm: &mut WrenVM, args: usize) -> bool {
    if let Some(cb) = vm.config.write_fn {
        let v = aget(vm, args, 1);
        let bytes = vm.heap.string(v.as_obj()).bytes.clone();
        cb(vm, &bytes);
    }
    ret!(vm, args, aget(vm, args, 1));
}

// -- bootstrap ------------------------------------------------------------------------------

fn define_class(vm: &mut WrenVM, module: ObjRef, name: &[u8]) -> ObjRef {
    let name_string = vm.new_string(name).as_obj();
    vm.push_root(name_string);
    let class_obj = vm.new_single_class(0, name_string);
    let mut line = 0;
    vm.define_variable(module, name, Value::obj(class_obj), &mut line);
    vm.pop_root();
    class_obj
}

fn primitive(vm: &mut WrenVM, class: ObjRef, sig: &[u8], f: Prim) {
    let symbol = vm.ensure_method_symbol(sig);
    vm.bind_method(class, symbol, Method::Primitive(f));
}

fn function_call(vm: &mut WrenVM, class: ObjRef, sig: &[u8], f: Prim) {
    let symbol = vm.ensure_method_symbol(sig);
    vm.bind_method(class, symbol, Method::FunctionCall(f));
}

fn meta(vm: &WrenVM, class: ObjRef) -> ObjRef {
    vm.heap.get(class).class
}

/// wrenInitializeCore.
pub fn initialize_core(vm: &mut WrenVM) {
    let core_module = vm.new_module(NULL_OBJ);
    vm.push_root(core_module);
    let modules = vm.modules;
    vm.map_set(modules, NULL_VAL, Value::obj(core_module));
    vm.pop_root();

    vm.object_class = define_class(vm, core_module, b"Object");
    let oc = vm.object_class;
    primitive(vm, oc, b"!", object_not);
    primitive(vm, oc, b"==(_)", object_eqeq);
    primitive(vm, oc, b"!=(_)", object_bangeq);
    primitive(vm, oc, b"is(_)", object_is);
    primitive(vm, oc, b"toString", object_to_string);
    primitive(vm, oc, b"type", object_type);

    vm.class_class = define_class(vm, core_module, b"Class");
    let cc = vm.class_class;
    vm.bind_superclass(cc, oc);
    primitive(vm, cc, b"name", class_name);
    primitive(vm, cc, b"supertype", class_supertype);
    primitive(vm, cc, b"toString", class_to_string);
    primitive(vm, cc, b"attributes", class_attributes);

    let object_metaclass = define_class(vm, core_module, b"Object metaclass");
    vm.heap.get_mut(vm.object_class).class = object_metaclass;
    vm.heap.get_mut(object_metaclass).class = cc;
    vm.heap.get_mut(cc).class = cc;
    vm.bind_superclass(object_metaclass, cc);
    primitive(vm, object_metaclass, b"same(_,_)", object_same);

    // The core module source defines everything else.
    vm.interpret(None, CORE_MODULE_SOURCE);

    vm.bool_class = vm.find_variable(core_module, b"Bool").as_obj();
    let bc = vm.bool_class;
    primitive(vm, bc, b"toString", bool_to_string);
    primitive(vm, bc, b"!", bool_not);

    vm.fiber_class = vm.find_variable(core_module, b"Fiber").as_obj();
    let fc = vm.fiber_class;
    primitive(vm, meta(vm, fc), b"new(_)", fiber_new);
    primitive(vm, meta(vm, fc), b"abort(_)", fiber_abort);
    primitive(vm, meta(vm, fc), b"current", fiber_current);
    primitive(vm, meta(vm, fc), b"suspend()", fiber_suspend);
    primitive(vm, meta(vm, fc), b"yield()", fiber_yield);
    primitive(vm, meta(vm, fc), b"yield(_)", fiber_yield1);
    primitive(vm, fc, b"call()", fiber_call);
    primitive(vm, fc, b"call(_)", fiber_call1);
    primitive(vm, fc, b"error", fiber_error);
    primitive(vm, fc, b"isDone", fiber_is_done);
    primitive(vm, fc, b"transfer()", fiber_transfer);
    primitive(vm, fc, b"transfer(_)", fiber_transfer1);
    primitive(vm, fc, b"transferError(_)", fiber_transfer_error);
    primitive(vm, fc, b"try()", fiber_try);
    primitive(vm, fc, b"try(_)", fiber_try1);

    vm.fn_class = vm.find_variable(core_module, b"Fn").as_obj();
    let fc = vm.fn_class;
    primitive(vm, meta(vm, fc), b"new(_)", fn_new);
    primitive(vm, fc, b"arity", fn_arity);
    function_call(vm, fc, b"call()", fn_call0);
    function_call(vm, fc, b"call(_)", fn_call1);
    function_call(vm, fc, b"call(_,_)", fn_call2);
    function_call(vm, fc, b"call(_,_,_)", fn_call3);
    function_call(vm, fc, b"call(_,_,_,_)", fn_call4);
    function_call(vm, fc, b"call(_,_,_,_,_)", fn_call5);
    function_call(vm, fc, b"call(_,_,_,_,_,_)", fn_call6);
    function_call(vm, fc, b"call(_,_,_,_,_,_,_)", fn_call7);
    function_call(vm, fc, b"call(_,_,_,_,_,_,_,_)", fn_call8);
    function_call(vm, fc, b"call(_,_,_,_,_,_,_,_,_)", fn_call9);
    function_call(vm, fc, b"call(_,_,_,_,_,_,_,_,_,_)", fn_call10);
    function_call(vm, fc, b"call(_,_,_,_,_,_,_,_,_,_,_)", fn_call11);
    function_call(vm, fc, b"call(_,_,_,_,_,_,_,_,_,_,_,_)", fn_call12);
    function_call(vm, fc, b"call(_,_,_,_,_,_,_,_,_,_,_,_,_)", fn_call13);
    function_call(vm, fc, b"call(_,_,_,_,_,_,_,_,_,_,_,_,_,_)", fn_call14);
    function_call(vm, fc, b"call(_,_,_,_,_,_,_,_,_,_,_,_,_,_,_)", fn_call15);
    function_call(vm, fc, b"call(_,_,_,_,_,_,_,_,_,_,_,_,_,_,_,_)", fn_call16);
    primitive(vm, fc, b"toString", fn_to_string);

    vm.null_class = vm.find_variable(core_module, b"Null").as_obj();
    let nc = vm.null_class;
    primitive(vm, nc, b"!", null_not);
    primitive(vm, nc, b"toString", null_to_string);

    vm.num_class = vm.find_variable(core_module, b"Num").as_obj();
    let nc = vm.num_class;
    primitive(vm, meta(vm, nc), b"fromString(_)", num_from_string);
    primitive(vm, meta(vm, nc), b"infinity", num_infinity);
    primitive(vm, meta(vm, nc), b"nan", num_nan);
    primitive(vm, meta(vm, nc), b"pi", num_pi);
    primitive(vm, meta(vm, nc), b"tau", num_tau);
    primitive(vm, meta(vm, nc), b"largest", num_largest);
    primitive(vm, meta(vm, nc), b"smallest", num_smallest);
    primitive(vm, meta(vm, nc), b"maxSafeInteger", num_max_safe_integer);
    primitive(vm, meta(vm, nc), b"minSafeInteger", num_min_safe_integer);
    primitive(vm, nc, b"-(_)", num_minus);
    primitive(vm, nc, b"+(_)", num_plus);
    primitive(vm, nc, b"*(_)", num_multiply);
    primitive(vm, nc, b"/(_)", num_divide);
    primitive(vm, nc, b"<(_)", num_lt);
    primitive(vm, nc, b">(_)", num_gt);
    primitive(vm, nc, b"<=(_)", num_lte);
    primitive(vm, nc, b">=(_)", num_gte);
    primitive(vm, nc, b"&(_)", num_bitwise_and);
    primitive(vm, nc, b"|(_)", num_bitwise_or);
    primitive(vm, nc, b"^(_)", num_bitwise_xor);
    primitive(vm, nc, b"<<(_)", num_bitwise_left_shift);
    primitive(vm, nc, b">>(_)", num_bitwise_right_shift);
    primitive(vm, nc, b"abs", num_abs);
    primitive(vm, nc, b"acos", num_acos);
    primitive(vm, nc, b"asin", num_asin);
    primitive(vm, nc, b"atan", num_atan);
    primitive(vm, nc, b"cbrt", num_cbrt);
    primitive(vm, nc, b"ceil", num_ceil);
    primitive(vm, nc, b"cos", num_cos);
    primitive(vm, nc, b"floor", num_floor);
    primitive(vm, nc, b"-", num_negate);
    primitive(vm, nc, b"round", num_round);
    primitive(vm, nc, b"min(_)", num_min);
    primitive(vm, nc, b"max(_)", num_max);
    primitive(vm, nc, b"clamp(_,_)", num_clamp);
    primitive(vm, nc, b"sin", num_sin);
    primitive(vm, nc, b"sqrt", num_sqrt);
    primitive(vm, nc, b"tan", num_tan);
    primitive(vm, nc, b"log", num_log);
    primitive(vm, nc, b"log2", num_log2);
    primitive(vm, nc, b"exp", num_exp);
    primitive(vm, nc, b"%(_)", num_mod);
    primitive(vm, nc, b"~", num_bitwise_not);
    primitive(vm, nc, b"..(_)", num_dot_dot);
    primitive(vm, nc, b"...(_)", num_dot_dot_dot);
    primitive(vm, nc, b"atan(_)", num_atan2);
    primitive(vm, nc, b"pow(_)", num_pow);
    primitive(vm, nc, b"fraction", num_fraction);
    primitive(vm, nc, b"isInfinity", num_is_infinity);
    primitive(vm, nc, b"isInteger", num_is_integer);
    primitive(vm, nc, b"isNan", num_is_nan);
    primitive(vm, nc, b"sign", num_sign);
    primitive(vm, nc, b"toString", num_to_string);
    primitive(vm, nc, b"truncate", num_truncate);
    primitive(vm, nc, b"==(_)", num_eqeq);
    primitive(vm, nc, b"!=(_)", num_bangeq);

    vm.string_class = vm.find_variable(core_module, b"String").as_obj();
    let sc = vm.string_class;
    primitive(
        vm,
        meta(vm, sc),
        b"fromCodePoint(_)",
        string_from_code_point,
    );
    primitive(vm, meta(vm, sc), b"fromByte(_)", string_from_byte);
    primitive(vm, sc, b"+(_)", string_plus);
    primitive(vm, sc, b"[_]", string_subscript);
    primitive(vm, sc, b"byteAt_(_)", string_byte_at);
    primitive(vm, sc, b"byteCount_", string_byte_count);
    primitive(vm, sc, b"codePointAt_(_)", string_code_point_at);
    primitive(vm, sc, b"contains(_)", string_contains);
    primitive(vm, sc, b"endsWith(_)", string_ends_with);
    primitive(vm, sc, b"indexOf(_)", string_index_of1);
    primitive(vm, sc, b"indexOf(_,_)", string_index_of2);
    primitive(vm, sc, b"iterate(_)", string_iterate);
    primitive(vm, sc, b"iterateByte_(_)", string_iterate_byte);
    primitive(vm, sc, b"iteratorValue(_)", string_iterator_value);
    primitive(vm, sc, b"startsWith(_)", string_starts_with);
    primitive(vm, sc, b"toString", string_to_string);

    vm.list_class = vm.find_variable(core_module, b"List").as_obj();
    let lc = vm.list_class;
    primitive(vm, meta(vm, lc), b"filled(_,_)", list_filled);
    primitive(vm, meta(vm, lc), b"new()", list_new);
    primitive(vm, lc, b"[_]", list_subscript);
    primitive(vm, lc, b"[_]=(_)", list_subscript_setter);
    primitive(vm, lc, b"add(_)", list_add);
    primitive(vm, lc, b"addCore_(_)", list_add_core);
    primitive(vm, lc, b"clear()", list_clear);
    primitive(vm, lc, b"count", list_count);
    primitive(vm, lc, b"insert(_,_)", list_insert);
    primitive(vm, lc, b"iterate(_)", list_iterate);
    primitive(vm, lc, b"iteratorValue(_)", list_iterator_value);
    primitive(vm, lc, b"removeAt(_)", list_remove_at);
    primitive(vm, lc, b"remove(_)", list_remove_value);
    primitive(vm, lc, b"indexOf(_)", list_index_of);
    primitive(vm, lc, b"swap(_,_)", list_swap);

    vm.map_class = vm.find_variable(core_module, b"Map").as_obj();
    let mc = vm.map_class;
    primitive(vm, meta(vm, mc), b"new()", map_new);
    primitive(vm, mc, b"[_]", map_subscript);
    primitive(vm, mc, b"[_]=(_)", map_subscript_setter);
    primitive(vm, mc, b"addCore_(_,_)", map_add_core);
    primitive(vm, mc, b"clear()", map_clear);
    primitive(vm, mc, b"containsKey(_)", map_contains_key);
    primitive(vm, mc, b"count", map_count);
    primitive(vm, mc, b"remove(_)", map_remove);
    primitive(vm, mc, b"iterate(_)", map_iterate);
    primitive(vm, mc, b"keyIteratorValue_(_)", map_key_iterator_value);
    primitive(vm, mc, b"valueIteratorValue_(_)", map_value_iterator_value);

    vm.range_class = vm.find_variable(core_module, b"Range").as_obj();
    let rc = vm.range_class;
    primitive(vm, rc, b"from", range_from);
    primitive(vm, rc, b"to", range_to);
    primitive(vm, rc, b"min", range_min);
    primitive(vm, rc, b"max", range_max);
    primitive(vm, rc, b"isInclusive", range_is_inclusive);
    primitive(vm, rc, b"iterate(_)", range_iterate);
    primitive(vm, rc, b"iteratorValue(_)", range_iterator_value);
    primitive(vm, rc, b"toString", range_to_string);

    let system_class = vm.find_variable(core_module, b"System").as_obj();
    let sysc = meta(vm, system_class);
    primitive(vm, sysc, b"clock", system_clock);
    primitive(vm, sysc, b"gc()", system_gc);
    primitive(vm, sysc, b"writeString_(_)", system_write_string);

    // String objects created during bootstrap have NULL class; fix them now.
    let sc = vm.string_class;
    for i in 0..vm.heap.objs.len() {
        if let Some(o) = &mut vm.heap.objs[i] {
            if matches!(o.data, ObjData::Str(_)) {
                o.class = sc;
            }
        }
    }
}
