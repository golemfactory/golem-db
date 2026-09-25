use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use golemdb_cells::CellType;
use golemdb_index::{BitmapContainer, Index, IndexTerm, PostingChange};
use golemdb_merkle::{HashAlgorithm, RootRef};
use golemdb_storage::{Database, MemoryDatabase, WriteTransaction};

const HASH: HashAlgorithm = HashAlgorithm::Keccak256;

fn codecs(c: &mut Criterion) {
    let mut group = c.benchmark_group("index/container");
    for (name, values) in [
        ("sparse", (0..64).map(|i| i * 997).collect::<Vec<u16>>()),
        ("dense_even", (0..=u16::MAX).step_by(2).collect()),
        ("run", (1000..21000).collect()),
        ("full", (0..=u16::MAX).collect()),
    ] {
        let container = BitmapContainer::from_values(42, values).unwrap();
        let bytes = container.canonical_bytes().unwrap();
        assert_eq!(BitmapContainer::decode(&bytes).unwrap(), container);
        group.bench_function(BenchmarkId::new("canonical_encode", name), |b| {
            b.iter(|| black_box(&container).canonical_bytes().unwrap())
        });
        group.bench_function(BenchmarkId::new("strict_decode", name), |b| {
            b.iter(|| BitmapContainer::decode(black_box(&bytes)).unwrap())
        });
        // Includes canonical encoding, as required by the public leaf_hash API.
        group.bench_function(BenchmarkId::new("leaf_hash_with_encoding", name), |b| {
            b.iter(|| black_box(&container).leaf_hash(&HASH).unwrap())
        });
    }
    group.finish();
    let mut group = c.benchmark_group("index/term");
    for length in [16, 256, 1024] {
        let value = vec![b'x'; length];
        let term = IndexTerm::new("name", CellType::Str, &value).unwrap();
        group.bench_function(BenchmarkId::new("encode_string", length), |b| {
            b.iter(|| IndexTerm::new("name", CellType::Str, black_box(&value)).unwrap())
        });
        group.bench_function(BenchmarkId::new("decode_string", length), |b| {
            b.iter(|| IndexTerm::decode(black_box(term.as_bytes())).unwrap())
        });
        group.bench_function(BenchmarkId::new("routing_hash", length), |b| {
            b.iter(|| black_box(&term).routing_path(&HASH))
        });
    }
    group.finish();
}

fn term(i: u8) -> IndexTerm {
    IndexTerm::new("tag", CellType::Str, &[b'a' + i]).unwrap()
}
fn add(t: u8, hi: u64, lo: u16) -> PostingChange {
    PostingChange::Add {
        term: term(t),
        record_id: (hi << 16) | u64::from(lo),
    }
}

fn seed(db: &impl Database, chunks: u64) -> RootRef<32> {
    let mut tx = db.begin_write().unwrap();
    let changes =
        (0..4).flat_map(|t| (0..chunks).flat_map(move |hi| (0..32).map(move |lo| add(t, hi, lo))));
    let root = Index::new(&HASH)
        .apply(&mut tx, RootRef::Empty, changes)
        .unwrap()
        .root;
    tx.commit().unwrap();
    root
}

fn workloads(chunks: u64) -> Vec<(&'static str, Vec<PostingChange>)> {
    vec![
        ("one_posting", vec![add(0, 0, 128)]),
        (
            "same_chunk_64",
            (128..192).map(|lo| add(0, 0, lo)).collect(),
        ),
        (
            "across_chunks_64",
            (0..64).map(|hi| add(0, hi % chunks, 128)).collect(),
        ),
        (
            "across_terms_64",
            (0..4)
                .flat_map(|t| (0..16).map(move |hi| add(t, hi % chunks, 128)))
                .collect(),
        ),
    ]
}

fn backend(c: &mut Criterion, name: &str, db: &impl Database, chunks: u64) {
    let index = Index::new(&HASH);
    let root = seed(db, chunks);
    let read = db.begin_read().unwrap();
    let selected = term(0);
    let bitmap = index.bitmap(&read, &selected).unwrap().unwrap();
    assert!(bitmap.treemap().contains(0));
    assert!(!bitmap.treemap().contains(128));
    assert_eq!(bitmap.treemap().len(), chunks * 32);
    let mut group = c.benchmark_group(format!("index/{name}/chunks={chunks}"));
    group.bench_function("materialize", |b| {
        b.iter(|| index.bitmap(&read, black_box(&selected)).unwrap())
    });
    let prefix = IndexTerm::prefix("tag", CellType::Str).unwrap();
    group.bench_function("scan_four_terms", |b| {
        b.iter(|| {
            for entry in index
                .terms_with_prefix(&read, black_box(prefix.clone()))
                .unwrap()
            {
                black_box(entry.unwrap());
            }
        })
    });
    // Read transaction lifetime is excluded from these warm-snapshot queries.
    drop(read);
    for (label, changes) in workloads(chunks) {
        let mut check = db.begin_write().unwrap();
        let updated = index.apply(&mut check, root, changes.clone()).unwrap();
        assert_ne!(updated.root, root);
        assert!(!updated.changed_terms.is_empty());
        check.abort();
        group.throughput(Throughput::Elements(changes.len() as u64));
        group.bench_function(BenchmarkId::new("apply_only", label), |b| {
            b.iter_batched_ref(
                || (db.begin_write().unwrap(), changes.clone()),
                |(tx, changes)| {
                    black_box(
                        index
                            .apply(tx, black_box(root), std::mem::take(changes))
                            .unwrap(),
                    )
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

#[cfg(feature = "mdbx")]
fn durable_commit(c: &mut Criterion) {
    let index = Index::new(&HASH);
    c.bench_function("index/mdbx/transaction_begin_apply_durable_commit", |b| {
        // Fresh seeded environment for EVERY iteration. Setup and environment
        // destruction are excluded; begin/apply/commit are all inside timing.
        b.iter_batched_ref(
            || {
                let dir = tempfile::tempdir().unwrap();
                let db = golemdb_storage::MdbxDatabase::open(dir.path()).unwrap();
                let root = seed(&db, 8);
                (db, dir, root, vec![add(0, 0, 128)])
            },
            |(db, _, root, changes)| {
                let mut tx = db.begin_write().unwrap();
                let update = index
                    .apply(&mut tx, black_box(*root), std::mem::take(changes))
                    .unwrap();
                tx.commit().unwrap();
                black_box(update)
            },
            BatchSize::PerIteration,
        );
    });
}

fn index(c: &mut Criterion) {
    codecs(c);
    for chunks in [64, 256] {
        backend(c, "memory", &MemoryDatabase::new(), chunks);
        #[cfg(feature = "mdbx")]
        {
            let dir = tempfile::tempdir().unwrap();
            let db = golemdb_storage::MdbxDatabase::open(dir.path()).unwrap();
            backend(c, "mdbx", &db, chunks);
        }
    }
    #[cfg(feature = "mdbx")]
    durable_commit(c);
}

criterion_group!(benches, index);
criterion_main!(benches);
