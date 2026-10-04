// Port of test/api/user_data.c
use wren_rs::vm::*;

const DATA: usize = 0x0DA7A;
const OTHER_DATA: usize = 0x07BE2;

fn test_reallocate(ptr: *mut u8, new_size: usize, user_data: usize) -> *mut u8 {
    if user_data != DATA {
        return std::ptr::null_mut();
    }
    if new_size == 0 {
        if !ptr.is_null() {
            unsafe {
                std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(16, 8).unwrap());
            }
        }
        return std::ptr::null_mut();
    }
    unsafe {
        let layout = std::alloc::Layout::from_size_align(16, 8).unwrap();
        if ptr.is_null() {
            std::alloc::alloc(layout)
        } else {
            std::alloc::realloc(ptr, layout, new_size)
        }
    }
}

fn test(vm: &mut WrenVM) {
    let mut config = WrenConfiguration::new();

    // Should default to 0 (NULL in C).
    if config.user_data != 0 {
        vm.set_slot_bool(0, false);
        return;
    }

    config.reallocate_fn = test_reallocate;
    config.user_data = DATA;

    let mut other = WrenVM::new_vm(Some(config));

    // Should be able to get it.
    if other.get_user_data() != DATA {
        vm.set_slot_bool(0, false);
        other.free_vm();
        return;
    }

    // Should be able to set it.
    other.set_user_data(OTHER_DATA);
    if other.get_user_data() != OTHER_DATA {
        vm.set_slot_bool(0, false);
        other.free_vm();
        return;
    }

    vm.set_slot_bool(0, true);
    other.free_vm();
}

pub fn bind_method(_vm: &mut WrenVM, signature: &str) -> Option<WrenForeignMethodFn> {
    match signature {
        "static UserData.test" => Some(test),
        _ => None,
    }
}
