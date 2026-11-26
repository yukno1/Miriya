//! 场景子系统：Actor、Level、层级变换与场景快照。
//!
//! `Transform` 归 `mir-math` 所有（M3），此处重导出以保持旧导入路径可用。

mod actor;
mod camera;
mod error;
mod hierarchy;
mod scene;
mod serialize;

pub use actor::{ActorId, ActorMeta, Children, Parent};
pub use camera::{Camera, CameraController, CameraInput};
pub use error::{Result, SceneError};
pub use hierarchy::{WorldTransforms, compute_world_transforms, would_create_cycle};
pub use mir_math::Transform;
pub use scene::{Level, Scene};
pub use serialize::{ActorSnapshot, SCENE_SCHEMA_HASH, SceneSnapshot, decode_scene, encode_scene};
