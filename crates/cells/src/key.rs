//! Complete cell addresses: `record_id` (eight bytes BE) followed by the name.

use core::fmt;

use crate::{CellName, CellNameRef};

/// A complete address in the Cell table. Ordering matches encoded byte order.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellKey {
    record_id: u64,
    name: CellName,
}

impl CellKey {
    pub fn new(record_id: u64, name: impl Into<CellName>) -> Self {
        Self {
            record_id,
            name: name.into(),
        }
    }

    pub const fn record_id(&self) -> u64 {
        self.record_id
    }

    pub fn name(&self) -> CellNameRef<'_> {
        self.name.as_view()
    }

    /// Decode a storage key, preserving raw names used by reserved records.
    /// Name admission and record authorization belong to the caller.
    pub fn decode(bytes: &[u8]) -> Result<Self, CellKeyError> {
        let (id, name) = bytes
            .split_first_chunk::<8>()
            .ok_or(CellKeyError::MissingRecordId {
                actual: bytes.len(),
            })?;
        Ok(Self::new(u64::from_be_bytes(*id), CellNameRef::raw(name)))
    }

    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.record_id.to_be_bytes());
        out.extend_from_slice(self.name.as_bytes());
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + self.name.as_bytes().len());
        self.encode_into(&mut out);
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKeyError {
    MissingRecordId { actual: usize },
}

impl fmt::Display for CellKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRecordId { actual } => write!(
                f,
                "cell key requires an 8-byte record ID, got {actual} bytes"
            ),
        }
    }
}

impl core::error::Error for CellKeyError {}
