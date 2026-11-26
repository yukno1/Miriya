//! `hecs::World` 的门面封装：稳定 API、世代句柄与类型化资源。

use std::ops::{Deref, DerefMut};

use hecs::Ref as HecsRef;
use hecs::RefMut as HecsRefMut;

use crate::change::{ChangeKind, ChangeTracker};
use crate::entity::Entity;
use crate::error::{EcsError, Result};
use crate::resource::{Resource, Resources};

/// 可挂载到实体上的组件类型（由 hecs 提供实际约束）。
pub trait Component: hecs::Component {}
impl<T: hecs::Component> Component for T {}

/// 共享组件访问守卫；隐藏 hecs 的 `Ref` 类型。
pub struct Ref<'a, T: Component>(HecsRef<'a, T>);

impl<T: Component> Deref for Ref<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// 可变组件访问守卫；隐藏 hecs 的 `RefMut` 类型。
pub struct Mut<'a, T: Component>(HecsRefMut<'a, T>);

impl<T: Component> Deref for Mut<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Component> DerefMut for Mut<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// 实体与组件的容器，同时持有类型化资源和变更追踪。
pub struct World {
    inner: hecs::World,
    resources: Resources,
    changes: ChangeTracker,
}

impl Default for World {
    fn default() -> Self {
        Self {
            inner: hecs::World::new(),
            resources: Resources::default(),
            changes: ChangeTracker::default(),
        }
    }
}

impl World {
    /// 创建空世界。
    pub fn new() -> Self {
        Self::default()
    }

    /// 生成实体；hecs 会为新实体分配新的世代。
    pub fn spawn(&mut self) -> Entity {
        Entity::new(self.inner.spawn(()))
    }

    /// 销毁实体及其全部组件；失效句柄后续访问会返回错误。
    pub fn despawn(&mut self, entity: Entity) -> Result<()> {
        self.inner
            .despawn(entity.0)
            .map_err(|_| EcsError::NoSuchEntity(entity))?;
        self.changes.remove(entity);
        Ok(())
    }

    /// 当前存活实体列表。
    pub fn entities(&self) -> Vec<Entity> {
        self.inner
            .iter()
            .map(|entry| Entity::new(entry.entity()))
            .collect()
    }

    /// 存活实体数量。
    pub fn len(&self) -> usize {
        self.inner.len() as usize
    }

    /// 世界是否为空。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 为实体挂载单个组件。
    pub fn insert<C: Component>(&mut self, entity: Entity, component: C) -> Result<()> {
        self.inner
            .insert_one(entity.0, component)
            .map_err(|error| EcsError::Component(error.to_string()))?;
        self.changes.mark_added::<C>(entity);
        Ok(())
    }

    /// 移除并返回实体组件。
    pub fn remove<C: Component>(&mut self, entity: Entity) -> Result<C> {
        let (component,) = self
            .inner
            .remove::<(C,)>(entity.0)
            .map_err(|error| EcsError::Component(error.to_string()))?;
        Ok(component)
    }

    /// 读取实体组件；缺失时返回 `None`。
    pub fn get<C: Component>(&self, entity: Entity) -> Option<Ref<'_, C>> {
        self.inner.get::<&C>(entity.0).map(Ref).ok()
    }

    /// 读取实体组件；缺失或句柄失效时返回 [`EcsError`]。
    pub fn try_get<C: Component>(&self, entity: Entity) -> Result<Ref<'_, C>> {
        self.inner.get::<&C>(entity.0).map(Ref).map_err(|_error| {
            if self.is_alive(entity) {
                EcsError::MissingComponent {
                    entity,
                    component: std::any::type_name::<C>(),
                }
            } else {
                EcsError::NoSuchEntity(entity)
            }
        })
    }

    /// 可变读取实体组件；访问会被记录为一次变更。
    pub fn get_mut<C: Component>(&mut self, entity: Entity) -> Option<Mut<'_, C>> {
        match self.inner.get::<&mut C>(entity.0) {
            Ok(value) => {
                self.changes.mark_changed::<C>(entity);
                Some(Mut(value))
            }
            Err(_) => None,
        }
    }

    /// 可变读取实体组件（错误版本）。
    pub fn try_get_mut<C: Component>(&mut self, entity: Entity) -> Result<Mut<'_, C>> {
        match self.inner.get::<&mut C>(entity.0) {
            Ok(value) => {
                self.changes.mark_changed::<C>(entity);
                Ok(Mut(value))
            }
            Err(error) => Err(EcsError::Component(error.to_string())),
        }
    }

    /// 遍历拥有组件 `C` 的实体，对每个实体调用 `visit`。
    ///
    /// hecs 0.10 的查询借用一个临时 `QueryBorrow`，因此门面以回调形式暴露，
    /// 避免把第三方生命周期泄漏到公共 API。
    pub fn query<C: Component>(&self, mut visit: impl FnMut(Entity, &C)) {
        let mut query = self.inner.query::<&C>();
        for (entity, component) in query.iter() {
            visit(Entity::new(entity), component);
        }
    }

    /// 实体是否仍然存活（世代匹配）。
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.inner.contains(entity.0)
    }

    /// 注册或替换资源，返回旧值。
    pub fn insert_resource<R: Resource>(&mut self, resource: R) -> Option<R> {
        self.resources.insert(resource)
    }

    /// 读取资源。
    pub fn get_resource<R: Resource>(&self) -> Option<&R> {
        self.resources.get::<R>()
    }

    /// 可变读取资源。
    pub fn get_resource_mut<R: Resource>(&mut self) -> Option<&mut R> {
        self.resources.get_mut::<R>()
    }

    /// 移除资源。
    pub fn remove_resource<R: Resource>(&mut self) -> Option<R> {
        self.resources.remove::<R>()
    }

    /// 当前变更 tick。
    pub fn change_tick(&self) -> u64 {
        self.changes.tick()
    }

    /// 查询组件 `C` 自 `since` 之后的新增/变更实体。
    pub fn changes_since<C: Component>(
        &self,
        since: u64,
    ) -> impl Iterator<Item = (Entity, ChangeKind)> + '_ {
        self.changes.changes_since::<C>(since)
    }

    /// 清空全部变更记录（保留 tick）。
    pub fn clear_changes(&mut self) {
        self.changes.clear();
    }
}
