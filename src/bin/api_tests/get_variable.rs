// Port of test/api/get_variable.c
use wren_rs::vm::*;

fn before_defined(vm: &mut WrenVM) {
    vm.get_variable(b"./test/api/get_variable", b"A", 0);
}

fn after_defined(vm: &mut WrenVM) {
    vm.get_variable(b"./test/api/get_variable", b"A", 0);
}

fn after_assigned(vm: &mut WrenVM) {
    vm.get_variable(b"./test/api/get_variable", b"A", 0);
}

fn other_slot(vm: &mut WrenVM) {
    vm.ensure_slots(3);
    vm.get_variable(b"./test/api/get_variable", b"B", 2);

    // Move it into return position.
    let string = vm.get_slot_string(2);
    vm.set_slot_string(0, &string);
}

fn other_module(vm: &mut WrenVM) {
    vm.get_variable(b"./test/api/get_variable_module", b"Variable", 0);
}

fn has_variable(vm: &mut WrenVM) {
    let module = vm.get_slot_string(1);
    let variable = vm.get_slot_string(2);
    let result = vm.has_variable(&module, &variable);
    vm.ensure_slots(1);
    vm.set_slot_bool(0, result);
}

fn has_module(vm: &mut WrenVM) {
    let module = vm.get_slot_string(1);
    let result = vm.has_module(&module);
    vm.ensure_slots(1);
    vm.set_slot_bool(0, result);
}

pub fn bind_method(_vm: &mut WrenVM, signature: &str) -> Option<WrenForeignMethodFn> {
    match signature {
        "static GetVariable.beforeDefined()" => Some(before_defined),
        "static GetVariable.afterDefined()" => Some(after_defined),
        "static GetVariable.afterAssigned()" => Some(after_assigned),
        "static GetVariable.otherSlot()" => Some(other_slot),
        "static GetVariable.otherModule()" => Some(other_module),
        "static Has.variable(_,_)" => Some(has_variable),
        "static Has.module(_)" => Some(has_module),
        _ => None,
    }
}
