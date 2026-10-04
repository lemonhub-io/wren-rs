// Port of test/api/resolution.c
use wren_rs::vm::*;

fn write_fn(_vm: &mut WrenVM, text: &[u8]) {
    print!("{}", String::from_utf8_lossy(text));
}

fn report_error(
    _vm: &mut WrenVM,
    etype: WrenErrorType,
    _module: Option<&[u8]>,
    _line: i32,
    message: &[u8],
) {
    if etype == WrenErrorType::Runtime {
        println!("{}", String::from_utf8_lossy(message));
    }
}

fn load_module(_vm: &mut WrenVM, module: &[u8]) -> WrenLoadModuleResult {
    println!("loading {}", String::from_utf8_lossy(module));
    let source: &[u8] = if module == b"main/baz/bang" {
        b"import \"foo|bar\""
    } else {
        b"System.print(\"ok\")"
    };
    WrenLoadModuleResult {
        source: Some(source.to_vec()),
        on_complete: None,
    }
}

fn run_test_vm(vm: &mut WrenVM, config: WrenConfiguration, source: &[u8]) {
    let mut config = config;
    config.write_fn = Some(write_fn);
    config.error_fn = Some(report_error);
    config.load_module_fn = Some(load_module);

    let mut other = WrenVM::new_vm(Some(config));
    let result = other.interpret(Some(b"main"), source);
    if result != WrenInterpretResult::Success {
        vm.set_slot_string(0, b"error");
    } else {
        vm.set_slot_string(0, b"success");
    }
    other.free_vm();
}

fn no_resolver(vm: &mut WrenVM) {
    let config = WrenConfiguration::new();
    // Should default to no resolution function.
    if config.resolve_module_fn.is_some() {
        vm.set_slot_string(0, b"Did not have null resolve function.");
        return;
    }
    run_test_vm(vm, config, b"import \"foo/bar\"");
}

fn resolve_to_null(_vm: &mut WrenVM, _importer: &[u8], _name: &[u8]) -> Option<Vec<u8>> {
    None
}

fn returns_null(vm: &mut WrenVM) {
    let mut config = WrenConfiguration::new();
    config.resolve_module_fn = Some(resolve_to_null);
    run_test_vm(vm, config, b"import \"foo/bar\"");
}

fn resolve_change(_vm: &mut WrenVM, importer: &[u8], name: &[u8]) -> Option<Vec<u8>> {
    // Concatenate importer and name, replacing "|" with "/".
    let mut result = Vec::with_capacity(importer.len() + 1 + name.len());
    result.extend_from_slice(importer);
    result.push(b'/');
    result.extend_from_slice(name);
    for b in result.iter_mut() {
        if *b == b'|' {
            *b = b'/';
        }
    }
    Some(result)
}

fn changes_string(vm: &mut WrenVM) {
    let mut config = WrenConfiguration::new();
    config.resolve_module_fn = Some(resolve_change);
    run_test_vm(vm, config, b"import \"foo|bar\"");
}

fn shared(vm: &mut WrenVM) {
    let mut config = WrenConfiguration::new();
    config.resolve_module_fn = Some(resolve_change);
    run_test_vm(vm, config, b"import \"foo|bar\"\nimport \"foo/bar\"");
}

fn importer(vm: &mut WrenVM) {
    let mut config = WrenConfiguration::new();
    config.resolve_module_fn = Some(resolve_change);
    run_test_vm(vm, config, b"import \"baz|bang\"");
}

pub fn bind_method(_vm: &mut WrenVM, signature: &str) -> Option<WrenForeignMethodFn> {
    match signature {
        "static Resolution.noResolver()" => Some(no_resolver),
        "static Resolution.returnsNull()" => Some(returns_null),
        "static Resolution.changesString()" => Some(changes_string),
        "static Resolution.shared()" => Some(shared),
        "static Resolution.importer()" => Some(importer),
        _ => None,
    }
}
