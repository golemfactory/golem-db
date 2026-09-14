//! [`CellKey`] and [`CellKeyRef`] — the cell *name*, and the §3 grammar it
//! must satisfy.
//!
//! A cell key is the component that follows `recordID` in a `Cell` key and
//! precedes the `0x00` separator in an `Index` key. The grammar is:
//!
//! ```text
//! name   = first *rest                ; 1 .. #maxCellNameLen bytes
//! first  = ALPHA
//! rest   = ALPHA / DIGIT / "_" / "-" / "." / ":"
//! ```
//!
//! Sixty-eight usable characters, and four properties fall out of that:
//!
//! - **ASCII only, case-sensitive, no normalization** — `Price` and `price`
//!   are distinct cells.
//! - **No `0x00`**, which is what makes the `Index` key separator unambiguous.
//! - **No leading digit**, so names stay visually distinct from numeric
//!   literals in query tooling.
//! - **Reserved prefixes are unreachable, not checked.** `#` (engine meta
//!   cells) and `$` (the admin class) are not ALPHA, so a user name cannot
//!   begin with one. There is deliberately no `starts_with('#')` rejection
//!   anywhere below: the grammar already covers it.
//!
//! There are no structural rules on separators — §3 chooses "versatility over
//! tidiness" — so `trailing.` and `a..b` are both valid names.

use core::borrow::Borrow;
use core::fmt;

/// The `#maxCellNameLen` a genesis file gets when it names no other value, and
/// what the tests parse against.
///
/// The cap itself is a chain parameter living in the `#params` system record
/// (§4), so [`CellKeyRef::parse_user`] takes it as an argument and never reads
/// this constant.
pub const DEFAULT_MAX_CELL_NAME_LEN: usize = 64;

/// Why a byte string is not a cell name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKeyError {
    /// A name of zero bytes; the grammar requires at least `first`.
    Empty,
    /// Longer than the caller's `#maxCellNameLen`.
    TooLong { max: usize, actual: usize },
    /// The first byte is not a letter. Digits, separators and the reserved
    /// `#`/`$` sigils all land here — the grammar rejects them by shape, not
    /// by a special case.
    NotAlphaFirst(u8),
    /// A byte outside `ALPHA / DIGIT / "_" / "-" / "." / ":"`, at `at`.
    InvalidByte { at: usize, byte: u8 },
}

impl fmt::Display for CellKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "cell name is empty"),
            Self::TooLong { max, actual } => {
                write!(f, "cell name is {actual} bytes, the maximum is {max}")
            }
            Self::NotAlphaFirst(b) => {
                write!(f, "cell name must begin with a letter, got {b:#04x}")
            }
            Self::InvalidByte { at, byte } => {
                write!(f, "byte {byte:#04x} at offset {at} is not a name character")
            }
        }
    }
}

impl core::error::Error for CellKeyError {}

/// `rest` in the grammar: `ALPHA / DIGIT / "_" / "-" / "." / ":"`.
const fn is_rest(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':')
}

/// `first *rest`, with error offsets shifted by `base` so a name behind a
/// sigil reports positions in the whole key.
const fn check(name: &[u8], base: usize) -> Result<(), CellKeyError> {
    let [first, rest @ ..] = name else {
        return Err(CellKeyError::Empty);
    };
    if !first.is_ascii_alphabetic() {
        return Err(CellKeyError::NotAlphaFirst(*first));
    }
    let mut i = 0;
    while i < rest.len() {
        if !is_rest(rest[i]) {
            return Err(CellKeyError::InvalidByte {
                at: base + 1 + i,
                byte: rest[i],
            });
        }
        i += 1;
    }
    Ok(())
}

/// A validated cell name borrowed from elsewhere — a decoded MDBX key, a
/// caller's request buffer. Construction costs only the validation walk.
///
/// `Copy`: one pointer and one length, so passing it by value is free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellKeyRef<'a>(&'a [u8]);

impl<'a> CellKeyRef<'a> {
    /// Data-plane names: the full §3 grammar, capped at `max_len`.
    ///
    /// `max_len` is `#maxCellNameLen`, which the caller reads from `#params`.
    pub const fn parse_user(name: &'a [u8], max_len: usize) -> Result<Self, CellKeyError> {
        if name.is_empty() {
            return Err(CellKeyError::Empty);
        }
        if name.len() > max_len {
            return Err(CellKeyError::TooLong {
                max: max_len,
                actual: name.len(),
            });
        }
        match check(name, 0) {
            Ok(()) => Ok(Self(name)),
            Err(e) => Err(e),
        }
    }

    /// Engine meta cells: `#` or `$` followed by the same grammar.
    ///
    /// Library code only — a user name must begin with a letter, so the data
    /// plane cannot reach this name space. No length cap: engine names are
    /// compile-time constants, not user input.
    pub const fn parse_engine(name: &'a [u8]) -> Result<Self, CellKeyError> {
        let [sigil, rest @ ..] = name else {
            return Err(CellKeyError::Empty);
        };
        if *sigil != b'#' && *sigil != b'$' {
            return Err(CellKeyError::InvalidByte {
                at: 0,
                byte: *sigil,
            });
        }
        match check(rest, 1) {
            Ok(()) => Ok(Self(name)),
            Err(e) => Err(e),
        }
    }

    /// Reserved records only (§4): the cell key *is* a value — a `commitNr`
    /// (8 B), a `recordKey` (32 B), `modelVersion ‖ weightName`. No grammar
    /// applies, so this validates nothing and returns no `Result`.
    ///
    /// Engine-internal. It is safe because reserved records are engine-written
    /// and the cell key is the trailing field of every key embedding it, so a
    /// raw fixed-width key still parses unambiguously behind its prefix.
    pub const fn raw(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> &'a [u8] {
        self.0
    }

    pub fn to_owned(self) -> CellKey {
        CellKey(self.0.into())
    }
}

impl<'a> AsRef<[u8]> for CellKeyRef<'a> {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

impl fmt::Display for CellKeyRef<'_> {
    /// Names are ASCII by construction; `raw` keys are not, so escape.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.escape_ascii().fmt(f)
    }
}

/// An owned validated cell name, for the branch overlay and wherever a key
/// must outlive the buffer it was read from.
///
/// `Box<[u8]>` rather than `Vec<u8>`: a validated name is never appended to,
/// so the capacity field is eight wasted bytes per key, and the overlay holds
/// a great many of them.
///
/// `Ord` is the derive's plain bytewise order, matching MDBX, so a
/// `BTreeMap<CellKey, _>` iterates in table order. Do not give it a custom
/// comparison.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellKey(Box<[u8]>);

impl CellKey {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn as_key_ref(&self) -> CellKeyRef<'_> {
        CellKeyRef(&self.0)
    }
}

/// So a `BTreeMap<CellKey, _>` or `HashMap<CellKey, _>` can be queried with a
/// `&[u8]`, without allocating a `CellKey` per lookup.
impl Borrow<[u8]> for CellKey {
    fn borrow(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for CellKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Display for CellKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_key_ref().fmt(f)
    }
}

impl<'a> From<CellKeyRef<'a>> for CellKey {
    fn from(key: CellKeyRef<'a>) -> Self {
        key.to_owned()
    }
}

/// The engine's reserved cell names (§3, §4). Each is `parse_engine`-valid;
/// [`super::tests`] pins that.
pub mod reserved {
    use super::CellKeyRef;

    /// A record's logical key cell.
    pub const KEY: CellKeyRef<'static> = CellKeyRef(b"#key");
    /// The next record ID the engine will hand out.
    pub const NEXT_RECORD_ID: CellKeyRef<'static> = CellKeyRef(b"#nextRecordID");
    /// Chain parameter: the longest `str` value.
    pub const MAX_STR_LEN: CellKeyRef<'static> = CellKeyRef(b"#maxStrLen");
    /// Chain parameter: the longest `bytes` value.
    pub const MAX_BYTES_LEN: CellKeyRef<'static> = CellKeyRef(b"#maxBytesLen");
    /// Chain parameter: the cap [`CellKeyRef::parse_user`] is handed.
    pub const MAX_CELL_NAME_LEN: CellKeyRef<'static> = CellKeyRef(b"#maxCellNameLen");

    /// Every constant above, for the tests that walk them.
    pub const ALL: &[CellKeyRef<'static>] = &[
        KEY,
        NEXT_RECORD_ID,
        MAX_STR_LEN,
        MAX_BYTES_LEN,
        MAX_CELL_NAME_LEN,
    ];
}
