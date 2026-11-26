//! `.miasset` 原生容器：版本头 + 64 KiB 分块 zstd + checksum（ADR-0013）。

use serde::{Deserialize, Serialize};

use crate::error::{AssetError, Result};

const MAGIC: [u8; 4] = *b"MIRI";
const VERSION: u16 = 1;
const SCHEMA_HASH: u32 = 0x6d69_4153; // "miAS"
const BLOCK_SIZE: usize = 64 * 1024;
const COMPRESSION_LEVEL: i32 = 3;

/// 容器头。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerHeader {
    pub magic: [u8; 4],
    pub version: u16,
    pub schema_hash: u32,
    pub block_count: u32,
    pub uncompressed_len: u64,
    /// 未压缩负载的 FNV-1a 校验和。
    pub checksum: u32,
}

impl ContainerHeader {
    /// 当前版本头。
    pub const fn current(block_count: u32, uncompressed_len: u64, checksum: u32) -> Self {
        Self {
            magic: MAGIC,
            version: VERSION,
            schema_hash: SCHEMA_HASH,
            block_count,
            uncompressed_len,
            checksum,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.magic != MAGIC {
            return Err(AssetError::Corrupt("miasset magic 不匹配".to_string()));
        }
        if self.version != VERSION {
            return Err(AssetError::Version {
                expected: VERSION,
                actual: self.version,
            });
        }
        if self.schema_hash != SCHEMA_HASH {
            return Err(AssetError::Corrupt(format!(
                "miasset schema 不匹配：{}",
                self.schema_hash
            )));
        }
        Ok(())
    }
}

fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5_u32;
    for byte in bytes {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

fn encode_impl(payload: &[u8], version: u16) -> Result<Vec<u8>> {
    let mut compressed = Vec::new();
    for chunk in payload.chunks(BLOCK_SIZE) {
        let block = zstd::bulk::compress(chunk, COMPRESSION_LEVEL)
            .map_err(|error| AssetError::Decode(format!("zstd 压缩失败：{error}")))?;
        compressed.push(block);
    }
    let header = ContainerHeader {
        version,
        ..ContainerHeader::current(
            compressed.len() as u32,
            payload.len() as u64,
            fnv1a(payload),
        )
    };
    let mut bytes = postcard::to_allocvec(&header)
        .map_err(|error| AssetError::Corrupt(format!("容器头编码失败：{error}")))?;
    for block in compressed {
        bytes.extend_from_slice(&(block.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&block);
    }
    Ok(bytes)
}

/// 用当前版本编码容器。
pub fn encode(payload: &[u8]) -> Result<Vec<u8>> {
    encode_impl(payload, VERSION)
}

/// 用指定版本编码（版本不匹配负例测试用）。
pub fn encode_with_version(payload: &[u8], version: u16) -> Result<Vec<u8>> {
    encode_impl(payload, version)
}

/// 校验并解码容器。
pub fn decode(bytes: &[u8]) -> Result<Vec<u8>> {
    let (header, mut rest): (ContainerHeader, &[u8]) = postcard::take_from_bytes(bytes)
        .map_err(|error| AssetError::Corrupt(format!("容器头损坏：{error}")))?;
    header.validate()?;

    let mut payload = Vec::with_capacity(header.uncompressed_len as usize);
    for _ in 0..header.block_count {
        if rest.len() < 4 {
            return Err(AssetError::Corrupt("块长度字段被截断".to_string()));
        }
        let length = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        rest = &rest[4..];
        if rest.len() < length {
            return Err(AssetError::Corrupt("压缩块被截断".to_string()));
        }
        let block = zstd::bulk::decompress(&rest[..length], BLOCK_SIZE)
            .map_err(|error| AssetError::Decode(format!("zstd 解压失败：{error}")))?;
        payload.extend_from_slice(&block);
        rest = &rest[length..];
    }

    if payload.len() as u64 != header.uncompressed_len {
        return Err(AssetError::Corrupt(format!(
            "解压缩长度不匹配：期望 {}，实际 {}",
            header.uncompressed_len,
            payload.len()
        )));
    }
    if fnv1a(&payload) != header.checksum {
        return Err(AssetError::Corrupt("checksum 不匹配".to_string()));
    }
    if !rest.is_empty() {
        return Err(AssetError::Corrupt("容器尾部存在多余字节".to_string()));
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_multiple_blocks() {
        let payload: Vec<u8> = (0..BLOCK_SIZE * 2 + 123).map(|index| index as u8).collect();
        let bytes = encode(&payload).unwrap();
        assert_eq!(decode(&bytes).unwrap(), payload);
        assert!(bytes.len() < payload.len(), "可压缩数据应明显变小");
    }

    #[test]
    fn truncated_and_checksum_corruption_are_errors() {
        let bytes = encode(b"hello miasset").unwrap();
        let truncated = &bytes[..bytes.len() - 1];
        assert!(matches!(decode(truncated), Err(AssetError::Corrupt(_))));

        let mut corrupted = bytes.clone();
        if let Some(last) = corrupted.last_mut() {
            *last ^= 0xFF;
        }
        assert!(matches!(decode(&corrupted), Err(AssetError::Corrupt(_))));
    }

    #[test]
    fn version_mismatch_is_error() {
        let bytes = encode_with_version(b"payload", 999).unwrap();
        assert!(matches!(
            decode(&bytes),
            Err(AssetError::Version {
                expected: 1,
                actual: 999
            })
        ));
    }
}
