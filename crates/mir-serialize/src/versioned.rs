//! 版本化快照编解码：magic + version + schema hash + postcard 负载。

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

/// 当前快照格式版本。
pub const SNAPSHOT_VERSION: u16 = 1;

/// Miriya 快照 magic：`MIRS`。
pub const SNAPSHOT_MAGIC: [u8; 4] = *b"MIRS";

/// 快照编解码错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SerializeError {
    /// 输入不是 Miriya 快照。
    #[error("快照 magic 错误：期望 MIRS，实际 {0:?}")]
    BadMagic([u8; 4]),
    /// 快照版本不匹配。
    #[error("快照版本不匹配：期望 {expected}，实际 {actual}")]
    VersionMismatch { expected: u16, actual: u16 },
    /// schema hash 不匹配（字段布局已变化）。
    #[error("快照 schema 不匹配：期望 {expected:08x}，实际 {actual:08x}")]
    SchemaMismatch { expected: u32, actual: u32 },
    /// 头部或负载被截断/损坏。
    #[error("快照数据损坏：{0}")]
    Corrupt(String),
    /// postcard 编码失败。
    #[error("快照编码失败：{0}")]
    Encode(String),
    /// postcard 解码失败。
    #[error("快照解码失败：{0}")]
    Decode(String),
}

/// 便捷结果类型。
pub type Result<T> = std::result::Result<T, SerializeError>;

/// 快照头。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotHeader {
    pub magic: [u8; 4],
    pub version: u16,
    pub schema_hash: u32,
}

impl SnapshotHeader {
    /// 构造当前版本的头。
    pub const fn current(schema_hash: u32) -> Self {
        Self {
            magic: SNAPSHOT_MAGIC,
            version: SNAPSHOT_VERSION,
            schema_hash,
        }
    }

    /// 校验 magic、版本与 schema hash。
    pub fn validate(&self, expected_schema_hash: u32) -> Result<()> {
        if self.magic != SNAPSHOT_MAGIC {
            return Err(SerializeError::BadMagic(self.magic));
        }
        if self.version != SNAPSHOT_VERSION {
            return Err(SerializeError::VersionMismatch {
                expected: SNAPSHOT_VERSION,
                actual: self.version,
            });
        }
        if self.schema_hash != expected_schema_hash {
            return Err(SerializeError::SchemaMismatch {
                expected: expected_schema_hash,
                actual: self.schema_hash,
            });
        }
        Ok(())
    }
}

/// 用指定版本编码（测试版本不匹配时使用）。
pub fn encode_with_version<T: Serialize>(
    value: &T,
    schema_hash: u32,
    version: u16,
) -> Result<Vec<u8>> {
    let header = SnapshotHeader {
        magic: SNAPSHOT_MAGIC,
        version,
        schema_hash,
    };
    let mut bytes =
        postcard::to_allocvec(&header).map_err(|e| SerializeError::Encode(e.to_string()))?;
    let payload =
        postcard::to_allocvec(value).map_err(|e| SerializeError::Encode(e.to_string()))?;
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

/// 用当前版本编码快照。
pub fn encode<T: Serialize>(value: &T, schema_hash: u32) -> Result<Vec<u8>> {
    encode_with_version(value, schema_hash, SNAPSHOT_VERSION)
}

/// 校验头并解码快照负载。
pub fn decode<T: DeserializeOwned>(bytes: &[u8], schema_hash: u32) -> Result<T> {
    let (header, payload): (SnapshotHeader, &[u8]) =
        postcard::take_from_bytes(bytes).map_err(|e| SerializeError::Corrupt(e.to_string()))?;
    header.validate(schema_hash)?;
    postcard::from_bytes(payload).map_err(|e| SerializeError::Decode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Payload {
        value: u32,
    }

    #[test]
    fn roundtrip_and_version_errors() {
        const SCHEMA: u32 = 0x1234_5678;
        let bytes = encode(&Payload { value: 42 }, SCHEMA).unwrap();
        let decoded: Payload = decode(&bytes, SCHEMA).unwrap();
        assert_eq!(decoded.value, 42);

        let wrong_version = encode_with_version(&Payload { value: 1 }, SCHEMA, 99).unwrap();
        assert!(matches!(
            decode::<Payload>(&wrong_version, SCHEMA),
            Err(SerializeError::VersionMismatch { .. })
        ));

        assert!(matches!(
            decode::<Payload>(&bytes, SCHEMA.wrapping_add(1)),
            Err(SerializeError::SchemaMismatch { .. })
        ));

        let truncated = &bytes[..bytes.len().saturating_sub(2)];
        assert!(matches!(
            decode::<Payload>(truncated, SCHEMA),
            Err(SerializeError::Corrupt(_)) | Err(SerializeError::Decode(_))
        ));
    }
}
