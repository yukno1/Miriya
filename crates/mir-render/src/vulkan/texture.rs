//! GPU 纹理资源：图像 + 视图 + 采样器 + staging 上传（M5 批次 A，R-02）。
//!
//! 上传路径与 mesh 一致（阻塞传输 + `queue_wait_idle`）：
//! staging（HOST_VISIBLE，经分配器）→ 单次命令缓冲逐 mip `copy_buffer_to_image`
//! → UNDEFINED → SHADER_READ_ONLY_OPTIMAL → 立即释放 staging。
//! 图像/视图/采样器常驻；销毁经 [`super::buffer::DeletionEntry`] 绑定 frame fence
//! 延迟回收（headless 无帧在飞行时立即释放）。

use vulkanalia::prelude::v1_0::*;

use crate::allocator::Allocation;
use crate::texture::{ImageFormat, TextureData, TextureHandle, mip_dimension};

use super::VulkanCore;
use super::buffer::{DeletionEntry, DeletionImage};

/// 纹理槽位上限（与 `MAX_MATERIALS` 语义一致：超限 `create_texture` 返回错误，
/// 槽位销毁后复用，世代号递增）。
pub(super) const MAX_TEXTURES: usize = 128;

/// 一个已就绪的 GPU 纹理：image + 显存 + 视图 + 采样器。
pub(super) struct GpuTexture {
    image: vk::Image,
    allocation: Allocation,
    pub(super) view: vk::ImageView,
    pub(super) sampler: vk::Sampler,
    // `format` / `mip_levels` 由批次 B 的材质描述符绑定（shader 采样）使用。
    #[allow(dead_code)]
    pub(super) format: vk::Format,
    #[allow(dead_code)]
    pub(super) mip_levels: u32,
}

/// 纹理槽位：世代号用于让销毁后的旧 [`TextureHandle`] 自动失效。
pub(super) struct TextureSlot {
    generation: u32,
    texture: Option<GpuTexture>,
}

/// [`ImageFormat`] → `vk::Format`（M5 支持的未压缩格式全集）。
pub(super) fn vk_format(format: ImageFormat) -> vk::Format {
    match format {
        ImageFormat::R8G8B8A8Unorm => vk::Format::R8G8B8A8_UNORM,
        ImageFormat::R8G8B8A8Srgb => vk::Format::R8G8B8A8_SRGB,
        ImageFormat::B10G11R11Ufloat => vk::Format::B10G11R11_UFLOAT_PACK32,
        ImageFormat::R16G16B16A16Sfloat => vk::Format::R16G16B16A16_SFLOAT,
        ImageFormat::R32Sfloat => vk::Format::R32_SFLOAT,
        ImageFormat::D32Sfloat => vk::Format::D32_SFLOAT,
    }
}

/// 采样 aspect：深度格式用 DEPTH，其余用 COLOR。
fn aspect_for(format: vk::Format) -> vk::ImageAspectFlags {
    if format == vk::Format::D32_SFLOAT {
        vk::ImageAspectFlags::DEPTH
    } else {
        vk::ImageAspectFlags::COLOR
    }
}

impl VulkanCore {
    /// 创建 GPU 纹理（M5 批次 A）：mip 链图像 → 子分配器显存 → 视图 + 采样器 →
    /// staging 逐 mip 上传 → 转入 `SHADER_READ_ONLY_OPTIMAL`。
    ///
    /// headless（无交换链/帧资源）同样可创建：设备与分配器均可用，销毁走立即释放。
    pub(crate) fn create_texture(&mut self, data: &TextureData) -> Result<TextureHandle, String> {
        if self.textures.len() >= MAX_TEXTURES && self.free_textures.is_empty() {
            return Err(format!("纹理槽位已满（MAX_TEXTURES = {MAX_TEXTURES}）"));
        }
        let format = vk_format(data.format);
        let mip_levels = data.mip_levels();
        let aspect = aspect_for(format);

        let image_info = vk::ImageCreateInfo::builder()
            .image_type(vk::ImageType::_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: data.width,
                height: data.height,
                depth: 1,
            })
            .mip_levels(mip_levels)
            .array_layers(1)
            .samples(vk::SampleCountFlags::_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: image_info 各字段有效（尺寸/mip 来自已校验的 TextureData）。
        let image = match unsafe { self.device.create_image(&image_info, None) } {
            Ok(image) => image,
            Err(error) => return Err(format!("创建纹理图像失败: {error}")),
        };

        // SAFETY: image 刚由本设备创建。
        let requirements = unsafe { self.device.get_image_memory_requirements(image) };
        let memory_type = match self.find_memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        ) {
            Ok(memory_type) => memory_type,
            Err(error) => {
                // SAFETY: image 由本设备创建且尚未绑定。
                unsafe { self.device.destroy_image(image, None) };
                return Err(error);
            }
        };
        let allocation = match self.allocator.alloc(
            requirements.size,
            requirements.alignment,
            memory_type,
            false,
        ) {
            Ok(allocation) => allocation,
            Err(error) => {
                // SAFETY: image 由本设备创建且尚未绑定。
                unsafe { self.device.destroy_image(image, None) };
                return Err(format!("分配纹理显存失败: {error}"));
            }
        };
        // SAFETY: image 与 allocation.memory 由本设备创建，偏移在分配范围内。
        if let Err(error) = unsafe {
            self.device
                .bind_image_memory(image, allocation.memory, allocation.offset)
        }
        .map_err(|error| format!("绑定纹理显存失败: {error}"))
        {
            // SAFETY: image 由本设备创建且绑定失败。
            unsafe { self.device.destroy_image(image, None) };
            self.allocator.free(&allocation);
            return Err(error);
        }

        let subresource = vk::ImageSubresourceRange::builder()
            .aspect_mask(aspect)
            .base_mip_level(0)
            .level_count(mip_levels)
            .base_array_layer(0)
            .layer_count(1)
            .build();
        let view_info = vk::ImageViewCreateInfo::builder()
            .image(image)
            .view_type(vk::ImageViewType::_2D)
            .format(format)
            .subresource_range(subresource);
        // SAFETY: image 与格式、子资源范围匹配。
        let view = match unsafe { self.device.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(error) => {
                // SAFETY: image 由本设备创建，尚无有效视图。
                unsafe { self.device.destroy_image(image, None) };
                self.allocator.free(&allocation);
                return Err(format!("创建纹理视图失败: {error}"));
            }
        };

        let sampler_info = vk::SamplerCreateInfo::builder()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::REPEAT)
            .address_mode_v(vk::SamplerAddressMode::REPEAT)
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            .mip_lod_bias(0.0)
            .anisotropy_enable(false)
            .min_lod(0.0)
            .max_lod(mip_levels as f32);
        // SAFETY: 采样器参数为默认组合，不依赖其他对象。
        let sampler = match unsafe { self.device.create_sampler(&sampler_info, None) } {
            Ok(sampler) => sampler,
            Err(error) => {
                // SAFETY: view/image 由本设备创建且未被提交引用。
                unsafe {
                    self.device.destroy_image_view(view, None);
                    self.device.destroy_image(image, None);
                }
                self.allocator.free(&allocation);
                return Err(format!("创建纹理采样器失败: {error}"));
            }
        };

        if let Err(error) = self.upload_texture_mips(data, image, aspect) {
            // SAFETY: sampler/view/image 由本设备创建且未被提交引用
            //（上传失败时命令缓冲已结束或未提交，queue_wait_idle 已返回）。
            unsafe {
                self.device.destroy_sampler(sampler, None);
                self.device.destroy_image_view(view, None);
                self.device.destroy_image(image, None);
            }
            self.allocator.free(&allocation);
            return Err(error);
        }

        let gpu = GpuTexture {
            image,
            allocation,
            view,
            sampler,
            format,
            mip_levels,
        };

        let index = self
            .free_textures
            .pop()
            .unwrap_or(self.textures.len() as u32);
        let generation = self
            .textures
            .get(index as usize)
            .map(|slot| slot.generation)
            .unwrap_or(0);
        let slot = TextureSlot {
            generation,
            texture: Some(gpu),
        };
        if index as usize == self.textures.len() {
            self.textures.push(slot);
        } else {
            self.textures[index as usize] = slot;
        }
        Ok(TextureHandle::from_parts(index, generation))
    }

    /// staging 上传 mip 链并转入 `SHADER_READ_ONLY_OPTIMAL`（阻塞：submit + wait_idle）。
    fn upload_texture_mips(
        &mut self,
        data: &TextureData,
        image: vk::Image,
        aspect: vk::ImageAspectFlags,
    ) -> Result<(), String> {
        let total = data.bytes.len() as u64;
        let staging = self.create_buffer(
            total,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            false,
        )?;
        // SAFETY: staging 内存为 HOST_VISIBLE|HOST_COHERENT，映射区间与 bytes 匹配；
        // 该分配不在持久映射块中（相机 UBO 为 dedicated），unmap 不影响其他映射。
        let map_result = unsafe {
            let mapped = self
                .device
                .map_memory(
                    staging.allocation.memory,
                    staging.allocation.offset,
                    total,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(|error| format!("映射纹理 staging 失败: {error}"))?;
            std::ptr::copy_nonoverlapping(
                data.bytes.as_ptr(),
                mapped.cast::<u8>(),
                data.bytes.len(),
            );
            self.device.unmap_memory(staging.allocation.memory);
            Ok::<(), String>(())
        };
        if let Err(error) = map_result {
            self.release_buffer(staging);
            return Err(error);
        }

        let result = self.copy_texture_blocking(data, staging.buffer, image, aspect);
        // 拷贝路径已等待队列空闲，staging 可立即释放。
        self.release_buffer(staging);
        result
    }

    /// 单次命令缓冲：逐 mip `copy_buffer_to_image` + 全 mip 布局转换 barrier。
    fn copy_texture_blocking(
        &mut self,
        data: &TextureData,
        source: vk::Buffer,
        image: vk::Image,
        aspect: vk::ImageAspectFlags,
    ) -> Result<(), String> {
        let mip_levels = data.mip_levels();
        let regions: Vec<vk::BufferImageCopy> = data
            .mip_data
            .iter()
            .enumerate()
            .map(|(level, mip)| {
                vk::BufferImageCopy::builder()
                    .buffer_offset(mip.offset as u64)
                    .buffer_row_length(0)
                    .buffer_image_height(0)
                    .image_subresource(
                        vk::ImageSubresourceLayers::builder()
                            .aspect_mask(aspect)
                            .mip_level(level as u32)
                            .base_array_layer(0)
                            .layer_count(1)
                            .build(),
                    )
                    .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                    .image_extent(vk::Extent3D {
                        width: mip_dimension(data.width, level),
                        height: mip_dimension(data.height, level),
                        depth: 1,
                    })
                    .build()
            })
            .collect();

        let barrier = vk::ImageMemoryBarrier::builder()
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(
                vk::ImageSubresourceRange::builder()
                    .aspect_mask(aspect)
                    .base_mip_level(0)
                    .level_count(mip_levels)
                    .base_array_layer(0)
                    .layer_count(1)
                    .build(),
            )
            .build();
        let begin_info = vk::CommandBufferBeginInfo::builder()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: 传输命令缓冲只在同步上传路径使用；barrier 引用的 image 由本对象持有。
        unsafe {
            self.device
                .begin_command_buffer(self.transfer_command_buffer, &begin_info)
                .map_err(|error| format!("开始纹理上传失败: {error}"))?;
            self.device.cmd_copy_buffer_to_image(
                self.transfer_command_buffer,
                source,
                image,
                vk::ImageLayout::UNDEFINED,
                &regions,
            );
            let no_memory_barriers: [vk::MemoryBarrier; 0] = [];
            let no_buffer_barriers: [vk::BufferMemoryBarrier; 0] = [];
            self.device.cmd_pipeline_barrier(
                self.transfer_command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &no_memory_barriers,
                &no_buffer_barriers,
                &[barrier],
            );
            self.device
                .end_command_buffer(self.transfer_command_buffer)
                .map_err(|error| format!("结束纹理上传失败: {error}"))?;
            let command_buffers = [self.transfer_command_buffer];
            let submit = vk::SubmitInfo::builder().command_buffers(&command_buffers);
            self.device
                .queue_submit(self.graphics_queue, &[submit], vk::Fence::null())
                .map_err(|error| format!("提交纹理上传失败: {error}"))?;
            self.device
                .queue_wait_idle(self.graphics_queue)
                .map_err(|error| format!("等待纹理上传完成失败: {error}"))?;
        }
        Ok(())
    }

    /// 销毁纹理；句柄无效或世代不匹配时返回 `false`（旧句柄不会误删新纹理）。
    ///
    /// GPU 资源经 DeletionQueue 绑定 frame fence 延迟回收（headless 立即释放）。
    pub(crate) fn destroy_texture(&mut self, handle: TextureHandle) -> bool {
        let index = handle.index();
        let Some(slot) = self.textures.get(index) else {
            return false;
        };
        if slot.generation != handle.generation() {
            return false;
        }

        let texture = self.textures[index].texture.take();
        self.textures[index].generation = handle.generation().wrapping_add(1);
        self.free_textures.push(index as u32);
        if let Some(texture) = texture {
            if self.frames.is_empty() {
                // headless：无帧在飞行，上传路径已同步等待队列，立即释放安全。
                self.release_texture(texture);
            } else {
                // 延迟回收（R-09）：绑定下一帧的 fence——其信号保证所有更早的
                // 在飞提交已完成（队列操作按提交序完成）。
                let fence = self.frames[self.frame_index].fence;
                self.deletions.push(DeletionEntry {
                    fence,
                    buffers: Vec::new(),
                    descriptor_sets: Vec::new(),
                    images: vec![DeletionImage {
                        image: texture.image,
                        allocation: texture.allocation,
                        view: texture.view,
                        sampler: texture.sampler,
                    }],
                });
            }
        }
        true
    }

    /// 立即销毁纹理 GPU 资源并归还显存（Drop / headless 路径）。
    pub(super) fn release_texture(&mut self, texture: GpuTexture) {
        // SAFETY: 纹理资源由本设备创建，且调用方保证 GPU 侧不再使用。
        unsafe {
            self.device.destroy_sampler(texture.sampler, None);
            self.device.destroy_image_view(texture.view, None);
            self.device.destroy_image(texture.image, None);
        }
        self.allocator.free(&texture.allocation);
    }

    /// Drop 收尾：释放全部存活纹理（设备已 wait_idle，可立即销毁）。
    pub(super) fn destroy_all_textures(&mut self) {
        let textures: Vec<GpuTexture> = self
            .textures
            .iter_mut()
            .filter_map(|slot| slot.texture.take())
            .collect();
        for texture in textures {
            self.release_texture(texture);
        }
    }

    /// 解析纹理句柄；世代号不匹配（已被销毁/复用）时返回 `None`。
    /// 批次 B 由材质描述符绑定路径（render.rs 绘制循环）调用。
    #[allow(dead_code)]
    pub(super) fn texture_lookup(
        textures: &[TextureSlot],
        handle: TextureHandle,
    ) -> Option<&GpuTexture> {
        let slot = textures.get(handle.index())?;
        if slot.generation != handle.generation() {
            return None;
        }
        slot.texture.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vk_format_mapping_is_complete() {
        assert_eq!(
            vk_format(ImageFormat::R8G8B8A8Unorm),
            vk::Format::R8G8B8A8_UNORM
        );
        assert_eq!(
            vk_format(ImageFormat::R8G8B8A8Srgb),
            vk::Format::R8G8B8A8_SRGB
        );
        assert_eq!(
            vk_format(ImageFormat::B10G11R11Ufloat),
            vk::Format::B10G11R11_UFLOAT_PACK32
        );
        assert_eq!(
            vk_format(ImageFormat::R16G16B16A16Sfloat),
            vk::Format::R16G16B16A16_SFLOAT
        );
        assert_eq!(vk_format(ImageFormat::R32Sfloat), vk::Format::R32_SFLOAT);
        assert_eq!(vk_format(ImageFormat::D32Sfloat), vk::Format::D32_SFLOAT);
    }

    #[test]
    fn depth_format_uses_depth_aspect() {
        assert_eq!(
            aspect_for(vk::Format::D32_SFLOAT),
            vk::ImageAspectFlags::DEPTH
        );
        assert_eq!(
            aspect_for(vk::Format::R8G8B8A8_UNORM),
            vk::ImageAspectFlags::COLOR
        );
    }
}
