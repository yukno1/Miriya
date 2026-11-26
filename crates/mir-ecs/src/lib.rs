//! 实体组件系统（ECS）门面。
//!
//! M3 起内部使用 `hecs`：对外提供世代实体句柄、稳定查询 API、
//! 类型化 Resource 与基础变更检测；上层不直接依赖 hecs 类型。

#![forbid(unsafe_code)]

mod change;
mod entity;
mod error;
mod resource;
mod world;

pub use change::{ChangeKind, ChangeTracker};
pub use entity::Entity;
pub use error::{EcsError, Result};
pub use resource::{Resource, Resources};
pub use world::{Component, Mut, Ref, World};

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Position(f32);

    #[derive(Debug, PartialEq)]
    struct Score(u32);

    #[test]
    fn spawn_query_and_despawn() {
        let mut world = World::new();
        let first = world.spawn();
        let second = world.spawn();
        assert_eq!(world.len(), 2);

        world.insert(first, Position(1.0)).unwrap();
        world.insert(second, Position(2.0)).unwrap();

        assert_eq!(
            world.get::<Position>(first).map(|p| *p),
            Some(Position(1.0))
        );
        let mut queried = Vec::new();
        world.query::<Position>(|_, position| queried.push(position.0));
        assert_eq!(queried, vec![1.0, 2.0]);

        world.despawn(first).unwrap();
        assert!(!world.is_alive(first));
        assert!(world.get::<Position>(first).is_none());
        assert!(matches!(
            world.try_get::<Position>(first),
            Err(EcsError::NoSuchEntity(_))
        ));
        assert!(world.is_alive(second));
    }

    #[test]
    fn stale_handle_does_not_alias_new_entity() {
        let mut world = World::new();
        let old = world.spawn();
        world.insert(old, Position(9.0)).unwrap();
        world.despawn(old).unwrap();
        let new = world.spawn();
        assert_ne!(old, new, "世代句柄必须区分复用索引的新实体");
        assert!(world.get::<Position>(new).is_none());
    }

    #[test]
    fn mutable_access_reports_changes() {
        let mut world = World::new();
        let entity = world.spawn();
        let before = world.change_tick();

        world.insert(entity, Position(1.0)).unwrap();
        let added: Vec<_> = world.changes_since::<Position>(before).collect();
        assert_eq!(added, vec![(entity, ChangeKind::Added)]);

        let since_added = world.change_tick();
        {
            let mut position = world.get_mut::<Position>(entity).unwrap();
            position.0 = 3.0;
        }
        let changed: Vec<_> = world.changes_since::<Position>(since_added).collect();
        assert_eq!(changed.len(), 1);
        assert!(matches!(changed[0].1, ChangeKind::Changed(_)));
        assert_eq!(
            world.get::<Position>(entity).map(|p| *p),
            Some(Position(3.0))
        );
    }

    #[test]
    fn typed_resources_are_separate_from_components() {
        let mut world = World::new();
        assert!(world.get_resource::<Score>().is_none());
        assert!(world.insert_resource(Score(7)).is_none());
        assert_eq!(world.get_resource::<Score>(), Some(&Score(7)));
        world.get_resource_mut::<Score>().unwrap().0 = 11;
        assert_eq!(world.remove_resource::<Score>(), Some(Score(11)));
        assert!(world.get_resource::<Score>().is_none());
    }
}
