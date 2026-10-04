// wren-rs: a Rust port of the Wren language VM (wren.io, version 0.4.0),
// covering the compiler, bytecode interpreter, mark/sweep GC, core library,
// optional meta/random modules, and the embedding API.

pub mod compiler;
mod core;
pub mod debug;
pub mod opcodes;
pub mod opt_meta;
pub mod opt_random;
pub mod utils;
pub mod value;
pub mod vm;

pub use value::{
    values_equal, values_same, ObjRef, Value, FALSE_VAL, NULL_OBJ, NULL_VAL, TRUE_VAL,
    UNDEFINED_VAL,
};
pub use vm::{
    WrenConfiguration, WrenErrorType, WrenForeignClassMethods, WrenHandle, WrenInterpretResult,
    WrenLoadModuleResult, WrenType, WrenVM, WREN_VERSION_NUMBER, WREN_VERSION_STRING,
};

pub fn get_version_number() -> i32 {
    WREN_VERSION_NUMBER
}
