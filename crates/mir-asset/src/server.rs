//! 资产服务器：异步加载、主线程 poll、依赖失效与热重载（ADR-0013）。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::AssetId;
use crate::asset::{Asset, AssetState, Handle, NativeAsset};
use crate::cache::AssetCache;
use crate::container;
use crate::dependency::DependencyGraph;
use crate::error::{AssetError, Result};
use crate::hot_reload::HotReloadWatcher;
use crate::importer::{self, ImportReport};
use crate::registry::{AssetMeta, AssetRegistry};

const SCENE_SCHEMA_HASH: u32 = 0x6d69_4d41; // "miMA"

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Initial,
    Reload,
    Dependent,
}

struct CompletedJob {
    id: AssetId,
    kind: JobKind,
    elapsed: Duration,
    result: Result<(NativeAsset, [u8; 32], ImportReport)>,
}

/// 资产服务器：注册表/缓存/作业池/监听器的组合根。
pub struct AssetServer {
    root: PathBuf,
    registry: AssetRegistry,
    graph: DependencyGraph,
    cache: AssetCache,
    tx: Sender<CompletedJob>,
    rx: Receiver<CompletedJob>,
    watcher: Option<HotReloadWatcher>,
    reloading: HashSet<AssetId>,
    reports: HashMap<AssetId, ImportReport>,
    last_reload_ms: Option<f64>,
}

impl AssetServer {
    /// 创建资产服务器（根目录默认为空，需调用 `set_root`）。
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            root: PathBuf::new(),
            registry: AssetRegistry::new(),
            graph: DependencyGraph::new(),
            cache: AssetCache::new(),
            tx,
            rx,
            watcher: None,
            reloading: HashSet::new(),
            reports: HashMap::new(),
            last_reload_ms: None,
        }
    }

    /// 设置资产根目录。
    pub fn set_root(&mut self, root: impl AsRef<Path>) {
        self.root = root.as_ref().to_path_buf();
    }

    /// 资产根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 只读注册表。
    pub fn registry(&self) -> &AssetRegistry {
        &self.registry
    }

    /// 只读依赖图。
    pub fn graph(&self) -> &DependencyGraph {
        &self.graph
    }

    /// 注册并异步加载资产；重复调用同一路径返回同一句柄。
    pub fn load<A: Asset>(&mut self, path: impl AsRef<Path>) -> Result<Handle<A>> {
        let path = AssetRegistry::normalize_path(path.as_ref())?;
        let type_tag = match self.registry.by_path(&path) {
            Some(id) => {
                let meta = self.registry.get(id).ok_or(AssetError::NotFound(id))?;
                if meta.type_tag != A::TYPE_TAG {
                    return Err(AssetError::TypeMismatch {
                        expected: A::TYPE_TAG,
                        actual: meta.type_tag.clone(),
                    });
                }
                meta.type_tag.clone()
            }
            None => A::TYPE_TAG.to_string(),
        };
        let id = self.registry.register(&path, &type_tag)?;
        if self.cache.state(id).is_some() {
            return Ok(Handle::new(id));
        }
        self.submit_job(id, path, type_tag, JobKind::Initial)?;
        Ok(Handle::new(id))
    }

    /// 读取已加载资产。
    pub fn get<A: Asset>(&self, handle: Handle<A>) -> Option<&A::Native> {
        self.cache.get::<A>(handle.id())
    }

    /// 资产状态。
    pub fn state(&self, id: AssetId) -> Option<&AssetState> {
        self.cache.state(id)
    }

    /// 最近一次导入报告。
    pub fn report(&self, id: AssetId) -> Option<&ImportReport> {
        self.reports.get(&id)
    }

    /// 最近一次热重载耗时（毫秒）。
    pub fn reload_latency_ms(&self) -> Option<f64> {
        self.last_reload_ms
    }

    /// 显式设置资产依赖（同时更新注册表与图）。
    pub fn set_dependencies(&mut self, dependent: AssetId, dependencies: &[AssetId]) -> Result<()> {
        for dependency in dependencies {
            if self.registry.get(*dependency).is_none() {
                return Err(AssetError::MissingDependency(*dependency));
            }
        }
        self.registry
            .set_dependencies(dependent, dependencies.to_vec())?;
        self.graph.replace_dependencies(dependent, dependencies);
        Ok(())
    }

    /// cook 闭包：依赖先于依赖者。
    pub fn cook_closure(&self, roots: &[AssetId]) -> Result<Vec<AssetId>> {
        self.graph.cook_order(roots)
    }

    /// 重新加载一个已注册资产及其依赖者。
    pub fn reload(&mut self, id: AssetId) -> Result<()> {
        self.start_reload_chain(id)
    }

    /// 开启热重载监听（根目录不存在时创建）。
    pub fn enable_hot_reload(&mut self) -> Result<()> {
        if self.root.as_os_str().is_empty() {
            return Err(AssetError::InvalidPath(self.root.clone()));
        }
        std::fs::create_dir_all(&self.root).map_err(|error| AssetError::Io {
            path: self.root.clone(),
            message: error.to_string(),
        })?;
        self.watcher = Some(HotReloadWatcher::new(&self.root)?);
        Ok(())
    }

    /// 每帧轮询完成队列与文件事件；返回本次结算的作业数。
    pub fn poll(&mut self) -> usize {
        let mut settled = 0;
        while let Ok(job) = self.rx.try_recv() {
            settled += 1;
            self.reloading.remove(&job.id);
            match job.result {
                Ok((native, hash, report)) => {
                    let _ = self.registry.set_hash(job.id, hash);
                    self.cache.insert_ready(job.id, native);
                    self.reports.insert(job.id, report);
                }
                Err(error) => {
                    self.cache.insert_failed(job.id, error.to_string());
                }
            }
            if matches!(job.kind, JobKind::Reload | JobKind::Dependent) {
                self.last_reload_ms = Some(job.elapsed.as_secs_f64() * 1000.0);
            }
        }

        if let Some(watcher) = &self.watcher {
            let changed = watcher.poll_events();
            for path in changed {
                let relative = path.strip_prefix(&self.root).unwrap_or(&path);
                if let Ok(normalized) = AssetRegistry::normalize_path(relative)
                    && let Some(id) = self.registry.by_path(&normalized)
                {
                    let _ = self.start_reload_chain(id);
                }
            }
        }
        settled
    }

    /// 卸载资产内容（保留注册表元数据）。
    pub fn unload(&mut self, id: AssetId) -> Result<()> {
        self.cache.unload(id)
    }

    /// 停止监听，等待在途作业由 poll 自然结算。
    pub fn shutdown(&mut self) {
        self.watcher = None;
    }

    /// 用版本化容器注册一段场景快照字节（供 CLI cook 使用）。
    pub fn register_snapshot(
        &mut self,
        name: impl AsRef<str>,
    ) -> Result<Handle<crate::SceneAsset>> {
        let path = format!("{}.mimap", name.as_ref());
        let id = self.registry.register(&path, crate::SceneAsset::TYPE_TAG)?;
        if self.cache.state(id).is_some() {
            return Ok(Handle::new(id));
        }
        self.cache.insert_ready(
            id,
            NativeAsset::Scene(
                mir_serialize::encode(&Vec::<u8>::new(), SCENE_SCHEMA_HASH)
                    .map_err(|error| AssetError::Corrupt(format!("快照编码失败：{error}")))?,
            ),
        );
        Ok(Handle::new(id))
    }

    fn start_reload_chain(&mut self, id: AssetId) -> Result<()> {
        let affected = self
            .graph
            .invalidation_order(&[id])
            .unwrap_or_else(|_| vec![id]);
        for affected_id in affected {
            let Some(meta) = self.registry.get(affected_id).cloned() else {
                continue;
            };
            let kind = if affected_id == id {
                JobKind::Reload
            } else {
                JobKind::Dependent
            };
            self.submit_job(affected_id, meta.path, meta.type_tag, kind)?;
        }
        Ok(())
    }

    fn submit_job(
        &mut self,
        id: AssetId,
        path: PathBuf,
        type_tag: String,
        kind: JobKind,
    ) -> Result<()> {
        if !self.reloading.insert(id) {
            return Ok(());
        }
        self.cache.mark_pending(id);
        let full = self.root.join(&path);
        let tx = self.tx.clone();
        let started = Instant::now();

        rayon::spawn(move || {
            let result = (|| -> Result<(NativeAsset, [u8; 32], ImportReport)> {
                let bytes = std::fs::read(&full).map_err(|error| AssetError::Io {
                    path: full.clone(),
                    message: error.to_string(),
                })?;
                let hash = content_hash(&bytes);
                let is_container = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("miasset"));
                let payload = if is_container {
                    container::decode(&bytes)?
                } else {
                    bytes
                };
                let mut report = ImportReport {
                    source: path.clone(),
                    type_tag: type_tag.clone(),
                    bytes: payload.len(),
                    ..ImportReport::default()
                };
                let native = importer::import_by_tag(&type_tag, &payload, &path, &mut report)?;
                Ok((native, hash, report))
            })();
            let _ = tx.send(CompletedJob {
                id,
                kind,
                elapsed: started.elapsed(),
                result,
            });
        });
        Ok(())
    }
}

impl Default for AssetServer {
    fn default() -> Self {
        Self::new()
    }
}

fn content_hash(bytes: &[u8]) -> [u8; 32] {
    let mut hashes = [
        0x811c_9dc5_u32,
        0x9e37_79b9_u32,
        0x85eb_ca6b_u32,
        0xc2b2_ae35_u32,
    ];
    for byte in bytes {
        for hash in &mut hashes {
            *hash ^= *byte as u32;
            *hash = hash.wrapping_mul(0x0100_0193);
        }
    }
    let mut output = [0u8; 32];
    for (chunk, hash) in output.chunks_mut(4).zip(hashes) {
        chunk.copy_from_slice(&hash.to_le_bytes());
    }
    output
}

impl AssetMeta {
    /// 路径的显示形式（用于日志）。
    pub fn display_path(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{AudioAsset, ImageAsset};
    use crate::error::AssetError;
    use std::thread::sleep;

    fn sample_wav() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&44100u32.to_le_bytes());
        bytes.extend_from_slice(&88200u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&0i16.to_le_bytes());
        bytes.extend_from_slice(&16384i16.to_le_bytes());
        bytes
    }

    fn temp_root() -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join("asset-tests")
            .join(AssetId::new().as_str());
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn wait_settled(server: &mut AssetServer, id: AssetId) {
        for _ in 0..500 {
            server.poll();
            if server.state(id).is_some_and(|state| state.is_settled()) {
                return;
            }
            sleep(Duration::from_millis(2));
        }
        panic!("资产作业超时");
    }

    #[test]
    fn async_load_returns_ready_audio_without_panic() {
        let root = temp_root();
        std::fs::write(root.join("tone.wav"), sample_wav()).unwrap();
        let mut server = AssetServer::new();
        server.set_root(&root);
        let handle = server.load::<AudioAsset>("tone.wav").unwrap();
        wait_settled(&mut server, handle.id());
        let audio = server.get(handle).expect("音频应加载成功");
        assert_eq!(audio.sample_rate, 44100);
        assert_eq!(audio.samples, vec![0.0, 0.5]);
        assert!(server.report(handle.id()).is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_asset_is_failed_state_not_panic() {
        let root = temp_root();
        let mut server = AssetServer::new();
        server.set_root(&root);
        let handle = server.load::<ImageAsset>("missing.png").unwrap();
        wait_settled(&mut server, handle.id());
        assert!(matches!(
            server.state(handle.id()),
            Some(AssetState::Failed(_))
        ));
        assert!(server.get(handle).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reload_records_latency_under_budget() {
        let root = temp_root();
        std::fs::write(root.join("tone.wav"), sample_wav()).unwrap();
        let mut server = AssetServer::new();
        server.set_root(&root);
        let handle = server.load::<AudioAsset>("tone.wav").unwrap();
        wait_settled(&mut server, handle.id());
        assert!(matches!(server.reload(handle.id()), Ok(())));
        assert!(
            matches!(server.state(handle.id()), Some(AssetState::Pending)),
            "reload 应重新提交作业: {:?}",
            server.state(handle.id())
        );
        wait_settled(&mut server, handle.id());
        let latency = server.reload_latency_ms().expect("应记录热重载耗时");
        assert!(latency < 200.0, "热重载耗时 {latency:.1} ms 超出预算");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn enable_hot_reload_requires_root() {
        let mut server = AssetServer::new();
        assert!(matches!(
            server.enable_hot_reload(),
            Err(AssetError::InvalidPath(_))
        ));
    }
}
