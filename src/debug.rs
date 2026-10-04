// Port of wren_debug.c: bytecode/stack dump utilities (used for debugging
// the VM; not called in normal operation).

use crate::opcodes::Code;
use crate::value::*;
use crate::vm::{format_num, WrenVM};
use std::io::Write;

fn dump_object(vm: &WrenVM, r: ObjRef) {
    match &vm.heap.get(r).data {
        ObjData::Class(c) => {
            let name = String::from_utf8_lossy(&vm.heap.string(c.name).bytes).into_owned();
            print!("[class {} {:?}]", name, r);
        }
        ObjData::Closure(_) => print!("[closure {:?}]", r),
        ObjData::Fiber(_) => print!("[fiber {:?}]", r),
        ObjData::Fn(_) => print!("[fn {:?}]", r),
        ObjData::Foreign(_) => print!("[foreign {:?}]", r),
        ObjData::Instance(_) => print!("[instance {:?}]", r),
        ObjData::List(_) => print!("[list {:?}]", r),
        ObjData::Map(_) => print!("[map {:?}]", r),
        ObjData::Module(_) => print!("[module {:?}]", r),
        ObjData::Range(_) => print!("[range {:?}]", r),
        ObjData::Str(s) => print!("{}", String::from_utf8_lossy(&s.bytes)),
        ObjData::Upvalue(_) => print!("[upvalue {:?}]", r),
    }
}

pub fn dump_value(vm: &WrenVM, value: Value) {
    if value.is_num() {
        print!("{}", format_num(value.as_num()));
    } else if value.is_obj() {
        dump_object(vm, value.as_obj_unchecked());
    } else if value == FALSE_VAL {
        print!("false");
    } else if value.is_null() {
        print!("null");
    } else if value == TRUE_VAL {
        print!("true");
    } else if value.is_undefined() {
        print!("undefined");
    } else {
        print!("NaN");
    }
}

fn method_name(vm: &WrenVM, symbol: usize) -> String {
    String::from_utf8_lossy(&vm.heap.string(vm.method_names[symbol]).bytes).into_owned()
}

fn var_name(vm: &WrenVM, module: ObjRef, slot: usize) -> String {
    String::from_utf8_lossy(
        &vm.heap
            .string(vm.heap.module(module).variable_names[slot])
            .bytes,
    )
    .into_owned()
}

fn dump_instruction(vm: &WrenVM, fn_ref: ObjRef, mut i: usize, last_line: Option<&mut i32>) -> i32 {
    let start = i;
    let (code_byte, line, module) = {
        let f = vm.heap.func(fn_ref);
        (
            f.code[i],
            f.debug.as_ref().map(|d| d.source_lines[i]).unwrap_or(0),
            f.module,
        )
    };
    let code = unsafe { std::mem::transmute::<u8, Code>(code_byte) };

    match last_line {
        None => print!("{:4}:", line),
        Some(ll) if *ll != line => {
            print!("{:4}:", line);
            *ll = line;
        }
        _ => print!("     "),
    }
    print!(" {:04}  ", i);
    i += 1;

    macro_rules! read_byte {
        () => {{
            let b = vm.heap.func(fn_ref).code[i];
            i += 1;
            b as usize
        }};
    }
    macro_rules! read_short {
        () => {{
            let c = &vm.heap.func(fn_ref).code;
            i += 2;
            ((c[i - 2] as usize) << 8) | c[i - 1] as usize
        }};
    }

    macro_rules! byte_instruction {
        ($name:expr) => {{
            println!("{:<16} {:5}", $name, read_byte!());
        }};
    }

    match code {
        Code::Constant => {
            let constant = read_short!();
            print!("{:<16} {:5} '", "CONSTANT", constant);
            dump_value(vm, vm.heap.func(fn_ref).constants[constant]);
            println!("'");
        }
        Code::Null => println!("NULL"),
        Code::False => println!("FALSE"),
        Code::True => println!("TRUE"),
        Code::LoadLocal0 => println!("LOAD_LOCAL_0"),
        Code::LoadLocal1 => println!("LOAD_LOCAL_1"),
        Code::LoadLocal2 => println!("LOAD_LOCAL_2"),
        Code::LoadLocal3 => println!("LOAD_LOCAL_3"),
        Code::LoadLocal4 => println!("LOAD_LOCAL_4"),
        Code::LoadLocal5 => println!("LOAD_LOCAL_5"),
        Code::LoadLocal6 => println!("LOAD_LOCAL_6"),
        Code::LoadLocal7 => println!("LOAD_LOCAL_7"),
        Code::LoadLocal8 => println!("LOAD_LOCAL_8"),
        Code::LoadLocal => byte_instruction!("LOAD_LOCAL"),
        Code::StoreLocal => byte_instruction!("STORE_LOCAL"),
        Code::LoadUpvalue => byte_instruction!("LOAD_UPVALUE"),
        Code::StoreUpvalue => byte_instruction!("STORE_UPVALUE"),
        Code::LoadModuleVar => {
            let slot = read_short!();
            println!(
                "{:<16} {:5} '{}'",
                "LOAD_MODULE_VAR",
                slot,
                var_name(vm, module, slot)
            );
        }
        Code::StoreModuleVar => {
            let slot = read_short!();
            println!(
                "{:<16} {:5} '{}'",
                "STORE_MODULE_VAR",
                slot,
                var_name(vm, module, slot)
            );
        }
        Code::LoadFieldThis => byte_instruction!("LOAD_FIELD_THIS"),
        Code::StoreFieldThis => byte_instruction!("STORE_FIELD_THIS"),
        Code::LoadField => byte_instruction!("LOAD_FIELD"),
        Code::StoreField => byte_instruction!("STORE_FIELD"),
        Code::Pop => println!("POP"),
        c if (Code::Call0 as u8) <= (c as u8) && (c as u8) <= (Code::Call16 as u8) => {
            let num_args = (c as u8) - (Code::Call0 as u8);
            let symbol = read_short!();
            println!(
                "CALL_{:<11} {:5} '{}'",
                num_args,
                symbol,
                method_name(vm, symbol)
            );
        }
        c if (Code::Super0 as u8) <= (c as u8) && (c as u8) <= (Code::Super16 as u8) => {
            let num_args = (c as u8) - (Code::Super0 as u8);
            let symbol = read_short!();
            let superclass = read_short!();
            println!(
                "SUPER_{:<10} {:5} '{}' {:5}",
                num_args,
                symbol,
                method_name(vm, symbol),
                superclass
            );
        }
        Code::Jump => {
            let offset = read_short!();
            println!("{:<16} {:5} to {}", "JUMP", offset, i + offset);
        }
        Code::Loop => {
            let offset = read_short!();
            println!("{:<16} {:5} to {}", "LOOP", offset, i - offset);
        }
        Code::JumpIf => {
            let offset = read_short!();
            println!("{:<16} {:5} to {}", "JUMP_IF", offset, i + offset);
        }
        Code::And => {
            let offset = read_short!();
            println!("{:<16} {:5} to {}", "AND", offset, i + offset);
        }
        Code::Or => {
            let offset = read_short!();
            println!("{:<16} {:5} to {}", "OR", offset, i + offset);
        }
        Code::CloseUpvalue => println!("CLOSE_UPVALUE"),
        Code::Return => println!("RETURN"),
        Code::Closure => {
            let constant = read_short!();
            print!("{:<16} {:5} ", "CLOSURE", constant);
            dump_value(vm, vm.heap.func(fn_ref).constants[constant]);
            print!(" ");
            let loaded = vm.heap.func(fn_ref).constants[constant].as_obj();
            let num_upvalues = vm.heap.func(loaded).num_upvalues;
            for j in 0..num_upvalues {
                let is_local = read_byte!();
                let index = read_byte!();
                if j > 0 {
                    print!(", ");
                }
                print!(
                    "{} {}",
                    if is_local != 0 { "local" } else { "upvalue" },
                    index
                );
            }
            println!();
        }
        Code::Construct => println!("CONSTRUCT"),
        Code::ForeignConstruct => println!("FOREIGN_CONSTRUCT"),
        Code::Class => {
            let num_fields = read_byte!();
            println!("{:<16} {:5} fields", "CLASS", num_fields);
        }
        Code::ForeignClass => println!("FOREIGN_CLASS"),
        Code::EndClass => println!("END_CLASS"),
        Code::MethodInstance => {
            let symbol = read_short!();
            println!(
                "{:<16} {:5} '{}'",
                "METHOD_INSTANCE",
                symbol,
                method_name(vm, symbol)
            );
        }
        Code::MethodStatic => {
            let symbol = read_short!();
            println!(
                "{:<16} {:5} '{}'",
                "METHOD_STATIC",
                symbol,
                method_name(vm, symbol)
            );
        }
        Code::EndModule => println!("END_MODULE"),
        Code::ImportModule => {
            let name = read_short!();
            print!("{:<16} {:5} '", "IMPORT_MODULE", name);
            dump_value(vm, vm.heap.func(fn_ref).constants[name]);
            println!("'");
        }
        Code::ImportVariable => {
            let variable = read_short!();
            print!("{:<16} {:5} '", "IMPORT_VARIABLE", variable);
            dump_value(vm, vm.heap.func(fn_ref).constants[variable]);
            println!("'");
        }
        Code::End => println!("END"),
        _ => println!("UKNOWN! [{}]", code_byte),
    }

    if code == Code::End {
        -1
    } else {
        (i - start) as i32
    }
}

pub fn dump_code(vm: &WrenVM, fn_ref: ObjRef) {
    let (module, name) = {
        let f = vm.heap.func(fn_ref);
        (
            f.module,
            f.debug.as_ref().map(|d| d.name.clone()).unwrap_or_default(),
        )
    };
    let module_name = vm.heap.module(module).name;
    let mod_str = if module_name == NULL_OBJ {
        "<core>".to_string()
    } else {
        String::from_utf8_lossy(&vm.heap.string(module_name).bytes).into_owned()
    };
    println!("{}: {}", mod_str, name);

    let mut i = 0usize;
    let mut last_line = -1i32;
    loop {
        let offset = dump_instruction(vm, fn_ref, i, Some(&mut last_line));
        if offset == -1 {
            break;
        }
        i += offset as usize;
    }
    println!();
}

pub fn dump_stack(vm: &WrenVM, fiber: ObjRef) {
    print!("(fiber {:?}) ", fiber);
    let f = vm.heap.fiber(fiber);
    for slot in 0..f.stack_top {
        dump_value(vm, f.stack[slot]);
        print!(" | ");
    }
    println!();
    let _ = std::io::stdout().flush();
}
