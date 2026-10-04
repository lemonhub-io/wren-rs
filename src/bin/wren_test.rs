// wren_test: port of wren-c/test/main.c + test.c.
// Test runner for language-level tests; not a general-purpose VM or REPL.

use std::process::exit;
use wren_rs::vm::*;

const WREN_EX_USAGE: i32 = 64;
const WREN_EX_DATAERR: i32 = 65;
const WREN_EX_NOINPUT: i32 = 66;
const WREN_EX_SOFTWARE: i32 = 70;
const WREN_EX_IOERR: i32 = 74;

mod api_tests;

// ---------------- path helpers (port of test.c Path) ----------------

fn is_separator(c: char) -> bool {
    c == '/' || (cfg!(windows) && c == '\\')
}

fn absolute_prefix_len(path: &str) -> usize {
    let b = path.as_bytes();
    if cfg!(windows) && b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        return if b.len() >= 3 && is_separator(b[2] as char) {
            3
        } else {
            2
        };
    }
    if !b.is_empty() && is_separator(b[0] as char) {
        1
    } else {
        0
    }
}

#[derive(PartialEq)]
enum PathType {
    Absolute,
    Relative,
    Simple,
}

fn path_type(path: &str) -> PathType {
    if absolute_prefix_len(path) > 0 {
        return PathType::Absolute;
    }
    let b = path.as_bytes();
    if b.len() >= 2 && b[0] == b'.' && is_separator(b[1] as char) {
        return PathType::Relative;
    }
    if b.len() >= 3 && b[0] == b'.' && b[1] == b'.' && is_separator(b[2] as char) {
        return PathType::Relative;
    }
    PathType::Simple
}

fn path_join(path: &mut String, s: &str) {
    if !path.is_empty() && !is_separator(path.chars().last().unwrap()) {
        path.push('/');
    }
    path.push_str(s);
}

fn path_dir_name(path: &mut String) {
    if let Some(i) = path.rfind(is_separator) {
        path.truncate(i);
    } else {
        path.clear();
    }
}

fn path_remove_extension(path: &mut String) {
    let bytes = path.as_bytes();
    for i in (0..bytes.len()).rev() {
        let c = bytes[i] as char;
        if is_separator(c) {
            return;
        }
        if c == '.' {
            path.truncate(i);
            return;
        }
    }
}

fn path_normalize(path: &mut String) {
    let mut components: Vec<String> = Vec::new();
    let mut leading_doubles = 0;
    for comp in path.split(is_separator) {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp == ".." {
            if !components.is_empty() {
                components.pop();
            } else {
                leading_doubles += 1;
            }
        } else {
            components.push(comp.to_string());
        }
    }

    let mut result = String::new();
    let mut needs_sep = false;
    let prefix = absolute_prefix_len(path);
    if prefix > 0 {
        result.push_str(&path[..prefix]);
    } else if leading_doubles > 0 {
        for _ in 0..leading_doubles {
            if needs_sep {
                result.push('/');
            }
            result.push_str("..");
            needs_sep = true;
        }
    } else if path.starts_with("./") {
        result.push('.');
        needs_sep = true;
    }
    for c in components {
        if needs_sep {
            result.push('/');
        }
        result.push_str(&c);
        needs_sep = true;
    }
    if result.is_empty() {
        result.push('.');
    }
    *path = result;
}

// ---------------- file helpers ----------------

fn read_file(path: &str) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            eprintln!("Could not read file \"{}\".", path);
            exit(WREN_EX_IOERR);
        }
    }
}

// ---------------- VM bindings ----------------

fn vm_write(_vm: &mut WrenVM, text: &[u8]) {
    // The C callback receives a `const char*`; printf("%s") stops at NUL.
    let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
    print!("{}", String::from_utf8_lossy(&text[..end]));
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

fn report_error(
    _vm: &mut WrenVM,
    etype: WrenErrorType,
    module: Option<&[u8]>,
    line: i32,
    message: &[u8],
) {
    let msg = String::from_utf8_lossy(message);
    match etype {
        WrenErrorType::Compile => {
            let m = module.map(|m| String::from_utf8_lossy(m).into_owned());
            let m = m.as_deref().unwrap_or("?");
            eprintln!("[{} line {}] {}", m, line, msg);
        }
        WrenErrorType::Runtime => eprintln!("{}", msg),
        WrenErrorType::StackTrace => {
            let m = module.map(|m| String::from_utf8_lossy(m).into_owned());
            let m = m.as_deref().unwrap_or("?");
            eprintln!("[{} line {}] in {}", m, line, msg);
        }
    }
}

fn read_module(_vm: &mut WrenVM, module: &[u8]) -> WrenLoadModuleResult {
    let file_path = format!("{}.wren", String::from_utf8_lossy(module));
    WrenLoadModuleResult {
        source: read_file(&file_path),
        on_complete: None,
    }
}

fn resolve_module(_vm: &mut WrenVM, importer: &[u8], module: &[u8]) -> Option<Vec<u8>> {
    let m = String::from_utf8_lossy(module);
    // Logical import strings are used as-is and need no resolution.
    if path_type(&m) == PathType::Simple {
        return Some(module.to_vec());
    }
    let mut path = String::from_utf8_lossy(importer).into_owned();
    path_dir_name(&mut path);
    path_join(&mut path, &m);
    path_normalize(&mut path);
    Some(path.into_bytes())
}

// ---------------- main helpers ----------------

fn is_module_an_api_test(module: &str) -> bool {
    module.starts_with("test/api") || module.starts_with("test/benchmark")
}

fn run_file(vm: &mut WrenVM, path: &str) -> WrenInterpretResult {
    let source = match read_file(path) {
        Some(s) => s,
        None => {
            eprintln!("Could not find file \"{}\".", path);
            exit(WREN_EX_NOINPUT);
        }
    };

    let mut module = path.to_string();
    if path_type(&module) == PathType::Simple {
        let mut relative = String::from(".");
        path_join(&mut relative, &module);
        module = relative;
    }
    path_remove_extension(&mut module);

    vm.interpret(Some(module.as_bytes()), &source)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        println!("This is a Wren test runner.\nUsage: wren_test [file]");
        exit(WREN_EX_USAGE);
    }
    if args.len() == 2 && args[1] == "--version" {
        println!(
            "wren_test is running on Wren version {}",
            WREN_VERSION_STRING
        );
        exit(1);
    }

    let test_name = args[1].clone();
    let is_api_test = is_module_an_api_test(&test_name);

    let mut config = WrenConfiguration::new();
    config.resolve_module_fn = Some(resolve_module);
    config.load_module_fn = Some(read_module);
    config.write_fn = Some(vm_write);
    config.error_fn = Some(report_error);
    if is_api_test {
        config.bind_foreign_class_fn = Some(api_tests::bind_foreign_class);
        config.bind_foreign_method_fn = Some(api_tests::bind_foreign_method);
    }
    config.initial_heap_size = 1024 * 1024 * 100;
    let mut vm = WrenVM::new_vm(Some(config));

    let result = run_file(&mut vm, &test_name);

    let mut exit_code = 0;
    if is_api_test {
        exit_code = api_tests::run(&mut vm, &test_name);
    }

    match result {
        WrenInterpretResult::CompileError => exit(WREN_EX_DATAERR),
        WrenInterpretResult::RuntimeError => exit(WREN_EX_SOFTWARE),
        WrenInterpretResult::Success => {}
    }

    vm.free_vm();
    exit(exit_code);
}

// ---------------- unit tests (port of test/unit/path_test.c) ----------------

#[cfg(test)]
mod path_tests {
    use super::path_normalize;

    fn expect_normalize(input: &str, expected: &str) {
        let mut p = input.to_string();
        path_normalize(&mut p);
        assert_eq!(p, expected, "input {:?}", input);
    }

    #[test]
    fn normalize() {
        expect_normalize("", ".");
        expect_normalize(".", ".");
        expect_normalize("..", "..");
        expect_normalize("a", "a");
        expect_normalize("/", "/");

        expect_normalize("a/b/c", "a/b/c");
        expect_normalize("a//b///c////d", "a/b/c/d");

        expect_normalize("./", ".");
        expect_normalize("/.", "/");
        expect_normalize("/./", "/");
        expect_normalize("./.", ".");
        expect_normalize("a/./b", "a/b");
        expect_normalize("a/.b/c", "a/.b/c");
        expect_normalize("a/././b/./c", "a/b/c");
        expect_normalize("././a", "./a");
        expect_normalize("a/./.", "a");

        expect_normalize("..", "..");
        expect_normalize("../", "..");
        expect_normalize("../../..", "../../..");
        expect_normalize("../../../", "../../..");
        expect_normalize("/..", "/");
        expect_normalize("/../../..", "/");
        expect_normalize("/../../../a", "/a");
        expect_normalize("a/..", ".");
        expect_normalize("a/b/..", "a");
        expect_normalize("a/../b", "b");
        expect_normalize("a/./../b", "b");
        expect_normalize("a/b/c/../../d/e/..", "a/d");
        expect_normalize("a/b/../../../../c", "../../c");

        expect_normalize("..", "..");
        expect_normalize("../", "..");
        expect_normalize("/..", "/");
        expect_normalize("a/..", ".");
        expect_normalize("../a", "../a");
        expect_normalize("/../a", "/a");
        expect_normalize("/../a", "/a");
        expect_normalize("a/b/..", "a");
        expect_normalize("../a/b/..", "../a");
        expect_normalize("a/../b", "b");
        expect_normalize("a/./../b", "b");
        expect_normalize("a/b/c/../../d/e/..", "a/d");
        expect_normalize("a/b/../../../../c", "../../c");
        expect_normalize("a/b/c/../../..d/./.e/f././", "a/..d/.e/f.");

        expect_normalize("./", ".");
        expect_normalize(".//", ".");
        expect_normalize("a/", "a");
        expect_normalize("a/b/", "a/b");
        expect_normalize("a/b///", "a/b");

        expect_normalize("foo/bar/baz", "foo/bar/baz");
        expect_normalize("foo", "foo");
        expect_normalize("foo/bar/", "foo/bar");
        expect_normalize("./foo/././bar/././", "./foo/bar");
    }
}
