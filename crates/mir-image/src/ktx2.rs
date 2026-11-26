//! KTX2 最小解析：校验 identifier，提取 vkFormat/尺寸并透传原始字节。
//!
//! 压缩纹理的实际 GPU 上传需要 M5 纹理绑定支持；M4 只保证元数据与数据完整性可测试。

use crate::ImageError;

const KTX2_IDENTIFIER: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// 解析 KTX2 头与负载。
pub fn parse_ktx2(bytes: &[u8]) -> Result<Ktx2Data, ImageError> {
    if bytes.len() < 28 {
        return Err(ImageError::Corrupt("头部不足 28 字节".to_string()));
    }
    if bytes[..12] != KTX2_IDENTIFIER {
        return Err(ImageError::Corrupt("KTX2 identifier 不匹配".to_string()));
    }
    let vk_format = read_u32(bytes, 12);
    let width = read_u32(bytes, 20);
    let height = read_u32(bytes, 24);
    if width == 0 || height == 0 {
        return Err(ImageError::Corrupt("纹理尺寸为 0".to_string()));
    }
    Ok(Ktx2Data {
        width,
        height,
        vk_format,
        bytes: bytes.to_vec(),
    })
}

/// KTX2 透传数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ktx2Data {
    pub width: u32,
    pub height: u32,
    pub vk_format: u32,
    pub bytes: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_identifier_is_rejected() {
        assert!(parse_ktx2(&[0u8; 32]).is_err());
    }

    #[test]
    fn header_fields_are_extracted() {
        let mut bytes = vec![0u8; 32];
        bytes[..12].copy_from_slice(&KTX2_IDENTIFIER);
        bytes[12..16].copy_from_slice(&37u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&8u32.to_le_bytes());
        bytes[24..28].copy_from_slice(&4u32.to_le_bytes());
        let parsed = parse_ktx2(&bytes).unwrap();
        assert_eq!(parsed.vk_format, 37);
        assert_eq!((parsed.width, parsed.height), (8, 4));
    }
}
