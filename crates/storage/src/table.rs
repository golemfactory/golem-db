use crate::{Result, StorageError};

/// Logical table name, created on the first `put` or `insert`.
///
/// Backends may use native tables or isolated key prefixes. Names must be
/// nonempty and contain no NUL bytes. Constants need no registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Table(pub &'static str);

impl Table {
    pub(crate) fn validate(self) -> Result<()> {
        if self.0.is_empty() || self.0.contains('\0') {
            return Err(StorageError::InvalidTableName(self));
        }
        Ok(())
    }
}
