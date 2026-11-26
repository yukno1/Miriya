//! PNG 解码与 KTX2 元信息解析（M4）。
//!
//! 只产出 CPU 侧纯数据；GPU 纹理创建留待 M5。

mod ktx2;
mod png;

pub use ktx2::{Ktx2Data, parse_ktx2};
pub use png::decode_png;

use thiserror::Error;

/// 图像解码错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ImageError {
    /// PNG 解码失败。
    #[error("PNG 解码失败：{0}")]
    Decode(String),
    /// KTX2 数据损坏或不支持。
    #[error("KTX2 数据无效：{0}")]
    Corrupt(String),
    /// 不支持的图像格式或特性。
    #[error("不支持的图像格式：{0}")]
    Unsupported(String),
}

/// CPU 侧 RGBA8 图像。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl ImageData {
    /// 构造图像数据，校验像素缓冲区长度。
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, ImageError> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| ImageError::Unsupported("图像尺寸溢出".to_string()))?;
        if rgba.len() != expected {
            return Err(ImageError::Corrupt(format!(
                "RGBA 长度不匹配：期望 {expected}，实际 {}",
                rgba.len()
            )));
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }
}
