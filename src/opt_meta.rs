// Port of wren_opt_meta.c: the built-in "meta" module.

use crate::value::*;
use crate::vm::{WrenForeignMethodFn, WrenVM};

pub const META_SOURCE: &[u8] = include_bytes!("wren_opt_meta.wren");

fn meta_compile(vm: &mut WrenVM) {
    let source = vm.get_slot_string(1);
    let is_expression = vm.get_slot_bool(2);
    let print_errors = vm.get_slot_bool(3);

    // Look up the module surrounding the callsite. The -2 walks up the
    // callstack assuming the meta module has one level of indirection.
    let module_name: Option<Vec<u8>> = {
        let fiber = vm.fiber;
        let f = vm.heap.fiber(fiber);
        let n = f.frames.len();
        let closure = f.frames[n - 2].closure;
        let fn_ref = vm.heap.closure(closure).fn_ref;
        let module = vm.heap.func(fn_ref).module;
        let name = vm.heap.module(module).name;
        if name == NULL_OBJ {
            None
        } else {
            Some(vm.heap.string(name).bytes.clone())
        }
    };

    let closure = vm.compile_source(module_name.as_deref(), &source, is_expression, print_errors);

    let v = if closure == NULL_OBJ {
        NULL_VAL
    } else {
        Value::obj(closure)
    };
    let (fiber, base) = vm.api_stack.unwrap();
    vm.heap.fiber_mut(fiber).stack[base] = v;
}

fn meta_get_module_variables(vm: &mut WrenVM) {
    vm.ensure_slots(3);
    let (fiber, base) = vm.api_stack.unwrap();
    let module_value_key = vm.heap.fiber(fiber).stack[base + 1];
    let module_value = vm.map_get(vm.modules, module_value_key);
    if module_value.is_undefined() {
        vm.heap.fiber_mut(fiber).stack[base] = NULL_VAL;
        return;
    }
    let module = module_value.as_obj();
    let names = vm.heap.module(module).variable_names.clone();
    let list = vm.new_list(names.len());
    // Initialize elements to null (C does this too for GC safety).
    vm.heap.fiber_mut(fiber).stack[base] = Value::obj(list);
    for (i, n) in names.iter().enumerate() {
        vm.heap.list_mut(list).elements[i] = Value::obj(*n);
    }
}

pub fn bind_foreign_method(
    _vm: &mut WrenVM,
    class_name: &[u8],
    is_static: bool,
    signature: &[u8],
) -> Option<WrenForeignMethodFn> {
    debug_assert!(class_name == b"Meta");
    debug_assert!(is_static);
    match signature {
        b"compile_(_,_,_)" => Some(meta_compile),
        b"getModuleVariables_(_)" => Some(meta_get_module_variables),
        _ => None,
    }
}
