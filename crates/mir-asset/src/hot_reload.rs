//! `notify` 文件监听：事件队列供 AssetServer 主线程 poll（ADR-0013）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use notify::{RecommendedWatcher, RecursiveMode, Watcher, recommended_watcher};

use crate::error::{AssetError, Result};

/// 文件监听句柄与去重事件队列。
pub struct HotReloadWatcher {
    _watcher: RecommendedWatcher,
    events: Arc<Mutex<Vec<PathBuf>>>,
}

impl HotReloadWatcher {
    /// 递归监听资产根目录。
    pub fn new(root: &Path) -> Result<Self> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let mut watcher =
            recommended_watcher(move |result: notify::Result<notify::Event>| match result {
                Ok(event)
                    if matches!(
                        event.kind,
                        notify::EventKind::Modify(_) | notify::EventKind::Create(_)
                    ) =>
                {
                    if let Ok(mut queue) = sink.lock() {
                        for path in event.paths {
                            if !queue.contains(&path) {
                                queue.push(path);
                            }
                        }
                    }
                }
                _ => {}
            })
            .map_err(|error| AssetError::Watch(error.to_string()))?;
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|error| AssetError::Watch(error.to_string()))?;
        Ok(Self {
            _watcher: watcher,
            events,
        })
    }

    /// 取出并清空自上次 poll 以来的变更路径。
    pub fn poll_events(&self) -> Vec<PathBuf> {
        match self.events.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(_) => Vec::new(),
        }
    }
}
