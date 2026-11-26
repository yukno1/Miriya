//! 资产系统领域错误：所有解析路径返回 Result，不 panic（ADR-0013）。

use std::path::PathBuf;

use thiserror::Error;

use crate::AssetId;

/// 资产系统错误。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AssetError {
    /// 资产未注册或未加载。
    #[error("资产不存在：{0}")]
    NotFound(AssetId),
    /// 路径非法（绝对路径、空路径或越出根目录）。
    #[error("非法资产路径：{0}")]
    InvalidPath(PathBuf),
    /// 文件 IO 失败。
    #[error("资产 IO 失败：{path}：{message}")]
    Io { path: PathBuf, message: String },
    /// 容器或输入数据损坏。
    #[error("资产损坏：{0}")]
    Corrupt(String),
    /// 版本或 schema 不匹配。
    #[error("资产版本不匹配：期望 {expected}，实际 {actual}")]
    Version { expected: u16, actual: u16 },
    /// 解码失败。
    #[error("资产解码失败：{0}")]
    Decode(String),
    /// 文件监听失败。
    #[error("资产监听失败：{0}")]
    Watch(String),
    /// 不支持的格式或特性。
    #[error("不支持的资产格式：{0}")]
    Unsupported(String),
    /// 依赖图出现循环。
    #[error("资产依赖循环：{0}")]
    Cycle(AssetId),
    /// 依赖未注册。
    #[error("资产依赖缺失：{0}")]
    MissingDependency(AssetId),
    /// 类型标签不匹配。
    #[error("资产类型不匹配：期望 {expected}，实际 {actual}")]
    TypeMismatch {
        expected: &'static str,
        actual: String,
    },
    /// 操作忙或状态非法。
    #[error("资产操作冲突：{0}")]
    Busy(String),
}

/// 便捷结果类型。
pub type Result<T> = std::result::Result<T, AssetError>;
