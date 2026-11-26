//! 实体句柄：对 `hecs::Entity` 的 newtype 包装。

/// 世代实体句柄（generation handle）。
///
/// 删除后再次生成实体会复用索引但递增世代，旧句柄不会被误用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity(pub(crate) hecs::Entity);

impl Entity {
    pub(crate) fn new(inner: hecs::Entity) -> Self {
        Self(inner)
    }
}

impl From<hecs::Entity> for Entity {
    #[inline]
    fn from(inner: hecs::Entity) -> Self {
        Self::new(inner)
    }
}

impl From<Entity> for hecs::Entity {
    #[inline]
    fn from(entity: Entity) -> Self {
        entity.0
    }
}
