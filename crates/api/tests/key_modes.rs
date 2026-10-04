//! Key modes: caller-assigned or generated keys, fixed in genesis.

use golemdb_api::*;
use golemdb_merkle::{Blake3Hasher, HashProvider, Keccak256Hasher};
use golemdb_storage::MemoryStore;

const SEED: [u8; 32] = [0x5e; 32];

fn generated(seed: [u8; 32], hash_function: HashAlgorithm) -> Genesis {
    Genesis {
        hash_function,
        record_keys: RecordKeys::Generated { seed },
        ..Genesis::DEV
    }
}

/// The documented derivation: `H("golemdb/record-key/v1" ‖ seed ‖ id)`.
fn expected_key(hasher: &impl HashProvider, seed: [u8; 32], id: u64) -> RecordKey {
    RecordKey(hasher.hash_parts(&[b"golemdb/record-key/v1", &seed, &id.to_be_bytes()]))
}

fn create_generated(db: &Database, branch: BranchId, price: i32) -> RecordKey {
    db.create(branch, RecordOp::create().field("price", price))
        .into_result()
        .unwrap()
}

#[test]
fn generated_keys_follow_the_derivation_for_both_hashes() {
    for (hash_function, keys) in [
        (
            HashAlgorithm::Keccak256,
            [64, 65].map(|id| expected_key(&Keccak256Hasher, SEED, id)),
        ),
        (
            HashAlgorithm::Blake3,
            [64, 65].map(|id| expected_key(&Blake3Hasher, SEED, id)),
        ),
    ] {
        let db = Database::open_memory(&generated(SEED, hash_function)).unwrap();
        let branch = db.begin().unwrap();
        assert_eq!(create_generated(&db, branch, 1), keys[0]);
        assert_eq!(create_generated(&db, branch, 2), keys[1]);
        let record = db
            .get(ReadTarget::Branch(branch), RecordOp::get(keys[1]))
            .into_result()
            .unwrap();
        assert_eq!(record.cells[b"price".as_slice()].as_i32(), Some(2));
        assert_eq!(
            record.cells[b"#key".as_slice()].as_bytes32(),
            Some(keys[1].0)
        );
    }
}

#[test]
fn mismatched_creates_fail_and_write_nothing() {
    // Generated mode: naming a key is a mismatch.
    let db = Database::open_memory(&generated(SEED, HashAlgorithm::Keccak256)).unwrap();
    let branch = db.begin().unwrap();
    let named = db.create(
        branch,
        RecordOp::create()
            .key(RecordKey([1; 32]))
            .field("price", 1i32),
    );
    assert_eq!(named.receipt.details, Details::default());
    assert!(matches!(
        named.into_result(),
        Err(ApiError::KeyModeMismatch)
    ));
    // Nothing was allocated: the next generated key is still record 64's.
    assert_eq!(
        create_generated(&db, branch, 1),
        expected_key(&Keccak256Hasher, SEED, 64)
    );

    // Caller-assigned mode: omitting the key is a mismatch.
    let db = Database::open_memory(&Genesis::DEV).unwrap();
    let branch = db.begin().unwrap();
    assert!(matches!(
        db.create(branch, RecordOp::create().field("price", 1i32))
            .into_result(),
        Err(ApiError::KeyModeMismatch)
    ));
    let key = RecordKey([1; 32]);
    db.create(branch, RecordOp::create().key(key))
        .into_result()
        .unwrap();
    assert!(matches!(
        db.create(branch, RecordOp::create().key(key)).into_result(),
        Err(ApiError::AlreadyExists)
    ));
}

#[test]
fn keys_are_deterministic_per_seed_and_differ_between_seeds() {
    let keys = |seed| {
        let db = Database::open_memory(&generated(seed, HashAlgorithm::Keccak256)).unwrap();
        let branch = db.begin().unwrap();
        [1, 2, 3].map(|price| create_generated(&db, branch, price))
    };
    assert_eq!(keys(SEED), keys(SEED));
    assert_ne!(keys(SEED), keys([0x5f; 32]));
}

#[test]
fn genesis_records_the_mode_and_seed_in_params() {
    let params = |genesis: &Genesis| {
        let db = Database::open_memory(genesis).unwrap();
        db.get(
            ReadTarget::Head,
            RecordOp::get(RecordKey(golemdb_cells::system::PARAMS.key))
                .only(["#keyMode", "#keySeed"]),
        )
        .into_result()
        .unwrap()
        .cells
    };
    let caller = params(&Genesis::DEV);
    assert_eq!(caller[b"#keyMode".as_slice()].as_u32(), Some(0));
    assert!(!caller.contains_key(b"#keySeed".as_slice()));
    let generated = params(&generated(SEED, HashAlgorithm::Keccak256));
    assert_eq!(generated[b"#keyMode".as_slice()].as_u32(), Some(1));
    assert_eq!(generated[b"#keySeed".as_slice()].as_bytes32(), Some(SEED));
}

#[test]
fn mode_and_seed_are_part_of_the_genesis_identity() {
    let store = MemoryStore::new();
    let original = generated(SEED, HashAlgorithm::Keccak256);
    drop(Database::from_store(store.clone(), &Config::new(original)).unwrap());
    for different in [
        Genesis::DEV,
        generated([0x5f; 32], HashAlgorithm::Keccak256),
    ] {
        assert!(matches!(
            Database::from_store(store.clone(), &Config::new(different)),
            Err(OpenError::GenesisMismatch)
        ));
    }
    assert!(Database::from_store(store, &Config::new(original)).is_ok());
}

#[test]
fn yaml_requires_the_mode_and_a_well_formed_seed() {
    let limits = "hash_function: keccak-256\ncell_limits:\n  max_cell_name_len: 32\n  max_str_len: 64\n  max_bytes_len: 128\n";
    let seed = format!("0x{}", "5e".repeat(32));
    assert_eq!(
        Genesis::from_yaml(&format!("{limits}record_keys: caller_assigned\n"))
            .unwrap()
            .record_keys,
        RecordKeys::CallerAssigned
    );
    assert_eq!(
        Genesis::from_yaml(&format!(
            "{limits}record_keys:\n  generated:\n    seed: \"{seed}\"\n"
        ))
        .unwrap()
        .record_keys,
        RecordKeys::Generated { seed: SEED }
    );
    for bad in [
        String::new(), // the mode is required
        "record_keys: generated\n".into(),
        "record_keys: random\n".into(),
        "record_keys:\n  generated:\n    seed: \"5e5e\"\n".into(),
        format!(
            "record_keys:\n  generated:\n    seed: \"{}\"\n",
            "5e".repeat(32)
        ),
        format!(
            "record_keys:\n  generated:\n    seed: \"0x{}\"\n",
            "zz".repeat(32)
        ),
        format!("record_keys:\n  generated:\n    seed: \"{seed}\"\n    extra: 1\n"),
    ] {
        assert!(
            matches!(
                Genesis::from_yaml(&format!("{limits}{bad}")),
                Err(OpenError::Yaml(_))
            ),
            "accepted: {bad}"
        );
    }
}
