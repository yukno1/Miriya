//! 实体组件系统（ECS）。
//!
//! 当前为最小原型：`World` 存储实体与组件，后续可考虑接入
//! hecs / bevy_ecs 等成熟实现。

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// 实体标识符。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity(u64);

/// 组件 trait：所有可挂载到实体上的数据类型。
pub trait Component: Any + 'static {}

/// 组件存储。
#[derive(Default)]
struct ComponentStore {
    data: HashMap<TypeId, HashMap<Entity, Box<dyn Any>>>,
}

/// 世界：实体与组件的容器。
#[derive(Default)]
pub struct World {
    next_entity: u64,
    entities: Vec<Entity>,
    components: ComponentStore,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    /// 创建新实体。
    pub fn spawn(&mut self) -> Entity {
        self.next_entity += 1;
        let e = Entity(self.next_entity);
        self.entities.push(e);
        e
    }

    /// 删除实体及其所有组件。
    pub fn despawn(&mut self, entity: Entity) {
        self.entities.retain(|e| *e != entity);
        for store in self.components.data.values_mut() {
            store.remove(&entity);
        }
    }

    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// 为实体挂载组件。
    pub fn insert<C: Component>(&mut self, entity: Entity, component: C) {
        self.components
            .data
            .entry(TypeId::of::<C>())
            .or_default()
            .insert(entity, Box::new(component));
    }

    /// 读取实体的组件。
    pub fn get<C: Component>(&self, entity: Entity) -> Option<&C> {
        self.components
            .data
            .get(&TypeId::of::<C>())?
            .get(&entity)?
            .downcast_ref::<C>()
    }

    /// 可变读取实体的组件。
    pub fn get_mut<C: Component>(&mut self, entity: Entity) -> Option<&mut C> {
        self.components
            .data
            .get_mut(&TypeId::of::<C>())?
            .get_mut(&entity)?
            .downcast_mut::<C>()
    }

    /// 遍历拥有指定组件类型 `C` 的所有实体。
    pub fn query<C: Component>(&self) -> impl Iterator<Item = (Entity, &C)> {
        self.components
            .data
            .get(&TypeId::of::<C>())
            .into_iter()
            .flat_map(|store| store.iter())
            .filter_map(|(e, c)| c.downcast_ref::<C>().map(|c| (*e, c)))
    }
}

impl Component for mir_math::Vec3 {}
