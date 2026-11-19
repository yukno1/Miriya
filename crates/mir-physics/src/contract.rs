//! 可复用的物理后端契约自检。
//!
//! 后端作者可在自己的 `#[test]` 中调用 [`assert_backend_contract`]，验证
//! 生命周期、句柄有效性、变换读写、步进与查询等基本约定；本函数故意使用
//! `assert!`/`expect` 以便测试直接失败。

use mir_math::{Quat, Vec3};

use crate::backend::{
    BodyTransform, ColliderDesc, ColliderShape, FixedStep, PhysicsBackend, Ray, RigidBodyDesc,
};
use crate::handle::Generation;

/// 对任意后端运行最小契约检查。
///
/// 不检查后端特有的数值行为，也不承诺物理确定性；只检查所有实现都必须满足的接口约定。
pub fn assert_backend_contract<B: PhysicsBackend>(backend: &mut B) {
    backend
        .create_world(Vec3::new(0.0, 0.0, -980.0))
        .expect("create_world must succeed");
    assert!(
        backend.create_world(Vec3::ZERO).is_err(),
        "creating a world twice must fail"
    );

    let a = backend
        .add_body(RigidBodyDesc {
            position: Vec3::new(10.0, 0.0, 0.0),
            ..Default::default()
        })
        .expect("add body a");
    let b = backend
        .add_body(RigidBodyDesc::default())
        .expect("add body b");
    assert_ne!(a, b, "body handles must be distinct");
    assert_eq!(a.generation(), Generation::ZERO);

    let transform = backend
        .body_transform(a)
        .expect("body transform must be readable");
    assert_eq!(transform.position, Vec3::new(10.0, 0.0, 0.0));
    assert_eq!(transform.rotation, Quat::IDENTITY);

    let moved = BodyTransform {
        position: Vec3::new(1.0, 2.0, 3.0),
        rotation: Quat::IDENTITY,
    };
    backend
        .set_body_transform(a, moved)
        .expect("set_body_transform");
    assert_eq!(backend.body_transform(a).expect("read moved"), moved);

    let collider = backend
        .add_collider(
            a,
            ColliderDesc {
                shape: ColliderShape::Sphere { radius: 50.0 },
                ..Default::default()
            },
        )
        .expect("add collider");
    assert!(backend.remove_collider(collider).is_ok());
    assert!(
        backend.remove_collider(collider).is_err(),
        "removing a collider twice must fail"
    );

    let step = FixedStep::default();
    assert!(step.is_valid(), "default fixed step must be valid");
    for _ in 0..60 {
        backend.step(step).expect("step");
    }
    let _ = backend.drain_contact_events();

    if let Some(hit) = backend
        .raycast(
            Ray {
                origin: Vec3::ZERO,
                direction: Vec3::Z,
            },
            1000.0,
        )
        .expect("raycast")
    {
        assert!(hit.distance >= 0.0, "ray hit distance must be non-negative");
    }
    let _ = backend
        .sweep(
            ColliderShape::Sphere { radius: 1.0 },
            BodyTransform::default(),
            Vec3::X,
            10.0,
        )
        .expect("sweep");
    let _ = backend
        .overlap(
            ColliderShape::Sphere { radius: 1.0 },
            BodyTransform::default(),
        )
        .expect("overlap");
    let _ = backend.debug_draw();
    let _ = backend.export_state();
    let _ = backend.is_deterministic();

    assert!(backend.remove_body(a).is_ok());
    assert!(
        backend.body_transform(a).is_err(),
        "removed body handle must become invalid"
    );

    backend.destroy_world().expect("destroy_world");
    assert!(
        backend.step(step).is_err(),
        "step after destroy_world must fail"
    );
}
