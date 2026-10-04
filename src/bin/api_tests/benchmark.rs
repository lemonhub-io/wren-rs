// Port of test/api/benchmark.c
use wren_rs::vm::*;

fn arguments(vm: &mut WrenVM) {
    let result = vm.get_slot_double(1)
        + vm.get_slot_double(2)
        + vm.get_slot_double(3)
        + vm.get_slot_double(4);
    vm.set_slot_double(0, result);
}

const TEST_SCRIPT: &[u8] = b"class Test {\n  static method(a, b, c, d) { a + b + c + d }\n}\n";

fn call(vm: &mut WrenVM) {
    let iterations = vm.get_slot_double(1) as i64;

    // Since the VM is not re-entrant, we can't call from within this foreign
    // method. Instead, make a new VM to run the call test in.
    let mut other = WrenVM::new_vm(None);
    other.interpret(Some(b"main"), TEST_SCRIPT);

    let method = other.make_call_handle(b"method(_,_,_,_)");

    other.ensure_slots(1);
    other.get_variable(b"main", b"Test", 0);
    let test_class = other.get_slot_handle(0);

    let start = std::time::Instant::now();

    let mut result = 0.0;
    for _ in 0..iterations {
        other.ensure_slots(5);
        other.set_slot_handle(0, test_class);
        other.set_slot_double(1, 1.0);
        other.set_slot_double(2, 2.0);
        other.set_slot_double(3, 3.0);
        other.set_slot_double(4, 4.0);
        other.call(method);
        result += other.get_slot_double(0);
    }

    let elapsed = start.elapsed().as_secs_f64();

    other.release_handle(test_class);
    other.release_handle(method);
    other.free_vm();

    if result == (1.0 + 2.0 + 3.0 + 4.0) * iterations as f64 {
        vm.set_slot_double(0, elapsed);
    } else {
        vm.set_slot_bool(0, false);
    }
}

pub fn bind_method(signature: &str) -> Option<WrenForeignMethodFn> {
    match signature {
        "static Benchmark.arguments(_,_,_,_)" => Some(arguments),
        "static Benchmark.call(_)" => Some(call),
        _ => None,
    }
}
