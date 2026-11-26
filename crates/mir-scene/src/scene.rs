//! Level / Scene 顶层模型。

use std::collections::HashMap;

use mir_ecs::{Entity, Ref, World};
use mir_math::{Transform, Vec3};

use crate::actor::{ActorId, ActorMeta, Children, Parent};
use crate::error::{Result, SceneError};
use crate::hierarchy::{WorldTransforms, would_create_cycle};

/// 层级计算用的紧凑记录，按 ActorId 排序；所有修改都经 Level API 同步。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ActorRecord {
    pub id: ActorId,
    pub transform: Transform,
    pub parent: Option<ActorId>,
}

/// 关卡：ECS World + 稳定 Actor 索引 + 层级关系。
pub struct Level {
    pub name: String,
    world: World,
    actors: HashMap<ActorId, Entity>,
    by_entity: HashMap<Entity, ActorId>,
    records: Vec<ActorRecord>,
    next_actor_id: u64,
}

impl Level {
    /// 创建空关卡。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            world: World::new(),
            actors: HashMap::new(),
            by_entity: HashMap::new(),
            records: Vec::new(),
            next_actor_id: 1,
        }
    }

    /// 创建一个带默认变换的 Actor。
    pub fn spawn_actor(&mut self, name: impl Into<String>) -> Result<ActorId> {
        self.spawn_actor_with_transform(name, Transform::default())
    }

    /// 创建一个带指定局部变换的 Actor。
    pub fn spawn_actor_with_transform(
        &mut self,
        name: impl Into<String>,
        transform: Transform,
    ) -> Result<ActorId> {
        let id = ActorId::new(self.next_actor_id);
        self.next_actor_id = self.next_actor_id.saturating_add(1);
        self.spawn_actor_with_id(id, name, transform)
    }

    /// 用快照中的稳定 ID 重建 Actor（仅供场景加载使用）。
    pub(crate) fn spawn_actor_with_id(
        &mut self,
        id: ActorId,
        name: impl Into<String>,
        transform: Transform,
    ) -> Result<ActorId> {
        if self.actors.contains_key(&id) {
            return Err(SceneError::Corrupt(format!("ActorId {id:?} 重复")));
        }
        self.next_actor_id = self.next_actor_id.max(id.get().saturating_add(1));
        let entity = self.world.spawn();

        if let Err(error) = self.world.insert(
            entity,
            ActorMeta {
                id,
                name: name.into(),
            },
        ) {
            let _ = self.world.despawn(entity);
            return Err(error.into());
        }
        if let Err(error) = self.world.insert(entity, transform) {
            let _ = self.world.despawn(entity);
            return Err(error.into());
        }
        if let Err(error) = self.world.insert(entity, Children::default()) {
            let _ = self.world.despawn(entity);
            return Err(error.into());
        }

        self.actors.insert(id, entity);
        self.by_entity.insert(entity, id);
        self.records.push(ActorRecord {
            id,
            transform,
            parent: None,
        });
        self.records.sort_unstable_by_key(|record| record.id);
        Ok(id)
    }

    /// 销毁没有子节点的 Actor。含子节点的 Actor 需先解除父子关系。
    pub fn despawn_actor(&mut self, id: ActorId) -> Result<()> {
        if !self.children(id).is_empty() {
            return Err(SceneError::Corrupt(format!(
                "Actor {id:?} 仍有子节点，需先解除父子关系"
            )));
        }
        if let Some(parent) = self.parent(id) {
            self.remove_child(parent, id)?;
        }
        let entity = self.try_entity(id)?;
        self.world.despawn(entity)?;
        self.actors.remove(&id);
        self.by_entity.remove(&entity);
        if let Some(index) = self.record_index(id) {
            self.records.remove(index);
        }
        Ok(())
    }

    /// Actor 对应的 ECS 实体。
    pub fn entity(&self, id: ActorId) -> Option<Entity> {
        self.actors.get(&id).copied()
    }

    /// 由 ECS 实体反查 ActorId。
    pub fn actor_id(&self, entity: Entity) -> Option<ActorId> {
        self.by_entity.get(&entity).copied()
    }

    /// 所有 ActorId（按数值排序，保证快照确定性）。
    pub fn actors(&self) -> impl Iterator<Item = ActorId> + '_ {
        self.records.iter().map(|record| record.id)
    }

    /// Actor 数量。
    pub fn actor_count(&self) -> usize {
        self.records.len()
    }

    /// 层级计算用的紧凑记录。
    pub(crate) fn records(&self) -> &[ActorRecord] {
        &self.records
    }

    /// Actor 元数据。
    pub fn meta(&self, id: ActorId) -> Option<Ref<'_, ActorMeta>> {
        self.world.get::<ActorMeta>(self.entity(id)?)
    }

    /// 修改 Actor 名称。
    pub fn set_name(&mut self, id: ActorId, name: impl Into<String>) -> Result<()> {
        let entity = self.try_entity(id)?;
        let mut meta = self
            .world
            .get_mut::<ActorMeta>(entity)
            .ok_or(SceneError::MissingActor(id))?;
        meta.name = name.into();
        Ok(())
    }

    /// 局部变换（只读）。
    pub fn transform(&self, id: ActorId) -> Option<Ref<'_, Transform>> {
        self.world.get::<Transform>(self.entity(id)?)
    }

    /// 通过闭包更新局部变换，并同步层级计算缓存。
    pub fn update_transform(
        &mut self,
        id: ActorId,
        update: impl FnOnce(&mut Transform),
    ) -> Result<()> {
        let entity = self.try_entity(id)?;
        let updated = {
            let mut transform = self
                .world
                .get_mut::<Transform>(entity)
                .ok_or(SceneError::MissingActor(id))?;
            update(&mut transform);
            *transform
        };
        self.set_record_transform(id, updated);
        Ok(())
    }

    /// 设置局部变换。
    pub fn set_transform(&mut self, id: ActorId, transform: Transform) -> Result<()> {
        let entity = self.try_entity(id)?;
        self.world.insert(entity, transform)?;
        self.set_record_transform(id, transform);
        Ok(())
    }

    /// 父 ActorId（若存在）。
    pub fn parent(&self, id: ActorId) -> Option<ActorId> {
        self.record_index(id)
            .and_then(|index| self.records[index].parent)
    }

    /// 子 ActorId 列表。
    pub fn children(&self, id: ActorId) -> Vec<ActorId> {
        let Some(entity) = self.entity(id) else {
            return Vec::new();
        };
        self.world
            .get::<Children>(entity)
            .map(|children| children.0.clone())
            .unwrap_or_default()
    }

    /// 设置父 Actor，自动从旧父节点移除并检测循环。
    pub fn set_parent(&mut self, child: ActorId, parent: ActorId) -> Result<()> {
        would_create_cycle(self, child, parent)?;
        if !self.actors.contains_key(&parent) {
            return Err(SceneError::MissingActor(parent));
        }
        if let Some(old_parent) = self.parent(child) {
            self.remove_child(old_parent, child)?;
        }
        let child_entity = self.try_entity(child)?;
        self.world.insert(child_entity, Parent(parent))?;
        let parent_entity = self.try_entity(parent)?;
        {
            let mut children = self
                .world
                .get_mut::<Children>(parent_entity)
                .ok_or(SceneError::MissingActor(parent))?;
            if !children.0.contains(&child) {
                children.0.push(child);
            }
        }
        self.set_record_parent(child, Some(parent));
        Ok(())
    }

    /// 解除父子关系。
    pub fn clear_parent(&mut self, child: ActorId) -> Result<()> {
        let Some(parent) = self.parent(child) else {
            return Ok(());
        };
        self.remove_child(parent, child)?;
        let entity = self.try_entity(child)?;
        self.world.remove::<Parent>(entity)?;
        self.set_record_parent(child, None);
        Ok(())
    }

    /// 计算全部 Actor 的世界矩阵。
    pub fn world_transforms(&self) -> Result<WorldTransforms> {
        crate::hierarchy::compute_world_transforms(self)
    }

    /// 只读访问底层 ECS World（提取/调试用）。
    pub fn world(&self) -> &World {
        &self.world
    }

    /// 可变访问底层 ECS World（插件/系统用）。
    ///
    /// 注意：直接修改 Transform/Parent 组件不会同步层级缓存，需使用 Level API。
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    fn try_entity(&self, id: ActorId) -> Result<Entity> {
        self.actors
            .get(&id)
            .copied()
            .ok_or(SceneError::MissingActor(id))
    }

    fn record_index(&self, id: ActorId) -> Option<usize> {
        self.records
            .binary_search_by_key(&id, |record| record.id)
            .ok()
    }

    fn set_record_transform(&mut self, id: ActorId, transform: Transform) {
        if let Some(index) = self.record_index(id) {
            self.records[index].transform = transform;
        }
    }

    fn set_record_parent(&mut self, id: ActorId, parent: Option<ActorId>) {
        if let Some(index) = self.record_index(id) {
            self.records[index].parent = parent;
        }
    }

    fn remove_child(&mut self, parent: ActorId, child: ActorId) -> Result<()> {
        let entity = self.try_entity(parent)?;
        let mut children = self
            .world
            .get_mut::<Children>(entity)
            .ok_or(SceneError::MissingActor(parent))?;
        children.0.retain(|id| *id != child);
        Ok(())
    }
}

/// 场景：关卡 + 场景级元数据。
pub struct Scene {
    pub name: String,
    pub level: Level,
    ambient: Vec3,
}

impl Scene {
    /// 创建包含一个默认关卡的场景。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            level: Level::new("main"),
            ambient: Vec3::ONE,
        }
    }

    /// 环境光颜色。
    pub fn ambient(&self) -> Vec3 {
        self.ambient
    }

    /// 设置环境光颜色。
    pub fn set_ambient(&mut self, color: Vec3) {
        self.ambient = color;
    }

    /// 只读访问底层 ECS World（兼容旧 API）。
    pub fn world(&self) -> &World {
        self.level.world()
    }

    /// 可变访问底层 ECS World（兼容旧 API）。
    pub fn world_mut(&mut self) -> &mut World {
        self.level.world_mut()
    }
}
