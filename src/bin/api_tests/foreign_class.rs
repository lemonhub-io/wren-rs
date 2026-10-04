// Port of test/api/foreign_class.c
use std::process::exit;
use std::sync::atomic::{AtomicI32, Ordering};
use wren_rs::vm::*;

static FINALIZED: AtomicI32 = AtomicI32::new(0);

fn api_finalized(vm: &mut WrenVM) {
    vm.set_slot_double(0, FINALIZED.load(Ordering::SeqCst) as f64);
}

fn counter_allocate(vm: &mut WrenVM) {
    let f = vm.set_slot_new_foreign(0, 0, 8);
    vm.foreign_data_mut(f)[..8].copy_from_slice(&0f64.to_le_bytes());
}

fn counter_increment(vm: &mut WrenVM) {
    let f = vm.get_slot_foreign(0);
    let increment = vm.get_slot_double(1);
    let data = vm.foreign_data_mut(f);
    let v = f64::from_le_bytes(data[..8].try_into().unwrap());
    data[..8].copy_from_slice(&(v + increment).to_le_bytes());
}

fn counter_value(vm: &mut WrenVM) {
    let f = vm.get_slot_foreign(0);
    let v = f64::from_le_bytes(vm.foreign_data(f)[..8].try_into().unwrap());
    vm.set_slot_double(0, v);
}

fn point_allocate(vm: &mut WrenVM) {
    let f = vm.set_slot_new_foreign(0, 0, 24);

    // This gets called by both constructors, so sniff the slot count to see
    // which one was invoked.
    let coords = if vm.get_slot_count() == 1 {
        [0.0, 0.0, 0.0]
    } else {
        [
            vm.get_slot_double(1),
            vm.get_slot_double(2),
            vm.get_slot_double(3),
        ]
    };
    let data = vm.foreign_data_mut(f);
    for (i, c) in coords.iter().enumerate() {
        data[i * 8..(i + 1) * 8].copy_from_slice(&c.to_le_bytes());
    }
}

fn get_coords(vm: &mut WrenVM) -> [f64; 3] {
    let f = vm.get_slot_foreign(0);
    let d = vm.foreign_data(f);
    [
        f64::from_le_bytes(d[0..8].try_into().unwrap()),
        f64::from_le_bytes(d[8..16].try_into().unwrap()),
        f64::from_le_bytes(d[16..24].try_into().unwrap()),
    ]
}

fn point_translate(vm: &mut WrenVM) {
    let f = vm.get_slot_foreign(0);
    let dx = vm.get_slot_double(1);
    let dy = vm.get_slot_double(2);
    let dz = vm.get_slot_double(3);
    let data = vm.foreign_data_mut(f);
    for (i, d) in [dx, dy, dz].iter().enumerate() {
        let v = f64::from_le_bytes(data[i * 8..(i + 1) * 8].try_into().unwrap());
        data[i * 8..(i + 1) * 8].copy_from_slice(&(v + d).to_le_bytes());
    }
}

fn point_to_string(vm: &mut WrenVM) {
    let c = get_coords(vm);
    let result = format!("({:?}, {:?}, {:?})", c[0], c[1], c[2]);
    // %g-style formatting: trim trailing zeros like C does.
    let result = result.replace(".0,", ",").replace(".0)", ")");
    vm.set_slot_string(0, result.as_bytes());
}

fn resource_allocate(vm: &mut WrenVM) {
    let f = vm.set_slot_new_foreign(0, 0, 4);
    vm.foreign_data_mut(f)[..4].copy_from_slice(&123i32.to_le_bytes());
}

fn resource_finalize(data: &mut [u8]) {
    // Make sure we get the right data back.
    let value = i32::from_le_bytes(data[..4].try_into().unwrap());
    if value != 123 {
        exit(1);
    }
    FINALIZED.fetch_add(1, Ordering::SeqCst);
}

fn bad_class_allocate(vm: &mut WrenVM) {
    vm.ensure_slots(1);
    vm.set_slot_string(0, b"Something went wrong");
    vm.abort_fiber(0);
}

pub fn bind_method(signature: &str) -> Option<WrenForeignMethodFn> {
    match signature {
        "static ForeignClass.finalized" => Some(api_finalized),
        "Counter.increment(_)" => Some(counter_increment),
        "Counter.value" => Some(counter_value),
        "Point.translate(_,_,_)" => Some(point_translate),
        "Point.toString" => Some(point_to_string),
        _ => None,
    }
}

pub fn bind_class(class_name: &str, methods: &mut WrenForeignClassMethods) {
    match class_name {
        "Counter" => methods.allocate = Some(counter_allocate),
        "Point" => methods.allocate = Some(point_allocate),
        "Resource" => {
            methods.allocate = Some(resource_allocate);
            methods.finalize = Some(resource_finalize);
        }
        "BadClass" => methods.allocate = Some(bad_class_allocate),
        _ => {}
    }
}
