//! 场景子系统领域错误。

use thiserror::Error;

use crate::actor::ActorId;

/// `mir-scene` 对外暴露的可恢复错误。
#[derive(Debug, Error)]
pub enum SceneError {
    /// 请求的 Actor 不存在。
    #[error("Actor {0:?} 不存在")]
    MissingActor(ActorId),
    /// ECS 底层访问失败。
    #[error("ECS 访问失败：{0}")]
    Ecs(#[from] mir_ecs::EcsError),
    /// 父子关系形成环。
    #[error("Actor {0:?} 的父子关系形成循环")]
    Cycle(ActorId),
    /// 场景数据不一致或输入损坏。
    #[error("场景数据损坏：{0}")]
    Corrupt(String),
    /// 快照版本不匹配。
    #[error("场景快照版本不匹配：期望 {expected}，实际 {actual}")]
    VersionMismatch { expected: u16, actual: u16 },
    /// 快照编解码失败。
    #[error("场景快照编解码失败：{0}")]
    Serialize(#[from] mir_serialize::SerializeError),
}

/// `mir-scene` 便捷结果类型。
pub type Result<T> = std::result::Result<T, SceneError>;
