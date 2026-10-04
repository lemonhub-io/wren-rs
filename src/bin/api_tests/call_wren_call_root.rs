// Port of test/api/call_wren_call_root.c
use wren_rs::vm::*;

const WREN_EX_SOFTWARE: i32 = 70;

pub fn run_tests(vm: &mut WrenVM) -> i32 {
    let mut exit_code = 0;
    vm.ensure_slots(1);
    vm.get_variable(b"./test/api/call_wren_call_root", b"Test", 0);
    let test_class = vm.get_slot_handle(0);

    let run = vm.make_call_handle(b"run()");

    vm.ensure_slots(1);
    vm.set_slot_handle(0, test_class);
    let result = vm.call(run);
    if result == WrenInterpretResult::RuntimeError {
        exit_code = WREN_EX_SOFTWARE;
    } else {
        println!("Missing runtime error.");
    }

    vm.release_handle(test_class);
    vm.release_handle(run);
    exit_code
}
