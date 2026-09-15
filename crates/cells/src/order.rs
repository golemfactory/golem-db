//! Stored forms that make bytewise order equal value order, for the types
//! whose natural bytes do not already sort. See the crate docs, "Order
//! encoding".

/// Two's complement big-endian ⇄ stored form: flip the sign bit. Flipping
/// twice is a no-op, so this both encodes and decodes.
///
/// ```text
/// i32::MIN  80 00 00 00  →  00 00 00 00
///       -1  FF FF FF FF  →  7F FF FF FF
///        0  00 00 00 00  →  80 00 00 00
/// i32::MAX  7F FF FF FF  →  FF FF FF FF
/// ```
///
/// Big-endian puts the sign bit in byte 0 at every width, so `i256` is the
/// same one-byte flip.
pub fn flip_sign<const N: usize>(mut bytes: [u8; N]) -> [u8; N] {
    bytes[0] ^= 0x80;
    bytes
}

/// IEEE-754 big-endian → stored form. A non-negative float flips its sign
/// bit, as an integer does. A negative float flips every bit, because its
/// bits grow with its magnitude and so sort backwards:
///
/// ```text
/// -2.0  C0 00 00 00  →  3F FF FF FF
/// -1.0  BF 80 00 00  →  40 7F FF FF    3F… < 40…, so -2.0 < -1.0
///  0.0  00 00 00 00  →  80 00 00 00
///  1.0  3F 80 00 00  →  BF 80 00 00
/// ```
///
/// NaN and `-0.0` encode to bytes that
/// [`CellType::validate`](crate::CellType::validate) rejects.
pub fn encode_float<const N: usize>(be: [u8; N]) -> [u8; N] {
    if be[0] & 0x80 == 0 {
        flip_sign(be)
    } else {
        be.map(|b| !b)
    }
}

/// Stored form → IEEE-754 big-endian. The stored top bit is set exactly for
/// non-negative values (see [`encode_float`]).
pub fn decode_float<const N: usize>(stored: [u8; N]) -> [u8; N] {
    if stored[0] & 0x80 != 0 {
        flip_sign(stored)
    } else {
        stored.map(|b| !b)
    }
}
