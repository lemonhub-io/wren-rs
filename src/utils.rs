// Port of wren_utils.c: UTF-8 encode/decode helpers, power-of-2 rounding,
// and index validation. Buffers/symbol tables are plain Vec<T> in Rust.

pub fn utf8_encode_num_bytes(value: i32) -> usize {
    debug_assert!(value >= 0, "Cannot encode a negative value.");
    if value <= 0x7f {
        1
    } else if value <= 0x7ff {
        2
    } else if value <= 0xffff {
        3
    } else if value <= 0x10ffff {
        4
    } else {
        0
    }
}

/// Encodes `value` as UTF-8, appending to `bytes`. Returns bytes written.
pub fn utf8_encode(value: i32, bytes: &mut Vec<u8>) -> usize {
    if value <= 0x7f {
        bytes.push((value & 0x7f) as u8);
        1
    } else if value <= 0x7ff {
        bytes.push((0xc0 | ((value & 0x7c0) >> 6)) as u8);
        bytes.push((0x80 | (value & 0x3f)) as u8);
        2
    } else if value <= 0xffff {
        bytes.push((0xe0 | ((value & 0xf000) >> 12)) as u8);
        bytes.push((0x80 | ((value & 0xfc0) >> 6)) as u8);
        bytes.push((0x80 | (value & 0x3f)) as u8);
        3
    } else if value <= 0x10ffff {
        bytes.push((0xf0 | ((value & 0x1c0000) >> 18)) as u8);
        bytes.push((0x80 | ((value & 0x3f000) >> 12)) as u8);
        bytes.push((0x80 | ((value & 0xfc0) >> 6)) as u8);
        bytes.push((0x80 | (value & 0x3f)) as u8);
        4
    } else {
        unreachable!()
    }
}

/// Decodes the UTF-8 sequence starting at `bytes[0]`, at most `length` bytes.
/// Returns the code point or -1 if invalid.
pub fn utf8_decode(bytes: &[u8]) -> i32 {
    let length = bytes.len() as u32;
    let b0 = bytes[0];
    if b0 <= 0x7f {
        return b0 as i32;
    }
    let (mut value, mut remaining): (i32, u32) = if (b0 & 0xe0) == 0xc0 {
        ((b0 & 0x1f) as i32, 1)
    } else if (b0 & 0xf0) == 0xe0 {
        ((b0 & 0x0f) as i32, 2)
    } else if (b0 & 0xf8) == 0xf0 {
        ((b0 & 0x07) as i32, 3)
    } else {
        return -1;
    };

    if remaining > length.saturating_sub(1) {
        return -1;
    }

    let mut i = 1usize;
    while remaining > 0 {
        let b = bytes[i];
        i += 1;
        remaining -= 1;
        if (b & 0xc0) != 0x80 {
            return -1;
        }
        value = (value << 6) | (b & 0x3f) as i32;
    }
    value
}

/// Number of bytes in the UTF-8 sequence starting with `byte`, or 0 if it is
/// a continuation byte.
pub fn utf8_decode_num_bytes(byte: u8) -> usize {
    if (byte & 0xc0) == 0x80 {
        return 0;
    }
    if (byte & 0xf8) == 0xf0 {
        return 4;
    }
    if (byte & 0xf0) == 0xe0 {
        return 3;
    }
    if (byte & 0xe0) == 0xc0 {
        return 2;
    }
    1
}

/// Smallest power of two >= n.
pub fn power_of_2_ceil(n: i32) -> i32 {
    let mut n = n;
    n -= 1;
    n |= n >> 1;
    n |= n >> 2;
    n |= n >> 4;
    n |= n >> 8;
    n |= n >> 16;
    n + 1
}

/// Validates `value` is within `[0, count)`, allowing negative indices to
/// count from the end. Returns u32::MAX if invalid.
pub fn validate_index(count: u32, value: i64) -> u32 {
    let mut v = value;
    if v < 0 {
        v += count as i64;
    }
    if v >= 0 && v < count as i64 {
        v as u32
    } else {
        u32::MAX
    }
}
