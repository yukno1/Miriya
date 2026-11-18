//! 资源子系统：加载与缓存贴图、模型、音频等。

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 资源句柄：轻量、可复制，指向缓存中的资源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Handle<T> {
    id: u64,
    _marker: std::marker::PhantomData<T>,
}

impl<T> Handle<T> {
    pub(crate) fn new(id: u64) -> Self {
        Self {
            id,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}

/// 资源服务器：负责加载与按类型缓存。
#[derive(Default)]
pub struct AssetServer {
    next_id: u64,
    root: PathBuf,
    cache: HashMap<TypeId, HashMap<u64, Box<dyn Any>>>,
}

impl AssetServer {
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置资源根目录（如 `assets/`）。
    pub fn set_root<P: AsRef<Path>>(&mut self, root: P) {
        self.root = root.as_ref().to_path_buf();
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 加载资源：命中缓存直接返回句柄，否则调用 `T::load`。
    pub fn load<T: Asset>(&mut self, path: impl AsRef<Path>) -> Handle<T> {
        let type_cache = self.cache.entry(TypeId::of::<T>()).or_default();
        // 简化处理：同类型下按路径字符串哈希做去重
        let key = path.as_ref().to_string_lossy().into_owned();
        let id = hash_id(&key, self.next_id);
        if !type_cache.contains_key(&id) {
            let full = self.root.join(path);
            let asset = T::load(&full).unwrap_or_else(|e| {
                panic!("加载资源失败 {}: {e}", full.display());
            });
            self.next_id += 1;
            type_cache.insert(id, Box::new(asset));
        }
        Handle::new(id)
    }

    /// 通过句柄读取资源。
    pub fn get<T: Asset>(&self, handle: Handle<T>) -> Option<&T> {
        self.cache
            .get(&TypeId::of::<T>())?
            .get(&handle.id())?
            .downcast_ref::<T>()
    }
}

/// 可加载资源 trait，具体格式（png、gltf 等）由实现方决定。
pub trait Asset: Any + 'static {
    fn load(path: &Path) -> Result<Self, String>
    where
        Self: Sized;
}

fn hash_id(key: &str, fallback: u64) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in key.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    if h == 0 { fallback.wrapping_add(1) } else { h }
}
