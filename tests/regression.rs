// Regression tests for bugs found during the port, plus API smoke tests.

use std::sync::atomic::{AtomicI32, Ordering};
use wren_rs::value::*;
use wren_rs::vm::*;

fn interp(src: &str) -> WrenInterpretResult {
    let mut vm = WrenVM::new_vm(None);
    let r = vm.interpret(Some(b"main"), src.as_bytes());
    vm.free_vm();
    r
}

#[test]
fn value_round_trips() {
    assert!(Value::num(1.5).is_num());
    assert_eq!(Value::num(-0.0).as_num().to_bits(), (-0.0f64).to_bits());
    assert!(TRUE_VAL.is_bool() && TRUE_VAL.as_bool());
    assert!(FALSE_VAL.is_bool() && !FALSE_VAL.as_bool());
    assert!(NULL_VAL.is_null());
    assert!(UNDEFINED_VAL.is_undefined());
}

#[test]
fn hash_bits_is_thomas_wang() {
    // Wren uses the Thomas Wang 64-bit->32-bit hash; pin a known value.
    let h = hash_bits(0x9e3779b97f4a7c15);
    assert_eq!(h, 0x2d4beb93);
}

#[test]
fn nan_boxed_objs_are_not_nums() {
    assert!(!NULL_VAL.is_num());
    assert!(!TRUE_VAL.is_num());
    assert!(!FALSE_VAL.is_num());
}

#[test]
fn simple_programs() {
    assert_eq!(interp("System.print(1 + 2)"), WrenInterpretResult::Success);
    assert_eq!(
        interp("var x = [1,2,3]\nSystem.print(x.map {|i| i * 2}.toList)"),
        WrenInterpretResult::Success
    );
    assert_eq!(
        interp("class A {\n  construct new() {}\n  m() { 42 }\n}\nSystem.print(A.new().m())"),
        WrenInterpretResult::Success
    );
}

#[test]
fn compile_error_reported() {
    assert_eq!(
        interp("this is not code"),
        WrenInterpretResult::CompileError
    );
}

#[test]
fn runtime_error_reported() {
    assert_eq!(interp("1 +"), WrenInterpretResult::CompileError);
    assert_eq!(
        interp("Fiber.abort(\"boom\")"),
        WrenInterpretResult::RuntimeError
    );
}

#[test]
fn fibers_and_yield() {
    assert_eq!(
        interp(
            r#"var fiber = Fiber.new {
  Fiber.abort(null)
  Fiber.yield("value")
}
fiber.try()"#
        ),
        WrenInterpretResult::Success
    );
}

#[test]
fn meta_and_random_modules() {
    assert_eq!(
        interp("import \"meta\" for Meta\nMeta.eval(\"1+1\")"),
        WrenInterpretResult::Success
    );
    assert_eq!(
        interp("import \"random\" for Random\nRandom.new(1).int(100)"),
        WrenInterpretResult::Success
    );
}

static FINALIZED: AtomicI32 = AtomicI32::new(0);

fn alloc_foo(vm: &mut WrenVM) {
    vm.set_slot_new_foreign(0, 0, 4);
}

fn fin_foo(_data: &mut [u8]) {
    FINALIZED.fetch_add(1, Ordering::SeqCst);
}

fn bind_class(_vm: &mut WrenVM, _module: &[u8], class: &[u8]) -> WrenForeignClassMethods {
    let mut m = WrenForeignClassMethods::default();
    if class == b"Foo" {
        m.allocate = Some(alloc_foo);
        m.finalize = Some(fin_foo);
    }
    m
}

#[test]
fn foreign_finalizer_runs_on_free_vm() {
    FINALIZED.store(0, Ordering::SeqCst);
    let mut config = WrenConfiguration::new();
    config.bind_foreign_class_fn = Some(bind_class);
    let mut vm = WrenVM::new_vm(Some(config));
    let r = vm.interpret(
        Some(b"main"),
        b"foreign class Foo { construct new() {} }\nvar f = Foo.new()\n",
    );
    assert_eq!(r, WrenInterpretResult::Success);
    vm.free_vm();
    assert_eq!(FINALIZED.load(Ordering::SeqCst), 1);
}

#[test]
fn string_map_keys_and_nul_bytes() {
    assert_eq!(
        interp(
            r#"var m = {}
m["a\0b\0c"] = 1
if (m["a\0b\0c"] != 1) Fiber.abort("bad")"#
        ),
        WrenInterpretResult::Success
    );
}

#[test]
fn slots_and_handles_api() {
    let mut vm = WrenVM::new_vm(None);
    assert_eq!(
        vm.interpret(Some(b"main"), b"var x = 42"),
        WrenInterpretResult::Success
    );
    vm.ensure_slots(1);
    vm.get_variable(b"main", b"x", 0);
    assert_eq!(vm.get_slot_double(0), 42.0);
    let h = vm.get_slot_handle(0);
    vm.set_slot_null(0);
    vm.set_slot_handle(0, h);
    assert_eq!(vm.get_slot_double(0), 42.0);
    vm.release_handle(h);
    vm.free_vm();
}

#[test]
fn call_handle_api() {
    let mut vm = WrenVM::new_vm(None);
    let abs = vm.make_call_handle(b"abs");
    vm.ensure_slots(1);
    vm.set_slot_double(0, -5.0);
    let r = vm.call(abs);
    assert_eq!(r, WrenInterpretResult::Success);
    assert_eq!(vm.get_slot_double(0), 5.0);
    vm.release_handle(abs);
    vm.free_vm();
}

#[test]
fn gc_under_pressure() {
    // Exercise the mark/sweep GC across fibers, closures and maps.
    assert_eq!(
        interp(
            r#"var head
for (i in 1..50000) {
  head = { "next": head }
}
System.gc()"#
        ),
        WrenInterpretResult::Success
    );
}

#[test]
fn debug_dump_smoke() {
    // Exercise the wren_debug.c port: compile a function and disassemble it.
    let mut vm = WrenVM::new_vm(None);
    let closure = vm.compile_source(Some(b"main"), b"var x = 1 + 2\nx * 3", false, true);
    assert_ne!(closure, NULL_OBJ);
    vm.push_root(closure);
    let fn_ref = vm.heap.closure(closure).fn_ref;
    wren_rs::debug::dump_code(&vm, fn_ref);
    let fiber = vm.new_fiber(closure);
    vm.pop_root();
    wren_rs::debug::dump_stack(&vm, fiber);
    vm.free_vm();
}
