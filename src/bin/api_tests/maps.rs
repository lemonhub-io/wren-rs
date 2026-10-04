// Port of test/api/maps.c
use wren_rs::vm::*;

fn new_map(vm: &mut WrenVM) {
    vm.set_slot_new_map(0);
}

fn invalid_insert(vm: &mut WrenVM) {
    vm.set_slot_new_map(0);
    vm.ensure_slots(3);
    // Foreign Class is in slot 1.
    vm.set_slot_string(2, b"England");
    vm.set_map_value(0, 1, 2); // expect this to cause errors
}

fn insert(vm: &mut WrenVM) {
    vm.set_slot_new_map(0);
    vm.ensure_slots(3);

    // Insert String.
    vm.set_slot_string(1, b"England");
    vm.set_slot_string(2, b"London");
    vm.set_map_value(0, 1, 2);

    // Insert Double.
    vm.set_slot_double(1, 1.0);
    vm.set_slot_double(2, 42.0);
    vm.set_map_value(0, 1, 2);

    // Insert Boolean.
    vm.set_slot_bool(1, false);
    vm.set_slot_bool(2, true);
    vm.set_map_value(0, 1, 2);

    // Insert Null.
    vm.set_slot_null(1);
    vm.set_slot_null(2);
    vm.set_map_value(0, 1, 2);

    // Insert List.
    vm.set_slot_string(1, b"Empty");
    vm.set_slot_new_list(2);
    vm.set_map_value(0, 1, 2);
}

fn remove_key(vm: &mut WrenVM) {
    vm.ensure_slots(3);
    vm.set_slot_string(2, b"key");
    vm.remove_map_value(1, 2, 0);
}

fn count_wren(vm: &mut WrenVM) {
    let count = vm.get_map_count(1);
    vm.set_slot_double(0, count as f64);
}

fn count_api(vm: &mut WrenVM) {
    insert(vm);
    let count = vm.get_map_count(0);
    vm.set_slot_double(0, count as f64);
}

fn contains_wren(vm: &mut WrenVM) {
    let result = vm.get_map_contains_key(1, 2);
    vm.set_slot_bool(0, result);
}

fn contains_api(vm: &mut WrenVM) {
    insert(vm);
    vm.ensure_slots(1);
    vm.set_slot_string(1, b"England");
    let result = vm.get_map_contains_key(0, 1);
    vm.set_slot_bool(0, result);
}

fn contains_api_false(vm: &mut WrenVM) {
    insert(vm);
    vm.ensure_slots(1);
    vm.set_slot_string(1, b"DefinitelyNotARealKey");
    let result = vm.get_map_contains_key(0, 1);
    vm.set_slot_bool(0, result);
}

pub fn bind_method(_vm: &mut WrenVM, signature: &str) -> Option<WrenForeignMethodFn> {
    match signature {
        "static Maps.newMap()" => Some(new_map),
        "static Maps.insert()" => Some(insert),
        "static Maps.remove(_)" => Some(remove_key),
        "static Maps.count(_)" => Some(count_wren),
        "static Maps.count()" => Some(count_api),
        "static Maps.contains()" => Some(contains_api),
        "static Maps.containsFalse()" => Some(contains_api_false),
        "static Maps.contains(_,_)" => Some(contains_wren),
        "static Maps.invalidInsert(_)" => Some(invalid_insert),
        _ => None,
    }
}
