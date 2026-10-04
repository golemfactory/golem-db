//! Store files: the node-local YAML that selects and tunes a store.

use golemdb_api::*;

#[test]
fn memory_store_file() {
    assert_eq!(
        StoreConfig::from_yaml("memory").unwrap(),
        StoreConfig::Memory
    );
}

#[test]
fn unknown_stores_are_rejected_as_store_yaml_errors() {
    let error = StoreConfig::from_yaml("rocksdb:\n  path: ./data\n").unwrap_err();
    assert!(matches!(error, OpenError::StoreYaml(_)));
    // The message names the store file, not the genesis file.
    assert!(error.to_string().starts_with("invalid store YAML"));
}

#[cfg(not(feature = "mdbx"))]
#[test]
fn mdbx_without_the_feature_is_an_invalid_config() {
    assert!(matches!(
        StoreConfig::from_yaml("mdbx:\n  path: ./data\n"),
        Err(OpenError::InvalidConfig(_))
    ));
}

#[cfg(feature = "mdbx")]
mod mdbx {
    use super::*;

    const GIB: usize = 1 << 30;

    fn mdbx(config: StoreConfig) -> (std::path::PathBuf, MdbxOptions) {
        match config {
            StoreConfig::Mdbx { path, options } => (path, options),
            other => panic!("expected MDBX, got {other:?}"),
        }
    }

    #[test]
    fn a_path_alone_uses_default_options() {
        let (path, options) = mdbx(StoreConfig::from_yaml("mdbx:\n  path: ./data\n").unwrap());
        assert_eq!(path, std::path::Path::new("./data"));
        assert_eq!(options, MdbxOptions::default());
    }

    #[test]
    fn options_override_only_what_is_given() {
        let yaml = "mdbx:\n  path: /var/lib/golemdb\n  options:\n    max_map_size: 68719476736\n";
        let (path, options) = mdbx(StoreConfig::from_yaml(yaml).unwrap());
        assert_eq!(path, std::path::Path::new("/var/lib/golemdb"));
        assert_eq!(options.max_map_size, 64 * GIB);
        assert_eq!(options.growth_step, MdbxOptions::default().growth_step);
        assert_eq!(options.max_tables, MdbxOptions::default().max_tables);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        for yaml in [
            "mdbx:\n  path: ./data\n  options:\n    max_map_sise: 1\n",
            "mdbx:\n  path: ./data\n  mode: existing\n",
            "mdbx:\n  options:\n    max_map_size: 1\n",
        ] {
            assert!(
                matches!(StoreConfig::from_yaml(yaml), Err(OpenError::StoreYaml(_))),
                "accepted: {yaml}"
            );
        }
    }

    #[test]
    fn load_resolves_relative_paths_against_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("store.yaml");
        std::fs::write(&file, "mdbx:\n  path: data\n").unwrap();
        assert_eq!(
            mdbx(StoreConfig::load(&file).unwrap()).0,
            dir.path().join("data")
        );

        std::fs::write(&file, "mdbx:\n  path: /srv/golemdb\n").unwrap();
        assert_eq!(
            mdbx(StoreConfig::load(&file).unwrap()).0,
            std::path::Path::new("/srv/golemdb")
        );

        // Without a file there is nothing to resolve against.
        assert_eq!(
            mdbx(StoreConfig::from_yaml("mdbx:\n  path: data\n").unwrap()).0,
            std::path::Path::new("data")
        );
    }

    #[test]
    fn a_loaded_store_file_opens_a_database() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("store.yaml");
        std::fs::write(
            &file,
            "mdbx:\n  path: data\n  options:\n    max_map_size: 67108864\n",
        )
        .unwrap();
        let store = StoreConfig::load(&file).unwrap();
        let db = Database::open(store, &Config::new(Genesis::DEV)).unwrap();
        assert_eq!(db.head().unwrap(), 0);
        assert!(dir.path().join("data").join("mdbx.dat").exists());
    }
}
