use golemdb_storage::*;
use std::{cell::Cell, ops::Bound::Unbounded, rc::Rc};

struct FailingTransaction(Rc<Cell<usize>>);
struct FailingCursor(Rc<Cell<usize>>);

impl ReadCursor for FailingCursor {
    fn prev(&mut self) -> Result<Option<Entry>> {
        self.next()
    }
    fn next(&mut self) -> Result<Option<Entry>> {
        let count = self.0.get();
        self.0.set(count + 1);
        if count == 0 {
            Ok(Some((b"a".to_vec(), vec![])))
        } else {
            Err(StorageError::Backend(
                std::io::Error::other("injected read failure").into(),
            ))
        }
    }
}

impl ReadTransaction for FailingTransaction {
    type Cursor<'tx> = FailingCursor;
    fn get(&self, _: Table, _: &[u8]) -> Result<Option<Vec<u8>>> {
        unreachable!()
    }
    fn cursor(&self, _: Table, _: &[u8]) -> Result<Self::Cursor<'_>> {
        Ok(FailingCursor(self.0.clone()))
    }
}

#[test]
fn scan_returns_error_once_and_never_advances_again() {
    let calls = Rc::new(Cell::new(0));
    let tx = FailingTransaction(calls.clone());
    let mut scan = scan(&tx, Table("test"), Unbounded, Unbounded).unwrap();
    assert!(scan.next().unwrap().is_ok());
    assert!(matches!(scan.next(), Some(Err(StorageError::Backend(_)))));
    assert!(scan.next().is_none());
    assert!(scan.next().is_none());
    assert_eq!(calls.get(), 2);
}
