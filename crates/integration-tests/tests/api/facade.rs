use std::sync::{Arc, Barrier};

use golemdb_api::*;
use golemdb_cells::system;
use golemdb_storage::MemoryDatabase;

const KEY: RecordKey = RecordKey([0x42; 32]);
const MISSING: RecordKey = RecordKey([0x43; 32]);

fn config(hash_function: HashAlgorithm) -> OpenConfig {
    OpenConfig::new(GenesisConfig {
        hash_function,
        cell_limits: CellLimits {
            max_cell_name_len: 32,
            max_str_len: 64,
            max_bytes_len: 128,
        },
    })
}

fn input(price: i32) -> RecordInput {
    RecordInput::new()
        .attribute("price", CellValue::from_i32(price))
        .unwrap()
        .field("description", CellValue::from_str("product"))
        .unwrap()
}

// Every contract is exercised through the same public handle on both hashes
// and both backends. A directory remains alive until all facade clones drop.
fn each_backend(contract: fn(GolemDb)) {
    for hash in [HashAlgorithm::Keccak256, HashAlgorithm::Blake3] {
        contract(GolemDb::open_memory(&config(hash)).unwrap());
        {
            let dir = tempfile::tempdir().unwrap();
            contract(GolemDb::open_database(dir.path(), &config(hash)).unwrap());
        }
    }
}

fn get(api: &dyn Api, target: ReadTarget) -> Record {
    api.get(target, KEY, Projection::All).unwrap()
}

fn allocator(api: &dyn Api, target: ReadTarget) -> u64 {
    api.get(
        target,
        RecordKey(system::ALLOC.key),
        Projection::only(["#nextRecordID"]),
    )
    .unwrap()
    .cells[b"#nextRecordID".as_slice()]
    .as_u64()
    .unwrap()
}

#[test]
fn crud_pending_reads_projections_and_recreation() {
    each_backend(|db| {
        let api: Arc<dyn Api + Send + Sync> = Arc::new(db.clone());
        assert_eq!(api.head().unwrap(), 0);
        let branch = api.begin().unwrap();
        let other = api.begin().unwrap();
        assert_eq!(api.create(branch, KEY, input(50)).unwrap(), KEY);
        assert!(matches!(
            api.create(branch, KEY, input(60)),
            Err(ApiError::AlreadyExists)
        ));
        for target in [
            ReadTarget::Head,
            ReadTarget::Commit(0),
            ReadTarget::Branch(other),
        ] {
            assert!(matches!(
                api.get(target, KEY, Projection::All),
                Err(ApiError::NotFound)
            ));
        }
        let pending = get(api.as_ref(), ReadTarget::Branch(branch));
        assert_eq!(pending.key, KEY);
        assert_eq!(pending.cells.len(), 3);
        assert_eq!(pending.cells[b"#key".as_slice()].as_bytes32(), Some(KEY.0));
        assert!(pending.cells[b"price".as_slice()].is_indexable());
        let selected = api
            .get(
                ReadTarget::Branch(branch),
                KEY,
                Projection::only(["price", "missing", "price"]),
            )
            .unwrap();
        assert_eq!(selected.cells.len(), 1);
        assert_eq!(selected.cells[b"price".as_slice()].as_i32(), Some(50));
        let empty = Projection::Only(vec![]);
        assert!(
            api.get(ReadTarget::Branch(branch), KEY, empty.clone())
                .unwrap()
                .cells
                .is_empty()
        );
        assert!(matches!(
            api.get(ReadTarget::Branch(branch), MISSING, empty),
            Err(ApiError::NotFound)
        ));
        let bindings = api
            .get(
                ReadTarget::Branch(branch),
                RecordKey(system::RECORD_KEYS.key),
                Projection::only([KEY.0]),
            )
            .unwrap();
        assert_eq!(bindings.cells[KEY.0.as_slice()].as_u64(), Some(64));

        api.patch(
            branch,
            KEY,
            PatchInput::new()
                .field("price", CellValue::from_i32(75))
                .unwrap()
                .remove("description")
                .unwrap(),
        )
        .unwrap();
        let edited = get(api.as_ref(), ReadTarget::Branch(branch));
        assert_eq!(edited.cells[b"price".as_slice()].as_i32(), Some(75));
        assert!(!edited.cells[b"price".as_slice()].is_indexable());
        let before = api.branch_info(branch).unwrap();
        api.patch(branch, KEY, PatchInput::new().remove("absent").unwrap())
            .unwrap();
        assert_eq!(api.branch_info(branch).unwrap(), before);
        assert_eq!(api.commit(branch).unwrap(), 1);
        assert_eq!(get(api.as_ref(), ReadTarget::Head), edited);
        assert_eq!(get(api.as_ref(), ReadTarget::Commit(1)), edited);
        assert_eq!(db.info().commit_id, 0); // opening metadata is not a live head
        assert_eq!(db.head().unwrap(), 1);
        assert!(matches!(
            api.get(ReadTarget::Commit(0), KEY, Projection::All),
            Err(ApiError::CommitUnavailable {
                requested: 0,
                head: 1
            })
        ));
        assert!(matches!(api.commit(other), Err(ApiError::Conflict)));

        let deleting = api.begin().unwrap();
        api.delete(deleting, KEY).unwrap();
        assert!(matches!(
            api.get(ReadTarget::Branch(deleting), KEY, Projection::All),
            Err(ApiError::NotFound)
        ));
        assert_eq!(get(api.as_ref(), ReadTarget::Head), edited);
        api.commit(deleting).unwrap();
        assert!(matches!(
            api.get(ReadTarget::Head, KEY, Projection::All),
            Err(ApiError::NotFound)
        ));
        let recreated = api.begin().unwrap();
        api.create(recreated, KEY, input(100)).unwrap();
        assert_eq!(allocator(api.as_ref(), ReadTarget::Branch(recreated)), 66);
        api.commit(recreated).unwrap();
        assert_eq!(
            get(api.as_ref(), ReadTarget::Head).cells[b"price".as_slice()].as_i32(),
            Some(100)
        );
    });
}

#[test]
fn checkpoints_rollback_seal_and_discard_share_state_across_clones() {
    each_backend(|db| {
        let clone = db.clone();
        let branch = db.begin().unwrap();
        clone.create(branch, KEY, input(50)).unwrap();
        let before = get(&db, ReadTarget::Branch(branch));
        db.checkpoint(branch).unwrap();
        clone
            .patch(
                branch,
                KEY,
                PatchInput::new()
                    .attribute("price", CellValue::from_i32(90))
                    .unwrap(),
            )
            .unwrap();
        db.rollback(branch).unwrap();
        assert_eq!(get(&clone, ReadTarget::Branch(branch)), before);
        clone.rollback(branch).unwrap();
        assert!(matches!(
            db.get(ReadTarget::Branch(branch), KEY, Projection::All),
            Err(ApiError::NotFound)
        ));
        assert_eq!(allocator(&db, ReadTarget::Branch(branch)), 64);
        assert!(matches!(
            db.rollback(branch),
            Err(ApiError::NoFrameToRollback)
        ));
        clone.create(branch, KEY, input(50)).unwrap();
        let sealed = db.seal(branch).unwrap();
        assert_eq!(clone.seal(branch).unwrap(), sealed);
        assert_eq!(sealed.commit_id, 1);
        assert_eq!(db.head().unwrap(), 0);
        assert!(clone.branch_info(branch).unwrap().sealed);
        assert!(matches!(
            db.get(ReadTarget::Branch(branch), KEY, Projection::All),
            Err(ApiError::Sealed)
        ));
        assert!(matches!(
            db.patch(branch, KEY, PatchInput::new()),
            Err(ApiError::Sealed)
        ));
        assert!(matches!(db.checkpoint(branch), Err(ApiError::Sealed)));
        assert!(matches!(db.rollback(branch), Err(ApiError::Sealed)));
        assert_eq!(clone.commit(branch).unwrap(), sealed.commit_id);
        assert!(matches!(db.commit(branch), Err(ApiError::HandleInvalid)));
        let discarded = db.begin().unwrap();
        db.delete(discarded, KEY).unwrap();
        db.seal(discarded).unwrap();
        clone.discard(discarded).unwrap();
        assert!(matches!(
            db.branch_info(discarded),
            Err(ApiError::HandleInvalid)
        ));
        assert_eq!(get(&db, ReadTarget::Head), before);
    });
}

#[test]
fn reserved_records_limits_and_failed_mutations_preserve_state() {
    each_backend(|db| {
        let branch = db.begin().unwrap();
        let original = db.branch_info(branch).unwrap();
        for system in system::ALL {
            let key = RecordKey(system.key);
            assert!(db.get(ReadTarget::Head, key, Projection::All).is_ok());
            assert!(matches!(
                db.create(branch, key, input(1)),
                Err(ApiError::Reserved)
            ));
            assert!(matches!(
                db.patch(branch, key, PatchInput::new()),
                Err(ApiError::Reserved)
            ));
            assert!(matches!(db.delete(branch, key), Err(ApiError::Reserved)));
        }
        assert_eq!(db.branch_info(branch).unwrap(), original);
        for bad in [
            RecordInput::new(),
            RecordInput::new()
                .field(&"a".repeat(33), CellValue::from_bool(true))
                .unwrap(),
            RecordInput::new()
                .field("long", CellValue::from_str(&"v".repeat(65)))
                .unwrap(),
            RecordInput::new()
                .field("long", CellValue::from_bytes(&[0; 129]))
                .unwrap(),
        ] {
            assert!(matches!(
                db.create(branch, KEY, bad),
                Err(ApiError::InvalidArgument { .. })
            ));
            assert_eq!(db.branch_info(branch).unwrap(), original);
            assert_eq!(allocator(&db, ReadTarget::Branch(branch)), 64);
        }
        assert!(matches!(
            db.patch(branch, MISSING, PatchInput::new()),
            Err(ApiError::NotFound)
        ));
        assert!(matches!(
            db.delete(branch, MISSING),
            Err(ApiError::NotFound)
        ));
        db.create(branch, KEY, input(50)).unwrap();
        let before = get(&db, ReadTarget::Branch(branch));
        let info = db.branch_info(branch).unwrap();
        for bad in [
            PatchInput::new()
                .remove("price")
                .unwrap()
                .remove("description")
                .unwrap(),
            PatchInput::new()
                .field("aaa", CellValue::from_bool(true))
                .unwrap()
                .field("zzz", CellValue::from_str(&"v".repeat(65)))
                .unwrap(),
        ] {
            assert!(matches!(
                db.patch(branch, KEY, bad),
                Err(ApiError::InvalidArgument { .. })
            ));
            assert_eq!(get(&db, ReadTarget::Branch(branch)), before);
            assert_eq!(db.branch_info(branch).unwrap(), info);
        }
        db.commit(branch).unwrap();
    });
}

#[test]
fn competing_commits_and_cross_thread_clones_use_one_registry() {
    each_backend(|db| {
        let first = db.begin().unwrap();
        let clone = db.clone();
        std::thread::spawn(move || clone.create(first, KEY, input(10)).unwrap())
            .join()
            .unwrap();
        let second = db.begin().unwrap();
        db.create(second, KEY, input(20)).unwrap();
        let gate = Arc::new(Barrier::new(2));
        let results = std::thread::scope(|scope| {
            let run = |branch| {
                gate.wait();
                db.commit(branch)
            };
            let one = scope.spawn(move || run(first));
            let two = scope.spawn(move || run(second));
            (one.join().unwrap(), two.join().unwrap())
        });
        let price = match results {
            (Ok(1), Err(ApiError::Conflict)) => 10,
            (Err(ApiError::Conflict), Ok(1)) => 20,
            other => panic!("unexpected commit results: {other:?}"),
        };
        assert_eq!(
            get(&db, ReadTarget::Head).cells[b"price".as_slice()].as_i32(),
            Some(price)
        );
        assert!(matches!(db.commit(first), Err(ApiError::HandleInvalid)));
        assert!(matches!(db.commit(second), Err(ApiError::HandleInvalid)));
        let survivor = db.clone();
        drop(db);
        assert_eq!(survivor.head().unwrap(), 1);
    });
}

#[test]
fn opening_setup_conversion_and_separate_engines_do_not_share_branch_handles() {
    let cfg = config(HashAlgorithm::Blake3);
    let storage = MemoryDatabase::new();
    let setup = open_backend(storage.clone(), &cfg).unwrap();
    let info = *setup.info();
    let first = setup.into_golem_db().unwrap();
    assert_eq!(first.info(), &info);
    let branch = first.begin().unwrap();
    first.create(branch, KEY, input(50)).unwrap();
    let second = GolemDb::from_backend(storage, &cfg).unwrap();
    assert!(matches!(
        second.branch_info(branch),
        Err(ApiError::HandleInvalid)
    ));
    assert!(matches!(
        second.get(ReadTarget::Head, KEY, Projection::All),
        Err(ApiError::NotFound)
    ));
    first.commit(branch).unwrap();
    assert_eq!(
        get(&first, ReadTarget::Head),
        get(&second, ReadTarget::Head)
    );
}

#[test]
fn durable_facade_reopens_committed_state_and_discards_pending_work() {
    for hash in [HashAlgorithm::Keccak256, HashAlgorithm::Blake3] {
        let cfg = config(hash);
        let dir = tempfile::tempdir().unwrap();
        let (sealed, pending, genesis_id) = {
            let db = GolemDb::open_with_options(dir.path(), &cfg, MdbxOptions::default()).unwrap();
            let branch = db.begin().unwrap();
            db.create(branch, KEY, input(50)).unwrap();
            let sealed = db.seal(branch).unwrap();
            db.commit(branch).unwrap();
            let pending = db.begin().unwrap();
            db.delete(pending, KEY).unwrap();
            (sealed, pending, db.info().genesis_id)
        };
        let db = GolemDb::open_database(
            dir.path(),
            &OpenConfig {
                mode: OpenMode::ExistingOnly,
                ..cfg
            },
        )
        .unwrap();
        assert!(!db.info().created);
        assert_eq!(db.info().genesis_id, genesis_id);
        assert_eq!(db.info().commit_id, sealed.commit_id);
        assert_eq!(db.info().state_root, sealed.state_root);
        assert_eq!(db.info().index_root, sealed.index_root);
        assert!(matches!(
            db.branch_info(pending),
            Err(ApiError::HandleInvalid)
        ));
        assert_eq!(
            get(&db, ReadTarget::Head).cells[b"price".as_slice()].as_i32(),
            Some(50)
        );
        let branch = db.begin().unwrap();
        db.patch(
            branch,
            KEY,
            PatchInput::new()
                .attribute("price", CellValue::from_i32(70))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(db.commit(branch).unwrap(), 2);
    }
}
