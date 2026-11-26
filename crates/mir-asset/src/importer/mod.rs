//! 导入器契约与按类型分派（M4）。

pub mod audio;
pub mod gltf;

use std::path::{Path, PathBuf};

use crate::asset::NativeAsset;
use crate::error::{AssetError, Result};

/// 导入报告：记录转换节点与可诊断警告，供 CLI/编辑器展示。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub source: PathBuf,
    pub type_tag: String,
    pub bytes: usize,
    /// 发生坐标/单位转换的节点或网格名。
    pub transformed_nodes: Vec<String>,
    pub warnings: Vec<String>,
}

/// 导入器：把源资产字节转成 CPU 侧纯数据。
pub trait Importer: Send + Sync {
    fn type_tag(&self) -> &'static str;
    fn import(&self, bytes: &[u8], path: &Path, report: &mut ImportReport) -> Result<NativeAsset>;
}

/// PNG → RGBA8。
pub struct PngImporter;

impl Importer for PngImporter {
    fn type_tag(&self) -> &'static str {
        "image"
    }

    fn import(
        &self,
        bytes: &[u8],
        _path: &Path,
        _report: &mut ImportReport,
    ) -> Result<NativeAsset> {
        let image = mir_image::decode_png(bytes)
            .map_err(|error| AssetError::Decode(format!("PNG: {error}")))?;
        Ok(NativeAsset::Image(image))
    }
}

/// KTX2 → 元信息 + 原样负载。
pub struct Ktx2Importer;

impl Importer for Ktx2Importer {
    fn type_tag(&self) -> &'static str {
        "ktx2"
    }

    fn import(
        &self,
        bytes: &[u8],
        _path: &Path,
        _report: &mut ImportReport,
    ) -> Result<NativeAsset> {
        let ktx2 = mir_image::parse_ktx2(bytes)
            .map_err(|error| AssetError::Corrupt(format!("KTX2: {error}")))?;
        Ok(NativeAsset::Ktx2(ktx2))
    }
}

/// 按类型标签分派导入器。
pub fn import_by_tag(
    type_tag: &str,
    bytes: &[u8],
    path: &Path,
    report: &mut ImportReport,
) -> Result<NativeAsset> {
    match type_tag {
        "mesh" => gltf::GltfImporter.import(bytes, path, report),
        "material" => gltf::GltfMaterialImporter.import(bytes, path, report),
        "image" => PngImporter.import(bytes, path, report),
        "ktx2" => Ktx2Importer.import(bytes, path, report),
        "audio" => audio::AudioImporter.import(bytes, path, report),
        "scene" => Ok(NativeAsset::Scene(bytes.to_vec())),
        other => Err(AssetError::Unsupported(format!("未知资产类型：{other}"))),
    }
}

/// 由文件扩展名推断类型标签。
pub fn type_tag_from_path(path: &Path) -> Result<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "glb" | "gltf" => Ok("mesh"),
        "png" => Ok("image"),
        "ktx2" => Ok("ktx2"),
        "wav" | "ogg" => Ok("audio"),
        "mimap" => Ok("scene"),
        "miasset" => Err(AssetError::Unsupported(
            ".miasset 需要注册表类型标签才能解码".to_string(),
        )),
        other => Err(AssetError::Unsupported(format!(
            "不支持的资产扩展名：{other}"
        ))),
    }
}
