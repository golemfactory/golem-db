use std::collections::{BTreeMap, btree_map::Entry};

use crate::{
    ApiError, CellKind, CellName, CellNameRef, CellPatch, CellValue, RecordCells, RecordPatch,
    Result,
};

/// Owned creation input. Kind helpers override kind; insert preserves it.
/// Syntax is checked here, while deployment limits and the nonempty-record rule
/// are checked when executing CRUD. Input is consumed by create.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordInput {
    cells: RecordCells,
}

impl RecordInput {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an already-constructed value, retaining its kind and type.
    /// Repeated names are errors rather than implicit overwrites.
    pub fn insert(mut self, name: &str, value: CellValue) -> Result<Self> {
        insert_named(&mut self.cells, name.as_bytes(), value)?;
        Ok(self)
    }

    pub fn attribute(self, name: &str, value: CellValue) -> Result<Self> {
        self.insert(name, value.with_kind(CellKind::Attribute)?)
    }

    pub fn field(self, name: &str, value: CellValue) -> Result<Self> {
        self.insert(name, value.with_kind(CellKind::Field)?)
    }

    pub fn as_cells(&self) -> &RecordCells {
        &self.cells
    }
    pub fn into_cells(self) -> RecordCells {
        self.cells
    }
}

impl TryFrom<RecordCells> for RecordInput {
    type Error = ApiError;

    fn try_from(cells: RecordCells) -> Result<Self> {
        for name in cells.keys() {
            validate_name(name.as_bytes())?;
        }
        Ok(Self { cells })
    }
}

/// Owned partial mutation. Each name may appear only once, even across sets
/// and removals. Final record shape and existence are execution-time checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PatchInput {
    patch: RecordPatch,
}

impl PatchInput {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a value while retaining its existing kind and type.
    pub fn set(self, name: &str, value: CellValue) -> Result<Self> {
        self.edit(name, CellPatch::Set(value))
    }

    pub fn attribute(self, name: &str, value: CellValue) -> Result<Self> {
        self.set(name, value.with_kind(CellKind::Attribute)?)
    }

    pub fn field(self, name: &str, value: CellValue) -> Result<Self> {
        self.set(name, value.with_kind(CellKind::Field)?)
    }

    pub fn remove(self, name: &str) -> Result<Self> {
        self.edit(name, CellPatch::Remove)
    }

    fn edit(mut self, name: &str, change: CellPatch) -> Result<Self> {
        insert_named(&mut self.patch, name.as_bytes(), change)?;
        Ok(self)
    }

    pub fn as_patch(&self) -> &RecordPatch {
        &self.patch
    }
    pub fn into_patch(self) -> RecordPatch {
        self.patch
    }
}

impl TryFrom<RecordPatch> for PatchInput {
    type Error = ApiError;

    fn try_from(patch: RecordPatch) -> Result<Self> {
        for name in patch.keys() {
            validate_name(name.as_bytes())?;
        }
        Ok(Self { patch })
    }
}

fn validate_name(name: &[u8]) -> Result<CellName> {
    // The builder has no database configuration. Validate only grammar; record
    // admission checks #maxCellNameLen against the deployment's actual value.
    Ok(CellNameRef::parse_user(name, usize::MAX)?.into())
}

fn insert_named<T>(values: &mut BTreeMap<CellName, T>, name: &[u8], value: T) -> Result<()> {
    match values.entry(validate_name(name)?) {
        Entry::Vacant(entry) => {
            entry.insert(value);
            Ok(())
        }
        Entry::Occupied(entry) => Err(ApiError::invalid_argument(format!(
            "duplicate cell name: {}",
            entry.key()
        ))),
    }
}
