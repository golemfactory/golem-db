//! Genesis-supplied admission limits, corresponding to the `#params` cells.

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::{CellNameError, CellNameRef, CellType, CellValueRef};

/// Deployment policy in bytes, independent of the canonical cell codec.
///
/// All fields are explicit: no local default may silently change admission.
/// The database must persist these values at genesis and validate them against
/// store ceilings, including the full attribute index key size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CellLimits {
    pub max_cell_name_len: u32,
    pub max_str_len: u32,
    pub max_bytes_len: u32,
}

impl CellLimits {
    pub fn parse_user_name<'a>(&self, name: &'a [u8]) -> Result<CellNameRef<'a>, CellNameError> {
        CellNameRef::parse_user(name, self.max_cell_name_len as usize)
    }

    /// Admit a structurally valid value under this deployment's size policy.
    /// Fixed-width types have no additional configurable size limit.
    pub fn validate_value(&self, cell: CellValueRef<'_>) -> Result<(), CellLimitError> {
        let max = match cell.cell_type() {
            CellType::Str => self.max_str_len,
            CellType::Bytes => self.max_bytes_len,
            _ => return Ok(()),
        };
        let actual = cell.value().len();
        if actual > max as usize {
            return Err(CellLimitError {
                ty: cell.cell_type(),
                max,
                actual,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellLimitError {
    pub ty: CellType,
    pub max: u32,
    pub actual: usize,
}

impl fmt::Display for CellLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} value is {} bytes, the configured maximum is {}",
            self.ty, self.actual, self.max
        )
    }
}

impl core::error::Error for CellLimitError {}
