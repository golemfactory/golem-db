use golemdb_merkle::{HashAlgorithm, HashConfig, HashProvider};

fn hex(hash: [u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn keccak_known_answers_and_chunking() {
    let hash = HashAlgorithm::Keccak256;
    // Standard Keccak-256 vectors. SHA3-256 has different padding and fails these.
    assert_eq!(
        hex(hash.hash(b"")),
        "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
    );
    assert_eq!(
        hex(hash.hash(b"abc")),
        "4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45"
    );
    assert_eq!(hash.hash_parts(&[b"a", b"", b"bc"]), hash.hash(b"abc"));
    assert_eq!(hash.hash_parts(&[]), hash.hash(b""));
}

#[test]
fn explicit_yaml_and_file_loading() {
    let config = HashConfig::from_yaml("# protocol\nhash_function: keccak-256\n").unwrap();
    assert_eq!(config.hash_function, HashAlgorithm::Keccak256);
    let from_file = HashConfig::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/hash.yaml"
    ))
    .unwrap();
    assert_eq!(from_file, config);
    assert!(HashConfig::load(concat!(env!("CARGO_MANIFEST_DIR"), "/no-such-config.yaml")).is_err());
}

#[test]
fn configuration_never_silently_falls_back() {
    for yaml in [
        "",
        "{}",
        "hash_function: sha3-256",
        "hash_function: unknown",
        "hash_function: null",
        "hash_function: 1",
        "hash_functon: keccak-256",
        "hash_function: keccak-256\nextra: true",
        "hash_function: [",
        "hash_function: keccak-256\nhash_function: keccak-256",
        "hash_function: keccak-256\n---\nhash_function: keccak-256",
    ] {
        assert!(
            HashConfig::from_yaml(yaml).is_err(),
            "unexpectedly accepted {yaml:?}"
        );
    }
}
