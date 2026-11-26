//! 已加载资产缓存：按 AssetId 索引，独立于注册表（ADR-0013）。

use std::collections::HashMap;

use crate::AssetId;
use crate::asset::{Asset, AssetState, NativeAsset};
use crate::error::Result;

/// 内容缓存与加载状态。
#[derive(Debug, Default)]
pub struct AssetCache {
    entries: HashMap<AssetId, AssetState>,
}

impl AssetCache {
    /// 创建空缓存。
    pub fn new() -> Self {
        Self::default()
    }

    /// 标记作业已提交。
    pub fn mark_pending(&mut self, id: AssetId) {
        self.entries.insert(id, AssetState::Pending);
    }

    /// 写入成功结果。
    pub fn insert_ready(&mut self, id: AssetId, native: NativeAsset) {
        self.entries.insert(id, AssetState::Ready(native));
    }

    /// 写入失败结果（不 panic）。
    pub fn insert_failed(&mut self, id: AssetId, message: String) {
        self.entries.insert(id, AssetState::Failed(message));
    }

    /// 读取状态。
    pub fn state(&self, id: AssetId) -> Option<&AssetState> {
        self.entries.get(&id)
    }

    /// 按类型读取具体资产数据。
    pub fn get<A: Asset>(&self, id: AssetId) -> Option<&A::Native> {
        match self.entries.get(&id)? {
            AssetState::Ready(native) => A::borrow_native(native),
            _ => None,
        }
    }

    /// 卸载资产。
    pub fn unload(&mut self, id: AssetId) -> Result<()> {
        self.entries
            .remove(&id)
            .map(|_| ())
            .ok_or(crate::error::AssetError::NotFound(id))
    }

    /// 所有已就绪资产 ID。
    pub fn ready_ids(&self) -> Vec<AssetId> {
        let mut ids: Vec<AssetId> = self
            .entries
            .iter()
            .filter(|(_, state)| matches!(state, AssetState::Ready(_)))
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// 已缓存条目数（含 Pending/Failed）。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{ImageAsset, MeshAsset};
    use mir_image::ImageData;
    use mir_render::MeshData;

    #[test]
    fn typed_get_downcasts_native_asset() {
        let mut cache = AssetCache::new();
        let id = AssetId::new();
        cache.insert_ready(id, NativeAsset::Mesh(MeshData::new(Vec::new(), Vec::new())));
        assert!(cache.get::<MeshAsset>(id).is_some());
        assert!(cache.get::<ImageAsset>(id).is_none());

        let image_id = AssetId::new();
        cache.insert_ready(
            image_id,
            NativeAsset::Image(ImageData::new(1, 1, vec![0; 4]).unwrap()),
        );
        assert!(cache.get::<ImageAsset>(image_id).is_some());
    }
}
