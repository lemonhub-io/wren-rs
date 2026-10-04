// Port of test/api/slots.c
use wren_rs::vm::*;

fn no_set(_vm: &mut WrenVM) {}

fn get_slots(vm: &mut WrenVM) {
    let mut result = true;
    if !vm.get_slot_bool(1) {
        result = false;
    }
    let bytes = vm.get_slot_bytes(2);
    if bytes.len() != 5 || bytes != b"by\0te" {
        result = false;
    }
    if vm.get_slot_double(3) != 1.5 {
        result = false;
    }
    if vm.get_slot_string(4) != b"str" {
        result = false;
    }
    let handle = vm.get_slot_handle(5);
    if result {
        vm.set_slot_handle(0, handle);
    } else {
        vm.set_slot_bool(0, false);
    }
    vm.release_handle(handle);
}

fn set_slots(vm: &mut WrenVM) {
    let handle = vm.get_slot_handle(1);

    vm.set_slot_bool(1, true);
    vm.set_slot_bytes(2, b"by\0te");
    vm.set_slot_double(3, 1.5);
    vm.set_slot_string(4, b"str");
    vm.set_slot_null(5);

    // Read the slots back to make sure they were set correctly.
    let mut result = true;
    if !vm.get_slot_bool(1) {
        result = false;
    }
    let bytes = vm.get_slot_bytes(2);
    if bytes.len() != 5 || bytes != b"by\0te" {
        result = false;
    }
    if vm.get_slot_double(3) != 1.5 {
        result = false;
    }
    if vm.get_slot_string(4) != b"str" {
        result = false;
    }
    if vm.get_slot_type(5) != WrenType::Null {
        result = false;
    }

    if result {
        vm.set_slot_handle(0, handle);
    } else {
        vm.set_slot_bool(0, false);
    }
    vm.release_handle(handle);
}

fn slot_types(vm: &mut WrenVM) {
    let result = vm.get_slot_type(1) == WrenType::Bool
        && vm.get_slot_type(2) == WrenType::Foreign
        && vm.get_slot_type(3) == WrenType::List
        && vm.get_slot_type(4) == WrenType::Map
        && vm.get_slot_type(5) == WrenType::Null
        && vm.get_slot_type(6) == WrenType::Num
        && vm.get_slot_type(7) == WrenType::Str
        && vm.get_slot_type(8) == WrenType::Unknown;
    vm.set_slot_bool(0, result);
}

fn ensure(vm: &mut WrenVM) {
    let before = vm.get_slot_count();
    vm.ensure_slots(20);
    let after = vm.get_slot_count();

    // Use the slots to make sure they're available.
    for i in 0..20 {
        vm.set_slot_double(i, i as f64);
    }
    let mut sum = 0;
    for i in 0..20 {
        sum += vm.get_slot_double(i) as i64;
    }
    let result = format!("{} -> {} ({})", before, after, sum);
    vm.set_slot_string(0, result.as_bytes());
}

fn ensure_outside_foreign(vm: &mut WrenVM) {
    // To test the behavior outside of a foreign method (which we're currently
    // in), create a new separate VM.
    let mut other = WrenVM::new_vm(None);
    let before = other.get_slot_count();
    other.ensure_slots(20);
    let after = other.get_slot_count();

    for i in 0..20 {
        other.set_slot_double(i, i as f64);
    }
    let mut sum = 0;
    for i in 0..20 {
        sum += other.get_slot_double(i) as i64;
    }
    other.free_vm();

    let result = format!("{} -> {} ({})", before, after, sum);
    vm.set_slot_string(0, result.as_bytes());
}

fn foreign_class_allocate(vm: &mut WrenVM) {
    vm.set_slot_new_foreign(0, 0, 4);
}

fn get_list_count(vm: &mut WrenVM) {
    let n = vm.get_list_count(1);
    vm.set_slot_double(0, n as f64);
}

fn get_list_element(vm: &mut WrenVM) {
    let index = vm.get_slot_double(2) as i64;
    vm.get_list_element(1, index, 0);
}

fn get_map_value(vm: &mut WrenVM) {
    vm.get_map_value(1, 2, 0);
}

pub fn bind_method(signature: &str) -> Option<WrenForeignMethodFn> {
    match signature {
        "static Slots.noSet" => Some(no_set),
        "static Slots.getSlots(_,_,_,_,_)" => Some(get_slots),
        "static Slots.setSlots(_,_,_,_,_)" => Some(set_slots),
        "static Slots.slotTypes(_,_,_,_,_,_,_,_)" => Some(slot_types),
        "static Slots.ensure()" => Some(ensure),
        "static Slots.ensureOutsideForeign()" => Some(ensure_outside_foreign),
        "static Slots.getListCount(_)" => Some(get_list_count),
        "static Slots.getListElement(_,_)" => Some(get_list_element),
        "static Slots.getMapValue(_,_)" => Some(get_map_value),
        _ => None,
    }
}

pub fn bind_class(_class_name: &str, methods: &mut WrenForeignClassMethods) {
    methods.allocate = Some(foreign_class_allocate);
}
