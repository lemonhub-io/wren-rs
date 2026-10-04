// Port of test/api/handle.c
use std::cell::Cell;
use wren_rs::vm::*;

thread_local! {
    static HANDLE: Cell<Option<WrenHandle>> = const { Cell::new(None) };
}

fn set_value(vm: &mut WrenVM) {
    let h = vm.get_slot_handle(1);
    HANDLE.with(|c| c.set(Some(h)));
}

fn get_value(vm: &mut WrenVM) {
    let h = HANDLE.with(|c| c.get()).expect("handle not set");
    vm.set_slot_handle(0, h);
    vm.release_handle(h);
}

pub fn bind_method(_vm: &mut WrenVM, signature: &str) -> Option<WrenForeignMethodFn> {
    match signature {
        "static Handle.value=(_)" => Some(set_value),
        "static Handle.value" => Some(get_value),
        _ => None,
    }
}
