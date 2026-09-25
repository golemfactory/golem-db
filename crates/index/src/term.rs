use golemdb_cells::{CellParseError, CellType, CellValue, FloatWidth};
use golemdb_merkle::{Hash, HashProvider};
use std::{error::Error, fmt};

/// Canonical ordered index key: `name || 0x00 || attribute_tag || ordered_value`.
///
/// Strings have no length prefix. Signed numbers flip the sign bit; floats use
/// the sortable IEEE transform after rejecting NaNs and normalizing zero.
/// Ordering is meaningful within one name/type prefix. Name/value size policies
/// beyond the cell codec belong to the engine, which knows deployment limits.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IndexTerm(Vec<u8>);

impl IndexTerm {
    pub fn new(name: &str, ty: CellType, value: &[u8]) -> Result<Self, TermError> {
        let mut bytes = Self::prefix(name, ty)?;
        ty.validate(value).map_err(TermError::Value)?;
        let mut ordered = value.to_vec();
        match ty {
            CellType::Int(_) | CellType::Decimal(_) | CellType::Date32 | CellType::Timestamp64 => {
                ordered[0] ^= 0x80;
            }
            CellType::Float(width) => {
                validate_float(&mut ordered, width)?;
                if ordered[0] & 0x80 != 0 {
                    for byte in &mut ordered {
                        *byte = !*byte;
                    }
                } else {
                    ordered[0] ^= 0x80;
                }
            }
            _ => {} // Supported by prefix(): unsigned, bool, string, fixed bytes.
        }
        bytes.extend_from_slice(&ordered);
        Ok(Self(bytes))
    }

    /// Fields, including reserved-record cells, produce no index term. Validate
    /// attribute names and types only after this check; system field keys may
    /// use names that are not user identifiers.
    pub fn from_cell(name: &str, cell: CellValue<'_>) -> Result<Option<Self>, TermError> {
        if !cell.is_indexable() {
            return Ok(None);
        }
        Self::new(name, cell.cell_type(), cell.value()).map(Some)
    }

    /// Prefix that confines range/prefix scans to one attribute name and type.
    pub fn prefix(name: &str, ty: CellType) -> Result<Vec<u8>, TermError> {
        validate_name(name)?;
        supported(ty)?;
        let mut out = Vec::with_capacity(name.len() + 2);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&[0, 0x80 | ty.id()]);
        Ok(out)
    }

    /// Decode exactly one term, rejecting noncanonical forms rather than
    /// normalizing persisted bytes. The trailing string consumes all remaining
    /// bytes; fixed-width types must have exactly their declared width.
    pub fn decode(bytes: &[u8]) -> Result<Self, TermError> {
        let split = bytes
            .iter()
            .position(|&b| b == 0)
            .ok_or(TermError::InvalidEncoding)?;
        let name = std::str::from_utf8(&bytes[..split]).map_err(|_| TermError::InvalidName)?;
        validate_name(name)?;
        let (&tag, value) = bytes[split + 1..]
            .split_first()
            .ok_or(TermError::InvalidEncoding)?;
        if tag & 0x80 == 0 {
            return Err(TermError::InvalidEncoding);
        }
        let ty = CellType::from_id(tag & 0x7f).map_err(TermError::Value)?;
        supported(ty)?;
        // Validate lengths before indexing a signed/float value. String and
        // bool validation is unchanged by the ordered representation.
        ty.validate(value).map_err(TermError::Value)?;
        let mut raw = value.to_vec();
        match ty {
            CellType::Int(_) | CellType::Decimal(_) | CellType::Date32 | CellType::Timestamp64 => {
                raw[0] ^= 0x80
            }
            CellType::Float(_) => {
                if raw[0] & 0x80 != 0 {
                    raw[0] ^= 0x80;
                } else {
                    for byte in &mut raw {
                        *byte = !*byte;
                    }
                }
            }
            _ => {}
        }
        let term = Self::new(name, ty, &raw)?;
        if term.as_bytes() != bytes {
            return Err(TermError::InvalidEncoding);
        }
        Ok(term)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// Full 32-byte IndexTrie routing path (the term itself has no hash domain).
    pub fn routing_path(&self, hasher: &(impl HashProvider + ?Sized)) -> Hash {
        hasher.hash(&self.0)
    }

    /// `H(0x02 || complete_term_path || bitmap_root)`.
    pub fn leaf_hash(&self, bitmap_root: &Hash, hasher: &(impl HashProvider + ?Sized)) -> Hash {
        hasher.hash_parts(&[&[0x02], &self.routing_path(hasher), bitmap_root])
    }
}

fn validate_name(name: &str) -> Result<(), TermError> {
    let bytes = name.as_bytes();
    let bytes = bytes.strip_prefix(b"$").unwrap_or(bytes);
    let Some((&first, rest)) = bytes.split_first() else {
        return Err(TermError::InvalidName);
    };
    if !first.is_ascii_alphabetic()
        || !rest
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(b))
    {
        return Err(TermError::InvalidName);
    }
    Ok(())
}

fn supported(ty: CellType) -> Result<(), TermError> {
    match ty {
        CellType::Bool
        | CellType::Str
        | CellType::Bytes20
        | CellType::FixedBytes(_)
        | CellType::Uint(_)
        | CellType::Int(_)
        | CellType::Decimal(_)
        | CellType::Date32
        | CellType::Timestamp64
        | CellType::Float(_) => Ok(()),
        _ => Err(TermError::UnsupportedType(ty)),
    }
}

// Classify using bits, avoiding floating-point arithmetic and preserving all
// nonzero finite/infinite encodings exactly, including subnormals.
fn validate_float(bytes: &mut [u8], width: FloatWidth) -> Result<(), TermError> {
    let (bits, sign, exponent, fraction) = match width {
        FloatWidth::F32 => (
            u32::from_be_bytes(bytes.try_into().expect("validated width")) as u64,
            0x8000_0000,
            0x7f80_0000,
            0x007f_ffff,
        ),
        FloatWidth::F64 => (
            u64::from_be_bytes(bytes.try_into().expect("validated width")),
            0x8000_0000_0000_0000,
            0x7ff0_0000_0000_0000,
            0x000f_ffff_ffff_ffff,
        ),
    };
    if bits & exponent == exponent && bits & fraction != 0 {
        return Err(TermError::NaN);
    }
    if bits & !sign == 0 {
        bytes.fill(0);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermError {
    InvalidName,
    UnsupportedType(CellType),
    Value(CellParseError),
    NaN,
    InvalidEncoding,
}

impl fmt::Display for TermError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName => f.write_str("invalid attribute name"),
            Self::UnsupportedType(ty) => write!(f, "no ordered index codec for {}", ty.name()),
            Self::Value(error) => write!(f, "invalid index value: {error}"),
            Self::NaN => f.write_str("NaN cannot be indexed"),
            Self::InvalidEncoding => f.write_str("invalid or noncanonical index term"),
        }
    }
}

impl Error for TermError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Value(error) => Some(error),
            _ => None,
        }
    }
}
