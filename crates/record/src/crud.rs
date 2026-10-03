use golemdb_branch::{BranchId, Branches, CellRead, read_head};
use golemdb_cells::{CellKey, CellLimits, CellName, CellReader, CellValue, reserved, system};
use golemdb_merkle::HashProvider;
use golemdb_storage::{ReadTransaction, Store};

use crate::{
    CellPatch, ReadTarget, Record, RecordCells, RecordError, RecordKey, RecordPatch, Result, state,
};

/// Cloneable record facade; clones share the supplied branch manager.
pub struct Records<S, H> {
    branches: Branches<S, H>,
}

impl<S, H> Clone for Records<S, H> {
    fn clone(&self) -> Self {
        Self {
            branches: self.branches.clone(),
        }
    }
}

impl<S: Store, H: HashProvider> Records<S, H> {
    pub fn new(branches: Branches<S, H>) -> Self {
        Self { branches }
    }

    /// Read a full record or an explicit projection. Missing projected cells
    /// are omitted; an empty projection returns only the record's key. Duplicate
    /// names collapse in the result. Names may be raw, including reserved cells.
    /// Record existence and identity are checked even for an empty projection.
    pub fn get(
        &self,
        target: ReadTarget,
        key: RecordKey,
        projection: Option<&[CellName]>,
    ) -> Result<Record> {
        match target {
            ReadTarget::Branch(branch) => self
                .branches
                .read(branch, |cell_reader| {
                    read_branch_record(cell_reader, key, projection)
                })
                .map_err(Into::into),
            ReadTarget::Head | ReadTarget::Commit(_) => {
                let tx = self.branches.store().begin_read()?;
                let head = read_head(&tx)?;
                if let ReadTarget::Commit(requested) = target
                    && requested != head
                {
                    return Err(RecordError::CommitUnavailable { requested, head });
                }
                read_committed_record(&CellReader::new(&tx), key, projection)
            }
        }
    }

    /// Create a record with at least one user cell. Exact reserved keys are
    /// rejected before branch access; all other validation follows handle checks.
    /// Names and values are checked in byte order before identity/allocation reads.
    pub fn create(
        &self,
        branch: BranchId,
        key: RecordKey,
        values: RecordCells,
    ) -> Result<RecordKey> {
        reject_reserved(key)?;
        self.branches
            .write(branch, |cell_writer| {
                let cell_reader = cell_writer.as_read();
                if values.is_empty() {
                    return Err(RecordError::InvalidArgument(
                        "a record needs at least one user cell".into(),
                    ));
                }
                let limits = state::limits(&cell_reader)?;
                for (name, value) in &values {
                    validate(&limits, name, Some(value))?;
                }
                if state::resolve(&cell_reader, key)?.is_some() {
                    return Err(RecordError::AlreadyExists);
                }
                let (id, next) = state::next_id(&cell_reader)?;
                if cell_writer
                    .scan_prefix(&id.to_be_bytes())?
                    .next()
                    .transpose()?
                    .is_some()
                {
                    return Err(RecordError::CorruptState(
                        "allocator points to occupied record",
                    ));
                }
                for (name, value) in values {
                    cell_writer.put(CellKey::new(id, name), value);
                }
                cell_writer.put(
                    CellKey::new(id, reserved::KEY),
                    CellValue::from_bytes32(key.0),
                );
                cell_writer.put(state::binding(key), CellValue::from_u64(id));
                cell_writer.put(state::allocator_key(), CellValue::from_u64(next));
                Ok(key)
            })
            .map_err(Into::into)
    }

    /// Apply a partial mutation. Empty patches and removing absent cells are
    /// no-ops on an existing record. At least one user cell must remain, excluding
    /// #key. A removal-only patch scans the record to check its final shape.
    pub fn patch(&self, branch: BranchId, key: RecordKey, changes: RecordPatch) -> Result<()> {
        reject_reserved(key)?;
        self.branches
            .write(branch, |cell_writer| {
                let cell_reader = cell_writer.as_read();
                let id = state::resolve(&cell_reader, key)?.ok_or(RecordError::NotFound)?;
                let limits = state::limits(&cell_reader)?;
                for (name, change) in &changes {
                    validate(
                        &limits,
                        name,
                        match change {
                            CellPatch::Set(value) => Some(value),
                            CellPatch::Remove => None,
                        },
                    )?;
                }
                if changes
                    .values()
                    .any(|change| matches!(change, CellPatch::Remove))
                {
                    let has_set = changes
                        .values()
                        .any(|change| matches!(change, CellPatch::Set(_)));
                    if !has_set {
                        let survives = cell_reader
                            .scan_prefix(&id.to_be_bytes())?
                            .collect::<golemdb_branch::Result<Vec<_>>>()?
                            .iter()
                            .any(|(address, _)| {
                                // Only valid user names count, not structural cells.
                                limits.parse_user_name(address.name().as_bytes()).is_ok()
                                    && !matches!(
                                        changes.get(address.name().as_bytes()),
                                        Some(CellPatch::Remove)
                                    )
                            });
                        if !survives {
                            return Err(RecordError::InvalidArgument(
                                "cannot remove the last user cell".into(),
                            ));
                        }
                    }
                }
                for (name, change) in changes {
                    let address = CellKey::new(id, name);
                    match change {
                        CellPatch::Set(value) => {
                            if cell_writer.get(&address)?.as_ref() != Some(&value) {
                                cell_writer.put(address, value);
                            }
                        }
                        CellPatch::Remove => {
                            if cell_writer.get(&address)?.is_some() {
                                cell_writer.delete(address);
                            }
                        }
                    }
                }
                Ok(())
            })
            .map_err(Into::into)
    }

    /// Remove the binding and every live cell, including #key. Successful
    /// deletion never rewinds the allocator; recreation receives a fresh ID.
    pub fn delete(&self, branch: BranchId, key: RecordKey) -> Result<()> {
        reject_reserved(key)?;
        self.branches
            .write(branch, |cell_writer| {
                let cell_reader = cell_writer.as_read();
                let id = state::resolve(&cell_reader, key)?.ok_or(RecordError::NotFound)?;
                let addresses = cell_reader
                    .scan_prefix(&id.to_be_bytes())?
                    .map(|row| row.map(|(address, _)| address))
                    .collect::<golemdb_branch::Result<Vec<_>>>()?;
                for address in addresses {
                    cell_writer.delete(address);
                }
                cell_writer.delete(state::binding(key));
                Ok(())
            })
            .map_err(Into::into)
    }
}

fn reject_reserved(key: RecordKey) -> Result<()> {
    if system::by_key(&key.0).is_some() {
        Err(RecordError::Reserved)
    } else {
        Ok(())
    }
}

fn validate(limits: &CellLimits, name: &CellName, value: Option<&CellValue>) -> Result<()> {
    limits
        .parse_user_name(name.as_bytes())
        .map_err(|error| RecordError::InvalidArgument(error.to_string()))?;
    if let Some(value) = value {
        limits
            .validate_value(value.as_view())
            .map_err(|error| RecordError::InvalidArgument(error.to_string()))?;
    }
    Ok(())
}

fn read_branch_record<R: ReadTransaction>(
    cell_reader: &CellRead<'_, R>,
    key: RecordKey,
    projection: Option<&[CellName]>,
) -> Result<Record> {
    let id = state::resolve(cell_reader, key)?.ok_or(RecordError::NotFound)?;
    let mut values = RecordCells::new();
    match projection {
        Some(names) => {
            for name in names {
                if let Some(value) = cell_reader.get(&CellKey::new(id, name.clone()))? {
                    values.insert(name.clone(), value);
                }
            }
        }
        None => {
            for row in cell_reader.scan_prefix(&id.to_be_bytes())? {
                let (address, value) = row?;
                values.insert(address.name().into(), value);
            }
        }
    }
    Ok(Record { key, cells: values })
}

// Explicit storage path: cell storage owns decoding and scanning; record owns
// the binding/identity checks. No branch or empty overlay is created for reads.
fn read_committed_record<R: ReadTransaction>(
    cell_reader: &CellReader<'_, R>,
    key: RecordKey,
    projection: Option<&[CellName]>,
) -> Result<Record> {
    let id = state::record_id(key, cell_reader.get(&state::binding(key))?)?
        .ok_or(RecordError::NotFound)?;
    state::check_identity(key, cell_reader.get(&CellKey::new(id, reserved::KEY))?)?;
    let mut values = RecordCells::new();
    match projection {
        Some(names) => {
            for name in names {
                if let Some(value) = cell_reader.get(&CellKey::new(id, name.clone()))? {
                    values.insert(name.clone(), value);
                }
            }
        }
        None => {
            for row in cell_reader.scan_record(id)? {
                let (address, value) = row?;
                values.insert(address.name().into(), value);
            }
        }
    }
    Ok(Record { key, cells: values })
}
