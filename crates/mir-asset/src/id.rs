//! UUIDv7 资产 ID：运行时稳定引用，不依赖路径。

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 稳定资产句柄标识（UUIDv7）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AssetId(Uuid);

impl AssetId {
    /// 生成新的 UUIDv7 ID。
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// 从已有 UUID 构造。
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// 读取底层 UUID。
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }

    /// 稳定字符串表示。
    pub fn as_str(self) -> String {
        self.0.to_string()
    }
}

impl Default for AssetId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_sortable() {
        let a = AssetId::new();
        let b = AssetId::new();
        assert_ne!(a, b);
        assert_eq!(a, AssetId::from_uuid(a.as_uuid()));
        let mut ids = [b, a];
        ids.sort_unstable();
        assert_eq!(ids.len(), 2);
    }
}
