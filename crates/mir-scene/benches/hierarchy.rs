//! M3 Gate：10k Actor 世界矩阵计算基准（扁平 + 纵深）。

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use mir_math::{Transform, Vec3};
use mir_scene::{ActorId, Level};

const ACTOR_COUNT: usize = 10_000;

fn flat_level() -> Level {
    let mut level = Level::new("flat");
    for i in 0..ACTOR_COUNT {
        level
            .spawn_actor_with_transform(
                format!("actor-{i}"),
                Transform::from_position(Vec3::new(i as f32, 0.0, 0.0)),
            )
            .expect("初始化扁平基准场景失败");
    }
    level
}

fn deep_level() -> Level {
    let mut level = Level::new("deep");
    let mut previous: Option<ActorId> = None;
    for i in 0..ACTOR_COUNT {
        let actor = level
            .spawn_actor_with_transform(
                format!("actor-{i}"),
                Transform::from_position(Vec3::new(1.0, 0.0, 0.0)),
            )
            .expect("初始化纵深基准场景失败");
        if let Some(parent) = previous {
            level
                .set_parent(actor, parent)
                .expect("设置基准父子关系失败");
        }
        previous = Some(actor);
    }
    level
}

fn hierarchy_benchmark(c: &mut Criterion) {
    let flat = flat_level();
    let deep = deep_level();

    c.bench_function("world_transforms_flat_10k", |b| {
        b.iter(|| black_box(flat.world_transforms().expect("扁平层级计算失败")));
    });
    c.bench_function("world_transforms_deep_10k", |b| {
        b.iter(|| black_box(deep.world_transforms().expect("纵深层级计算失败")));
    });
}

criterion_group!(benches, hierarchy_benchmark);
criterion_main!(benches);
