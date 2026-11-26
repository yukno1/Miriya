use image::ImageFormat;

use crate::{ImageData, ImageError};

/// 把 PNG 字节解码为 RGBA8。
pub fn decode_png(bytes: &[u8]) -> Result<ImageData, ImageError> {
    let decoded = image::load_from_memory_with_format(bytes, ImageFormat::Png)
        .map_err(|error| ImageError::Decode(error.to_string()))?
        .to_rgba8();
    let (width, height) = decoded.dimensions();
    ImageData::new(width, height, decoded.into_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_png_is_error() {
        assert!(matches!(decode_png(b"not png"), Err(ImageError::Decode(_))));
    }
}
