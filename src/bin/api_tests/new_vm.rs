// Port of test/api/new_vm.c
use wren_rs::vm::*;

fn null_config(vm: &mut WrenVM) {
    let mut other = WrenVM::new_vm(None);
    let result = other.interpret(Some(b"main"), b"1 + 2");
    vm.set_slot_bool(0, result == WrenInterpretResult::Success);
    other.free_vm();
}

fn multiple_interpret_calls(vm: &mut WrenVM) {
    let mut other = WrenVM::new_vm(None);
    let mut correct = true;

    // Handles should be valid across calls into Wren code.
    let abs_method = other.make_call_handle(b"abs");

    let result = other.interpret(Some(b"main"), b"import \"random\" for Random");
    correct = correct && result == WrenInterpretResult::Success;

    for i in 0..5 {
        // Calling `ensure_slots` before `interpret` should not introduce
        // problems later.
        other.ensure_slots(2);

        // Calling a foreign function should succeed.
        let result = other.interpret(Some(b"main"), b"Random.new(12345)");
        correct = correct && result == WrenInterpretResult::Success;

        other.ensure_slots(2);
        other.set_slot_double(0, -i as f64);
        let result = other.call(abs_method);
        correct = correct && result == WrenInterpretResult::Success;

        let abs_value = other.get_slot_double(0);
        correct = correct && (abs_value == i as f64);
    }

    vm.set_slot_bool(0, correct);
    other.release_handle(abs_method);
    other.free_vm();
}

pub fn bind_method(signature: &str) -> Option<WrenForeignMethodFn> {
    match signature {
        "static VM.nullConfig()" => Some(null_config),
        "static VM.multipleInterpretCalls()" => Some(multiple_interpret_calls),
        _ => None,
    }
}
