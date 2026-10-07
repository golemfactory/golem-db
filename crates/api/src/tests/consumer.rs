//! A scripted consumer mock, not an alternative database implementation.
use std::sync::{Arc, Mutex};

use crate::*;

struct MockApi {
    created: Mutex<Vec<(BranchId, RecordOp<op::Create>)>>,
    commit_result: Mutex<Option<Result<CommitId>>>,
}

impl Api for MockApi {
    fn create(&self, branch: BranchId, operation: RecordOp<op::Create>) -> Result<RecordKey> {
        let key = operation.record_key();
        self.created.lock().unwrap().push((branch, operation));
        Ok(key)
    }
    fn get(&self, _: ReadTarget, _: RecordOp<op::Get>) -> Result<Record> {
        panic!("get was not expected by this script")
    }
    fn patch(&self, _: BranchId, _: RecordOp<op::Patch>) -> Result<()> {
        panic!("patch was not expected by this script")
    }
    fn delete(&self, _: BranchId, _: RecordOp<op::Delete>) -> Result<()> {
        panic!("delete was not expected by this script")
    }
    fn begin(&self) -> Result<BranchId> {
        Ok(golemdb_branch::BranchId::new(7))
    }
    fn commit(&self, branch: BranchId) -> Result<CommitId> {
        assert_eq!(branch, golemdb_branch::BranchId::new(7));
        self.commit_result
            .lock()
            .unwrap()
            .take()
            .expect("one commit expected")
    }
    fn head(&self) -> Result<CommitId> {
        panic!("head not expected")
    }
    fn branch_info(&self, _: BranchId) -> Result<BranchInfo> {
        panic!("branch_info not expected")
    }
    fn checkpoint(&self, _: BranchId) -> Result<()> {
        panic!("checkpoint not expected")
    }
    fn rollback(&self, _: BranchId) -> Result<()> {
        panic!("rollback not expected")
    }
    fn seal(&self, _: BranchId) -> Result<SealInfo> {
        panic!("seal not expected")
    }
    fn discard(&self, _: BranchId) -> Result<()> {
        panic!("discard not expected")
    }

    fn immutable_data_append(
        &self,
        _: BranchId,
        _: &str,
        _: Option<ImmutableDataKey>,
        _: ImmutableDataRow,
    ) -> Result<ImmutableDataOrdinal> {
        Err(ApiError::NotImplemented {
            operation: "immutable_data_append",
        })
    }
    fn immutable_data_get(&self, _: &str, _: ImmutableDataAddress) -> Result<ImmutableDataRow> {
        Err(ApiError::NotImplemented {
            operation: "immutable_data_get",
        })
    }
    fn immutable_data_range_of(
        &self,
        _: &str,
        _: CommitId,
    ) -> Result<std::ops::Range<ImmutableDataOrdinal>> {
        Err(ApiError::NotImplemented {
            operation: "immutable_data_range_of",
        })
    }
    fn immutable_data_rows_of(&self, _: &str, _: CommitId) -> Result<Vec<ImmutableDataRow>> {
        Err(ApiError::NotImplemented {
            operation: "immutable_data_rows_of",
        })
    }
}

fn stage_price(api: &dyn Api, branch: BranchId, key: RecordKey) -> Result<RecordKey> {
    api.create(
        branch,
        RecordOp::create(key).attribute("price", CellValue::from_i32(50))?,
    )
}

fn create_price(api: Arc<dyn Api + Send + Sync>, key: RecordKey) -> Result<CommitId> {
    let branch = api.begin()?;
    if let Err(error) = stage_price(api.as_ref(), branch, key) {
        api.discard(branch)?;
        return Err(error);
    }
    api.commit(branch)
}

#[test]
fn facade_trait_accepts_thread_safe_mocks_and_injected_failures() {
    let key = RecordKey([1; 32]);
    for commit_result in [Ok(CommitId::new(1)), Err(ApiError::Conflict)] {
        let fails = commit_result.is_err();
        let mock = Arc::new(MockApi {
            created: Mutex::new(Vec::new()),
            commit_result: Mutex::new(Some(commit_result)),
        });
        let api: Arc<dyn Api + Send + Sync> = mock.clone();
        let result = create_price(api, key);
        if fails {
            assert!(matches!(result, Err(ApiError::Conflict)));
        } else {
            assert_eq!(result.unwrap(), golemdb_branch::CommitId::new(1));
        }
        assert_eq!(mock.created.lock().unwrap().len(), 1);
        assert!(mock.commit_result.lock().unwrap().is_none());
        let calls = mock.created.lock().unwrap();
        assert_eq!(
            (calls[0].0, calls[0].1.record_key()),
            (BranchId::new(7), key)
        );
        assert_eq!(calls[0].1.value("price").unwrap().as_i32(), Some(50));
    }
}
