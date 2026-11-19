//! `vulkanalia` 内部实现：实例、物理设备、逻辑设备与队列。
//!
//! 本模块是 crate 私有的；Vulkan 类型不得出现在 `mir-render` 的公共 API 中。

use std::ffi::{CStr, c_char};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use vulkanalia::loader::{LIBRARY, LibloadingLoader};
use vulkanalia::prelude::v1_0::*;
use vulkanalia::vk::{
    DeviceV1_3, KhrSurfaceExtensionInstanceCommands, KhrSwapchainExtensionDeviceCommands,
};
use vulkanalia::window as vulkan_window;
use vulkanalia::{Device, Entry, Instance};

use crate::RenderInitError;

const API_VERSION: u32 = vk::make_version(1, 3, 0);

/// 单帧录制与同步资源（当前只保留一帧在飞行，简化 CPU/GPU 同步）。
struct FrameResources {
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,
    image_available: vk::Semaphore,
    render_finished: vk::Semaphore,
    extent: vk::Extent2D,
}

struct PipelineResources {
    vertex_module: vk::ShaderModule,
    fragment_module: vk::ShaderModule,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
}

/// Vulkan 核心对象。按 pipeline → frame → image views → swapchain → surface → device → instance 的逆序销毁。
pub(super) struct VulkanCore {
    pipeline: Option<PipelineResources>,
    frame: Option<FrameResources>,
    image_views: Vec<vk::ImageView>,
    swapchain: Option<vk::SwapchainKHR>,
    surface: Option<vk::SurfaceKHR>,
    device: Device,
    instance: Instance,
    // Entry 持有加载器；必须比 Instance / Device 活得更久。
    _entry: Entry,
    physical_device: vk::PhysicalDevice,
    graphics_queue: vk::Queue,
    present_queue: vk::Queue,
    graphics_family: u32,
    present_family: u32,
    device_name: String,
    api_version: String,
    surface_format: Option<String>,
    present_mode: Option<String>,
    format_value: Option<vk::SurfaceFormatKHR>,
    mode_value: Option<vk::PresentModeKHR>,
    extent: vk::Extent2D,
}

impl VulkanCore {
    /// 无窗口（headless）初始化：不创建 surface，不启用交换链扩展。
    pub(super) fn new_headless() -> Result<Self, RenderInitError> {
        Self::create(None, None, 0, 0)
    }

    /// 带窗口初始化：创建 surface 与交换链，并要求物理设备支持呈现。
    pub(super) fn with_surface(
        display: &dyn HasDisplayHandle,
        window: &dyn HasWindowHandle,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderInitError> {
        let mut core = Self::create(Some(display), Some(window), width, height)?;
        core.create_swapchain(width, height)?;
        core.init_frame_resources()
            .map_err(RenderInitError::FrameResources)?;
        core.init_pipeline()
            .map_err(RenderInitError::FrameResources)?;
        Ok(core)
    }

    pub(super) fn device_name(&self) -> &str {
        &self.device_name
    }

    pub(super) fn api_version(&self) -> &str {
        &self.api_version
    }

    pub(super) fn has_surface(&self) -> bool {
        self.surface.is_some()
    }

    pub(super) fn queue_families(&self) -> (u32, u32) {
        (self.graphics_family, self.present_family)
    }

    pub(super) fn surface_format(&self) -> Option<&str> {
        self.surface_format.as_deref()
    }

    pub(super) fn present_mode(&self) -> Option<&str> {
        self.present_mode.as_deref()
    }

    pub(super) fn swapchain_image_count(&self) -> u32 {
        self.image_views.len() as u32
    }

    fn create(
        display: Option<&dyn HasDisplayHandle>,
        window: Option<&dyn HasWindowHandle>,
        _width: u32,
        _height: u32,
    ) -> Result<Self, RenderInitError> {
        // SAFETY: 从系统路径加载标准 Vulkan loader；返回的 Entry 在本函数返回后由 VulkanCore 持有。
        let loader = unsafe { LibloadingLoader::new(LIBRARY) }
            .map_err(|error| RenderInitError::Loader(error.to_string()))?;
        // SAFETY: loader 有效且被 Entry 接管。
        let entry = unsafe { Entry::new(loader) }
            .map_err(|error| RenderInitError::Loader(error.to_string()))?;

        let required_extensions: Vec<*const c_char> = match window {
            Some(window) => vulkan_window::get_required_instance_extensions(window)
                .iter()
                .map(|extension| extension.as_ptr())
                .collect(),
            None => Vec::new(),
        };

        let app_info = vk::ApplicationInfo::builder()
            .application_name(b"Miriya\0")
            .application_version(vk::make_version(0, 1, 0))
            .engine_name(b"Miriya\0")
            .engine_version(vk::make_version(0, 1, 0))
            .api_version(API_VERSION)
            .build();
        let instance_info = vk::InstanceCreateInfo::builder()
            .application_info(&app_info)
            .enabled_extension_names(&required_extensions);
        // SAFETY: app_info / instance_info / required_extensions 在调用期间有效，入口为有效 Vulkan 实例入口。
        let instance = unsafe { entry.create_instance(&instance_info, None) }
            .map_err(|error| RenderInitError::Instance(error.to_string()))?;

        let surface = match (display, window) {
            (Some(display), Some(window)) => {
                // SAFETY: 调用方保证窗口与 display 至少存活到 surface 被销毁；VulkanCore::drop 负责逆序销毁。
                let surface = unsafe { vulkan_window::create_surface(&instance, display, window) }
                    .map_err(|error| RenderInitError::Surface(error.to_string()))?;
                Some(surface)
            }
            _ => None,
        };

        // SAFETY: instance 句柄有效且在本函数内保持存活。
        let devices = unsafe { instance.enumerate_physical_devices() }
            .map_err(|error| RenderInitError::Device(error.to_string()))?;

        let mut chosen = None;
        for device in devices {
            // SAFETY: device 来自本 instance 的有效枚举结果。
            let properties = unsafe { instance.get_physical_device_properties(device) };
            if properties.api_version < API_VERSION {
                continue;
            }
            // SAFETY: properties.device_name 是设备返回的定长 C 字符串。
            let name = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            // SAFETY: device 有效。
            let families = unsafe { instance.get_physical_device_queue_family_properties(device) };
            let graphics = families
                .iter()
                .position(|family| family.queue_flags.contains(vk::QueueFlags::GRAPHICS));
            let present = match surface {
                Some(surface) => families.iter().enumerate().find_map(|(index, _)| {
                    // SAFETY: device 与 surface 均由本 instance 创建且仍然有效。
                    unsafe {
                        instance.get_physical_device_surface_support_khr(
                            device,
                            index as u32,
                            surface,
                        )
                    }
                    .ok()
                    .unwrap_or(false)
                    .then_some(index)
                }),
                None => graphics,
            };
            if let (Some(graphics), Some(present)) = (graphics, present) {
                chosen = Some((
                    device,
                    graphics as u32,
                    present as u32,
                    name,
                    properties.api_version,
                ));
                break;
            }
        }
        let (physical_device, graphics_family, present_family, device_name, api_version) =
            chosen.ok_or(RenderInitError::NoDevice)?;

        let priorities = [1.0_f32];
        let mut queue_infos = vec![
            vk::DeviceQueueCreateInfo::builder()
                .queue_family_index(graphics_family)
                .queue_priorities(&priorities)
                .build(),
        ];
        if present_family != graphics_family {
            queue_infos.push(
                vk::DeviceQueueCreateInfo::builder()
                    .queue_family_index(present_family)
                    .queue_priorities(&priorities)
                    .build(),
            );
        }

        let swapchain_name = vk::KHR_SWAPCHAIN_EXTENSION.name;
        let device_extensions: Vec<*const c_char> = if surface.is_some() {
            vec![swapchain_name.as_ptr()]
        } else {
            Vec::new()
        };
        let device_info = vk::DeviceCreateInfo::builder()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&device_extensions);
        // SAFETY: physical_device 支持所选队列族；device_info 及扩展名数组在调用期间有效。
        let device = unsafe { instance.create_device(physical_device, &device_info, None) }
            .map_err(|error| RenderInitError::Device(error.to_string()))?;
        // SAFETY: device 有效，且创建时请求了这两个队列族。
        let graphics_queue = unsafe { device.get_device_queue(graphics_family, 0) };
        let present_queue = unsafe { device.get_device_queue(present_family, 0) };

        let (surface_format, present_mode, format_value, mode_value) = match surface {
            Some(surface) => {
                // SAFETY: physical_device 与 surface 均有效且仍存活。
                let formats = unsafe {
                    instance.get_physical_device_surface_formats_khr(physical_device, surface)
                }
                .map_err(|error| RenderInitError::Surface(error.to_string()))?;
                // SAFETY: physical_device 与 surface 均有效且仍存活。
                let modes = unsafe {
                    instance.get_physical_device_surface_present_modes_khr(physical_device, surface)
                }
                .map_err(|error| RenderInitError::Surface(error.to_string()))?;
                let format = formats.first().copied().ok_or_else(|| {
                    RenderInitError::Surface("no surface format available".into())
                })?;
                let mode = modes
                    .iter()
                    .copied()
                    .find(|mode| *mode == vk::PresentModeKHR::IMMEDIATE)
                    .or_else(|| {
                        modes
                            .iter()
                            .copied()
                            .find(|mode| *mode == vk::PresentModeKHR::FIFO)
                    })
                    .ok_or_else(|| RenderInitError::Surface("no present mode available".into()))?;
                (
                    Some(format!("{:?}", format.format)),
                    Some(format!("{mode:?}")),
                    Some(format),
                    Some(mode),
                )
            }
            None => (None, None, None, None),
        };

        Ok(Self {
            pipeline: None,
            frame: None,
            image_views: Vec::new(),
            swapchain: None,
            surface,
            device,
            instance,
            _entry: entry,
            physical_device,
            graphics_queue,
            present_queue,
            graphics_family,
            present_family,
            device_name,
            api_version: format_version(api_version),
            surface_format,
            present_mode,
            format_value,
            mode_value,
            extent: vk::Extent2D {
                width: 0,
                height: 0,
            },
        })
    }

    fn create_swapchain(&mut self, width: u32, height: u32) -> Result<(), RenderInitError> {
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
        for image in images {
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
        self.extent = extent;
        Ok(())
    }

    fn init_frame_resources(&mut self) -> Result<(), String> {
        let pool_info = vk::CommandPoolCreateInfo::builder()
            .queue_family_index(self.graphics_family)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        // SAFETY: graphics_family 是本设备支持的队列族。
        let command_pool = unsafe { self.device.create_command_pool(&pool_info, None) }
            .map_err(|error| format!("创建命令池失败: {error}"))?;
        let allocate_info = vk::CommandBufferAllocateInfo::builder()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: command_pool 有效，请求一个主命令缓冲。
        let command_buffer = unsafe { self.device.allocate_command_buffers(&allocate_info) }
            .map_err(|error| format!("分配命令缓冲失败: {error}"))?
            .into_iter()
            .next()
            .ok_or_else(|| "命令缓冲分配结果为空".to_string())?;

        let fence_info = vk::FenceCreateInfo::builder().flags(vk::FenceCreateFlags::SIGNALED);
        // SAFETY: fence_info 有效。
        let fence = unsafe { self.device.create_fence(&fence_info, None) }
            .map_err(|error| format!("创建 fence 失败: {error}"))?;
        let semaphore_info = vk::SemaphoreCreateInfo::builder();
        // SAFETY: semaphore_info 有效。
        let image_available = unsafe { self.device.create_semaphore(&semaphore_info, None) }
            .map_err(|error| format!("创建 semaphore 失败: {error}"))?;
        // SAFETY: semaphore_info 有效。
        let render_finished = unsafe { self.device.create_semaphore(&semaphore_info, None) }
            .map_err(|error| format!("创建 semaphore 失败: {error}"))?;

        self.frame = Some(FrameResources {
            command_pool,
            command_buffer,
            fence,
            image_available,
            render_finished,
            extent: self.extent,
        });
        Ok(())
    }

    fn init_pipeline(&mut self) -> Result<(), String> {
        const VERTEX_CODE: &[u32] =
            vulkanalia::include_shader_code!("../../../shaders/hello/triangle.vert.spv");
        const FRAGMENT_CODE: &[u32] =
            vulkanalia::include_shader_code!("../../../shaders/hello/triangle.frag.spv");

        let vertex_info = vk::ShaderModuleCreateInfo::builder()
            .code_size(VERTEX_CODE.len() * 4)
            .code(VERTEX_CODE);
        // SAFETY: VERTEX_CODE 是已校验的 SPIR-V 字节码，生命周期覆盖模块创建。
        let vertex_module = unsafe { self.device.create_shader_module(&vertex_info, None) }
            .map_err(|error| format!("创建顶点着色器模块失败: {error}"))?;
        let fragment_info = vk::ShaderModuleCreateInfo::builder()
            .code_size(FRAGMENT_CODE.len() * 4)
            .code(FRAGMENT_CODE);
        // SAFETY: FRAGMENT_CODE 是已校验的 SPIR-V 字节码，生命周期覆盖模块创建。
        let fragment_module = unsafe { self.device.create_shader_module(&fragment_info, None) }
            .map_err(|error| format!("创建片元着色器模块失败: {error}"))?;

        let stages = [
            vk::PipelineShaderStageCreateInfo::builder()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vertex_module)
                .name(b"main\0")
                .build(),
            vk::PipelineShaderStageCreateInfo::builder()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fragment_module)
                .name(b"main\0")
                .build(),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::builder().build();
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::builder()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false)
            .build();
        let viewport_state = vk::PipelineViewportStateCreateInfo::builder()
            .viewport_count(1)
            .scissor_count(1)
            .build();
        let rasterization = vk::PipelineRasterizationStateCreateInfo::builder()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0)
            .build();
        let multisample = vk::PipelineMultisampleStateCreateInfo::builder()
            .rasterization_samples(vk::SampleCountFlags::_1)
            .build();
        let blend_attachment = vk::PipelineColorBlendAttachmentState::builder()
            .blend_enable(false)
            .color_write_mask(
                vk::ColorComponentFlags::R
                    | vk::ColorComponentFlags::G
                    | vk::ColorComponentFlags::B
                    | vk::ColorComponentFlags::A,
            )
            .build();
        let attachments = [blend_attachment];
        let color_blend = vk::PipelineColorBlendStateCreateInfo::builder()
            .attachments(&attachments)
            .build();
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state = vk::PipelineDynamicStateCreateInfo::builder()
            .dynamic_states(&dynamic_states)
            .build();

        let layout_info = vk::PipelineLayoutCreateInfo::builder().build();
        // SAFETY: layout_info 有效且无描述符集。
        let layout = unsafe { self.device.create_pipeline_layout(&layout_info, None) }
            .map_err(|error| format!("创建管线布局失败: {error}"))?;

        let format = self
            .format_value
            .ok_or_else(|| "缺少 surface 格式".to_string())?;
        let formats = [format.format];
        let mut rendering_info = vk::PipelineRenderingCreateInfo::builder()
            .color_attachment_formats(&formats)
            .build();
        let create_info = vk::GraphicsPipelineCreateInfo::builder()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .color_blend_state(&color_blend)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .push_next(&mut rendering_info)
            .build();
        // SAFETY: 所有状态结构与着色器模块均有效；使用动态渲染无需 render pass。
        let pipelines = unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[create_info], None)
        }
        .map_err(|error| format!("创建图形管线失败: {error:?}"))?
        .0;
        let pipeline = pipelines
            .into_iter()
            .next()
            .ok_or_else(|| "图形管线创建结果为空".to_string())?;

        self.pipeline = Some(PipelineResources {
            vertex_module,
            fragment_module,
            layout,
            pipeline,
        });
        Ok(())
    }

    fn destroy_pipeline(&mut self) {
        if let Some(resources) = self.pipeline.take() {
            // SAFETY: 这些对象由本对象创建且尚未销毁。
            unsafe {
                self.device.destroy_pipeline(resources.pipeline, None);
                self.device.destroy_pipeline_layout(resources.layout, None);
                self.device
                    .destroy_shader_module(resources.fragment_module, None);
                self.device
                    .destroy_shader_module(resources.vertex_module, None);
            }
        }
    }

    /// 录制一帧：清屏 → 绘制三角形 → 提交 → 呈现。无交换链时为空操作（headless）。
    pub(super) fn render_frame(&mut self, clear_color: [f32; 4]) -> Result<(), String> {
        let swapchain = match self.swapchain {
            Some(swapchain) => swapchain,
            None => return Ok(()),
        };
        let frame = match self.frame.as_mut() {
            Some(frame) => frame,
            None => return Ok(()),
        };

        // SAFETY: fence/semaphore/命令缓冲均有效且由本对象独占；等待上一帧完成后再复用。
        unsafe {
            self.device
                .wait_for_fences(&[frame.fence], true, u64::MAX)
                .map_err(|error| format!("等待 GPU 完成失败: {error}"))?;
            self.device
                .reset_fences(&[frame.fence])
                .map_err(|error| format!("重置 fence 失败: {error}"))?;
            let (index, _) = self
                .device
                .acquire_next_image_khr(
                    swapchain,
                    u64::MAX,
                    frame.image_available,
                    vk::Fence::null(),
                )
                .map_err(|error| format!("获取交换链图像失败: {error}"))?;

            let begin_info = vk::CommandBufferBeginInfo::builder()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device
                .begin_command_buffer(frame.command_buffer, &begin_info)
                .map_err(|error| format!("开始命令缓冲录制失败: {error}"))?;

            let clear = vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: clear_color,
                },
            };
            let attachment = vk::RenderingAttachmentInfo::builder()
                .image_view(self.image_views[index as usize])
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(clear);
            let render_area = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: frame.extent,
            };
            let attachments = [attachment];
            let rendering_info = vk::RenderingInfo::builder()
                .render_area(render_area)
                .layer_count(1)
                .color_attachments(&attachments);
            self.device
                .cmd_begin_rendering(frame.command_buffer, &rendering_info);
            if let Some(resources) = &self.pipeline {
                let viewport = vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: frame.extent.width as f32,
                    height: frame.extent.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                };
                let viewports = [viewport];
                let scissors = [render_area];
                self.device
                    .cmd_set_viewport(frame.command_buffer, 0, &viewports);
                self.device
                    .cmd_set_scissor(frame.command_buffer, 0, &scissors);
                self.device.cmd_bind_pipeline(
                    frame.command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    resources.pipeline,
                );
                self.device.cmd_draw(frame.command_buffer, 3, 1, 0, 0);
            }
            self.device.cmd_end_rendering(frame.command_buffer);
            self.device
                .end_command_buffer(frame.command_buffer)
                .map_err(|error| format!("结束命令缓冲录制失败: {error}"))?;

            let wait_semaphores = [frame.image_available];
            let stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
            let command_buffers = [frame.command_buffer];
            let signal_semaphores = [frame.render_finished];
            let submit = vk::SubmitInfo::builder()
                .wait_semaphores(&wait_semaphores)
                .wait_dst_stage_mask(&stages)
                .command_buffers(&command_buffers)
                .signal_semaphores(&signal_semaphores);
            let submits = [submit];
            self.device
                .queue_submit(self.graphics_queue, &submits, frame.fence)
                .map_err(|error| format!("提交命令缓冲失败: {error}"))?;

            let present_wait = [frame.render_finished];
            let swapchains = [swapchain];
            let image_indices = [index];
            let present = vk::PresentInfoKHR::builder()
                .wait_semaphores(&present_wait)
                .swapchains(&swapchains)
                .image_indices(&image_indices);
            self.device
                .queue_present_khr(self.present_queue, &present)
                .map_err(|error| format!("呈现失败: {error}"))?;
        }
        Ok(())
    }

    /// 窗口尺寸变化后重建交换链（同步等待 GPU 空闲）。
    pub(super) fn recreate_swapchain(&mut self, width: u32, height: u32) -> Result<(), String> {
        if self.surface.is_none() {
            return Ok(());
        }
        // SAFETY: 等待所有已提交工作结束后再销毁交换链资源。
        unsafe { self.device.device_wait_idle() }
            .map_err(|error| format!("等待设备空闲失败: {error}"))?;
        for view in self.image_views.drain(..) {
            // SAFETY: view 由本对象创建且尚未销毁。
            unsafe { self.device.destroy_image_view(view, None) };
        }
        if let Some(swapchain) = self.swapchain.take() {
            // SAFETY: swapchain 由本对象创建且设备已空闲。
            unsafe { self.device.destroy_swapchain_khr(swapchain, None) };
        }
        let old_format = self.format_value;
        self.create_swapchain(width.max(1), height.max(1))
            .map_err(|error| error.to_string())?;
        if self.format_value != old_format {
            self.destroy_pipeline();
            self.init_pipeline()?;
        }
        if let Some(frame) = self.frame.as_mut() {
            frame.extent = self.extent;
        }
        Ok(())
    }
}

impl Drop for VulkanCore {
    fn drop(&mut self) {
        // SAFETY: 本对象仍持有有效的 Vulkan 句柄；按 frame → image views → swapchain → surface → device → instance 逆序释放。
        unsafe {
            self.destroy_pipeline();
            if let Some(frame) = self.frame.take() {
                let _ = self.device.device_wait_idle();
                self.device.destroy_semaphore(frame.render_finished, None);
                self.device.destroy_semaphore(frame.image_available, None);
                self.device.destroy_fence(frame.fence, None);
                self.device.destroy_command_pool(frame.command_pool, None);
            }
            for view in self.image_views.drain(..) {
                self.device.destroy_image_view(view, None);
            }
            if let Some(swapchain) = self.swapchain.take() {
                self.device.destroy_swapchain_khr(swapchain, None);
            }
            if let Some(surface) = self.surface.take() {
                self.instance.destroy_surface_khr(surface, None);
            }
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

fn format_version(version: u32) -> String {
    format!(
        "{}.{}.{}",
        version >> 22,
        (version >> 12) & 0x3ff,
        version & 0xfff
    )
}
