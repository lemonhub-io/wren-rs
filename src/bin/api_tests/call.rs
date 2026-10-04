// Port of test/api/call.c
use wren_rs::vm::*;

pub fn run_tests(vm: &mut WrenVM) -> i32 {
    vm.ensure_slots(1);
    vm.get_variable(b"./test/api/call", b"Call", 0);
    let call_class = vm.get_slot_handle(0);

    let no_params = vm.make_call_handle(b"noParams");
    let zero = vm.make_call_handle(b"zero()");
    let one = vm.make_call_handle(b"one(_)");
    let two = vm.make_call_handle(b"two(_,_)");
    let unary = vm.make_call_handle(b"-");
    let binary = vm.make_call_handle(b"-(_)");
    let subscript = vm.make_call_handle(b"[_,_]");
    let subscript_set = vm.make_call_handle(b"[_,_]=(_)");

    // Different arity.
    vm.ensure_slots(1);
    vm.set_slot_handle(0, call_class);
    vm.call(no_params);

    vm.ensure_slots(1);
    vm.set_slot_handle(0, call_class);
    vm.call(zero);

    vm.ensure_slots(2);
    vm.set_slot_handle(0, call_class);
    vm.set_slot_double(1, 1.0);
    vm.call(one);

    vm.ensure_slots(3);
    vm.set_slot_handle(0, call_class);
    vm.set_slot_double(1, 1.0);
    vm.set_slot_double(2, 2.0);
    vm.call(two);

    // Operators.
    vm.ensure_slots(1);
    vm.set_slot_handle(0, call_class);
    vm.call(unary);

    vm.ensure_slots(2);
    vm.set_slot_handle(0, call_class);
    vm.set_slot_double(1, 1.0);
    vm.call(binary);

    vm.ensure_slots(3);
    vm.set_slot_handle(0, call_class);
    vm.set_slot_double(1, 1.0);
    vm.set_slot_double(2, 2.0);
    vm.call(subscript);

    vm.ensure_slots(4);
    vm.set_slot_handle(0, call_class);
    vm.set_slot_double(1, 1.0);
    vm.set_slot_double(2, 2.0);
    vm.set_slot_double(3, 3.0);
    vm.call(subscript_set);

    // Returning a value.
    let get_value = vm.make_call_handle(b"getValue()");
    vm.ensure_slots(1);
    vm.set_slot_handle(0, call_class);
    vm.call(get_value);
    println!("slots after call: {}", vm.get_slot_count());
    let value = vm.get_slot_handle(0);

    // Different argument types.
    vm.ensure_slots(3);
    vm.set_slot_handle(0, call_class);
    vm.set_slot_bool(1, true);
    vm.set_slot_bool(2, false);
    vm.call(two);

    vm.ensure_slots(3);
    vm.set_slot_handle(0, call_class);
    vm.set_slot_double(1, 1.2);
    vm.set_slot_double(2, 3.4);
    vm.call(two);

    vm.ensure_slots(3);
    vm.set_slot_handle(0, call_class);
    vm.set_slot_string(1, b"string");
    vm.set_slot_string(2, b"another");
    vm.call(two);

    vm.ensure_slots(3);
    vm.set_slot_handle(0, call_class);
    vm.set_slot_null(1);
    vm.set_slot_handle(2, value);
    vm.call(two);

    // Truncate a string, or allow null bytes.
    vm.ensure_slots(3);
    vm.set_slot_handle(0, call_class);
    vm.set_slot_bytes(1, b"string"[..3].as_ref());
    vm.set_slot_bytes(2, b"b\0y\0t\0e");
    vm.call(two);

    // Call ignores with extra temporary slots on stack.
    vm.ensure_slots(10);
    vm.set_slot_handle(0, call_class);
    for i in 1..10 {
        vm.set_slot_double(i, i as f64 * 0.1);
    }
    vm.call(one);

    vm.release_handle(call_class);
    vm.release_handle(no_params);
    vm.release_handle(zero);
    vm.release_handle(one);
    vm.release_handle(two);
    vm.release_handle(get_value);
    vm.release_handle(value);
    vm.release_handle(unary);
    vm.release_handle(binary);
    vm.release_handle(subscript);
    vm.release_handle(subscript_set);
    0
}
