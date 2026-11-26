//! 管线缓存：常驻 `VkPipelineCache` 与磁盘持久化（R-07）。

use vulkanalia::prelude::v1_0::*;

use super::VulkanCore;

/// 管线缓存文件头 magic 与格式版本（R-07）。
const PIPELINE_CACHE_MAGIC: [u8; 4] = *b"MRPC";
const PIPELINE_CACHE_VERSION: u32 = 1;
/// 管线缓存文件头大小：magic(4) + version(4) + key(8)。
const PIPELINE_CACHE_HEADER_LEN: usize = 16;
/// 管线状态标签：顶点布局/光栅化/深度等待机状态变化时递增，参与缓存键哈希。
const PIPELINE_STATE_TAG: u32 = 1;

/// FNV-1a 64 位哈希：跨进程稳定，仅用于管线缓存键命名，非安全用途。
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 管线状态哈希：着色器字节码 + 状态标签（R-07 缓存键的第三分量）。
pub(super) fn pipeline_state_hash(shader_code: &[u32]) -> u64 {
    let mut bytes = Vec::with_capacity(shader_code.len() * 4 + 4);
    for word in shader_code {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(&PIPELINE_STATE_TAG.to_le_bytes());
    fnv1a64(&bytes)
}

/// 管线缓存磁盘目录（R-07）：Windows 用 `%LOCALAPPDATA%/Miriya/cache/pipeline`；
/// Linux 占位等价路径 `XDG_CACHE_HOME`（缺省 `$HOME/.cache`）下的 `Miriya/cache/pipeline`。
/// 环境变量缺失时返回 `None`（缓存静默降级为内存态）。
fn pipeline_cache_dir() -> Option<std::path::PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)?
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cache"))
            })?
    };
    Some(base.join("Miriya").join("cache").join("pipeline"))
}

impl VulkanCore {
    /// 计算管线缓存键与磁盘路径（R-07）。键 = 设备 UUID + 驱动版本 + 管线状态哈希；
    /// 目录不可用时路径为 `None`（仅内存态缓存）。
    pub(super) fn pipeline_cache_key_and_path(
        &self,
        state_hash: u64,
    ) -> (u64, Option<std::path::PathBuf>) {
        // SAFETY: physical_device 由本实例创建且仍存活。
        let properties = unsafe {
            self.instance
                .get_physical_device_properties(self.physical_device)
        };
        let uuid_hex: String = properties
            .pipeline_cache_uuid
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let mut key_bytes = Vec::with_capacity(28);
        key_bytes.extend_from_slice(properties.pipeline_cache_uuid.as_slice());
        key_bytes.extend_from_slice(&properties.driver_version.to_le_bytes());
        key_bytes.extend_from_slice(&state_hash.to_le_bytes());
        let key = fnv1a64(&key_bytes);
        let path = pipeline_cache_dir().map(|dir| {
            dir.join(format!(
                "{uuid_hex}_{}_{key:x}.bin",
                properties.driver_version
            ))
        });
        (key, path)
    }

    /// 创建（或复用）常驻管线缓存（R-07）。首次创建时尝试从磁盘加载；
    /// 文件缺失为正常首启路径，损坏/IO 异常仅告警并退化为空缓存。
    /// `VkPipelineCache` 本身创建失败时返回 null 缓存（对 `create_graphics_pipelines` 合法）。
    pub(super) fn ensure_pipeline_cache(
        &mut self,
        key: u64,
        path: Option<&std::path::Path>,
    ) -> vk::PipelineCache {
        if let Some(cache) = self.pipeline_cache {
            return cache;
        }

        // 读取缓存文件（文件级命中判定：magic + 版本 + 键一致，不做管线级 diff）。
        let initial_data = path.and_then(|path| match std::fs::read(path) {
            Ok(bytes) => {
                if bytes.len() > PIPELINE_CACHE_HEADER_LEN
                    && bytes[..4] == PIPELINE_CACHE_MAGIC
                    && bytes[4..8] == PIPELINE_CACHE_VERSION.to_le_bytes()
                    && bytes[8..16] == key.to_le_bytes()
                {
                    mir_log::info!(target: "vulkan", "管线缓存命中: {} 字节", bytes.len() - PIPELINE_CACHE_HEADER_LEN);
                    Some(bytes[PIPELINE_CACHE_HEADER_LEN..].to_vec())
                } else {
                    mir_log::warn!(target: "vulkan", "管线缓存文件损坏或键不匹配，忽略重建: {}", path.display());
                    None
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                mir_log::info!(target: "vulkan", "管线缓存未命中（首次启动）: {}", path.display());
                None
            }
            Err(error) => {
                mir_log::warn!(target: "vulkan", "管线缓存文件读取失败，忽略重建: {error}");
                None
            }
        });

        let cache = match &initial_data {
            Some(data) => {
                let info = vk::PipelineCacheCreateInfo::builder().initial_data(data);
                // SAFETY: initial_data 在调用期间持有且内容有效。
                unsafe { self.device.create_pipeline_cache(&info, None) }
            }
            None => {
                let info = vk::PipelineCacheCreateInfo::builder();
                // SAFETY: 空 info 有效。
                unsafe { self.device.create_pipeline_cache(&info, None) }
            }
        };
        match cache {
            Ok(cache) => {
                self.pipeline_cache = Some(cache);
                cache
            }
            Err(error) => {
                mir_log::warn!(target: "vulkan", "创建管线缓存失败，降级为无缓存: {error}");
                vk::PipelineCache::null()
            }
        }
    }

    /// 把管线缓存写回磁盘（R-07）：magic + 版本 + 键头 + 驱动导出的缓存数据。
    /// IO 失败仅告警，绝不阻塞/panic。
    pub(super) fn save_pipeline_cache(&self, key: u64, path: Option<&std::path::Path>) {
        let Some(path) = path else { return };
        let Some(cache) = self.pipeline_cache else {
            return;
        };
        // SAFETY: cache 由本设备创建且有效；返回驱动导出的缓存数据。
        let data = match unsafe { self.device.get_pipeline_cache_data(cache) } {
            Ok(data) => data,
            Err(error) => {
                mir_log::warn!(target: "vulkan", "管线缓存导出失败，跳过写回: {error}");
                return;
            }
        };
        if data.is_empty() {
            mir_log::warn!(target: "vulkan", "管线缓存导出为空，跳过写回");
            return;
        }
        let mut bytes = Vec::with_capacity(PIPELINE_CACHE_HEADER_LEN + data.len());
        bytes.extend_from_slice(&PIPELINE_CACHE_MAGIC);
        bytes.extend_from_slice(&PIPELINE_CACHE_VERSION.to_le_bytes());
        bytes.extend_from_slice(&key.to_le_bytes());
        bytes.extend_from_slice(&data);
        match std::fs::create_dir_all(path.parent().unwrap_or(path))
            .and_then(|()| std::fs::write(path, &bytes))
        {
            Ok(()) => {
                mir_log::info!(target: "vulkan", "管线缓存已写回: {} 字节", bytes.len());
            }
            Err(error) => {
                mir_log::warn!(target: "vulkan", "管线缓存写回失败: {error}");
            }
        }
    }
}
