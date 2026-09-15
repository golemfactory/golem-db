use criterion::{Criterion, black_box, criterion_group, criterion_main};
use golemdb_merkle::Tree;
use sha2::{Digest, Sha256};

const STATE: u64 = 100_000;
const BLOCK: u64 = 1_000;

/// Uniform keys, as when entity keys are hashes.
fn random(i: u64) -> [u8; 32] {
    Sha256::digest(i.to_be_bytes()).into()
}

/// Worst case for a tree without extension nodes: 24 shared zero bytes.
fn sequential(i: u64) -> [u8; 32] {
    let mut k = [0; 32];
    k[24..].copy_from_slice(&i.to_be_bytes());
    k
}

fn state<const B: usize>(key: fn(u64) -> [u8; 32]) -> Tree<Sha256, B, 32> {
    let mut t = Tree::new();
    (0..STATE).for_each(|i| t.insert(key(i), &i.to_be_bytes()));
    t.root(); // hash once so blocks only pay for their own path
    t
}

/// One block: fork the parent, write `BLOCK` new keys, compute the root, drop.
fn bench_block<const B: usize>(c: &mut Criterion, name: &str, key: fn(u64) -> [u8; 32]) {
    let parent = state::<B>(key);
    let writes: Vec<_> = (STATE..STATE + BLOCK).map(key).collect();
    c.bench_function(&format!("block_{name}/{B}"), |b| {
        b.iter(|| {
            let mut t = parent.clone();
            writes.iter().for_each(|k| t.insert(*k, k));
            black_box(t.root())
        })
    });
}

fn bench_blocks(c: &mut Criterion) {
    bench_block::<2>(c, "random", random);
    bench_block::<4>(c, "random", random);
    bench_block::<16>(c, "random", random);
    bench_block::<256>(c, "random", random);
    bench_block::<16>(c, "sequential", sequential);
}

fn bench_get(c: &mut Criterion) {
    let t = state::<16>(random);
    let k = random(STATE / 2);
    c.bench_function("get_16", |b| b.iter(|| black_box(t.get(black_box(&k)))));
}

criterion_group!(benches, bench_blocks, bench_get);
criterion_main!(benches);
