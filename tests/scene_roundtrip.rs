//! M3 Gate：场景保存 → 加载 → 再保存 golden 与错误输入回归。

use std::f32::consts::FRAC_PI_2;

use mir_math::{Quat, Transform, Vec3};
use mir_scene::{
    ActorId, ActorSnapshot, SCENE_SCHEMA_HASH, Scene, SceneError, SceneSnapshot, decode_scene,
    encode_scene,
};

fn build_scene() -> (Scene, ActorId) {
    let mut scene = Scene::new("roundtrip");
    scene.set_ambient(Vec3::new(0.8, 0.9, 1.0));

    let root = scene
        .level
        .spawn_actor_with_transform("root", Transform::from_position(Vec3::new(100.0, 0.0, 0.0)))
        .unwrap();
    let child = scene
        .level
        .spawn_actor_with_transform(
            "child",
            Transform::new(
                Vec3::new(200.0, 0.0, 0.0),
                Quat::from_rotation_z(FRAC_PI_2),
                Vec3::splat(2.0),
            ),
        )
        .unwrap();
    scene.level.set_parent(child, root).unwrap();
    (scene, child)
}

#[test]
fn save_load_save_is_golden_and_preserves_hierarchy() {
    let (scene, child) = build_scene();
    let bytes = encode_scene(&scene).expect("保存场景失败");
    let loaded = decode_scene(&bytes).expect("加载场景失败");
    let bytes_again = encode_scene(&loaded).expect("再次保存场景失败");

    assert_eq!(bytes, bytes_again, "保存→加载→再保存必须逐字节一致");
    assert_eq!(
        SceneSnapshot::from_scene(&scene),
        SceneSnapshot::from_scene(&loaded)
    );

    let original = scene
        .level
        .world_transforms()
        .unwrap()
        .get(child)
        .unwrap()
        .transform_point(Vec3::ZERO);
    let restored = loaded
        .level
        .world_transforms()
        .unwrap()
        .get(child)
        .unwrap()
        .transform_point(Vec3::ZERO);
    for (a, b) in original.to_array().iter().zip(restored.to_array()) {
        assert!(
            (*a - b).abs() < 1e-4,
            "层级世界变换未保持: {original:?} vs {restored:?}"
        );
    }
}

#[test]
fn truncated_snapshot_returns_readable_error() {
    let (scene, _) = build_scene();
    let bytes = encode_scene(&scene).unwrap();
    let truncated = &bytes[..bytes.len().saturating_sub(3)];
    let error = match decode_scene(truncated) {
        Ok(_) => panic!("截断数据必须失败"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(message.contains("快照"), "错误应可诊断: {message}");
}

#[test]
fn version_mismatch_is_rejected() {
    let snapshot = SceneSnapshot::from_scene(&build_scene().0);
    let bytes = mir_serialize::encode_with_version(&snapshot, SCENE_SCHEMA_HASH, 999).unwrap();
    assert!(matches!(
        decode_scene(&bytes),
        Err(SceneError::Serialize(
            mir_serialize::SerializeError::VersionMismatch { .. }
        ))
    ));
}

#[test]
fn cyclic_parent_snapshot_is_rejected() {
    let snapshot = SceneSnapshot {
        name: "cycle".to_string(),
        ambient: [1.0; 3],
        actors: vec![
            ActorSnapshot {
                id: ActorId::new(1),
                name: "a".to_string(),
                parent: Some(ActorId::new(2)),
                transform: Transform::default(),
            },
            ActorSnapshot {
                id: ActorId::new(2),
                name: "b".to_string(),
                parent: Some(ActorId::new(1)),
                transform: Transform::default(),
            },
        ],
    };
    assert!(matches!(snapshot.to_scene(), Err(SceneError::Cycle(_))));
}
