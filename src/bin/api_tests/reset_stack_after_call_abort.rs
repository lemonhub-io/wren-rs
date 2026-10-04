// Port of test/api/reset_stack_after_call_abort.c
use wren_rs::vm::*;

pub fn run_tests(vm: &mut WrenVM) -> i32 {
    vm.ensure_slots(1);
    vm.get_variable(b"./test/api/reset_stack_after_call_abort", b"Test", 0);
    let test_class = vm.get_slot_handle(0);

    let abort_fiber = vm.make_call_handle(b"abortFiber()");
    let after_abort = vm.make_call_handle(b"afterAbort(_,_)");

    vm.ensure_slots(1);
    vm.set_slot_handle(0, test_class);
    vm.call(abort_fiber);

    vm.ensure_slots(3);
    vm.set_slot_handle(0, test_class);
    vm.set_slot_double(1, 1.0);
    vm.set_slot_double(2, 2.0);
    vm.call(after_abort);

    vm.release_handle(test_class);
    vm.release_handle(abort_fiber);
    vm.release_handle(after_abort);
    0
}
