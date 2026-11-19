//! Miriya 物理后端契约（**不含求解器**）。
//!
//! 本 crate 只定义 [`PhysicsBackend`]、世代句柄、固定步、查询/事件/调试绘制
//! 契约，以及 [`NullPhysicsBackend`] 与可复用的 [`assert_backend_contract`]。
//! 宽相/窄相、约束求解、刚体动力学、角色控制器、布料与破坏均不在本仓库实现；
//! 外部后端以 [`PhysicsPlugin`] 注册（ADR 0008 / README §9.9）。
//!
//! 坐标与单位沿用引擎约定：厘米、Z-up、左手系、X 前/Y 右。后端约定不同时，
//! 转换只允许发生在引擎自己的 `interop` 边界，不得泄漏到公共 API。
//!
//! ```
//! use mir_physics::{assert_backend_contract, NullPhysicsBackend};
//!
//! let mut backend = NullPhysicsBackend::new();
//! assert_backend_contract(&mut backend);
//! ```

#![forbid(unsafe_code)]

pub mod backend;
pub mod contract;
pub mod handle;
pub mod null;

pub use backend::{
    BodyKind, BodyTransform, ColliderDesc, ColliderShape, ContactPair, ContactState, DebugColor,
    DebugShape, FixedStep, OverlapHit, PhysicsBackend, PhysicsError, PhysicsPlugin, Ray, RayHit,
    RigidBodyDesc, SweepHit,
};
pub use contract::assert_backend_contract;
pub use handle::{BodyId, ColliderId, Generation};
pub use null::NullPhysicsBackend;
