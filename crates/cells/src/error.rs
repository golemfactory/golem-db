//! [`CellParseError`]: why bytes are not a cell.

use core::fmt;

use crate::types::CellType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellParseError {
    /// No metadata byte.
    Empty,
    /// Type id `0x00`, the absent marker. See the crate docs.
    AbsentTag,
    /// A type id the spec reserves.
    ReservedType(u8),
    /// A `str`/`bytes` cell that ends inside its 4-byte length prefix.
    MissingLength,
    /// A fixed-width value of the wrong width.
    LengthMismatch {
        ty: CellType,
        expected: usize,
        actual: usize,
    },
    /// A length prefix declaring more bytes than follow.
    Truncated { declared: usize, actual: usize },
    /// Bytes after the cell. [`CellValue::parse_prefix`](crate::CellValue::parse_prefix)
    /// walks packed cells.
    TrailingBytes { extra: usize },
    /// A `str`/`bytes` value longer than [`MAX_VALUE_LEN`](crate::MAX_VALUE_LEN).
    TooLong { actual: usize },
    /// A `bool` byte other than 0 or 1.
    InvalidBool(u8),
    /// A `str` that stops being UTF-8 at byte `valid_up_to`.
    InvalidUtf8 { valid_up_to: usize },
    /// A NaN float: many bit patterns, no order.
    FloatNaN,
    /// A `-0.0` float: zero is stored as `+0.0` only.
    NegativeZero,
    /// The indexable bit on a `bytes` cell, which has no order.
    NotIndexable,
}

impl fmt::Display for CellParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "cell is empty"),
            Self::AbsentTag => write!(f, "type id 0 is the absent marker, not a type"),
            Self::ReservedType(id) => write!(f, "type id {id} is reserved"),
            Self::MissingLength => write!(f, "cell is missing its length prefix"),
            Self::LengthMismatch {
                ty,
                expected,
                actual,
            } => write!(f, "{ty:?} expects {expected} value bytes, got {actual}"),
            Self::Truncated { declared, actual } => {
                write!(
                    f,
                    "cell declares {declared} value bytes but carries {actual}"
                )
            }
            Self::TrailingBytes { extra } => write!(f, "{extra} bytes left over after the cell"),
            Self::TooLong { actual } => write!(
                f,
                "value is {actual} bytes, the maximum is {}",
                crate::MAX_VALUE_LEN
            ),
            Self::InvalidBool(b) => write!(f, "bool byte must be 0 or 1, got {b}"),
            Self::InvalidUtf8 { valid_up_to } => {
                write!(f, "str is not valid UTF-8 at byte {valid_up_to}")
            }
            Self::FloatNaN => write!(f, "float is NaN, which has no order"),
            Self::NegativeZero => write!(f, "float is -0.0; zero is stored as +0.0"),
            Self::NotIndexable => write!(f, "a bytes cell cannot be indexable"),
        }
    }
}

impl core::error::Error for CellParseError {}
