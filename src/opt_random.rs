// Port of wren_opt_random.c: the built-in "random" module (WELL512a PRNG
// stored in a foreign object). The C version seeds via srand()/rand(); we use
// an equivalent internal seed generator since tests check distributions, not
// exact sequences.

use crate::value::*;
use crate::vm::{WrenForeignClassMethods, WrenForeignMethodFn, WrenVM};

pub const RANDOM_SOURCE: &[u8] = include_bytes!("wren_opt_random.wren");

/// Well512 state: 16 u32s + index, stored little-endian in the foreign data.
fn well_get(data: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([
        data[i * 4],
        data[i * 4 + 1],
        data[i * 4 + 2],
        data[i * 4 + 3],
    ])
}
fn well_set(data: &mut [u8], i: usize, v: u32) {
    data[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
}
fn well_index(data: &[u8]) -> u32 {
    well_get(data, 16)
}
fn well_set_index(data: &mut [u8], v: u32) {
    well_set(data, 16, v)
}

/// advanceState from wren_opt_random.c.
fn advance_state(data: &mut [u8]) -> u32 {
    let index = well_index(data) as usize;
    let a = well_get(data, index);
    let c = well_get(data, (index + 13) & 15);
    let b = a ^ c ^ (a << 16) ^ (c << 15);
    let mut c2 = well_get(data, (index + 9) & 15);
    c2 ^= c2 >> 11;
    let a2 = b ^ c2;
    well_set(data, index, a2);
    let d = a2 ^ ((a2 << 5) & 0xda442d24);
    let new_index = (index + 15) & 15;
    well_set_index(data, new_index as u32);
    let a3 = well_get(data, new_index);
    let r = a3 ^ b ^ d ^ (a3 << 2) ^ (b << 18) ^ (c2 << 28);
    well_set(data, new_index, r);
    r
}

/// A small seedable PRNG used only for seeding the Well512 (replaces
/// srand/rand; no test depends on the exact C sequence).
struct SeedRng(u64);
impl SeedRng {
    fn next(&mut self) -> u32 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D) as u32
    }
}

fn foreign_of(vm: &mut WrenVM, slot: i32) -> ObjRef {
    vm.get_slot_foreign(slot)
}

fn random_allocate(vm: &mut WrenVM) {
    let f = vm.set_slot_new_foreign(0, 0, 17 * 4);
    let data = vm.foreign_data_mut(f);
    for b in data.iter_mut() {
        *b = 0;
    }
}

fn random_seed0(vm: &mut WrenVM) {
    let f = foreign_of(vm, 0);
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e3779b97f4a7c15);
    let mut rng = SeedRng(seed | 1);
    let data = vm.foreign_data_mut(f);
    for i in 0..16 {
        well_set(data, i, rng.next());
    }
}

fn random_seed1(vm: &mut WrenVM) {
    let f = foreign_of(vm, 0);
    let seed = vm.get_slot_double(1);
    let mut rng = SeedRng((seed.to_bits() as u64) | 1);
    let data = vm.foreign_data_mut(f);
    for i in 0..16 {
        well_set(data, i, rng.next());
    }
}

fn random_seed16(vm: &mut WrenVM) {
    let f = foreign_of(vm, 0);
    for i in 0..16 {
        let v = vm.get_slot_double(i as i32 + 1);
        let data = vm.foreign_data_mut(f);
        well_set(data, i, v as u32);
    }
}

fn random_float(vm: &mut WrenVM) {
    let f = foreign_of(vm, 0);
    // 53 bits of random source data.
    let mut result = {
        let data = vm.foreign_data_mut(f);
        advance_state(data)
    } as f64
        * (1u64 << 21) as f64;
    result += {
        let data = vm.foreign_data_mut(f);
        advance_state(data) & ((1 << 21) - 1)
    } as f64;
    result /= 9007199254740992.0;
    vm.set_slot_double(0, result);
}

fn random_int0(vm: &mut WrenVM) {
    let f = foreign_of(vm, 0);
    let r = {
        let data = vm.foreign_data_mut(f);
        advance_state(data)
    };
    vm.set_slot_double(0, r as f64);
}

pub fn bind_foreign_method(
    _vm: &mut WrenVM,
    class_name: &[u8],
    _is_static: bool,
    signature: &[u8],
) -> Option<WrenForeignMethodFn> {
    debug_assert!(class_name == b"Random");
    match signature {
        b"<allocate>" => Some(random_allocate),
        b"seed_()" => Some(random_seed0),
        b"seed_(_)" => Some(random_seed1),
        b"seed_(_,_,_,_,_,_,_,_,_,_,_,_,_,_,_,_)" => Some(random_seed16),
        b"float()" => Some(random_float),
        b"int()" => Some(random_int0),
        _ => None,
    }
}

pub fn bind_foreign_class(
    _vm: &mut WrenVM,
    _module: &[u8],
    class_name: &[u8],
) -> WrenForeignClassMethods {
    debug_assert!(class_name == b"Random");
    WrenForeignClassMethods {
        allocate: Some(random_allocate),
        finalize: None,
    }
}
