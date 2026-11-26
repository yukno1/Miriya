//! 基础变更检测：记录组件的新增与可变访问版本。

use std::any::TypeId;
use std::collections::{HashMap, HashSet};

use crate::Entity;

/// 组件变更类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// 自上次检查后首次挂载。
    Added,
    /// 自上次检查后被可变访问，携带变更 tick。
    Changed(u64),
}

#[derive(Default)]
struct TypeChanges {
    added: HashSet<Entity>,
    changed: HashMap<Entity, u64>,
}

impl TypeChanges {
    fn remove(&mut self, entity: Entity) {
        self.added.remove(&entity);
        self.changed.remove(&entity);
    }
}

/// 组件变更追踪器；按组件类型分桶，避免业务代码扫描全部实体。
#[derive(Default)]
pub struct ChangeTracker {
    tick: u64,
    by_type: HashMap<TypeId, TypeChanges>,
}

impl ChangeTracker {
    /// 当前 tick（每次标记变更递增）。
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// 标记组件新增。
    pub fn mark_added<C: 'static>(&mut self, entity: Entity) {
        self.tick = self.tick.saturating_add(1);
        let changes = self.by_type.entry(TypeId::of::<C>()).or_default();
        changes.added.insert(entity);
        changes.changed.insert(entity, self.tick);
    }

    /// 标记组件被可变访问（保守地视为已变更）。
    pub fn mark_changed<C: 'static>(&mut self, entity: Entity) {
        self.tick = self.tick.saturating_add(1);
        let changes = self.by_type.entry(TypeId::of::<C>()).or_default();
        // 首次可变访问后不再视作“新增”，后续查询应报告 Changed。
        changes.added.remove(&entity);
        changes.changed.insert(entity, self.tick);
    }

    /// 清理指定实体的追踪记录（销毁时调用）。
    pub fn remove(&mut self, entity: Entity) {
        for changes in self.by_type.values_mut() {
            changes.remove(entity);
        }
    }

    /// 查询指定组件类型在 `since` 之后发生变化的实体。
    pub fn changes_since<C: 'static>(
        &self,
        since: u64,
    ) -> impl Iterator<Item = (Entity, ChangeKind)> + '_ {
        self.by_type
            .get(&TypeId::of::<C>())
            .into_iter()
            .flat_map(move |changes| {
                changes.changed.iter().filter_map(move |(entity, version)| {
                    if *version <= since {
                        return None;
                    }
                    let kind = if changes.added.contains(entity) {
                        ChangeKind::Added
                    } else {
                        ChangeKind::Changed(*version)
                    };
                    Some((*entity, kind))
                })
            })
    }

    /// 清空所有追踪记录（保留 tick）。
    pub fn clear(&mut self) {
        self.by_type.clear();
    }
}
