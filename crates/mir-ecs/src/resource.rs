//! 类型化 Resource 容器：与组件存储分离。

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// 可注册到 [`World`](crate::World) 的资源类型。
pub trait Resource: Any + Send + Sync {}
impl<T: Any + Send + Sync> Resource for T {}

/// 按类型索引的资源表。
#[derive(Default)]
pub struct Resources {
    data: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Resources {
    /// 插入资源，返回被替换的旧值。
    pub fn insert<R: Resource>(&mut self, resource: R) -> Option<R> {
        self.data
            .insert(TypeId::of::<R>(), Box::new(resource))
            .and_then(|old| old.downcast::<R>().ok().map(|boxed| *boxed))
    }

    /// 读取资源。
    pub fn get<R: Resource>(&self) -> Option<&R> {
        self.data
            .get(&TypeId::of::<R>())
            .and_then(|boxed| boxed.downcast_ref::<R>())
    }

    /// 可变读取资源。
    pub fn get_mut<R: Resource>(&mut self) -> Option<&mut R> {
        self.data
            .get_mut(&TypeId::of::<R>())
            .and_then(|boxed| boxed.downcast_mut::<R>())
    }

    /// 移除并返回资源。
    pub fn remove<R: Resource>(&mut self) -> Option<R> {
        self.data
            .remove(&TypeId::of::<R>())
            .and_then(|boxed| boxed.downcast::<R>().ok().map(|boxed| *boxed))
    }

    /// 已注册资源数量。
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// 是否没有任何资源。
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}
