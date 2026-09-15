//! [`CellValue`]: a parsed cell and its typed accessors.

use crate::INDEXABLE_BIT;
use crate::error::CellParseError;
use crate::order::{decode_float, flip_sign};
use crate::types::{CellType, FloatWidth, Width};

/// A cell: a type, its stored value bytes, and whether it is indexable.
/// Borrows the value straight out of the storage buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellValue<'a> {
    ty: CellType,
    value: &'a [u8],
    indexable: bool,
}

impl<'a> CellValue<'a> {
    /// Build a cell from a stored value, validating it.
    pub fn new(ty: CellType, value: &'a [u8], indexable: bool) -> Result<Self, CellParseError> {
        if indexable && ty == CellType::Bytes {
            return Err(CellParseError::NotIndexable);
        }
        ty.validate(value)?;
        Ok(Self {
            ty,
            value,
            indexable,
        })
    }

    /// Decode exactly one cell; leftover bytes are an error.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, CellParseError> {
        match Self::parse_prefix(bytes)? {
            (cell, []) => Ok(cell),
            (_, rest) => Err(CellParseError::TrailingBytes { extra: rest.len() }),
        }
    }

    /// Decode the cell at the front of `bytes`, returning it and the rest.
    pub fn parse_prefix(bytes: &'a [u8]) -> Result<(Self, &'a [u8]), CellParseError> {
        let (&metadata, rest) = bytes.split_first().ok_or(CellParseError::Empty)?;
        let ty = CellType::from_id(metadata & !INDEXABLE_BIT)?;
        let (value, rest) = match ty.width() {
            Some(n) if rest.len() < n => {
                return Err(CellParseError::LengthMismatch {
                    ty,
                    expected: n,
                    actual: rest.len(),
                });
            }
            Some(n) => rest.split_at(n),
            None => {
                let (len, rest) = rest
                    .split_first_chunk::<4>()
                    .ok_or(CellParseError::MissingLength)?;
                let len = u32::from_be_bytes(*len) as usize;
                if rest.len() < len {
                    return Err(CellParseError::Truncated {
                        declared: len,
                        actual: rest.len(),
                    });
                }
                rest.split_at(len)
            }
        };
        Ok((Self::new(ty, value, metadata & INDEXABLE_BIT != 0)?, rest))
    }

    /// Append this cell's wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.metadata());
        if self.ty.width().is_none() {
            // `new` caps the value at MAX_VALUE_LEN, so this fits.
            out.extend_from_slice(&(self.value.len() as u32).to_be_bytes());
        }
        out.extend_from_slice(self.value);
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    /// The indexable bit over the type id.
    pub const fn metadata(&self) -> u8 {
        self.ty.id() | if self.indexable { INDEXABLE_BIT } else { 0 }
    }

    pub const fn cell_type(&self) -> CellType {
        self.ty
    }

    /// The stored value bytes.
    pub const fn value(&self) -> &'a [u8] {
        self.value
    }

    pub const fn is_indexable(&self) -> bool {
        self.indexable
    }

    // Typed accessors: `None` unless the cell holds exactly that type.

    /// The value as an array, if the cell is of type `want`.
    fn exact<const N: usize>(&self, want: CellType) -> Option<[u8; N]> {
        // `validate` fixed the width, and each caller asks for its type's own.
        (self.ty == want).then(|| self.value.try_into().unwrap())
    }

    pub fn as_bool(&self) -> Option<bool> {
        self.exact(CellType::Bool).map(|[b]| b == 1)
    }

    pub fn as_str(&self) -> Option<&'a str> {
        match self.ty {
            CellType::Str => core::str::from_utf8(self.value).ok(),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&'a [u8]> {
        (self.ty == CellType::Bytes).then_some(self.value)
    }

    pub fn as_bytes4(&self) -> Option<[u8; 4]> {
        self.exact(CellType::FixedBytes(Width::W4))
    }

    pub fn as_bytes8(&self) -> Option<[u8; 8]> {
        self.exact(CellType::FixedBytes(Width::W8))
    }

    pub fn as_bytes16(&self) -> Option<[u8; 16]> {
        self.exact(CellType::FixedBytes(Width::W16))
    }

    pub fn as_bytes20(&self) -> Option<[u8; 20]> {
        self.exact(CellType::Bytes20)
    }

    pub fn as_bytes32(&self) -> Option<[u8; 32]> {
        self.exact(CellType::FixedBytes(Width::W32))
    }

    pub fn as_u32(&self) -> Option<u32> {
        self.exact(CellType::Uint(Width::W4))
            .map(u32::from_be_bytes)
    }

    pub fn as_u64(&self) -> Option<u64> {
        self.exact(CellType::Uint(Width::W8))
            .map(u64::from_be_bytes)
    }

    pub fn as_u128(&self) -> Option<u128> {
        self.exact(CellType::Uint(Width::W16))
            .map(u128::from_be_bytes)
    }

    /// Big-endian; Rust has no `u256`.
    pub fn as_u256_be(&self) -> Option<[u8; 32]> {
        self.exact(CellType::Uint(Width::W32))
    }

    // Signed types are stored sign-flipped (`7F FF FF FF` is -1); these
    // flip it back.

    pub fn as_i32(&self) -> Option<i32> {
        self.exact(CellType::Int(Width::W4))
            .map(|b| i32::from_be_bytes(flip_sign(b)))
    }

    pub fn as_i64(&self) -> Option<i64> {
        self.exact(CellType::Int(Width::W8))
            .map(|b| i64::from_be_bytes(flip_sign(b)))
    }

    pub fn as_i128(&self) -> Option<i128> {
        self.exact(CellType::Int(Width::W16))
            .map(|b| i128::from_be_bytes(flip_sign(b)))
    }

    /// Two's complement big-endian; Rust has no `i256`.
    pub fn as_i256_be(&self) -> Option<[u8; 32]> {
        self.exact(CellType::Int(Width::W32)).map(flip_sign)
    }

    // Decimals return the unscaled integer; the scale is `Width::decimal_scale`.

    pub fn as_dec32_unscaled(&self) -> Option<i32> {
        self.exact(CellType::Decimal(Width::W4))
            .map(|b| i32::from_be_bytes(flip_sign(b)))
    }

    pub fn as_dec64_unscaled(&self) -> Option<i64> {
        self.exact(CellType::Decimal(Width::W8))
            .map(|b| i64::from_be_bytes(flip_sign(b)))
    }

    pub fn as_dec128_unscaled(&self) -> Option<i128> {
        self.exact(CellType::Decimal(Width::W16))
            .map(|b| i128::from_be_bytes(flip_sign(b)))
    }

    /// Two's complement big-endian.
    pub fn as_dec256_unscaled_be(&self) -> Option<[u8; 32]> {
        self.exact(CellType::Decimal(Width::W32)).map(flip_sign)
    }

    pub fn as_f32(&self) -> Option<f32> {
        self.exact(CellType::Float(FloatWidth::F32))
            .map(|b| f32::from_be_bytes(decode_float(b)))
    }

    pub fn as_f64(&self) -> Option<f64> {
        self.exact(CellType::Float(FloatWidth::F64))
            .map(|b| f64::from_be_bytes(decode_float(b)))
    }

    /// Days since the Unix epoch.
    pub fn as_date32(&self) -> Option<i32> {
        self.exact(CellType::Date32)
            .map(|b| i32::from_be_bytes(flip_sign(b)))
    }

    /// Microseconds since the Unix epoch.
    pub fn as_timestamp64(&self) -> Option<i64> {
        self.exact(CellType::Timestamp64)
            .map(|b| i64::from_be_bytes(flip_sign(b)))
    }
}
