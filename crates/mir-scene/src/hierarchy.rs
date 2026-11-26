//! 层级世界变换：拓扑序单次遍历，避免递归与 N+1 查询。
//!
//! 实现使用平行数组 + 邻接链表，避免为每个 Actor 分配 HashMap/子 Vec；
//! 结果中的 ActorId 有序，可用二分查找读取世界矩阵。

use std::collections::HashSet;

use mir_math::Mat4;

use crate::actor::ActorId;
use crate::error::{Result, SceneError};
use crate::scene::Level;

/// 每个 Actor 的世界矩阵快照。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldTransforms {
    actors: Vec<ActorId>,
    matrices: Vec<Mat4>,
}

impl WorldTransforms {
    /// 读取 Actor 的世界矩阵。
    pub fn get(&self, actor: ActorId) -> Option<Mat4> {
        self.actors
            .binary_search(&actor)
            .ok()
            .map(|index| self.matrices[index])
    }

    /// 遍历全部世界矩阵。
    pub fn iter(&self) -> impl Iterator<Item = (ActorId, Mat4)> + '_ {
        self.actors
            .iter()
            .copied()
            .zip(self.matrices.iter().copied())
    }

    /// 已计算的世界矩阵数量。
    pub fn len(&self) -> usize {
        self.matrices.len()
    }

    /// 是否没有任何 Actor。
    pub fn is_empty(&self) -> bool {
        self.matrices.is_empty()
    }
}

/// 按拓扑序计算所有 Actor 的世界矩阵。
///
/// 根节点世界矩阵等于局部矩阵；子节点为 `parent_world * child_local`。
/// 父链缺失或出现环时返回 [`SceneError`]，不做部分提交。
pub fn compute_world_transforms(level: &Level) -> Result<WorldTransforms> {
    let records = level.records();
    let count = records.len();
    if count == 0 {
        return Ok(WorldTransforms::default());
    }

    if records.iter().all(|record| record.parent.is_none()) {
        return Ok(WorldTransforms {
            actors: records.iter().map(|record| record.id).collect(),
            matrices: records
                .iter()
                .map(|record| record.transform.to_matrix())
                .collect(),
        });
    }

    let actors: Vec<ActorId> = records.iter().map(|record| record.id).collect();
    let base_id = records[0].id.get();
    let dense_ids = records[count - 1].id.get() - base_id + 1 == count as u64;
    let index_of = |id: ActorId| -> Option<usize> {
        if dense_ids {
            let offset = id.get().checked_sub(base_id)? as usize;
            (offset < count && records[offset].id == id).then_some(offset)
        } else {
            records.binary_search_by_key(&id, |record| record.id).ok()
        }
    };

    let mut locals = Vec::with_capacity(count);
    let mut parents = Vec::with_capacity(count);
    for record in records {
        locals.push(record.transform);
        parents.push(match record.parent {
            Some(parent) => Some(index_of(parent).ok_or(SceneError::MissingActor(parent))?),
            None => None,
        });
    }

    // 邻接链表：first_child[i] 是第一个子节点下标，next_sibling[c] 指向下一个兄弟。
    let mut first_child = vec![None; count];
    let mut next_sibling = vec![None; count];
    let mut indegree = vec![0_usize; count];
    for child in 0..count {
        if let Some(parent) = parents[child] {
            next_sibling[child] = first_child[parent];
            first_child[parent] = Some(child);
            indegree[child] = 1;
        }
    }

    let mut queue: Vec<usize> = (0..count).filter(|i| indegree[*i] == 0).collect();
    let mut world = vec![Mat4::IDENTITY; count];
    let mut head = 0_usize;

    while head < queue.len() {
        let index = queue[head];
        head += 1;
        let local_matrix = locals[index].to_matrix();
        world[index] = match parents[index] {
            Some(parent) => world[parent] * local_matrix,
            None => local_matrix,
        };

        let mut child = first_child[index];
        while let Some(next) = child {
            indegree[next] = indegree[next].saturating_sub(1);
            if indegree[next] == 0 {
                queue.push(next);
            }
            child = next_sibling[next];
        }
    }

    let processed = head;

    if processed != count {
        let unresolved = (0..count)
            .find(|index| indegree[*index] > 0)
            .map(|index| actors[index])
            .unwrap_or_else(|| ActorId::new(0));
        return Err(SceneError::Cycle(unresolved));
    }

    Ok(WorldTransforms {
        actors,
        matrices: world,
    })
}

/// 检查把 `child` 挂到 `parent` 下是否会形成循环。
pub fn would_create_cycle(level: &Level, child: ActorId, parent: ActorId) -> Result<()> {
    if child == parent {
        return Err(SceneError::Cycle(child));
    }
    let mut seen = HashSet::new();
    let mut current = Some(parent);
    while let Some(id) = current {
        if !seen.insert(id) {
            return Err(SceneError::Cycle(id));
        }
        if id == child {
            return Err(SceneError::Cycle(child));
        }
        current = level.parent(id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mir_math::{Quat, Transform, Vec3};

    #[test]
    fn child_follows_parent_translation_and_rotation() {
        let mut level = Level::new("hierarchy");
        let parent = level
            .spawn_actor_with_transform(
                "parent",
                Transform::from_position(Vec3::new(100.0, 0.0, 0.0)),
            )
            .unwrap();
        let child = level
            .spawn_actor_with_transform(
                "child",
                Transform::from_position(Vec3::new(200.0, 0.0, 0.0)),
            )
            .unwrap();
        level.set_parent(child, parent).unwrap();

        let matrices = level.world_transforms().unwrap();
        let initial = matrices.get(child).unwrap().transform_point(Vec3::ZERO);
        assert!((initial.x - 300.0).abs() < 1e-4);
        assert!(initial.y.abs() < 1e-4);

        level
            .update_transform(parent, |transform| {
                transform.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
            })
            .unwrap();
        let rotated = level
            .world_transforms()
            .unwrap()
            .get(child)
            .unwrap()
            .transform_point(Vec3::ZERO);
        assert!((rotated.x - 100.0).abs() < 1e-4, "{rotated:?}");
        assert!((rotated.y - 200.0).abs() < 1e-4, "{rotated:?}");
    }

    #[test]
    fn cycle_is_rejected() {
        let mut level = Level::new("cycle");
        let root = level.spawn_actor("root").unwrap();
        let child = level.spawn_actor("child").unwrap();
        level.set_parent(child, root).unwrap();
        assert!(matches!(
            level.set_parent(root, child),
            Err(SceneError::Cycle(_))
        ));
    }

    #[test]
    fn clear_parent_restores_local_position() {
        let mut level = Level::new("detach");
        let parent = level
            .spawn_actor_with_transform(
                "parent",
                Transform::from_position(Vec3::new(500.0, 0.0, 0.0)),
            )
            .unwrap();
        let child = level
            .spawn_actor_with_transform(
                "child",
                Transform::from_position(Vec3::new(100.0, 0.0, 0.0)),
            )
            .unwrap();
        level.set_parent(child, parent).unwrap();
        assert_eq!(level.parent(child), Some(parent));
        level.clear_parent(child).unwrap();
        assert_eq!(level.parent(child), None);
        let world = level.world_transforms().unwrap();
        let position = world.get(child).unwrap().transform_point(Vec3::ZERO);
        assert!((position.x - 100.0).abs() < 1e-4);
    }
}
