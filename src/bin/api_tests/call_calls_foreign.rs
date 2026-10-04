// Port of test/api/call_calls_foreign.c
use wren_rs::vm::*;

fn api(vm: &mut WrenVM) {
    // Grow the slot array. This should trigger the stack to be moved.
    vm.ensure_slots(10);
    vm.set_slot_new_list(0);
    for i in 1..10 {
        vm.set_slot_double(i, i as f64);
        vm.insert_in_list(0, -1, i);
    }
}

pub fn bind_method(_vm: &mut WrenVM, signature: &str) -> Option<WrenForeignMethodFn> {
    match signature {
        "static CallCallsForeign.api()" => Some(api),
        _ => None,
    }
}

pub fn run_tests(vm: &mut WrenVM) -> i32 {
    vm.ensure_slots(1);
    vm.get_variable(b"./test/api/call_calls_foreign", b"CallCallsForeign", 0);
    let api_class = vm.get_slot_handle(0);
    let call = vm.make_call_handle(b"call(_)");

    vm.ensure_slots(2);
    vm.set_slot_handle(0, api_class);
    vm.set_slot_string(1, b"parameter");

    println!("slots before {}", vm.get_slot_count());
    vm.call(call);

    // We should have a single slot count for the return.
    println!("slots after {}", vm.get_slot_count());

    vm.release_handle(call);
    vm.release_handle(api_class);
    0
}
