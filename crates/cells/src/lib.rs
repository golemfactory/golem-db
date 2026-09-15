//! GolemDB cells: typed values with a self-delimiting wire format.
//!
//! # Wire format
//!
//! ```text
//! [ indexable: 1 bit | type id: 7 bits ] [ len: u32 BE ]? [ value … ]
//!
//! u64 7, indexable   8D 00 00 00 00 00 00 00 07
//! str "hi"           02 00 00 00 02 68 69
//! ```
//!
//! Fixed-width types carry their width in the type id; `str` and `bytes`
//! carry a 4-byte length. Every cell is therefore self-delimiting: cells pack
//! adjacently ([`CellValue::parse_prefix`]) and a truncated cell is rejected.
//!
//! Type id `0x00` is not a type. It is the branch overlay's "absent" marker
//! ([`CellParseError::AbsentTag`]); a tombstone is `Option<CellValue>::None`.
//!
//! # Order encoding
//!
//! Stored value bytes sort as their values do, so a value is copied verbatim
//! into its index term `cellKey ‖ 0x00 ‖ typeTag ‖ value`, and a cursor walk
//! over that is a range query. A value has one byte form everywhere.
//!
//! | stored form                         | types                                                 |
//! | ----------------------------------- | ----------------------------------------------------- |
//! | natural bytes                       | `bool`, `str`, `bytes20`, `bytes4..32`, `u32..u256`   |
//! | sign bit flipped ([`flip_sign`])    | `i32..i256`, `dec32..dec256`, `date32`, `timestamp64` |
//! | IEEE total order ([`encode_float`]) | `f32`, `f64`                                          |
//! | not indexable                       | `bytes`                                               |
//!
//! ```text
//! i32 -1   FF FF FF FF  stored  7F FF FF FF
//! i32 100  00 00 00 64  stored  80 00 00 64   7F… < 80…, so -1 < 100
//! ```
//!
//! NaN and `-0.0` are rejected, so zero has one byte form. `str` is the last
//! field of an index term, so it needs no terminator; the term omits the wire
//! length prefix, which would sort by length first.
//!
//! Decimals are signed integers at a fixed scale per width
//! ([`Width::decimal_scale`]): `dec32` 4, `dec64` 6, `dec128` 18, `dec256` 18.
//! `dec256` is fixed by Arkiv (wei); the others await sign-off.
//!
//! # Open question: the length prefix
//!
//! [`MAX_VALUE_LEN`] fits in 2 bytes, so two of the four length bytes are
//! always zero, and may be hashed into the `CellTrie` preimage. Narrow the
//! prefix to 2 bytes before genesis.

mod error;
mod key;
mod order;
mod types;
mod value;

#[cfg(test)]
mod tests;

pub use error::CellParseError;
pub use key::{CellKey, CellKeyError, CellKeyRef, DEFAULT_MAX_CELL_NAME_LEN, reserved};
pub use order::{decode_float, encode_float, flip_sign};
pub use types::{CellType, FloatWidth, Width};
pub use value::CellValue;

/// The longest `str` or `bytes` value.
pub const MAX_VALUE_LEN: usize = u16::MAX as usize;

/// The metadata byte's high bit: whether the cell is indexable. The low 7
/// bits are the type id.
pub(crate) const INDEXABLE_BIT: u8 = 0x80;
