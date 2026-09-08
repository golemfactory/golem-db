//! Rust definitions for "Cells" in GolemDB, alongwith the logic for encoding and decoding them
//!
//! # Wire format
//!
//! A cell is one metadata byte, a 4-byte big-endian length for the
//! variable-width types only, then the value bytes:
//!
//! ```text
//! [ indexable: 1 bit | type id: 7 bits ] [ len: u32 BE ]? [ value bytes … ]
//! ```
//!
//! The type id says which of the two shapes a cell has. A fixed-width type
//! carries its width in its id, so no length prefix; `str`, `bytes` and the
//! custom types carry one.
//!
//! That makes every cell **self-delimiting**: its length is knowable from its
//! own bytes. Cells can be packed adjacently and walked with
//! [`CellValue::parse_prefix`], and a cell truncated in transit is rejected rather
//! than read as a shorter valid value.

mod error;
mod types;
mod value;

#[cfg(test)]
mod tests;

pub use error::CellParseError;
#[cfg(feature = "custom_types")]
pub use types::CustomTypeId;
pub use types::{CellType, FloatWidth, ValueLayout, Width};
pub use value::CellValue;

/// The longest variable-width value a cell may carry.
///
/// The wire format's 4-byte length prefix could express up to `u32::MAX`;
/// this layer pins a tighter cap of 64 KiB instead, so `TooLong` stays cheap
/// to hit in practice. A deployment is free to enforce something tighter
/// still on top.
pub const MAX_VALUE_LEN: usize = u16::MAX as usize;

/// How many bytes the length prefix occupies on the wire, for the
/// variable-width types.
pub(crate) const LENGTH_PREFIX_BYTES: usize = 4;

/// The metadata byte's high bit: whether the cell is indexable.
pub(crate) const INDEXABLE_BIT: u8 = 0b1000_0000;

/// The metadata byte's low 7 bits: the type id.
pub(crate) const TYPE_ID_MASK: u8 = !INDEXABLE_BIT;

/// One past the last type id: the id space is 7 bits wide.
pub const TYPE_ID_SPACE: u8 = 128;

/// The first type id belonging to the custom block.
pub const CUSTOM_TYPE_ID_BASE: u8 = 64;

/// How many bytes [`CellParseError::InvalidUtf8`] quotes back. Enough to show
/// the longest UTF-8 sequence and its neighbours, short enough for a log line.
pub const UTF8_SNIPPET_LEN: usize = 8;
