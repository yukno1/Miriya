//! 深度附件资源：单采样、静态布局（创建后即为 `DEPTH_ATTACHMENT_OPTIMAL`）。

use vulkanalia::prelude::v1_0::*;

use crate::allocator::Allocation;

use super::VulkanCore;

pub(super) struct DepthResources {
    pub(super) image: vk::Image,
    allocation: Allocation,
    pub(super) view: vk::ImageView,
}

impl VulkanCore {
    /// 选择深度格式：优先纯深度 `D32_SFLOAT`，回退到带模板的 32/24 位格式。
    fn select_depth_format(&self) -> Result<vk::Format, String> {
        let candidates = [
            vk::Format::D32_SFLOAT,
            vk::Format::D32_SFLOAT_S8_UINT,
            vk::Format::D24_UNORM_S8_UINT,
        ];
        for format in candidates {
            // SAFETY: physical_device 来自本实例的有效枚举结果。
            let properties = unsafe {
                self.instance
                    .get_physical_device_format_properties(self.physical_device, format)
            };
            if properties
                .optimal_tiling_features
                .contains(vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT)
            {
                return Ok(format);
            }
        }
        Err("没有可用于深度附件的格式".to_string())
    }

    /// 在 `memory_type_bits` 中找一个满足属性要求的内存类型。
    pub(super) fn find_memory_type(
        &self,
        memory_type_bits: u32,
        required: vk::MemoryPropertyFlags,
    ) -> Result<u32, String> {
        // SAFETY: physical_device 来自本实例的有效枚举结果。
        let properties = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        (0..properties.memory_type_count)
            .find(|index| {
                memory_type_bits & (1 << index) != 0
                    && properties.memory_types[*index as usize]
                        .property_flags
                        .contains(required)
            })
            .ok_or_else(|| "找不到满足要求的内存类型".to_string())
    }

    /// 创建（或重建）与当前交换链尺寸匹配的深度附件，并转入 `DEPTH_ATTACHMENT_OPTIMAL`。
    pub(super) fn create_depth_resources(&mut self) -> Result<(), String> {
        if self.surface.is_none() {
            return Ok(());
        }
        self.destroy_depth_resources();

        let format = self.select_depth_format()?;
        let extent = self.extent;
        let image_info = vk::ImageCreateInfo::builder()
            .image_type(vk::ImageType::_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: extent.width.max(1),
                height: extent.height.max(1),
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: image_info 各字段有效，尺寸来自当前交换链。
        let image = unsafe { self.device.create_image(&image_info, None) }
            .map_err(|error| format!("创建深度图像失败: {error}"))?;

        // SAFETY: image 刚由本设备创建。
        let requirements = unsafe { self.device.get_image_memory_requirements(image) };
        let memory_type = self.find_memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let allocation = self
            .allocator
            .alloc(
                requirements.size,
                requirements.alignment,
                memory_type,
                false,
            )
            .map_err(|error| format!("分配深度图像内存失败: {error}"));
        let allocation = match allocation {
            Ok(allocation) => allocation,
            Err(error) => {
                // SAFETY: image 由本设备创建且尚未绑定。
                unsafe { self.device.destroy_image(image, None) };
                return Err(error);
            }
        };
        // SAFETY: image 与 allocation.memory 由本设备创建，偏移在分配范围内。
        if let Err(error) = unsafe {
            self.device
                .bind_image_memory(image, allocation.memory, allocation.offset)
        }
        .map_err(|error| format!("绑定深度图像内存失败: {error}"))
        {
            // SAFETY: image 由本设备创建且尚未绑定/绑定失败。
            unsafe { self.device.destroy_image(image, None) };
            self.allocator.free(&allocation);
            return Err(error);
        }

        let subresource = vk::ImageSubresourceRange::builder()
            .aspect_mask(vk::ImageAspectFlags::DEPTH)
            .base_mip_level(0)
            .level_count(1)
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
                return Err(format!("创建深度图像视图失败: {error}"));
            }
        };

        if let Err(error) = self.transition_depth_layout(image, subresource) {
            // SAFETY: view/image 由本设备创建且未被提交引用。
            unsafe {
                self.device.destroy_image_view(view, None);
                self.device.destroy_image(image, None);
            }
            self.allocator.free(&allocation);
            return Err(error);
        }

        self.depth = Some(DepthResources {
            image,
            allocation,
            view,
        });
        self.depth_format = Some(format);
        Ok(())
    }

    /// 一次性把深度图像从 `UNDEFINED` 转入 `DEPTH_ATTACHMENT_OPTIMAL`。
    fn transition_depth_layout(
        &mut self,
        image: vk::Image,
        subresource: vk::ImageSubresourceRange,
    ) -> Result<(), String> {
        let command_buffer = self.transfer_command_buffer;
        let no_memory_barriers: [vk::MemoryBarrier; 0] = [];
        let no_buffer_barriers: [vk::BufferMemoryBarrier; 0] = [];
        let barrier = vk::ImageMemoryBarrier::builder()
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(
                vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            )
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource)
            .build();
        let begin_info = vk::CommandBufferBeginInfo::builder()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: 命令缓冲来自本对象且当前空闲；barrier 引用的 image 由本对象持有。
        unsafe {
            self.device
                .begin_command_buffer(command_buffer, &begin_info)
                .map_err(|error| format!("开始深度布局转换失败: {error}"))?;
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                    | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                vk::DependencyFlags::empty(),
                &no_memory_barriers,
                &no_buffer_barriers,
                &[barrier],
            );
            self.device
                .end_command_buffer(command_buffer)
                .map_err(|error| format!("结束深度布局转换失败: {error}"))?;
            let command_buffers = [command_buffer];
            let submit = vk::SubmitInfo::builder().command_buffers(&command_buffers);
            self.device
                .queue_submit(self.graphics_queue, &[submit], vk::Fence::null())
                .map_err(|error| format!("提交深度布局转换失败: {error}"))?;
            self.device
                .queue_wait_idle(self.graphics_queue)
                .map_err(|error| format!("等待深度布局转换完成失败: {error}"))?;
        }
        Ok(())
    }

    pub(super) fn destroy_depth_resources(&mut self) {
        if let Some(depth) = self.depth.take() {
            // SAFETY: 深度资源由本对象创建；调用方保证 GPU 空闲或资源未被提交引用
            // （仅在创建路径与 wait_idle 之后调用）。
            unsafe {
                self.device.destroy_image_view(depth.view, None);
                self.device.destroy_image(depth.image, None);
            }
            self.allocator.free(&depth.allocation);
        }
    }
}
