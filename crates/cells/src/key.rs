//! [`CellKey`] and [`CellKeyRef`]: cell names and the §3 grammar.
//!
//! ```text
//! name  = first *rest              ; 1 .. #maxCellNameLen bytes
//! first = ALPHA
//! rest  = ALPHA / DIGIT / "_" / "-" / "." / ":"
//! ```
//!
//! ASCII only and case-sensitive (`Price` ≠ `price`). No `0x00`, so the index
//! key's `0x00` separator is unambiguous. Engine names start with `#` or `$`,
//! which are not ALPHA, so a user name cannot reach them without any prefix
//! check.

use core::borrow::Borrow;
use core::fmt;

/// `#maxCellNameLen` when genesis names no other value. The live cap is in
/// `#params`, so [`CellKeyRef::parse_user`] takes it as an argument.
pub const DEFAULT_MAX_CELL_NAME_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKeyError {
    Empty,
    /// Longer than the caller's `#maxCellNameLen`.
    TooLong {
        max: usize,
        actual: usize,
    },
    /// The first byte is not a letter: a digit, a separator, `#` or `$`.
    NotAlphaFirst(u8),
    /// A byte outside the grammar, at offset `at` in the whole key.
    InvalidByte {
        at: usize,
        byte: u8,
    },
}

impl fmt::Display for CellKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "cell name is empty"),
            Self::TooLong { max, actual } => {
                write!(f, "cell name is {actual} bytes, the maximum is {max}")
            }
            Self::NotAlphaFirst(b) => write!(f, "cell name must begin with a letter, got {b:#04x}"),
            Self::InvalidByte { at, byte } => {
                write!(f, "byte {byte:#04x} at offset {at} is not a name character")
            }
        }
    }
}

impl core::error::Error for CellKeyError {}

/// `first *rest`. `base` shifts error offsets, so a name behind a sigil
/// reports positions in the whole key: `#max Len` fails at offset 4.
fn check(name: &[u8], base: usize) -> Result<(), CellKeyError> {
    let [first, rest @ ..] = name else {
        return Err(CellKeyError::Empty);
    };
    if !first.is_ascii_alphabetic() {
        return Err(CellKeyError::NotAlphaFirst(*first));
    }
    let is_rest = |b: &u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':');
    match rest.iter().position(|b| !is_rest(b)) {
        Some(i) => Err(CellKeyError::InvalidByte {
            at: base + 1 + i,
            byte: rest[i],
        }),
        None => Ok(()),
    }
}

/// A validated cell name borrowed from elsewhere: an MDBX key, a request
/// buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellKeyRef<'a>(&'a [u8]);

impl<'a> CellKeyRef<'a> {
    /// A user name: the grammar, capped at `max_len` (`#maxCellNameLen`).
    pub fn parse_user(name: &'a [u8], max_len: usize) -> Result<Self, CellKeyError> {
        if name.len() > max_len {
            return Err(CellKeyError::TooLong {
                max: max_len,
                actual: name.len(),
            });
        }
        check(name, 0)?;
        Ok(Self(name))
    }

    /// An engine name: `#` or `$`, then the grammar. No cap; engine names are
    /// constants.
    pub fn parse_engine(name: &'a [u8]) -> Result<Self, CellKeyError> {
        match name {
            [] => Err(CellKeyError::Empty),
            [b'#' | b'$', rest @ ..] => check(rest, 1).map(|()| Self(name)),
            [byte, ..] => Err(CellKeyError::InvalidByte { at: 0, byte: *byte }),
        }
    }

    /// A reserved record's key (§4), which is itself a value such as a
    /// `commitNr`, so no grammar applies. Engine-internal.
    pub const fn raw(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> &'a [u8] {
        self.0
    }
}

impl fmt::Display for CellKeyRef<'_> {
    /// Escaped, because `raw` keys need not be ASCII.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.escape_ascii().fmt(f)
    }
}

/// An owned cell name. `Ord` is bytewise, matching MDBX, so a
/// `BTreeMap<CellKey, _>` iterates in table order; `Borrow<[u8]>` lets it be
/// probed with a plain slice.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellKey(Box<[u8]>);

impl CellKey {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Borrow<[u8]> for CellKey {
    fn borrow(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Display for CellKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        CellKeyRef(&self.0).fmt(f)
    }
}

impl From<CellKeyRef<'_>> for CellKey {
    fn from(key: CellKeyRef<'_>) -> Self {
        Self(key.0.into())
    }
}

/// The engine's reserved cell names (§3, §4).
pub mod reserved {
    use super::CellKeyRef;

    /// A record's logical key.
    pub const KEY: CellKeyRef<'static> = CellKeyRef(b"#key");
    /// The next record ID to hand out.
    pub const NEXT_RECORD_ID: CellKeyRef<'static> = CellKeyRef(b"#nextRecordID");
    /// Chain parameter: the longest `str` value.
    pub const MAX_STR_LEN: CellKeyRef<'static> = CellKeyRef(b"#maxStrLen");
    /// Chain parameter: the longest `bytes` value.
    pub const MAX_BYTES_LEN: CellKeyRef<'static> = CellKeyRef(b"#maxBytesLen");
    /// Chain parameter: the cap [`CellKeyRef::parse_user`] is given.
    pub const MAX_CELL_NAME_LEN: CellKeyRef<'static> = CellKeyRef(b"#maxCellNameLen");

    pub const ALL: &[CellKeyRef<'static>] = &[
        KEY,
        NEXT_RECORD_ID,
        MAX_STR_LEN,
        MAX_BYTES_LEN,
        MAX_CELL_NAME_LEN,
    ];
}
