//! 纹理纯数据与句柄（M5 批次 A，R-01）。
//!
//! 渲染层只接受纯数据（ADR-0012）：应用层从 `mir-image` 的 `ImageData`（RGBA8 单 mip）
//! 或 `Ktx2Data`（多 mip，含层级元数据）转换为 [`TextureData`] 后调用
//! `RenderContext::create_texture`；GPU 上传细节见 `vulkan::texture`。
//!
//! [`ImageFormat`] 是全 crate 唯一的图像格式来源（RenderGraph transient 声明
//! 与纹理上传共用，避免双枚举漂移）。

use std::fmt;

use thiserror::Error;

/// 纹理槽位上限（两后端共享）。
pub(crate) const MAX_TEXTURES: usize = 128;

/// 图像格式（纯数据枚举，覆盖 M5 实际用到的未压缩格式）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    /// 8 位 RGBA，线性空间（贴图默认）。
    R8G8B8A8Unorm,
    /// 8 位 RGBA，sRGB 传输函数（albedo 常用）。
    R8G8B8A8Srgb,
    /// 11-11-10 位打包浮点（HDR 渲染目标常用）。
    B10G11R11Ufloat,
    /// 16 位 RGBA 浮点（HDR 渲染目标）。
    R16G16B16A16Sfloat,
    /// 32 位单通道浮点。
    R32Sfloat,
    /// 32 位深度。
    D32Sfloat,
}

impl fmt::Display for ImageFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::R8G8B8A8Unorm => "R8G8B8A8_UNORM",
            Self::R8G8B8A8Srgb => "R8G8B8A8_SRGB",
            Self::B10G11R11Ufloat => "B10G11R11_UFLOAT_PACK32",
            Self::R16G16B16A16Sfloat => "R16G16B16A16_SFLOAT",
            Self::R32Sfloat => "R32_SFLOAT",
            Self::D32Sfloat => "D32_SFLOAT",
        };
        formatter.write_str(name)
    }
}

impl ImageFormat {
    /// 每个 texel 的字节数（上述格式均无块压缩，texel 尺寸固定）。
    pub fn bytes_per_texel(self) -> usize {
        match self {
            Self::R8G8B8A8Unorm | Self::R8G8B8A8Srgb => 4,
            Self::B10G11R11Ufloat => 4,
            Self::R16G16B16A16Sfloat => 8,
            Self::R32Sfloat => 4,
            Self::D32Sfloat => 4,
        }
    }
}

/// 单个 mip 层在 [`TextureData::bytes`] 内的字节范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MipRange {
    pub offset: usize,
    pub length: usize,
}

/// 纹理纯数据：格式 + 尺寸 + mip 链字节缓冲。
///
/// 构造时校验 mip 布局（层级尺寸按 `max(1, dim >> i)` 推导），避免越界 copy
/// 在 GPU 路径上触发校验层 VUID。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureData {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    /// mip 层范围，`mip_data[i]` 对应 mip level `i`（长度 == mip 层数）。
    pub mip_data: Vec<MipRange>,
    pub bytes: Vec<u8>,
}

/// 纹理数据校验错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TextureDataError {
    #[error("纹理尺寸非法：{width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("mip 层数 {count} 超过该尺寸上限 {max}")]
    TooManyMips { count: usize, max: usize },
    #[error("mip {level} 字节数不匹配：期望 {expected}，实际 {actual}")]
    MipSizeMismatch {
        level: usize,
        expected: usize,
        actual: usize,
    },
    #[error("mip {level} 数据范围 [{offset}, {end}) 越界（总长 {total}）")]
    MipOutOfRange {
        level: usize,
        offset: usize,
        end: usize,
        total: usize,
    },
}

impl TextureData {
    /// 构造并校验纹理数据。
    pub fn new(
        format: ImageFormat,
        width: u32,
        height: u32,
        mip_data: Vec<MipRange>,
        bytes: Vec<u8>,
    ) -> Result<Self, TextureDataError> {
        if width == 0 || height == 0 {
            return Err(TextureDataError::InvalidDimensions { width, height });
        }
        let max_mips = mip_level_count(width, height);
        if mip_data.is_empty() {
            return Err(TextureDataError::TooManyMips {
                count: 0,
                max: max_mips,
            });
        }
        if mip_data.len() > max_mips {
            return Err(TextureDataError::TooManyMips {
                count: mip_data.len(),
                max: max_mips,
            });
        }
        let texel = format.bytes_per_texel();
        for (level, mip) in mip_data.iter().enumerate() {
            let level_width = mip_dimension(width, level);
            let level_height = mip_dimension(height, level);
            let expected = (level_width as usize)
                .checked_mul(level_height as usize)
                .and_then(|pixels| pixels.checked_mul(texel))
                .expect("mip 尺寸在 u32 范围内不会溢出 usize");
            if mip.length != expected {
                return Err(TextureDataError::MipSizeMismatch {
                    level,
                    expected,
                    actual: mip.length,
                });
            }
            let end = mip.offset + mip.length;
            if end > bytes.len() {
                return Err(TextureDataError::MipOutOfRange {
                    level,
                    offset: mip.offset,
                    end,
                    total: bytes.len(),
                });
            }
        }
        Ok(Self {
            format,
            width,
            height,
            mip_data,
            bytes,
        })
    }

    /// 单 mip RGBA8 便捷构造（PNG/`ImageData` 路径）。
    pub fn from_rgba8(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, TextureDataError> {
        let length = rgba.len();
        Self::new(
            ImageFormat::R8G8B8A8Unorm,
            width,
            height,
            vec![MipRange { offset: 0, length }],
            rgba,
        )
    }

    /// mip 层数。
    pub fn mip_levels(&self) -> u32 {
        self.mip_data.len() as u32
    }
}

/// mip level `i` 的维度：`max(1, dim >> i)`。
pub(crate) fn mip_dimension(dimension: u32, level: usize) -> u32 {
    dimension.checked_shr(level as u32).unwrap_or(0).max(1)
}

/// 给定尺寸的完整 mip 链层数：`floor(log2(max(w, h))) + 1`。
pub(crate) fn mip_level_count(width: u32, height: u32) -> usize {
    let max = width.max(height);
    32 - max.leading_zeros() as usize
}

/// 纹理句柄：不透明、可复制；内部编码槽位与世代号，销毁后旧句柄自动失效
/// （风格对齐 [`crate::MaterialHandle`] / [`crate::MeshHandle`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureHandle(u64);

impl TextureHandle {
    pub(crate) fn from_parts(index: u32, generation: u32) -> Self {
        Self(((generation as u64) << 32) | index as u64)
    }

    pub(crate) fn index(self) -> usize {
        (self.0 & 0xFFFF_FFFF) as usize
    }

    pub(crate) fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_mip_rgba8_is_accepted() {
        let data = TextureData::from_rgba8(4, 4, vec![0u8; 64]).unwrap();
        assert_eq!(data.format, ImageFormat::R8G8B8A8Unorm);
        assert_eq!(data.mip_levels(), 1);
        assert_eq!(data.mip_data[0].length, 64);
    }

    #[test]
    fn mip_chain_dimensions_match_expected_sizes() {
        // 4x4 RGBA8 完整链：4x4 → 2x2 → 1x1（64 + 16 + 4 字节）。
        let bytes = vec![0u8; 64 + 16 + 4];
        let mips = vec![
            MipRange {
                offset: 0,
                length: 64,
            },
            MipRange {
                offset: 64,
                length: 16,
            },
            MipRange {
                offset: 80,
                length: 4,
            },
        ];
        let data = TextureData::new(ImageFormat::R8G8B8A8Unorm, 4, 4, mips, bytes).unwrap();
        assert_eq!(data.mip_levels(), 3);
    }

    #[test]
    fn zero_dimensions_are_rejected() {
        let err = TextureData::from_rgba8(0, 4, Vec::new()).unwrap_err();
        assert!(matches!(err, TextureDataError::InvalidDimensions { .. }));
    }

    #[test]
    fn empty_mip_list_is_rejected() {
        let err =
            TextureData::new(ImageFormat::R32Sfloat, 4, 4, Vec::new(), Vec::new()).unwrap_err();
        assert!(matches!(
            err,
            TextureDataError::TooManyMips { count: 0, .. }
        ));
    }

    #[test]
    fn mip_size_mismatch_is_rejected() {
        let err = TextureData::new(
            ImageFormat::R8G8B8A8Unorm,
            4,
            4,
            vec![MipRange {
                offset: 0,
                length: 32,
            }],
            vec![0u8; 32],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            TextureDataError::MipSizeMismatch {
                level: 0,
                expected: 64,
                actual: 32
            }
        ));
    }

    #[test]
    fn mip_out_of_range_is_rejected() {
        let err = TextureData::new(
            ImageFormat::R8G8B8A8Unorm,
            4,
            4,
            vec![MipRange {
                offset: 60,
                length: 64,
            }],
            vec![0u8; 64],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            TextureDataError::MipOutOfRange {
                level: 0,
                offset: 60,
                end: 124,
                total: 64
            }
        ));
    }

    #[test]
    fn too_many_mips_are_rejected() {
        // 1x1 只允许 1 层 mip。
        let err = TextureData::new(
            ImageFormat::R8G8B8A8Unorm,
            1,
            1,
            vec![
                MipRange {
                    offset: 0,
                    length: 4,
                },
                MipRange {
                    offset: 0,
                    length: 4,
                },
            ],
            vec![0u8; 4],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            TextureDataError::TooManyMips { count: 2, max: 1 }
        ));
    }

    #[test]
    fn bytes_per_texel_matches_format_width() {
        assert_eq!(ImageFormat::R8G8B8A8Unorm.bytes_per_texel(), 4);
        assert_eq!(ImageFormat::R8G8B8A8Srgb.bytes_per_texel(), 4);
        assert_eq!(ImageFormat::B10G11R11Ufloat.bytes_per_texel(), 4);
        assert_eq!(ImageFormat::R16G16B16A16Sfloat.bytes_per_texel(), 8);
        assert_eq!(ImageFormat::R32Sfloat.bytes_per_texel(), 4);
        assert_eq!(ImageFormat::D32Sfloat.bytes_per_texel(), 4);
    }

    #[test]
    fn handle_encodes_index_and_generation() {
        let handle = TextureHandle::from_parts(7, 5);
        assert_eq!(handle.index(), 7);
        assert_eq!(handle.generation(), 5);
        assert_ne!(handle, TextureHandle::from_parts(7, 6));
        assert_ne!(handle, TextureHandle::from_parts(8, 5));
    }

    #[test]
    fn mip_helpers_match_full_chain() {
        assert_eq!(mip_level_count(1920, 1080), 11);
        assert_eq!(mip_level_count(1, 1), 1);
        assert_eq!(mip_dimension(1080, 1), 540);
        assert_eq!(mip_dimension(1080, 20), 1);
    }
}
