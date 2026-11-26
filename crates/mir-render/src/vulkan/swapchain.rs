//! 交换链创建与重建。

use vulkanalia::prelude::v1_0::*;
use vulkanalia::vk::{KhrSurfaceExtensionInstanceCommands, KhrSwapchainExtensionDeviceCommands};

use crate::RenderInitError;

use super::VulkanCore;

impl VulkanCore {
    pub(super) fn create_swapchain(&mut self, width: u32, height: u32) -> Result<(), RenderInitError> {
        let surface = self.surface.ok_or(RenderInitError::NoDevice)?;
        let format = self.format_value.ok_or(RenderInitError::NoDevice)?;
        let mode = self.mode_value.ok_or(RenderInitError::NoDevice)?;

        // SAFETY: physical_device 与 surface 仍有效且未被销毁。
        let capabilities = unsafe {
            self.instance
                .get_physical_device_surface_capabilities_khr(self.physical_device, surface)
        }
        .map_err(|error| RenderInitError::Swapchain(error.to_string()))?;

        let mut image_count = capabilities.min_image_count + 1;
        if capabilities.max_image_count > 0 && image_count > capabilities.max_image_count {
            image_count = capabilities.max_image_count;
        }
        let extent = if capabilities.current_extent.width == u32::MAX {
            vk::Extent2D {
                width: width
                    .max(capabilities.min_image_extent.width)
                    .min(capabilities.max_image_extent.width),
                height: height
                    .max(capabilities.min_image_extent.height)
                    .min(capabilities.max_image_extent.height),
            }
        } else {
            capabilities.current_extent
        };

        let mut families = vec![self.graphics_family];
        if self.present_family != self.graphics_family {
            families.push(self.present_family);
        }
        let sharing_mode = if families.len() > 1 {
            vk::SharingMode::CONCURRENT
        } else {
            vk::SharingMode::EXCLUSIVE
        };
        let composite_alpha = if capabilities
            .supported_composite_alpha
            .contains(vk::CompositeAlphaFlagsKHR::OPAQUE)
        {
            vk::CompositeAlphaFlagsKHR::OPAQUE
        } else {
            vk::CompositeAlphaFlagsKHR::INHERIT
        };
        let info = vk::SwapchainCreateInfoKHR::builder()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(sharing_mode)
            .queue_family_indices(&families)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(composite_alpha)
            .present_mode(mode)
            .clipped(true);
        // SAFETY: surface/device/format/mode 均来自本对象持有的有效 Vulkan 资源。
        let swapchain = unsafe { self.device.create_swapchain_khr(&info, None) }
            .map_err(|error| RenderInitError::Swapchain(error.to_string()))?;
        // SAFETY: swapchain 刚刚创建成功且仍有效。
        let images = unsafe { self.device.get_swapchain_images_khr(swapchain) }
            .map_err(|error| RenderInitError::Swapchain(error.to_string()))?;

        let mut views = Vec::with_capacity(images.len());
        for &image in &images {
            let subresource = vk::ImageSubresourceRange::builder()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1)
                .build();
            let view_info = vk::ImageViewCreateInfo::builder()
                .image(image)
                .view_type(vk::ImageViewType::_2D)
                .format(format.format)
                .components(vk::ComponentMapping::builder().build())
                .subresource_range(subresource);
            // SAFETY: image 来自本 swapchain，格式与子资源范围匹配。
            let view = unsafe { self.device.create_image_view(&view_info, None) }
                .map_err(|error| RenderInitError::Swapchain(error.to_string()))?;
            views.push(view);
        }
        self.swapchain = Some(swapchain);
        self.image_views = views;
        self.swapchain_images = images;
        self.extent = extent;
        self.swapchain_outdated = false;
        Ok(())
    }

    /// 交换链重建后同步更新各帧的呈现信号量（数量跟随交换链图像数）。
    ///
    /// 调用方必须先 `device_wait_idle`，否则旧信号量可能仍被呈现操作引用。
    fn recreate_render_finished(&mut self) -> Result<(), String> {
        let semaphore_info = vk::SemaphoreCreateInfo::builder();
        let image_count = self.image_views.len();
        for frame in &mut self.frames {
            // SAFETY: device 已空闲，旧信号量不再被任何提交或呈现操作引用。
            unsafe {
                for semaphore in frame.render_finished.drain(..) {
                    self.device.destroy_semaphore(semaphore, None);
                }
            }
            for _ in 0..image_count {
                // SAFETY: semaphore_info 有效。
                let semaphore = unsafe { self.device.create_semaphore(&semaphore_info, None) }
                    .map_err(|error| format!("创建 semaphore 失败: {error}"))?;
                frame.render_finished.push(semaphore);
            }
        }
        Ok(())
    }

    /// 窗口尺寸变化后重建交换链（同步等待 GPU 空闲）。
    pub(crate) fn recreate_swapchain(&mut self, width: u32, height: u32) -> Result<(), String> {
        if self.surface.is_none() {
            return Ok(());
        }
        // SAFETY: 等待所有已提交工作结束后再销毁交换链资源。
        unsafe { self.device.device_wait_idle() }
            .map_err(|error| format!("等待设备空闲失败: {error}"))?;
        // 设备已空闲：顺带回收全部待销毁缓冲（R-09）。
        self.flush_deletions(None);
        for view in self.image_views.drain(..) {
            // SAFETY: view 由本对象创建且尚未销毁。
            unsafe { self.device.destroy_image_view(view, None) };
        }
        if let Some(swapchain) = self.swapchain.take() {
            // SAFETY: swapchain 由本对象创建且设备已空闲。
            unsafe { self.device.destroy_swapchain_khr(swapchain, None) };
        }
        // 旧交换链的图像句柄随交换链销毁而失效，重建前先清空，避免引用悬垂句柄。
        self.swapchain_images.clear();
        let old_format = self.format_value;
        self.create_swapchain(width.max(1), height.max(1))
            .map_err(|error| error.to_string())?;
        self.create_depth_resources()?;
        self.recreate_render_finished()?;
        if self.format_value != old_format {
            self.destroy_pipeline();
            self.init_pipeline()?;
        }
        for frame in &mut self.frames {
            frame.extent = self.extent;
        }
        Ok(())
    }
}
