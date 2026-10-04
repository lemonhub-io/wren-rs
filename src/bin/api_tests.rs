// api_tests: port of wren-c/test/api/*.c — foreign bindings used by the
// test/api/*.wren tests.

use std::process::exit;
use wren_rs::vm::*;

// Each sub-test file gets a module below. Names mirror the C files.

mod benchmark;
mod call;
mod call_calls_foreign;
mod call_wren_call_root;
mod error;
mod foreign_class;
mod get_variable;
mod handle;
mod lists;
mod maps;
mod new_vm;
mod reset_stack_after_call_abort;
mod reset_stack_after_foreign_construct;
mod resolution;
mod slots;
mod user_data;

thread_local! {
    static TEST_NAME: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
}

pub fn bind_foreign_method(
    vm: &mut WrenVM,
    module: &[u8],
    class_name: &[u8],
    is_static: bool,
    signature: &[u8],
) -> Option<WrenForeignMethodFn> {
    if !module.starts_with(b"./test/") {
        return None;
    }

    // Concatenate qualifiers into "static Class.signature" like the C code.
    let mut full = String::new();
    if is_static {
        full.push_str("static ");
    }
    full.push_str(&String::from_utf8_lossy(class_name));
    full.push('.');
    full.push_str(&String::from_utf8_lossy(signature));

    let tries: &[fn(&mut WrenVM, &str) -> Option<WrenForeignMethodFn>] = &[
        |_, s| benchmark::bind_method(s),
        |vm, s| call_calls_foreign::bind_method(vm, s),
        |_, s| error::bind_method(s),
        |vm, s| get_variable::bind_method(vm, s),
        |_, s| foreign_class::bind_method(s),
        |vm, s| handle::bind_method(vm, s),
        |_, s| lists::bind_method(s),
        |vm, s| maps::bind_method(vm, s),
        |_, s| new_vm::bind_method(s),
        |vm, s| resolution::bind_method(vm, s),
        |_, s| slots::bind_method(s),
        |vm, s| user_data::bind_method(vm, s),
    ];
    for t in tries {
        if let Some(m) = t(vm, &full) {
            return Some(m);
        }
    }
    TEST_NAME.with(|t| {
        eprintln!(
            "Unknown foreign method '{}' for test '{}'",
            full,
            t.borrow()
        );
    });
    exit(1);
}

pub fn bind_foreign_class(
    _vm: &mut WrenVM,
    module: &[u8],
    class_name: &[u8],
) -> WrenForeignClassMethods {
    let mut methods = WrenForeignClassMethods::default();
    if !module.starts_with(b"./test/api") {
        return methods;
    }
    let name = String::from_utf8_lossy(class_name).into_owned();
    foreign_class::bind_class(&name, &mut methods);
    if methods.allocate.is_some() {
        return methods;
    }
    reset_stack_after_foreign_construct::bind_class(&name, &mut methods);
    if methods.allocate.is_some() {
        return methods;
    }
    slots::bind_class(&name, &mut methods);
    if methods.allocate.is_some() {
        return methods;
    }
    TEST_NAME.with(|t| {
        eprintln!("Unknown foreign class '{}' for test '{}'", name, t.borrow());
    });
    exit(1);
}

pub fn run(vm: &mut WrenVM, test_name: &str) -> i32 {
    TEST_NAME.with(|t| *t.borrow_mut() = test_name.to_string());
    if test_name.contains("/call.wren") {
        call::run_tests(vm)
    } else if test_name.contains("/call_calls_foreign.wren") {
        call_calls_foreign::run_tests(vm)
    } else if test_name.contains("/call_wren_call_root.wren") {
        call_wren_call_root::run_tests(vm)
    } else if test_name.contains("/reset_stack_after_call_abort.wren") {
        reset_stack_after_call_abort::run_tests(vm)
    } else if test_name.contains("/reset_stack_after_foreign_construct.wren") {
        reset_stack_after_foreign_construct::run_tests(vm)
    } else {
        0
    }
}
