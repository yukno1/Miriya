//! `vulkanalia` 内部实现：实例、物理设备、逻辑设备与队列。
//!
//! 本模块是 crate 私有的；Vulkan 类型不得出现在 `mir-render` 的公共 API 中。

use std::ffi::{CStr, c_char};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use vulkanalia::loader::{LIBRARY, LibloadingLoader};
use vulkanalia::prelude::v1_0::*;
use vulkanalia::vk::{
    DeviceV1_3, InstanceV1_1, KhrSurfaceExtensionInstanceCommands,
    KhrSwapchainExtensionDeviceCommands,
};
use vulkanalia::window as vulkan_window;
use vulkanalia::{Device, Entry, Instance};

use crate::RenderInitError;
use crate::mesh::{MeshData, MeshHandle, indices_as_bytes, vertices_as_bytes};

const API_VERSION: u32 = vk::make_version(1, 3, 0);

/// 单帧录制与同步资源（当前只保留一帧在飞行，简化 CPU/GPU 同步）。
struct FrameResources {
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,
    image_available: vk::Semaphore,
    /// 每张交换链图像一个呈现信号量：避免同一信号量在“已呈现但尚未重新 acquire”的图像间复用。
    render_finished: Vec<vk::Semaphore>,
    extent: vk::Extent2D,
}

struct PipelineResources {
    vertex_module: vk::ShaderModule,
    fragment_module: vk::ShaderModule,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
}

/// 深度附件资源：单采样、静态布局（创建后即为 `DEPTH_ATTACHMENT_OPTIMAL`）。
struct DepthResources {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

/// 一个已上传到显存的网格（顶点/索引各一次独立分配，v0.2 无子分配器）。
struct GpuMesh {
    vertex_buffer: vk::Buffer,
    vertex_memory: vk::DeviceMemory,
    index_buffer: vk::Buffer,
    index_memory: vk::DeviceMemory,
    /// 绘制时读取（V-13 提交 API）。
    #[allow(dead_code)]
    index_count: u32,
}

/// 网格槽位：世代号用于让销毁后的旧 [`MeshHandle`] 自动失效。
struct MeshSlot {
    generation: u32,
    mesh: Option<GpuMesh>,
}

/// Vulkan 核心对象。按 pipeline → frame → image views → swapchain → surface → device → instance 的逆序销毁。
pub(super) struct VulkanCore {
    pipeline: Option<PipelineResources>,
    frame: Option<FrameResources>,
    depth: Option<DepthResources>,
    depth_format: Option<vk::Format>,
    /// 交换链被 acquire/present 报告为失效，需在下一帧重建。
    swapchain_outdated: bool,
    image_views: Vec<vk::ImageView>,
    /// 交换链图像句柄，用于每帧的 layout 转换（由交换链持有，不单独销毁）。
    swapchain_images: Vec<vk::Image>,
    swapchain: Option<vk::SwapchainKHR>,
    surface: Option<vk::SurfaceKHR>,
    device: Device,
    instance: Instance,
    // Entry 持有加载器；必须比 Instance / Device 活得更久。
    _entry: Entry,
    physical_device: vk::PhysicalDevice,
    graphics_queue: vk::Queue,
    present_queue: vk::Queue,
    /// 上传/布局转换专用的命令池与命令缓冲（同步使用，不参与每帧录制）。
    transfer_pool: vk::CommandPool,
    transfer_command_buffer: vk::CommandBuffer,
    meshes: Vec<MeshSlot>,
    free_meshes: Vec<u32>,
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
        core.create_depth_resources()
            .map_err(RenderInitError::Depth)?;
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

        // 查询并校验本渲染器依赖的两项可选特性：
        // - shaderDrawParameters：Slang 对 `SV_VertexID` 会声明 DrawParameters 能力；
        // - dynamicRendering：整个渲染路径都走 `vkCmdBeginRendering`，没有 render pass。
        let mut probe11 = vk::PhysicalDeviceVulkan11Features::default();
        let mut probe13 = vk::PhysicalDeviceVulkan13Features::default();
        let mut probe2 = vk::PhysicalDeviceFeatures2::builder()
            .push_next(&mut probe11)
            .push_next(&mut probe13)
            .build();
        // SAFETY: physical_device 来自本实例的有效枚举结果；特性结构链在调用期间有效。
        unsafe { instance.get_physical_device_features2(physical_device, &mut probe2) };
        if probe11.shader_draw_parameters == 0 {
            return Err(RenderInitError::Device(
                "设备不支持 shaderDrawParameters（Vulkan 1.1 可选特性）".into(),
            ));
        }
        if probe13.dynamic_rendering == 0 {
            return Err(RenderInitError::Device(
                "设备不支持 dynamicRendering（Vulkan 1.3 可选特性）".into(),
            ));
        }

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
        // 创建逻辑设备时启用已校验过的特性（用独立的链，避免复用查询阶段的 pNext）。
        let mut features11 = vk::PhysicalDeviceVulkan11Features::builder()
            .shader_draw_parameters(true)
            .build();
        let mut features13 = vk::PhysicalDeviceVulkan13Features::builder()
            .dynamic_rendering(true)
            .build();
        let device_info = vk::DeviceCreateInfo::builder()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&device_extensions)
            .push_next(&mut features11)
            .push_next(&mut features13);
        // SAFETY: physical_device 支持所选队列族与已查询的特性；device_info 及扩展名数组在调用期间有效。
        let device = unsafe { instance.create_device(physical_device, &device_info, None) }
            .map_err(|error| RenderInitError::Device(error.to_string()))?;
        // SAFETY: device 有效，且创建时请求了这两个队列族。
        let graphics_queue = unsafe { device.get_device_queue(graphics_family, 0) };
        let present_queue = unsafe { device.get_device_queue(present_family, 0) };

        // 上传/布局转换用独立命令池：不依赖每帧资源，headless 下也能创建网格。
        let pool_info = vk::CommandPoolCreateInfo::builder()
            .queue_family_index(graphics_family)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        // SAFETY: graphics_family 是本设备支持的队列族。
        let transfer_pool = unsafe { device.create_command_pool(&pool_info, None) }
            .map_err(|error| RenderInitError::Transfer(error.to_string()))?;
        let allocate_info = vk::CommandBufferAllocateInfo::builder()
            .command_pool(transfer_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: transfer_pool 有效，请求一个主命令缓冲。
        let transfer_command_buffer = unsafe { device.allocate_command_buffers(&allocate_info) }
            .map_err(|error| RenderInitError::Transfer(error.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| RenderInitError::Transfer("传输命令缓冲分配结果为空".into()))?;

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
            depth: None,
            depth_format: None,
            swapchain_outdated: false,
            image_views: Vec::new(),
            swapchain_images: Vec::new(),
            swapchain: None,
            surface,
            device,
            instance,
            _entry: entry,
            physical_device,
            graphics_queue,
            present_queue,
            transfer_pool,
            transfer_command_buffer,
            meshes: Vec::new(),
            free_meshes: Vec::new(),
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
        let mut render_finished = Vec::with_capacity(self.image_views.len());
        for _ in 0..self.image_views.len() {
            // SAFETY: semaphore_info 有效。
            let semaphore = unsafe { self.device.create_semaphore(&semaphore_info, None) }
                .map_err(|error| format!("创建 semaphore 失败: {error}"))?;
            render_finished.push(semaphore);
        }

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

    /// 交换链重建后同步更新呈现信号量（数量跟随交换链图像数）。
    ///
    /// 调用方必须先 `device_wait_idle`，否则旧信号量可能仍被呈现操作引用。
    fn recreate_render_finished(&mut self) -> Result<(), String> {
        let semaphore_info = vk::SemaphoreCreateInfo::builder();
        let image_count = self.image_views.len();
        let frame = self
            .frame
            .as_mut()
            .ok_or_else(|| "帧资源尚未初始化".to_string())?;

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
        Ok(())
    }

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
    fn find_memory_type(
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
    fn create_depth_resources(&mut self) -> Result<(), String> {
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
        let allocate_info = vk::MemoryAllocateInfo::builder()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type)
            .build();
        // SAFETY: 分配参数来自 image 的内存需求，memory_type 已在设备内存类型中校验。
        let memory = unsafe { self.device.allocate_memory(&allocate_info, None) }
            .map_err(|error| format!("分配深度图像内存失败: {error}"))?;
        // SAFETY: image 与 memory 由本设备创建，偏移 0 在分配范围内。
        unsafe { self.device.bind_image_memory(image, memory, 0) }
            .map_err(|error| format!("绑定深度图像内存失败: {error}"))?;

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
        let view = unsafe { self.device.create_image_view(&view_info, None) }
            .map_err(|error| format!("创建深度图像视图失败: {error}"))?;

        self.transition_depth_layout(image, subresource)?;

        self.depth = Some(DepthResources {
            image,
            memory,
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

    fn destroy_depth_resources(&mut self) {
        if let Some(depth) = self.depth.take() {
            // SAFETY: 深度资源由本对象创建；调用方保证 GPU 空闲或资源未被提交引用。
            unsafe {
                self.device.destroy_image_view(depth.view, None);
                self.device.destroy_image(depth.image, None);
                self.device.free_memory(depth.memory, None);
            }
        }
    }

    /// 创建 GPU 网格：staging → device-local，阻塞式上传（v0.2 网格极少，成本可忽略）。
    pub(super) fn create_mesh(&mut self, data: &MeshData) -> Result<MeshHandle, String> {
        if !data.is_valid() {
            return Err("网格数据无效：顶点/索引为空或索引越界".to_string());
        }
        let mesh = self.upload_mesh(data)?;

        let index = self.free_meshes.pop().unwrap_or(self.meshes.len() as u32);
        let generation = self
            .meshes
            .get(index as usize)
            .map(|slot| slot.generation)
            .unwrap_or(0);
        let slot = MeshSlot {
            generation,
            mesh: Some(mesh),
        };
        if index as usize == self.meshes.len() {
            self.meshes.push(slot);
        } else {
            self.meshes[index as usize] = slot;
        }
        Ok(MeshHandle::from_parts(index, generation))
    }

    /// 销毁网格；句柄无效或世代不匹配时返回 `false`（旧句柄不会误删新网格）。
    pub(super) fn destroy_mesh(&mut self, handle: MeshHandle) -> bool {
        let index = handle.index();
        let Some(slot) = self.meshes.get(index) else {
            return false;
        };
        if slot.generation != handle.generation() {
            return false;
        }

        let mesh = self.meshes[index].mesh.take();
        self.meshes[index].generation = handle.generation().wrapping_add(1);
        self.free_meshes.push(index as u32);
        if let Some(mesh) = mesh {
            self.release_mesh(mesh);
        }
        true
    }

    fn release_mesh(&self, mesh: GpuMesh) {
        // SAFETY: 网格缓冲由本设备创建，且调用方保证 GPU 侧不再使用（创建/销毁均同步）。
        unsafe {
            self.device.destroy_buffer(mesh.vertex_buffer, None);
            self.device.free_memory(mesh.vertex_memory, None);
            self.device.destroy_buffer(mesh.index_buffer, None);
            self.device.free_memory(mesh.index_memory, None);
        }
    }

    fn destroy_all_meshes(&mut self) {
        // 先收集再释放，避免同时借用 `self.meshes` 与 `self.device`。
        let meshes: Vec<GpuMesh> = self
            .meshes
            .iter_mut()
            .filter_map(|slot| slot.mesh.take())
            .collect();
        for mesh in meshes {
            self.release_mesh(mesh);
        }
    }

    /// 分配 buffer 并绑定设备内存。
    fn create_buffer(
        &self,
        size: u64,
        usage: vk::BufferUsageFlags,
        properties: vk::MemoryPropertyFlags,
    ) -> Result<(vk::Buffer, vk::DeviceMemory), String> {
        let info = vk::BufferCreateInfo::builder()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: BufferCreateInfo 有效。
        let buffer = unsafe { self.device.create_buffer(&info, None) }
            .map_err(|error| format!("创建缓冲失败: {error}"))?;
        // SAFETY: buffer 刚由本设备创建。
        let requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        let memory_type = self.find_memory_type(requirements.memory_type_bits, properties)?;
        let allocate_info = vk::MemoryAllocateInfo::builder()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type)
            .build();
        // SAFETY: 分配参数来自 buffer 的内存需求，内存类型已校验。
        let memory = unsafe { self.device.allocate_memory(&allocate_info, None) }
            .map_err(|error| format!("分配缓冲内存失败: {error}"))?;
        // SAFETY: buffer 与 memory 由本设备创建，偏移 0 在分配范围内。
        unsafe { self.device.bind_buffer_memory(buffer, memory, 0) }
            .map_err(|error| format!("绑定缓冲内存失败: {error}"))?;
        Ok((buffer, memory))
    }

    fn release_buffer(&self, buffer: vk::Buffer, memory: vk::DeviceMemory) {
        // SAFETY: buffer/memory 由本设备创建且不再被使用。
        unsafe {
            self.device.destroy_buffer(buffer, None);
            self.device.free_memory(memory, None);
        }
    }

    /// 把字节上传到 device-local 缓冲（staging → transfer copy → 同步等待）。
    fn upload_bytes(
        &mut self,
        bytes: &[u8],
        usage: vk::BufferUsageFlags,
    ) -> Result<(vk::Buffer, vk::DeviceMemory), String> {
        let size = bytes.len() as u64;
        let (staging, staging_memory) = self.create_buffer(
            size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        // SAFETY: staging 内存为 HOST_VISIBLE|HOST_COHERENT，映射大小与 bytes 匹配。
        unsafe {
            let mapped = self
                .device
                .map_memory(staging_memory, 0, size, vk::MemoryMapFlags::empty())
                .map_err(|error| format!("映射 staging 内存失败: {error}"))?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast::<u8>(), bytes.len());
            self.device.unmap_memory(staging_memory);
        }

        let (buffer, memory) = match self.create_buffer(
            size,
            usage | vk::BufferUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        ) {
            Ok(created) => created,
            Err(error) => {
                self.release_buffer(staging, staging_memory);
                return Err(error);
            }
        };

        let result = self.copy_buffer_blocking(staging, buffer, size);
        // staging 只在拷贝期间需要。
        self.release_buffer(staging, staging_memory);
        match result {
            Ok(()) => Ok((buffer, memory)),
            Err(error) => {
                self.release_buffer(buffer, memory);
                Err(error)
            }
        }
    }

    /// 用传输命令缓冲做一次同步缓冲拷贝（提交后等待队列空闲）。
    fn copy_buffer_blocking(
        &mut self,
        source: vk::Buffer,
        destination: vk::Buffer,
        size: u64,
    ) -> Result<(), String> {
        let begin_info = vk::CommandBufferBeginInfo::builder()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: 传输命令缓冲只在同步上传路径使用，提交后立即等待完成。
        unsafe {
            self.device
                .begin_command_buffer(self.transfer_command_buffer, &begin_info)
                .map_err(|error| format!("开始缓冲拷贝失败: {error}"))?;
            let region = vk::BufferCopy {
                src_offset: 0,
                dst_offset: 0,
                size,
            };
            self.device.cmd_copy_buffer(
                self.transfer_command_buffer,
                source,
                destination,
                &[region],
            );
            // 传输写入 → 顶点/索引读取的可见性依赖；不依赖后续提交顺序自动保证。
            let no_memory_barriers: [vk::MemoryBarrier; 0] = [];
            let no_image_barriers: [vk::ImageMemoryBarrier; 0] = [];
            let buffer_barrier = vk::BufferMemoryBarrier::builder()
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(
                    vk::AccessFlags::INDEX_READ | vk::AccessFlags::VERTEX_ATTRIBUTE_READ,
                )
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .buffer(destination)
                .offset(0)
                .size(size)
                .build();
            self.device.cmd_pipeline_barrier(
                self.transfer_command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::VERTEX_INPUT,
                vk::DependencyFlags::empty(),
                &no_memory_barriers,
                &[buffer_barrier],
                &no_image_barriers,
            );
            self.device
                .end_command_buffer(self.transfer_command_buffer)
                .map_err(|error| format!("结束缓冲拷贝失败: {error}"))?;
            let command_buffers = [self.transfer_command_buffer];
            let submit = vk::SubmitInfo::builder().command_buffers(&command_buffers);
            self.device
                .queue_submit(self.graphics_queue, &[submit], vk::Fence::null())
                .map_err(|error| format!("提交缓冲拷贝失败: {error}"))?;
            self.device
                .queue_wait_idle(self.graphics_queue)
                .map_err(|error| format!("等待缓冲拷贝完成失败: {error}"))?;
        }
        Ok(())
    }

    fn upload_mesh(&mut self, data: &MeshData) -> Result<GpuMesh, String> {
        let vertex_bytes = vertices_as_bytes(&data.vertices);
        let index_bytes = indices_as_bytes(&data.indices);

        let (vertex_buffer, vertex_memory) =
            self.upload_bytes(vertex_bytes, vk::BufferUsageFlags::VERTEX_BUFFER)?;
        let (index_buffer, index_memory) =
            match self.upload_bytes(index_bytes, vk::BufferUsageFlags::INDEX_BUFFER) {
                Ok(created) => created,
                Err(error) => {
                    self.release_buffer(vertex_buffer, vertex_memory);
                    return Err(error);
                }
            };

        Ok(GpuMesh {
            vertex_buffer,
            vertex_memory,
            index_buffer,
            index_memory,
            index_count: data.indices.len() as u32,
        })
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
        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::builder()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS)
            .depth_bounds_test_enable(false)
            .stencil_test_enable(false)
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
        let depth_format = self
            .depth_format
            .ok_or_else(|| "缺少深度格式".to_string())?;
        let mut rendering_info = vk::PipelineRenderingCreateInfo::builder()
            .color_attachment_formats(&formats)
            .depth_attachment_format(depth_format)
            .build();
        let create_info = vk::GraphicsPipelineCreateInfo::builder()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .depth_stencil_state(&depth_stencil)
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

    /// 当前交换链尺寸是否与应用层已知的窗口尺寸不一致。
    fn target_differs(&self, width: u32, height: u32) -> bool {
        self.extent.width != width || self.extent.height != height
    }

    /// 录制一帧：清屏 → 绘制三角形 → 提交 → 呈现。无交换链时为空操作（headless）。
    ///
    /// 交换链失效（`OUT_OF_DATE`）不视为帧错误：标记后本帧跳过，下一帧开头按当前窗口尺寸重建。
    /// `width` / `height` 是应用层已知的窗口尺寸，用于重建时取最新目标大小。
    pub(super) fn render_frame(
        &mut self,
        clear_color: [f32; 4],
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        if self.surface.is_none() {
            return Ok(());
        }
        if self.swapchain_outdated {
            self.swapchain_outdated = false;
            self.recreate_swapchain(width.max(1), height.max(1))?;
        }

        let swapchain = match self.swapchain {
            Some(swapchain) => swapchain,
            None => return Ok(()),
        };
        // 交换链尺寸与窗口尺寸是否不一致；先算好，避免持有 `frame` 可变借用时再借 `self`。
        let target_differs = self.target_differs(width, height);
        let frame = match self.frame.as_mut() {
            Some(frame) => frame,
            None => return Ok(()),
        };

        // SAFETY: fence/semaphore/命令缓冲均有效且由本对象独占；等待上一帧完成后再复用。
        unsafe {
            self.device
                .wait_for_fences(&[frame.fence], true, u64::MAX)
                .map_err(|error| format!("等待 GPU 完成失败: {error}"))?;
            let (index, suboptimal) = match self.device.acquire_next_image_khr(
                swapchain,
                u64::MAX,
                frame.image_available,
                vk::Fence::null(),
            ) {
                Ok(acquired) => acquired,
                Err(vk::ErrorCode::OUT_OF_DATE_KHR) => {
                    // 提前返回时不能重置 fence，否则下一帧会永久等待一个未信号的 fence。
                    self.swapchain_outdated = true;
                    return Ok(());
                }
                Err(error) => return Err(format!("获取交换链图像失败: {error}")),
            };
            self.device
                .reset_fences(&[frame.fence])
                .map_err(|error| format!("重置 fence 失败: {error}"))?;
            if suboptimal == vk::SuccessCode::SUBOPTIMAL_KHR && target_differs {
                self.swapchain_outdated = true;
            }

            let begin_info = vk::CommandBufferBeginInfo::builder()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device
                .begin_command_buffer(frame.command_buffer, &begin_info)
                .map_err(|error| format!("开始命令缓冲录制失败: {error}"))?;

            let image = self.swapchain_images[index as usize];
            // 空切片需要显式类型，否则无法推导 `Cast` 目标。
            let no_memory_barriers: [vk::MemoryBarrier; 0] = [];
            let no_buffer_barriers: [vk::BufferMemoryBarrier; 0] = [];
            let subresource = vk::ImageSubresourceRange::builder()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1)
                .build();

            // 从“未定义”转到颜色附件：本帧首操作是 CLEAR，旧内容本就要丢弃，
            // 因此用 UNDEFINED 作 oldLayout 即可，无需跟踪每张图像的历史布局。
            let to_color_attachment = vk::ImageMemoryBarrier::builder()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(subresource)
                .build();
            self.device.cmd_pipeline_barrier(
                frame.command_buffer,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::DependencyFlags::empty(),
                &no_memory_barriers,
                &no_buffer_barriers,
                &[to_color_attachment],
            );

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
            let depth = self
                .depth
                .as_ref()
                .ok_or_else(|| "深度资源未初始化".to_string())?;
            let depth_attachment = vk::RenderingAttachmentInfo::builder()
                .image_view(depth.view)
                .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::DONT_CARE)
                .clear_value(vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: 1.0,
                        stencil: 0,
                    },
                })
                .build();
            let attachments = [attachment];
            let rendering_info = vk::RenderingInfo::builder()
                .render_area(render_area)
                .layer_count(1)
                .color_attachments(&attachments)
                .depth_attachment(&depth_attachment);
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

            // 呈现前转回 PRESENT_SRC_KHR，保证 next acquire 与 present engine 看到一致的布局。
            let to_present = vk::ImageMemoryBarrier::builder()
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::empty())
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(subresource)
                .build();
            self.device.cmd_pipeline_barrier(
                frame.command_buffer,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &no_memory_barriers,
                &no_buffer_barriers,
                &[to_present],
            );

            self.device
                .end_command_buffer(frame.command_buffer)
                .map_err(|error| format!("结束命令缓冲录制失败: {error}"))?;

            let wait_semaphores = [frame.image_available];
            let stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
            let command_buffers = [frame.command_buffer];
            // 用本张图像专属的呈现信号量，避免在图像未重新 acquire 前复用。
            let render_finished = frame.render_finished[index as usize];
            let signal_semaphores = [render_finished];
            let submit = vk::SubmitInfo::builder()
                .wait_semaphores(&wait_semaphores)
                .wait_dst_stage_mask(&stages)
                .command_buffers(&command_buffers)
                .signal_semaphores(&signal_semaphores);
            let submits = [submit];
            self.device
                .queue_submit(self.graphics_queue, &submits, frame.fence)
                .map_err(|error| format!("提交命令缓冲失败: {error}"))?;

            let present_wait = [render_finished];
            let swapchains = [swapchain];
            let image_indices = [index];
            let present = vk::PresentInfoKHR::builder()
                .wait_semaphores(&present_wait)
                .swapchains(&swapchains)
                .image_indices(&image_indices);
            match self.device.queue_present_khr(self.present_queue, &present) {
                Ok(suboptimal) => {
                    if suboptimal == vk::SuccessCode::SUBOPTIMAL_KHR && target_differs {
                        self.swapchain_outdated = true;
                    }
                }
                Err(vk::ErrorCode::OUT_OF_DATE_KHR) => {
                    // 呈现失败但提交已完成；下一帧重建交换链后重新呈现新帧。
                    self.swapchain_outdated = true;
                }
                Err(error) => return Err(format!("呈现失败: {error}")),
            }
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
        if let Some(frame) = self.frame.as_mut() {
            frame.extent = self.extent;
        }
        Ok(())
    }
}

impl Drop for VulkanCore {
    fn drop(&mut self) {
        // SAFETY: 本对象仍持有有效的 Vulkan 句柄；先等待 GPU 空闲，再按
        // pipeline → frame → image views → swapchain → surface → device → instance 逆序释放。
        unsafe {
            if let Some(frame) = self.frame.take() {
                let _ = self.device.device_wait_idle();
                for semaphore in frame.render_finished {
                    self.device.destroy_semaphore(semaphore, None);
                }
                self.device.destroy_semaphore(frame.image_available, None);
                self.device.destroy_fence(frame.fence, None);
                self.device.destroy_command_pool(frame.command_pool, None);
            }
            // 管线销毁必须发生在命令缓冲执行完毕之后。
            self.destroy_pipeline();
            self.destroy_depth_resources();
            self.destroy_all_meshes();
            self.device.destroy_command_pool(self.transfer_pool, None);
            for view in self.image_views.drain(..) {
                self.device.destroy_image_view(view, None);
            }
            if let Some(swapchain) = self.swapchain.take() {
                self.device.destroy_swapchain_khr(swapchain, None);
            }
            self.swapchain_images.clear();
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
