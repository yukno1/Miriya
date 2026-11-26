//! glTF（GLB）导入：网格坐标/单位转换集中在 `mir-math::interop`。

use std::path::Path;

use mir_math::interop::{gltf_direction_to_runtime, gltf_position_to_runtime};
use mir_render::{MaterialData, MeshData, Vertex};

use crate::asset::NativeAsset;
use crate::error::{AssetError, Result};
use crate::importer::{ImportReport, Importer};

fn get_blob(bytes: &[u8]) -> Result<gltf::Gltf> {
    gltf::Gltf::from_slice(bytes).map_err(|error| AssetError::Decode(format!("glTF: {error}")))
}

fn first_material(gltf: &gltf::Gltf) -> Option<MaterialData> {
    gltf.meshes()
        .flat_map(|mesh| mesh.primitives())
        .filter_map(|primitive| {
            let material = primitive.material();
            material.index().map(|_| material)
        })
        .next()
        .map(|material| {
            let pbr = material.pbr_metallic_roughness();
            MaterialData::new(
                pbr.base_color_factor(),
                pbr.metallic_factor(),
                pbr.roughness_factor(),
                material.emissive_factor(),
            )
        })
}

/// 导入 GLB 网格（所有 primitive 合并为一个 MeshData）。
pub struct GltfImporter;

impl Importer for GltfImporter {
    fn type_tag(&self) -> &'static str {
        "mesh"
    }

    fn import(&self, bytes: &[u8], _path: &Path, report: &mut ImportReport) -> Result<NativeAsset> {
        let gltf = get_blob(bytes)?;
        let blob = gltf.blob.as_deref().ok_or_else(|| {
            AssetError::Unsupported("glTF 缺少内嵌 BIN 数据（仅支持 GLB）".to_string())
        })?;
        let get_buffer_data = move |_buffer: gltf::Buffer<'_>| -> Option<&[u8]> { Some(blob) };

        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        for mesh in gltf.meshes() {
            for primitive in mesh.primitives() {
                let reader = primitive.reader(get_buffer_data);
                let Some(positions) = reader.read_positions() else {
                    continue;
                };
                let mut normals = reader.read_normals();
                let vertex_offset = vertices.len() as u32;

                for position in positions {
                    let runtime_position = gltf_position_to_runtime(position);
                    let runtime_normal = normals
                        .as_mut()
                        .and_then(|iter| iter.next())
                        .map(gltf_direction_to_runtime)
                        .unwrap_or_else(|| mir_math::Vec3::new(0.0, 0.0, 1.0));
                    vertices.push(Vertex::new(
                        runtime_position.to_array(),
                        [1.0, 1.0, 1.0],
                        runtime_normal.to_array(),
                    ));
                }

                let primitive_indices: Vec<u32> = match reader.read_indices() {
                    Some(iter) => iter.into_u32().collect(),
                    None => (0..(vertices.len() - vertex_offset as usize) as u32).collect(),
                };
                indices.extend(
                    primitive_indices
                        .into_iter()
                        .map(|index| index + vertex_offset),
                );

                if let Some(name) = mesh.name() {
                    report.transformed_nodes.push(name.to_string());
                }
            }
        }

        if vertices.is_empty() || indices.is_empty() {
            return Err(AssetError::Decode(
                "glTF 未包含可绘制的三角网格".to_string(),
            ));
        }
        report
            .warnings
            .push("glTF Y-up/米 已转换为运行时 Z-up/厘米".to_string());
        Ok(NativeAsset::Mesh(MeshData::new(vertices, indices)))
    }
}

/// 导入 GLB 第一个 PBR 材质。
pub struct GltfMaterialImporter;

impl Importer for GltfMaterialImporter {
    fn type_tag(&self) -> &'static str {
        "material"
    }

    fn import(&self, bytes: &[u8], _path: &Path, report: &mut ImportReport) -> Result<NativeAsset> {
        let gltf = get_blob(bytes)?;
        let material = first_material(&gltf)
            .ok_or_else(|| AssetError::Decode("glTF 未包含 PBR 材质".to_string()))?;
        report
            .warnings
            .push("材质纹理绑定留待 M5，本阶段只导入 PBR 参数".to_string());
        Ok(NativeAsset::Material(material))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_glb_is_error() {
        let mut report = ImportReport::default();
        assert!(matches!(
            GltfImporter.import(b"not glb", Path::new("bad.glb"), &mut report),
            Err(AssetError::Decode(_))
        ));
    }
}
