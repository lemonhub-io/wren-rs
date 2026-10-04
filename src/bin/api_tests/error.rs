// Port of test/api/error.c
use wren_rs::vm::*;

fn runtime_error(vm: &mut WrenVM) {
    vm.ensure_slots(1);
    vm.set_slot_string(0, b"Error!");
    vm.abort_fiber(0);
}

pub fn bind_method(signature: &str) -> Option<WrenForeignMethodFn> {
    match signature {
        "static Error.runtimeError" => Some(runtime_error),
        _ => None,
    }
}
