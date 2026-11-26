//! 场景快照：只保存稳定 ActorId、父子关系、局部变换与显式元数据。

use serde::{Deserialize, Serialize};

use mir_math::{Transform, Vec3};

use crate::actor::ActorId;
use crate::error::Result;
use crate::scene::Scene;

const fn fnv1a(input: &str) -> u32 {
    let bytes = input.as_bytes();
    let mut hash = 0x811c9dc5_u32;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u32;
        hash = hash.wrapping_mul(0x0100_0193);
        index += 1;
    }
    hash
}

/// `SceneSnapshot` 字段布局的确定性哈希；字段变化时需更新版本字符串。
pub const SCENE_SCHEMA_HASH: u32 = fnv1a("mir-scene::SceneSnapshot@v1");

/// 单个 Actor 的快照记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActorSnapshot {
    pub id: ActorId,
    pub name: String,
    pub parent: Option<ActorId>,
    pub transform: Transform,
}

/// 场景快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneSnapshot {
    pub name: String,
    pub ambient: [f32; 3],
    pub actors: Vec<ActorSnapshot>,
}

impl SceneSnapshot {
    /// 从运行时场景提取快照（ActorId 有序）。
    pub fn from_scene(scene: &Scene) -> Self {
        let actors = scene
            .level
            .actors()
            .filter_map(|id| {
                let meta = scene.level.meta(id)?;
                let transform = scene.level.transform(id)?;
                Some(ActorSnapshot {
                    id,
                    name: meta.name.clone(),
                    parent: scene.level.parent(id),
                    transform: *transform,
                })
            })
            .collect();
        let ambient = scene.ambient();
        Self {
            name: scene.name.clone(),
            ambient: [ambient.x, ambient.y, ambient.z],
            actors,
        }
    }

    /// 由快照重建场景；ECS 内部实体编号不参与保存。
    pub fn to_scene(&self) -> Result<Scene> {
        let mut scene = Scene::new(self.name.clone());
        scene.set_ambient(Vec3::new(self.ambient[0], self.ambient[1], self.ambient[2]));
        for actor in &self.actors {
            scene
                .level
                .spawn_actor_with_id(actor.id, actor.name.clone(), actor.transform)?;
        }
        for actor in &self.actors {
            if let Some(parent) = actor.parent {
                scene.level.set_parent(actor.id, parent)?;
            }
        }
        Ok(scene)
    }
}

/// 把场景编码为版本化字节流。
pub fn encode_scene(scene: &Scene) -> Result<Vec<u8>> {
    let snapshot = SceneSnapshot::from_scene(scene);
    Ok(mir_serialize::encode(&snapshot, SCENE_SCHEMA_HASH)?)
}

/// 从版本化字节流加载场景。
pub fn decode_scene(bytes: &[u8]) -> Result<Scene> {
    let snapshot: SceneSnapshot = mir_serialize::decode(bytes, SCENE_SCHEMA_HASH)?;
    snapshot.to_scene()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SceneError;

    #[test]
    fn schema_hash_is_stable() {
        assert_eq!(SCENE_SCHEMA_HASH, fnv1a("mir-scene::SceneSnapshot@v1"));
    }

    #[test]
    fn version_mismatch_surfaces_as_scene_error() {
        let bytes = mir_serialize::encode_with_version(
            &SceneSnapshot {
                name: "x".to_string(),
                ambient: [1.0, 1.0, 1.0],
                actors: Vec::new(),
            },
            SCENE_SCHEMA_HASH,
            999,
        )
        .unwrap();
        assert!(matches!(
            decode_scene(&bytes),
            Err(SceneError::Serialize(
                mir_serialize::SerializeError::VersionMismatch { .. }
            ))
        ));
    }
}
