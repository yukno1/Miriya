//! `PhysicsBackend` 契约与配套数据类型。
//!
//! 本模块只描述后端必须提供的能力；宽相/窄相、约束求解、刚体动力学、
//! 角色控制器、布料与破坏均不在此定义，也不在本 crate 内实现。
//!
//! 所有位置单位为厘米，坐标系为 Z-up、左手系、X 前/Y 右（README §9.2）。

use mir_math::{Quat, Vec3};
use thiserror::Error;

use crate::handle::{BodyId, ColliderId};

/// 引擎驱动的固定步长。后端不自行决定步进节奏。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixedStep {
    seconds: f32,
}

impl FixedStep {
    /// 以秒构造固定步长。调用方需保证 `seconds > 0` 且有限。
    pub const fn from_secs(seconds: f32) -> Self {
        Self { seconds }
    }

    /// 以每秒步数构造（例如 60 Hz）。
    pub fn from_hz(hz: f32) -> Self {
        Self { seconds: 1.0 / hz }
    }

    /// 步长秒数。
    pub const fn as_secs(self) -> f32 {
        self.seconds
    }

    /// 步长是否可用于 `step`。
    pub fn is_valid(self) -> bool {
        self.seconds.is_finite() && self.seconds > 0.0
    }
}

impl Default for FixedStep {
    fn default() -> Self {
        Self {
            seconds: 1.0 / 60.0,
        }
    }
}

/// 刚体类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BodyKind {
    Static,
    Kinematic,
    #[default]
    Dynamic,
}

/// 刚体变换（厘米，Z-up，左手系）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyTransform {
    pub position: Vec3,
    pub rotation: Quat,
}

impl Default for BodyTransform {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
        }
    }
}

/// 创建刚体的描述。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RigidBodyDesc {
    pub kind: BodyKind,
    pub position: Vec3,
    pub rotation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
}

impl Default for RigidBodyDesc {
    fn default() -> Self {
        Self {
            kind: BodyKind::Dynamic,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
        }
    }
}

/// 碰撞体形状（尺寸单位为厘米）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColliderShape {
    Sphere { radius: f32 },
    Box { half_extents: Vec3 },
    Capsule { half_height: f32, radius: f32 },
}

/// 创建碰撞体的描述。相对所属刚体的局部偏移/旋转。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColliderDesc {
    pub shape: ColliderShape,
    pub offset: Vec3,
    pub rotation: Quat,
    /// 传感器只产生重叠事件，不参与碰撞响应。
    pub sensor: bool,
}

impl Default for ColliderDesc {
    fn default() -> Self {
        Self {
            shape: ColliderShape::Sphere { radius: 1.0 },
            offset: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            sensor: false,
        }
    }
}

/// 射线。方向由调用方归一化。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

/// 射线命中。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    pub body: BodyId,
    pub collider: ColliderId,
    /// 沿射线方向的距离（厘米），非负。
    pub distance: f32,
    pub normal: Vec3,
}

/// 扫掠命中。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SweepHit {
    pub body: BodyId,
    pub collider: ColliderId,
    pub distance: f32,
    pub normal: Vec3,
}

/// 重叠命中。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlapHit {
    pub body: BodyId,
    pub collider: ColliderId,
}

/// 接触状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactState {
    Started,
    Ended,
}

/// 接触对事件。由引擎在每帧固定步之后排空。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactPair {
    pub a: BodyId,
    pub b: BodyId,
    pub state: ContactState,
}

/// 调试绘制颜色（RGBA，各通道 0..=255）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebugColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Default for DebugColor {
    fn default() -> Self {
        Self {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        }
    }
}

/// 调试绘制形状。引擎负责收集，渲染层负责呈现；后端不接触 GPU。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DebugShape {
    Line {
        from: Vec3,
        to: Vec3,
        color: DebugColor,
    },
    Box {
        center: Vec3,
        half_extents: Vec3,
        rotation: Quat,
        color: DebugColor,
    },
    Sphere {
        center: Vec3,
        radius: f32,
        color: DebugColor,
    },
}

/// 物理后端错误。
#[derive(Debug, Clone, PartialEq, Error)]
pub enum PhysicsError {
    #[error("physics world is not created")]
    WorldNotCreated,
    #[error("physics world is already created")]
    WorldAlreadyCreated,
    #[error("invalid rigid body handle: {0}")]
    InvalidBody(BodyId),
    #[error("invalid collider handle: {0}")]
    InvalidCollider(ColliderId),
    #[error("invalid fixed step: {0} seconds")]
    InvalidFixedStep(f32),
    #[error("feature unsupported by physics backend: {0}")]
    Unsupported(&'static str),
    #[error("physics backend error: {0}")]
    Backend(String),
}

/// 外部物理后端必须实现的契约。
///
/// 引擎负责固定步累加、句柄表、查询转发、坐标转换（仅在引擎 `interop` 边界）
/// 与调试绘制收集；后端不拥有窗口、渲染或游戏逻辑依赖。
pub trait PhysicsBackend: Send + Sync {
    /// 创建世界。`gravity` 使用引擎坐标约定（厘米/秒²，Z-up）。
    fn create_world(&mut self, gravity: Vec3) -> Result<(), PhysicsError>;
    /// 销毁世界并释放后端资源。
    fn destroy_world(&mut self) -> Result<(), PhysicsError>;

    /// 添加刚体，返回世代句柄。
    fn add_body(&mut self, desc: RigidBodyDesc) -> Result<BodyId, PhysicsError>;
    /// 删除刚体；其碰撞体一并失效。
    fn remove_body(&mut self, body: BodyId) -> Result<(), PhysicsError>;
    /// 写入刚体世界变换。
    fn set_body_transform(
        &mut self,
        body: BodyId,
        transform: BodyTransform,
    ) -> Result<(), PhysicsError>;
    /// 读取刚体世界变换。
    fn body_transform(&self, body: BodyId) -> Result<BodyTransform, PhysicsError>;
    /// 写入线速度与角速度。
    fn set_body_velocity(
        &mut self,
        body: BodyId,
        linear: Vec3,
        angular: Vec3,
    ) -> Result<(), PhysicsError>;

    /// 为刚体添加碰撞体。
    fn add_collider(
        &mut self,
        body: BodyId,
        desc: ColliderDesc,
    ) -> Result<ColliderId, PhysicsError>;
    /// 删除碰撞体。
    fn remove_collider(&mut self, collider: ColliderId) -> Result<(), PhysicsError>;

    /// 按引擎给定的固定步推进一次。后端不自行累加时间。
    fn step(&mut self, step: FixedStep) -> Result<(), PhysicsError>;
    /// 取出自上次调用以来累积的接触事件。
    fn drain_contact_events(&mut self) -> Vec<ContactPair>;

    /// 射线查询；最近命中。
    fn raycast(&self, ray: Ray, max_distance: f32) -> Result<Option<RayHit>, PhysicsError>;
    /// 形状扫掠查询。
    fn sweep(
        &self,
        shape: ColliderShape,
        transform: BodyTransform,
        direction: Vec3,
        max_distance: f32,
    ) -> Result<Option<SweepHit>, PhysicsError>;
    /// 重叠查询。
    fn overlap(
        &self,
        shape: ColliderShape,
        transform: BodyTransform,
    ) -> Result<Vec<OverlapHit>, PhysicsError>;

    /// 收集本帧调试绘制形状。
    fn debug_draw(&self) -> Vec<DebugShape>;

    /// 可选：导出后端内部状态（引擎不解析其内容）。默认不支持。
    fn export_state(&self) -> Result<Option<Vec<u8>>, PhysicsError> {
        Ok(None)
    }
    /// 可选：导入后端内部状态。默认不支持。
    fn import_state(&mut self, _state: &[u8]) -> Result<(), PhysicsError> {
        Err(PhysicsError::Unsupported("state import"))
    }

    /// 后端是否声明确定性。引擎不以此名义做任何承诺。
    fn is_deterministic(&self) -> bool {
        false
    }
}

/// 后端注册入口。外部后端以独立 crate 实现本 trait 并注入引擎。
pub trait PhysicsPlugin: Send + Sync {
    /// 插件名（用于诊断与配置）。
    fn name(&self) -> &'static str;
    /// 创建一个后端实例。
    fn create_backend(&self) -> Box<dyn PhysicsBackend>;
}
