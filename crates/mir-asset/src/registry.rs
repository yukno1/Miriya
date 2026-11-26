//! 资产注册表：只保存元数据，不持有内容（ADR-0013）。

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::AssetId;
use crate::error::{AssetError, Result};

/// 单个资产的元数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetMeta {
    pub id: AssetId,
    pub path: PathBuf,
    pub type_tag: String,
    /// BLAKE3/SHA 等内容的 32 字节摘要；M4 使用 FNV-1a x4 拼装。
    pub hash: [u8; 32],
    pub dependencies: Vec<AssetId>,
}

/// 路径/类型/依赖索引。
#[derive(Debug, Default)]
pub struct AssetRegistry {
    assets: HashMap<AssetId, AssetMeta>,
    by_path: HashMap<PathBuf, AssetId>,
}

impl AssetRegistry {
    /// 创建空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 归一化相对路径：统一分隔符、去掉 `./` 和空组件；拒绝绝对路径与 `..`。
    pub fn normalize_path(path: &Path) -> Result<PathBuf> {
        if path.as_os_str().is_empty() {
            return Err(AssetError::InvalidPath(path.to_path_buf()));
        }
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => normalized.push(part),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(AssetError::InvalidPath(path.to_path_buf()));
                }
            }
        }
        if normalized.as_os_str().is_empty() {
            return Err(AssetError::InvalidPath(path.to_path_buf()));
        }
        Ok(normalized)
    }

    /// 注册资产；同路径重复注册返回原 ID。
    pub fn register(
        &mut self,
        path: impl AsRef<Path>,
        type_tag: impl Into<String>,
    ) -> Result<AssetId> {
        let path = Self::normalize_path(path.as_ref())?;
        if let Some(id) = self.by_path.get(&path) {
            return Ok(*id);
        }
        let id = AssetId::new();
        self.assets.insert(
            id,
            AssetMeta {
                id,
                path: path.clone(),
                type_tag: type_tag.into(),
                hash: [0; 32],
                dependencies: Vec::new(),
            },
        );
        self.by_path.insert(path, id);
        Ok(id)
    }

    /// 按 ID 读取元数据。
    pub fn get(&self, id: AssetId) -> Option<&AssetMeta> {
        self.assets.get(&id)
    }

    /// 按归一化路径查找 ID。
    pub fn by_path(&self, path: &Path) -> Option<AssetId> {
        self.by_path.get(path).copied()
    }

    /// 更新内容哈希。
    pub fn set_hash(&mut self, id: AssetId, hash: [u8; 32]) -> Result<()> {
        self.assets
            .get_mut(&id)
            .ok_or(AssetError::NotFound(id))?
            .hash = hash;
        Ok(())
    }

    /// 设置直接依赖（去重并保持确定性顺序）。
    pub fn set_dependencies(&mut self, id: AssetId, dependencies: Vec<AssetId>) -> Result<()> {
        let meta = self.assets.get_mut(&id).ok_or(AssetError::NotFound(id))?;
        let mut unique = dependencies;
        unique.sort_unstable();
        unique.dedup();
        meta.dependencies = unique;
        Ok(())
    }

    /// 移除资产元数据。
    pub fn remove(&mut self, id: AssetId) -> Result<AssetMeta> {
        let meta = self.assets.remove(&id).ok_or(AssetError::NotFound(id))?;
        self.by_path.remove(&meta.path);
        Ok(meta)
    }

    /// 所有资产 ID（按 UUID 排序，保证快照确定性）。
    pub fn ids(&self) -> Vec<AssetId> {
        let mut ids: Vec<AssetId> = self.assets.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// 资产数量。
    pub fn len(&self) -> usize {
        self.assets.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_lookup_by_normalized_path() {
        let mut registry = AssetRegistry::new();
        let id = registry.register("./textures/brick.png", "image").unwrap();
        assert_eq!(registry.by_path(Path::new("textures/brick.png")), Some(id));
        assert_eq!(
            registry.register("textures/brick.png", "image").unwrap(),
            id
        );
        assert!(registry.register("../escape.png", "image").is_err());
        assert!(registry.register("/absolute.png", "image").is_err());
    }

    #[test]
    fn dependencies_are_deduplicated() {
        let mut registry = AssetRegistry::new();
        let a = registry.register("a.miasset", "mesh").unwrap();
        let b = registry.register("b.png", "image").unwrap();
        registry.set_dependencies(a, vec![b, b]).unwrap();
        assert_eq!(registry.get(a).unwrap().dependencies, vec![b]);
    }
}
