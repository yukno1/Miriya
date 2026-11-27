//! KTX2 解析：校验 identifier，提取 vkFormat/尺寸/mip 层级并透传原始字节。
//!
//! M5 批次 A：补齐 mip levels/layers/faces 与 levelIndex 解析；未压缩 R8G8B8A8
//! （UNORM/SRGB）直接可用于 GPU 上传，压缩格式返回可读错误（压缩上传随 M13 cook）。

use crate::ImageError;

const KTX2_IDENTIFIER: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

/// KTX2 头固定长度（identifier 12 + 头字段 68）。
const KTX2_HEADER_SIZE: usize = 80;
/// levelIndex 单条目长度（byteOffset/byteLength/uncompressedByteLength 各 u64）。
const KTX2_LEVEL_ENTRY_SIZE: usize = 24;

/// `VK_FORMAT_R8G8B8A8_UNORM`。
pub const VK_FORMAT_R8G8B8A8_UNORM: u32 = 37;
/// `VK_FORMAT_R8G8B8A8_SRGB`。
pub const VK_FORMAT_R8G8B8A8_SRGB: u32 = 43;

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

/// 解析 KTX2 头与负载。
///
/// 返回的 `mip_data` 条目为各 mip 层在 `bytes` 内的 `[offset, offset + length)` 范围。
pub fn parse_ktx2(bytes: &[u8]) -> Result<Ktx2Data, ImageError> {
    if bytes.len() < KTX2_HEADER_SIZE {
        return Err(ImageError::Corrupt(format!(
            "KTX2 头部不足 {KTX2_HEADER_SIZE} 字节：实际 {}",
            bytes.len()
        )));
    }
    if bytes[..12] != KTX2_IDENTIFIER {
        return Err(ImageError::Corrupt("KTX2 identifier 不匹配".to_string()));
    }
    let vk_format = read_u32(bytes, 12);
    let pixel_depth = read_u32(bytes, 28);
    let array_layers = read_u32(bytes, 32);
    let face_count = read_u32(bytes, 36);
    let mip_levels = read_u32(bytes, 40);
    let width = read_u32(bytes, 20);
    let height = read_u32(bytes, 24);

    if width == 0 || height == 0 {
        return Err(ImageError::Corrupt("纹理尺寸为 0".to_string()));
    }
    match vk_format {
        0 => {
            return Err(ImageError::Unsupported(
                "vkFormat 为 0（无数据或依赖 supercompressionScheme），暂不支持".to_string(),
            ));
        }
        VK_FORMAT_R8G8B8A8_UNORM | VK_FORMAT_R8G8B8A8_SRGB => {}
        other => {
            return Err(ImageError::Unsupported(format!(
                "不支持的 KTX2 vkFormat {other}（当前仅支持未压缩 R8G8B8A8 UNORM/SRGB）"
            )));
        }
    }
    if pixel_depth > 1 {
        return Err(ImageError::Unsupported(format!(
            "不支持 3D 纹理：pixelDepth = {pixel_depth}"
        )));
    }
    if mip_levels == 0 {
        return Err(ImageError::Corrupt("mip 层数为 0".to_string()));
    }
    if array_layers == 0 {
        return Err(ImageError::Corrupt("array layer 数为 0".to_string()));
    }
    if face_count == 0 || face_count > 6 {
        return Err(ImageError::Corrupt(format!(
            "face 数非法：{face_count}（应为 1 或 6）"
        )));
    }

    let levels_size = (mip_levels as usize)
        .checked_mul(KTX2_LEVEL_ENTRY_SIZE)
        .ok_or_else(|| ImageError::Unsupported("mip 层数溢出".to_string()))?;
    let levels_end = KTX2_HEADER_SIZE
        .checked_add(levels_size)
        .ok_or_else(|| ImageError::Unsupported("mip 层数溢出".to_string()))?;
    if bytes.len() < levels_end {
        return Err(ImageError::Corrupt(format!(
            "levelIndex 越界：需要 {levels_end} 字节，实际 {}",
            bytes.len()
        )));
    }

    let mut mip_data = Vec::with_capacity(mip_levels as usize);
    for level in 0..mip_levels as usize {
        let entry = KTX2_HEADER_SIZE + level * KTX2_LEVEL_ENTRY_SIZE;
        let offset = read_u64(bytes, entry);
        let length = read_u64(bytes, entry + 8);
        let end = offset
            .checked_add(length)
            .ok_or_else(|| ImageError::Corrupt(format!("mip {level} 范围溢出")))?;
        if end > bytes.len() as u64 {
            return Err(ImageError::Corrupt(format!(
                "mip {level} 数据越界：[{offset}, {end}) 超出文件长度 {}",
                bytes.len()
            )));
        }
        mip_data.push(Ktx2Mip { offset, length });
    }

    Ok(Ktx2Data {
        width,
        height,
        vk_format,
        mip_levels,
        array_layers,
        face_count,
        mip_data,
        bytes: bytes.to_vec(),
    })
}

impl Ktx2Data {
    /// 是否为 sRGB 变体（决定 GPU 端是否启用 sRGB 视图）。
    pub fn is_srgb(&self) -> bool {
        self.vk_format == VK_FORMAT_R8G8B8A8_SRGB
    }
}

/// 单个 mip 层在 `Ktx2Data::bytes` 内的字节范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ktx2Mip {
    pub offset: u64,
    pub length: u64,
}

/// KTX2 透传数据（含 M5 批次 A 补齐的层级元数据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ktx2Data {
    pub width: u32,
    pub height: u32,
    pub vk_format: u32,
    pub mip_levels: u32,
    pub array_layers: u32,
    pub face_count: u32,
    /// 各 mip 层在 `bytes` 内的字节范围（长度 == `mip_levels`）。
    pub mip_data: Vec<Ktx2Mip>,
    pub bytes: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造最小合法 KTX2：80 字节头 + levelIndex + 负载。
    fn fixture(vk_format: u32, mip_levels: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; KTX2_HEADER_SIZE + mip_levels as usize * KTX2_LEVEL_ENTRY_SIZE];
        bytes[..12].copy_from_slice(&KTX2_IDENTIFIER);
        bytes[12..16].copy_from_slice(&vk_format.to_le_bytes());
        bytes[16..20].copy_from_slice(&4u32.to_le_bytes()); // typeSize
        bytes[20..24].copy_from_slice(&8u32.to_le_bytes()); // pixelWidth
        bytes[24..28].copy_from_slice(&4u32.to_le_bytes()); // pixelHeight
        bytes[32..36].copy_from_slice(&1u32.to_le_bytes()); // layerCount
        bytes[36..40].copy_from_slice(&1u32.to_le_bytes()); // faceCount
        bytes[40..44].copy_from_slice(&mip_levels.to_le_bytes());
        let mut file = bytes;
        file.extend_from_slice(payload);
        let payload_start =
            KTX2_HEADER_SIZE as u64 + mip_levels as u64 * KTX2_LEVEL_ENTRY_SIZE as u64;
        for level in 0..mip_levels as usize {
            let entry = KTX2_HEADER_SIZE + level * KTX2_LEVEL_ENTRY_SIZE;
            let offset = payload_start + level as u64 * 4;
            let length = 4u64;
            file[entry..entry + 8].copy_from_slice(&offset.to_le_bytes());
            file[entry + 8..entry + 16].copy_from_slice(&length.to_le_bytes());
        }
        file
    }

    #[test]
    fn invalid_identifier_is_rejected() {
        assert!(parse_ktx2(&[0u8; 32]).is_err());
    }

    #[test]
    fn truncated_header_is_rejected() {
        let mut bytes = vec![0u8; KTX2_HEADER_SIZE - 1];
        bytes[..12].copy_from_slice(&KTX2_IDENTIFIER);
        let err = parse_ktx2(&bytes).unwrap_err();
        assert!(matches!(err, ImageError::Corrupt(msg) if msg.contains("80")));
    }

    #[test]
    fn header_fields_are_extracted() {
        let parsed = parse_ktx2(&fixture(VK_FORMAT_R8G8B8A8_UNORM, 1, &[1, 2, 3, 4])).unwrap();
        assert_eq!(parsed.vk_format, 37);
        assert_eq!((parsed.width, parsed.height), (8, 4));
        assert_eq!(
            (parsed.mip_levels, parsed.array_layers, parsed.face_count),
            (1, 1, 1)
        );
        assert_eq!(parsed.mip_data.len(), 1);
        let mip = &parsed.mip_data[0];
        let start = mip.offset as usize;
        assert_eq!(
            &parsed.bytes[start..start + mip.length as usize],
            &[1, 2, 3, 4]
        );
        assert!(!parsed.is_srgb());
    }

    #[test]
    fn multi_mip_ranges_are_parsed() {
        let parsed = parse_ktx2(&fixture(VK_FORMAT_R8G8B8A8_UNORM, 2, &[0; 8])).unwrap();
        assert_eq!(parsed.mip_data.len(), 2);
        let payload_start = (KTX2_HEADER_SIZE + 2 * KTX2_LEVEL_ENTRY_SIZE) as u64;
        assert_eq!(parsed.mip_data[0].offset, payload_start);
        assert_eq!(parsed.mip_data[1].offset, payload_start + 4);
    }

    #[test]
    fn srgb_variant_is_detected() {
        let parsed = parse_ktx2(&fixture(VK_FORMAT_R8G8B8A8_SRGB, 1, &[0; 4])).unwrap();
        assert!(parsed.is_srgb());
    }

    #[test]
    fn unsupported_vk_format_is_rejected() {
        // 压缩格式（如 BC1 = 131）暂不支持，错误信息可读。
        let err = parse_ktx2(&fixture(131, 1, &[0; 4])).unwrap_err();
        assert!(matches!(err, ImageError::Unsupported(msg) if msg.contains("131")));
    }

    #[test]
    fn zero_vk_format_is_rejected() {
        let err = parse_ktx2(&fixture(0, 1, &[0; 4])).unwrap_err();
        assert!(matches!(err, ImageError::Unsupported(_)));
    }

    #[test]
    fn zero_mip_levels_is_rejected() {
        let err = parse_ktx2(&fixture(VK_FORMAT_R8G8B8A8_UNORM, 0, &[])).unwrap_err();
        assert!(matches!(err, ImageError::Corrupt(msg) if msg.contains("mip")));
    }

    #[test]
    fn invalid_face_count_is_rejected() {
        let mut bytes = fixture(VK_FORMAT_R8G8B8A8_UNORM, 1, &[0; 4]);
        bytes[36..40].copy_from_slice(&7u32.to_le_bytes());
        let err = parse_ktx2(&bytes).unwrap_err();
        assert!(matches!(err, ImageError::Corrupt(msg) if msg.contains("face")));
    }

    #[test]
    fn mip_index_out_of_range_is_rejected() {
        let mut bytes = fixture(VK_FORMAT_R8G8B8A8_UNORM, 1, &[0; 4]);
        // levelIndex[0].byteLength 超出文件长度。
        bytes[KTX2_HEADER_SIZE + 8..KTX2_HEADER_SIZE + 16].copy_from_slice(&4096u64.to_le_bytes());
        let err = parse_ktx2(&bytes).unwrap_err();
        assert!(matches!(err, ImageError::Corrupt(msg) if msg.contains("越界")));
    }

    #[test]
    fn truncated_level_index_is_rejected() {
        // 头部合法但 levelIndex 被截断。
        let mut bytes = vec![0u8; KTX2_HEADER_SIZE + KTX2_LEVEL_ENTRY_SIZE - 4];
        bytes[..12].copy_from_slice(&KTX2_IDENTIFIER);
        bytes[12..16].copy_from_slice(&VK_FORMAT_R8G8B8A8_UNORM.to_le_bytes());
        bytes[20..24].copy_from_slice(&8u32.to_le_bytes());
        bytes[24..28].copy_from_slice(&4u32.to_le_bytes());
        bytes[32..36].copy_from_slice(&1u32.to_le_bytes());
        bytes[36..40].copy_from_slice(&1u32.to_le_bytes());
        bytes[40..44].copy_from_slice(&1u32.to_le_bytes());
        let err = parse_ktx2(&bytes).unwrap_err();
        assert!(matches!(err, ImageError::Corrupt(msg) if msg.contains("levelIndex")));
    }
}
