//! wgpu GPU 资源：网格 / 材质 / 纹理，世代句柄与 Vulkan 共用 `SlotTable`。

use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    AddressMode, BindGroup, BindGroupDescriptor, BindGroupEntry, BindingResource, Buffer,
    BufferBinding, BufferUsages, Extent3d, FilterMode, MipmapFilterMode, Origin3d, Sampler,
    SamplerDescriptor, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor,
};

use crate::frame::CAMERA_UBO_SIZE;
use crate::material::{MAX_MATERIALS, MaterialData, MaterialHandle, material_data_bytes};
use crate::mesh::{MeshData, MeshHandle, indices_as_bytes, vertices_as_bytes};
use crate::texture::{ImageFormat, MAX_TEXTURES, TextureData, TextureHandle, mip_dimension};

use super::WgpuCore;

pub(crate) struct GpuMesh {
    pub(crate) vertex_buffer: Buffer,
    pub(crate) index_buffer: Buffer,
    pub(crate) index_count: u32,
    bytes: u64,
}

pub(crate) struct GpuMaterial {
    pub(crate) bind_group: BindGroup,
    #[allow(dead_code)] // 由 bind group 持有；保留 buffer 延长生命周期。
    buffer: Buffer,
    bytes: u64,
}

pub(crate) struct GpuTexture {
    #[allow(dead_code)] // 当前 PBR 着色器绑定材质 UBO，纹理 binding 留待后续 pass。
    view: TextureView,
    #[allow(dead_code)]
    sampler: Sampler,
    #[allow(dead_code)]
    texture: Texture,
    bytes: u64,
}

/// [`ImageFormat`] → `wgpu::TextureFormat`（与 Vulkan 侧格式集合一一对应）。
pub(crate) fn wgpu_format(format: ImageFormat) -> TextureFormat {
    match format {
        ImageFormat::R8G8B8A8Unorm => TextureFormat::Rgba8Unorm,
        ImageFormat::R8G8B8A8Srgb => TextureFormat::Rgba8UnormSrgb,
        ImageFormat::B10G11R11Ufloat => TextureFormat::Rgb10a2Unorm,
        ImageFormat::R16G16B16A16Sfloat => TextureFormat::Rgba16Float,
        ImageFormat::R32Sfloat => TextureFormat::R32Float,
        ImageFormat::D32Sfloat => TextureFormat::Depth32Float,
    }
}

impl WgpuCore {
    pub(crate) fn create_mesh(&mut self, data: &MeshData) -> Result<MeshHandle, String> {
        if !data.is_valid() {
            return Err("网格数据无效：顶点/索引为空或索引越界".to_string());
        }
        let vertices = vertices_as_bytes(&data.vertices);
        let indices = indices_as_bytes(&data.indices);
        let vertex_buffer = self.device.create_buffer_init(&BufferInitDescriptor {
            label: Some("wgpu vertex buffer"),
            contents: vertices,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
        });
        let index_buffer = self.device.create_buffer_init(&BufferInitDescriptor {
            label: Some("wgpu index buffer"),
            contents: indices,
            usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
        });
        let bytes = (vertices.len() + indices.len()) as u64;
        self.track_alloc(bytes);
        let (index, generation) = self.meshes.insert(GpuMesh {
            vertex_buffer,
            index_buffer,
            index_count: data.indices.len() as u32,
            bytes,
        });
        Ok(MeshHandle::from_parts(index as u32, generation))
    }

    pub(crate) fn destroy_mesh(&mut self, handle: MeshHandle) -> bool {
        let Some(mesh) = self.meshes.remove(handle.index(), handle.generation()) else {
            return false;
        };
        self.track_free(mesh.bytes);
        true
    }

    pub(crate) fn create_material(
        &mut self,
        data: &MaterialData,
    ) -> Result<MaterialHandle, String> {
        if self.materials.len() >= MAX_MATERIALS && !self.materials.has_free() {
            return Err(format!("材质槽位已满（MAX_MATERIALS = {MAX_MATERIALS}）"));
        }
        let layout = self
            .pipeline
            .as_ref()
            .ok_or_else(|| "管线尚未初始化".to_string())?
            .material_bgl
            .clone();
        let bytes = material_data_bytes(data);
        let buffer = self.device.create_buffer_init(&BufferInitDescriptor {
            label: Some("wgpu material ubo"),
            contents: bytes,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });
        let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("wgpu material bind group"),
            layout: &layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: None,
                }),
            }],
        });
        let size = bytes.len() as u64;
        self.track_alloc(size);
        let (index, generation) = self.materials.insert(GpuMaterial {
            bind_group,
            buffer,
            bytes: size,
        });
        Ok(MaterialHandle::from_parts(index as u32, generation))
    }

    pub(crate) fn create_default_material(&mut self) -> Result<(), String> {
        let handle = self.create_material(&MaterialData::default())?;
        self.default_material = Some(
            self.materials
                .remove(handle.index(), handle.generation())
                .expect("默认材质槽位刚创建"),
        );
        Ok(())
    }

    pub(crate) fn destroy_material(&mut self, handle: MaterialHandle) -> bool {
        let Some(material) = self.materials.remove(handle.index(), handle.generation()) else {
            return false;
        };
        self.track_free(material.bytes);
        true
    }

    pub(crate) fn create_texture(&mut self, data: &TextureData) -> Result<TextureHandle, String> {
        if self.textures.len() >= MAX_TEXTURES && !self.textures.has_free() {
            return Err(format!("纹理槽位已满（MAX_TEXTURES = {MAX_TEXTURES}）"));
        }
        let format = wgpu_format(data.format);
        let mip_levels = data.mip_levels();
        let texture = self.device.create_texture(&TextureDescriptor {
            label: Some("wgpu texture"),
            size: Extent3d {
                width: data.width,
                height: data.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_levels,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, mip) in data.mip_data.iter().enumerate() {
            let width = mip_dimension(data.width, level);
            let height = mip_dimension(data.height, level);
            self.queue.write_texture(
                TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                &data.bytes[mip.offset..mip.offset + mip.length],
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * data.format.bytes_per_texel() as u32),
                    rows_per_image: Some(height),
                },
                Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&TextureViewDescriptor::default());
        let sampler = self.device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Linear,
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::Repeat,
            address_mode_w: AddressMode::Repeat,
            lod_max_clamp: mip_levels as f32,
            ..Default::default()
        });
        let bytes = data.bytes.len() as u64;
        self.track_alloc(bytes);
        let (index, generation) = self.textures.insert(GpuTexture {
            view,
            sampler,
            texture,
            bytes,
        });
        Ok(TextureHandle::from_parts(index as u32, generation))
    }

    pub(crate) fn destroy_texture(&mut self, handle: TextureHandle) -> bool {
        let Some(texture) = self.textures.remove(handle.index(), handle.generation()) else {
            return false;
        };
        self.track_free(texture.bytes);
        true
    }
}

impl Drop for WgpuCore {
    fn drop(&mut self) {
        // wgpu 句柄由 Arc 管理，显式 drain 只为维持 allocated 计数语义并尽早释放资源。
        let meshes: Vec<GpuMesh> = self.meshes.drain();
        for mesh in meshes {
            self.track_free(mesh.bytes);
        }
        let materials: Vec<GpuMaterial> = self.materials.drain();
        for material in materials {
            self.track_free(material.bytes);
        }
        let textures: Vec<GpuTexture> = self.textures.drain();
        for texture in textures {
            self.track_free(texture.bytes);
        }
        if let Some(material) = self.default_material.take() {
            self.track_free(material.bytes);
        }
        let frame_count = self.camera_frames.len() as u64;
        self.camera_frames.clear();
        self.track_free(CAMERA_UBO_SIZE * frame_count);
        if self.depth_bytes > 0 {
            self.track_free(self.depth_bytes);
            self.depth_bytes = 0;
        }
        if let Some(timing) = &self.timing {
            self.track_free(timing.allocated_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_mapping_is_complete() {
        assert_eq!(
            wgpu_format(ImageFormat::R8G8B8A8Unorm),
            TextureFormat::Rgba8Unorm
        );
        assert_eq!(
            wgpu_format(ImageFormat::R8G8B8A8Srgb),
            TextureFormat::Rgba8UnormSrgb
        );
        assert_eq!(
            wgpu_format(ImageFormat::D32Sfloat),
            TextureFormat::Depth32Float
        );
    }
}
