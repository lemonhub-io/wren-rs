// Port of test/api/reset_stack_after_foreign_construct.c
use wren_rs::vm::*;

fn counter_allocate(vm: &mut WrenVM) {
    let f = vm.set_slot_new_foreign(0, 0, 8);
    let v = vm.get_slot_double(1);
    vm.foreign_data_mut(f)[..8].copy_from_slice(&v.to_le_bytes());
}

pub fn bind_class(class_name: &str, methods: &mut WrenForeignClassMethods) {
    if class_name == "ResetStackForeign" {
        methods.allocate = Some(counter_allocate);
    }
}

pub fn run_tests(vm: &mut WrenVM) -> i32 {
    vm.ensure_slots(1);
    vm.get_variable(
        b"./test/api/reset_stack_after_foreign_construct",
        b"Test",
        0,
    );
    let test_class = vm.get_slot_handle(0);

    let call_construct = vm.make_call_handle(b"callConstruct()");
    let after_construct = vm.make_call_handle(b"afterConstruct(_,_)");

    vm.ensure_slots(1);
    vm.set_slot_handle(0, test_class);
    vm.call(call_construct);

    vm.ensure_slots(3);
    vm.set_slot_handle(0, test_class);
    vm.set_slot_double(1, 1.0);
    vm.set_slot_double(2, 2.0);
    vm.call(after_construct);

    vm.release_handle(test_class);
    vm.release_handle(call_construct);
    vm.release_handle(after_construct);
    0
}
