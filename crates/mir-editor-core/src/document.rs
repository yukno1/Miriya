//! UI 无关的关卡文档与选择集模型。

use std::fmt;

use mir_math::Vec3;

/// 场景中可编辑对象的稳定标识。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(u64);

impl ObjectId {
    /// 空标识。
    pub const NULL: Self = Self(u64::MAX);

    /// 由原始值构造。
    #[inline]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// 原始值。
    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// 是否为空标识。
    #[inline]
    pub const fn is_null(self) -> bool {
        self.0 == u64::MAX
    }
}

impl Default for ObjectId {
    fn default() -> Self {
        Self::NULL
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({})", self.0)
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 关卡中的一个对象（仅编辑器视图数据，不替代运行时 ECS）。
#[derive(Debug, Clone, PartialEq)]
pub struct SceneObject {
    pub id: ObjectId,
    pub name: String,
    /// 位置使用引擎约定：厘米、Z-up、左手系。
    pub position: Vec3,
}

/// 打开的关卡文档。
#[derive(Debug, Clone, PartialEq)]
pub struct LevelDocument {
    name: String,
    objects: Vec<SceneObject>,
}

impl LevelDocument {
    /// 创建空关卡。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            objects: Vec::new(),
        }
    }

    /// 关卡名。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 全部对象。
    pub fn objects(&self) -> &[SceneObject] {
        &self.objects
    }

    /// 按标识查询对象。
    pub fn object(&self, id: ObjectId) -> Option<&SceneObject> {
        self.objects.iter().find(|object| object.id == id)
    }

    /// 按标识可变查询对象。
    pub fn object_mut(&mut self, id: ObjectId) -> Option<&mut SceneObject> {
        self.objects.iter_mut().find(|object| object.id == id)
    }

    /// 是否包含对象。
    pub fn contains(&self, id: ObjectId) -> bool {
        self.object(id).is_some()
    }

    /// 添加对象；标识重复时返回 `false`。
    pub fn add_object(&mut self, object: SceneObject) -> bool {
        if self.contains(object.id) {
            return false;
        }
        self.objects.push(object);
        true
    }

    /// 移除并返回对象。
    pub fn remove_object(&mut self, id: ObjectId) -> Option<SceneObject> {
        let index = self.objects.iter().position(|object| object.id == id)?;
        Some(self.objects.remove(index))
    }
}

/// 当前选择集。命令栈会记录其前后状态以便撤销。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    ids: Vec<ObjectId>,
}

impl Selection {
    /// 由标识集合构造。
    pub fn from_ids(ids: impl IntoIterator<Item = ObjectId>) -> Self {
        Self {
            ids: ids.into_iter().collect(),
        }
    }

    /// 选中的标识。
    pub fn ids(&self) -> &[ObjectId] {
        &self.ids
    }

    /// 是否选中指定对象。
    pub fn contains(&self, id: ObjectId) -> bool {
        self.ids.contains(&id)
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// 清空选择。
    pub fn clear(&mut self) {
        self.ids.clear();
    }

    /// 从选择集中移除对象。
    pub fn remove(&mut self, id: ObjectId) {
        self.ids.retain(|candidate| *candidate != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_add_remove_and_lookup() {
        let mut document = LevelDocument::new("level");
        let object = SceneObject {
            id: ObjectId::new(7),
            name: "Chair".to_string(),
            position: Vec3::new(1.0, 2.0, 3.0),
        };
        assert!(document.add_object(object.clone()));
        assert!(!document.add_object(object.clone()));
        assert_eq!(document.objects().len(), 1);
        assert_eq!(document.object(object.id), Some(&object));

        let id = object.id;
        assert_eq!(document.remove_object(id), Some(object));
        assert_eq!(document.remove_object(id), None);
    }
}
