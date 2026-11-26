//! Miriya 版本化序列化：为场景/资产快照提供稳定头与可诊断错误。
//!
//! 负载格式由调用方定义（`SceneSnapshot` 等），本 crate 只负责版本校验与 postcard 编解码。

mod versioned;

pub use versioned::{
    Result, SNAPSHOT_MAGIC, SNAPSHOT_VERSION, SerializeError, SnapshotHeader, decode, encode,
    encode_with_version,
};
