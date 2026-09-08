use criterion::{Criterion, black_box, criterion_group, criterion_main};
use golemdb_cells::{CellType, CellValue};

fn bench_encode(c: &mut Criterion) {
    let cell = CellValue::new(CellType::Str, b"hello, golemdb", false).unwrap();
    c.bench_function("cell_encode_str", |b| b.iter(|| black_box(cell).encode()));
}

fn bench_parse(c: &mut Criterion) {
    let bytes = CellValue::new(CellType::Str, b"hello, golemdb", false)
        .unwrap()
        .encode();
    c.bench_function("cell_parse_str", |b| {
        b.iter(|| CellValue::parse(black_box(&bytes)).unwrap())
    });
}

criterion_group!(benches, bench_encode, bench_parse);
criterion_main!(benches);
