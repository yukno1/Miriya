//! Actor 外观：稳定 ID、元数据与父子组件。

use serde::{Deserialize, Serialize};

/// 稳定 Actor 标识：保存到场景快照，不随 ECS 内部实体编号变化。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ActorId(u64);

impl ActorId {
    /// 由稳定整数构造（加载场景时由快照恢复）。
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// 原始数值（仅用于调试与快照）。
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Actor 元数据组件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorMeta {
    pub id: ActorId,
    pub name: String,
}

/// 父 Actor 引用组件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parent(pub ActorId);

/// 子 Actor 列表组件（保持插入顺序，便于确定性快照）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Children(pub Vec<ActorId>);
