//! The cell vectors: byte strings that must parse, and ones that must not.

use crate::*;

/// Name, wire bytes, indexable, type, stored value.
pub(super) type Vector = (&'static str, &'static [u8], bool, CellType, &'static [u8]);

/// `ty`'s type id followed by `N - 1` zero bytes, for the wide rows below.
const fn zero_cell<const N: usize>(ty: CellType) -> [u8; N] {
    let mut out = [0; N];
    out[0] = ty.id();
    out
}

const BYTES16: CellType = CellType::FixedBytes(Width::W16);
const BYTES32: CellType = CellType::FixedBytes(Width::W32);
const U128: CellType = CellType::Uint(Width::W16);
const U256: CellType = CellType::Uint(Width::W32);
const I128: CellType = CellType::Int(Width::W16);
const I256: CellType = CellType::Int(Width::W32);
const DEC128: CellType = CellType::Decimal(Width::W16);
const DEC256: CellType = CellType::Decimal(Width::W32);

/// Every type at least once, at both settings of the indexable bit.
#[rustfmt::skip]
pub(super) const VECTORS: &[Vector] = &[
    // name             wire bytes                                        idx    type                              value
    ("bool false",      &[0x01, 0x00],                                    false, CellType::Bool,                   &[0x00]),
    ("bool true, idx",  &[0x81, 0x01],                                    true,  CellType::Bool,                   &[0x01]),
    ("str empty",       &[0x02, 0x00, 0x00, 0x00, 0x00],                  false, CellType::Str,                    &[]),
    ("str ascii",       &[0x02, 0x00, 0x00, 0x00, 0x02, b'h', b'i'],      false, CellType::Str,                    b"hi"),
    ("str 4-byte, idx", &[0x82, 0x00, 0x00, 0x00, 0x04, 0xF0, 0x9F, 0xA6, 0x80], true, CellType::Str,              &[0xF0, 0x9F, 0xA6, 0x80]),
    ("str with a NUL",  &[0x02, 0x00, 0x00, 0x00, 0x03, b'a', 0x00, b'b'], false, CellType::Str,                   &[b'a', 0x00, b'b']),
    ("bytes empty",     &[0x03, 0x00, 0x00, 0x00, 0x00],                  false, CellType::Bytes,                  &[]),
    ("bytes 2",         &[0x03, 0x00, 0x00, 0x00, 0x02, 0xDE, 0xAD],      false, CellType::Bytes,                  &[0xDE, 0xAD]),
    ("bytes20",         &zero_cell::<21>(CellType::Bytes20),              false, CellType::Bytes20,                &[0; 20]),
    ("bytes4",          &[0x08, 1, 2, 3, 4],                              false, CellType::FixedBytes(Width::W4),  &[1, 2, 3, 4]),
    ("bytes8, idx",     &[0x89, 1, 2, 3, 4, 5, 6, 7, 8],                  true,  CellType::FixedBytes(Width::W8),  &[1, 2, 3, 4, 5, 6, 7, 8]),
    ("bytes16",         &zero_cell::<17>(BYTES16),                        false, BYTES16,                          &[0; 16]),
    ("bytes32",         &zero_cell::<33>(BYTES32),                        false, BYTES32,                          &[0; 32]),
    ("u32 7",           &[0x0C, 0, 0, 0, 7],                              false, CellType::Uint(Width::W4),        &[0, 0, 0, 7]),
    ("u64 7, idx",      &[0x8D, 0, 0, 0, 0, 0, 0, 0, 7],                  true,  CellType::Uint(Width::W8),        &[0, 0, 0, 0, 0, 0, 0, 7]),
    ("u128 0",          &zero_cell::<17>(U128),                           false, U128,                             &[0; 16]),
    ("u256 0",          &zero_cell::<33>(U256),                           false, U256,                             &[0; 32]),
    // Signed values are stored sign-flipped: 7F FF… is -1, 00 00… is MIN.
    ("i32 -1",          &[0x10, 0x7F, 0xFF, 0xFF, 0xFF],                  false, CellType::Int(Width::W4),         &[0x7F, 0xFF, 0xFF, 0xFF]),
    ("i64 -1",          &[0x11, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], false, CellType::Int(Width::W8), &[0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
    ("i128 MIN",        &zero_cell::<17>(I128),                           false, I128,                             &[0; 16]),
    ("i256 MIN",        &zero_cell::<33>(I256),                           false, I256,                             &[0; 32]),
    ("dec32 -1",        &[0x14, 0x7F, 0xFF, 0xFF, 0xFF],                  false, CellType::Decimal(Width::W4),     &[0x7F, 0xFF, 0xFF, 0xFF]),
    ("dec64 MIN",       &[0x15, 0, 0, 0, 0, 0, 0, 0, 0],                  false, CellType::Decimal(Width::W8),     &[0, 0, 0, 0, 0, 0, 0, 0]),
    ("dec128 MIN",      &zero_cell::<17>(DEC128),                         false, DEC128,                           &[0; 16]),
    ("dec256 MIN",      &zero_cell::<33>(DEC256),                         false, DEC256,                           &[0; 32]),
    // 1.0 is 3F 80 00 00; stored with the sign bit flipped.
    ("f32 1.0",         &[0x18, 0xBF, 0x80, 0, 0],                        false, CellType::Float(FloatWidth::F32), &[0xBF, 0x80, 0, 0]),
    ("f64 1.0",         &[0x19, 0xBF, 0xF0, 0, 0, 0, 0, 0, 0],            false, CellType::Float(FloatWidth::F64), &[0xBF, 0xF0, 0, 0, 0, 0, 0, 0]),
    ("date32 20000",    &[0x1C, 0x80, 0, 0x4E, 0x20],                     false, CellType::Date32,                 &[0x80, 0, 0x4E, 0x20]),
    ("timestamp64 1",   &[0x9D, 0x80, 0, 0, 0, 0, 0, 0, 1],               true,  CellType::Timestamp64,            &[0x80, 0, 0, 0, 0, 0, 0, 1]),
];

/// Name, wire bytes, and the exact error they must produce.
pub(super) type BadVector = (&'static str, &'static [u8], CellParseError);

#[rustfmt::skip]
pub(super) const BAD_VECTORS: &[BadVector] = &[
    ("empty slice",            &[],                                      CellParseError::Empty),
    ("absent tag",             &[0x00],                                  CellParseError::AbsentTag),
    ("absent tag, idx bit",    &[0x80, 9],                               CellParseError::AbsentTag),
    ("reserved 5",             &[5],                                     CellParseError::ReservedType(5)),
    ("reserved 64, payload",   &[64, 0, 0, 0, 0],                        CellParseError::ReservedType(64)),
    ("reserved 127, idx bit",  &[0xFF, 1, 2, 3],                         CellParseError::ReservedType(127)),

    ("bytes20 too short",      &[0x04, 0, 0],                            CellParseError::LengthMismatch { ty: CellType::Bytes20, expected: 20, actual: 2 }),
    ("u64 empty",              &[0x0D],                                  CellParseError::LengthMismatch { ty: CellType::Uint(Width::W8), expected: 8, actual: 0 }),
    ("bool, spare byte",       &[0x01, 1, 9],                            CellParseError::TrailingBytes { extra: 1 }),
    ("str, partial length",    &[0x02, 0x00, 0x00],                      CellParseError::MissingLength),
    ("str cut short",          &[0x02, 0x00, 0x00, 0x00, 4, b'h', b'i'], CellParseError::Truncated { declared: 4, actual: 2 }),
    ("bytes, extra past end",  &[0x03, 0x00, 0x00, 0x00, 1, 1, 9, 9],    CellParseError::TrailingBytes { extra: 2 }),

    ("bool byte 2",            &[0x01, 2],                               CellParseError::InvalidBool(2)),
    ("str, bad byte mid-way",  &[0x02, 0x00, 0x00, 0x00, 3, b'h', 0x80, b'i'], CellParseError::InvalidUtf8 { valid_up_to: 1 }),
    // The length is right for the framing but cuts `é` (C3 A9) in half.
    ("str, len splits a char", &[0x02, 0x00, 0x00, 0x00, 1, 0xC3, 0xA9], CellParseError::InvalidUtf8 { valid_up_to: 0 }),

    // Stored forms: NaN 7FC00000 flips to FFC00000; -0.0 80000000 inverts to 7FFFFFFF.
    ("f32 NaN",                &[0x18, 0xFF, 0xC0, 0, 0],                CellParseError::FloatNaN),
    ("f32 -0.0",               &[0x18, 0x7F, 0xFF, 0xFF, 0xFF],          CellParseError::NegativeZero),
    ("f64 negative NaN",       &[0x19, 0x00, 0x07, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], CellParseError::FloatNaN),
    ("f64 -0.0",               &[0x19, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], CellParseError::NegativeZero),

    ("bytes, idx bit",         &[0x83, 0x00, 0x00, 0x00, 0x00],          CellParseError::NotIndexable),
];
