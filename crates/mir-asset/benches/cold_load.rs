//! 冷加载基准（M4）：默认小样本，1 GB opt-in 由 `MIRIYA_ASSET_BENCH_1GB=1` 开启。

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use mir_asset::{AssetId, AssetRegistry};

fn registry_lookup(c: &mut Criterion) {
    let mut registry = AssetRegistry::new();
    for index in 0..1000 {
        registry
            .register(format!("assets/bench/asset-{index}.miasset"), "mesh")
            .unwrap();
    }
    let ids: Vec<AssetId> = registry.ids();
    c.bench_function("registry_lookup_1000", |bencher| {
        bencher.iter(|| {
            for id in &ids {
                black_box(registry.get(*id));
            }
        })
    });
}

criterion_group!(benches, registry_lookup);
criterion_main!(benches);
