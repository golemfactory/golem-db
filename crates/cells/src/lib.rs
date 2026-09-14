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
//!
//! # Cell names
//!
//! [`CellKey`] and [`CellKeyRef`] carry the *name* half of a cell — the §3
//! grammar, the engine's `#`/`$` names, and the raw byte keys reserved records
//! use. See [`CellKeyRef`].
//!
//! # The `0x00` tag
//!
//! **Type ids start at `0x01`; `0x00` is not a type.** It is the engine's
//! unambiguous "absent" marker, and [`CellType::from_id`] rejects it with
//! [`CellParseError::AbsentTag`]. There is no `CellType::Tombstone`.
//!
//! A tombstone is a property of the **branch overlay** (§10) — volatile,
//! node-local, outside the commitment — so it is spelled `Option<CellValue>`
//! there rather than added to the committed type grid. `Option` is Rust's own
//! representation of absence, the compiler folds it into the `CellType` niche
//! at no cost, and a `match` that forgets the `None` arm does not compile.
//! Resolves the A2 spec gap; pinned by `absent_tag_is_not_a_type`.
//!
//! # Open question: the length prefix
//!
//! The length prefix is 4 bytes while [`MAX_VALUE_LEN`] is 64 KiB, so **two of
//! every variable cell's four length bytes are permanently zero**. Those bytes
//! may enter the `CellTrie` leaf preimage (B1 owns that call), and the
//! encoding freezes at genesis.
//!
//! **Recommendation: narrow the prefix to 2 bytes.** It exactly fits the
//! existing 64 KiB cap, costs nothing to change today, and saves two bytes on
//! every `str`, `bytes` and custom cell — including inside the trie preimage,
//! where the saving is hashed rather than merely stored. Raising the cap to
//! use the four bytes is the defensible alternative, but 64 KiB is already
//! generous for a cell value and a 4 GiB cell is not a shape this engine wants
//! to admit. Either way the decision must land before genesis; shipping two
//! dead bytes per variable cell by default is the one outcome to avoid.

mod error;
mod key;
mod types;
mod value;

#[cfg(test)]
mod tests;

pub use error::CellParseError;
pub use key::{CellKey, CellKeyError, CellKeyRef, DEFAULT_MAX_CELL_NAME_LEN, reserved};
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
