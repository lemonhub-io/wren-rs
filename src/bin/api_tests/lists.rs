// Port of test/api/lists.c
use wren_rs::vm::*;

fn new_list(vm: &mut WrenVM) {
    vm.set_slot_new_list(0);
}

// Helper to store a double in a slot then insert it into the list at slot 0.
fn insert_number(vm: &mut WrenVM, index: i64, value: f64) {
    vm.set_slot_double(1, value);
    vm.insert_in_list(0, index, 1);
}

fn append_number(vm: &mut WrenVM, value: f64) {
    vm.set_slot_double(1, value);
    vm.insert_in_list(0, -1, 1);
}

fn insert(vm: &mut WrenVM) {
    vm.set_slot_new_list(0);
    vm.ensure_slots(2);

    // Appending.
    insert_number(vm, 0, 1.0);
    insert_number(vm, 1, 2.0);
    insert_number(vm, 2, 3.0);

    // Inserting.
    insert_number(vm, 0, 4.0);
    insert_number(vm, 1, 5.0);
    insert_number(vm, 2, 6.0);

    // Negative indexes.
    insert_number(vm, -1, 7.0);
    insert_number(vm, -2, 8.0);
    insert_number(vm, -3, 9.0);
}

fn get(vm: &mut WrenVM) {
    let index = vm.get_slot_double(2) as i64;
    vm.get_list_element(1, index, 0);
}

fn set(vm: &mut WrenVM) {
    vm.set_slot_new_list(0);
    vm.ensure_slots(2);

    append_number(vm, 1.0);
    append_number(vm, 2.0);
    append_number(vm, 3.0);
    append_number(vm, 4.0);

    // list[2] = 33
    vm.set_slot_double(1, 33.0);
    vm.set_list_element(0, 2, 1);

    // list[-1] = 44
    vm.set_slot_double(1, 44.0);
    vm.set_list_element(0, -1, 1);
}

pub fn bind_method(signature: &str) -> Option<WrenForeignMethodFn> {
    match signature {
        "static Lists.newList()" => Some(new_list),
        "static Lists.insert()" => Some(insert),
        "static Lists.set()" => Some(set),
        "static Lists.get(_,_)" => Some(get),
        _ => None,
    }
}
