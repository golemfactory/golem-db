//! The stored form of signed integers and floats, laid out so that bytewise
//! order is value order. Every other type's natural bytes are already in
//! order. See the crate docs, "Order encoding".

/// Two's complement big-endian → stored form: flip the sign bit.
pub fn encode_int<const N: usize>(mut be: [u8; N]) -> [u8; N] {
    flip_sign(&mut be);
    be
}

/// Stored form → two's complement big-endian.
pub fn decode_int<const N: usize>(stored: [u8; N]) -> [u8; N] {
    encode_int(stored)
}

/// IEEE-754 big-endian → stored form. NaN and `-0.0` encode to bytes that
/// [`CellType::validate`](crate::CellType::validate) rejects.
pub fn encode_float<const N: usize>(mut be: [u8; N]) -> [u8; N] {
    float_to_stored(&mut be);
    be
}

/// Stored form → IEEE-754 big-endian.
pub fn decode_float<const N: usize>(mut stored: [u8; N]) -> [u8; N] {
    float_from_stored(&mut stored);
    stored
}

// Big-endian puts the sign bit in byte 0 at every width, so none of these
// needs to know the width.

pub(crate) fn flip_sign(bytes: &mut [u8]) {
    bytes[0] ^= 0x80;
}

// A negative float's bits grow with its magnitude, so it is inverted whole;
// a non-negative one only flips the sign. Stored, the top bit is set exactly
// for non-negatives.

pub(crate) fn float_to_stored(bytes: &mut [u8]) {
    if bytes[0] & 0x80 != 0 {
        invert(bytes);
    } else {
        flip_sign(bytes);
    }
}

pub(crate) fn float_from_stored(bytes: &mut [u8]) {
    if bytes[0] & 0x80 != 0 {
        flip_sign(bytes);
    } else {
        invert(bytes);
    }
}

fn invert(bytes: &mut [u8]) {
    for b in bytes {
        *b = !*b;
    }
}
