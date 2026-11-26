//! ECS 领域错误。

use thiserror::Error;

use crate::Entity;

/// `mir-ecs` 对外暴露的可恢复错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EcsError {
    /// 实体不存在，或句柄世代已失效。
    #[error("实体 {0:?} 不存在或已失效")]
    NoSuchEntity(Entity),
    /// 实体存在但没有请求的组件。
    #[error("实体 {entity:?} 缺少组件 {component}")]
    MissingComponent {
        entity: Entity,
        component: &'static str,
    },
    /// hecs 内核返回的组件访问错误（字符串化，避免泄漏第三方类型）。
    #[error("组件访问失败：{0}")]
    Component(String),
    /// 同名资源重复注册。
    #[error("资源 {0} 已存在")]
    ResourceAlreadyExists(&'static str),
    /// 请求的资源未注册。
    #[error("资源 {0} 不存在")]
    MissingResource(&'static str),
}

/// `mir-ecs` 便捷结果类型。
pub type Result<T> = std::result::Result<T, EcsError>;
