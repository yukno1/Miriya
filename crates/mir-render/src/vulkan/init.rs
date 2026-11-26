//! 实例、surface、物理设备与逻辑设备创建。

use std::ffi::{CStr, c_char};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use vulkanalia::loader::{LIBRARY, LibloadingLoader};
use vulkanalia::prelude::v1_0::*;
use vulkanalia::vk::{InstanceV1_1, KhrSurfaceExtensionInstanceCommands};
use vulkanalia::window as vulkan_window;
use vulkanalia::Entry;

use crate::RenderInitError;
use crate::allocator::MemoryAllocator;

use super::VulkanCore;
use super::debug::{create_debug_messenger, validation_layer_available};

const API_VERSION: u32 = vk::make_version(1, 3, 0);

fn format_version(version: u32) -> String {
    format!(
        "{}.{}.{}",
        version >> 22,
        (version >> 12) & 0x3ff,
        version & 0xfff
    )
}

impl VulkanCore {
    pub(super) fn create(
        display: Option<&dyn HasDisplayHandle>,
        window: Option<&dyn HasWindowHandle>,
        _width: u32,
        _height: u32,
        validation: bool,
    ) -> Result<Self, RenderInitError> {
        // SAFETY: 从系统路径加载标准 Vulkan loader；返回的 Entry 在本函数返回后由 VulkanCore 持有。
        let loader = unsafe { LibloadingLoader::new(LIBRARY) }
            .map_err(|error| RenderInitError::Loader(error.to_string()))?;
        // SAFETY: loader 有效且被 Entry 接管。
        let entry = unsafe { Entry::new(loader) }
            .map_err(|error| RenderInitError::Loader(error.to_string()))?;

        let validation_enabled = validation && validation_layer_available(&entry);
        if validation && !validation_enabled {
            mir_log::warn!(
                target: "vulkan",
                "未找到 VK_LAYER_KHRONOS_validation，校验层已关闭"
            );
        }

        let mut required_extensions: Vec<*const c_char> = match window {
            Some(window) => vulkan_window::get_required_instance_extensions(window)
                .iter()
                .map(|extension| extension.as_ptr())
                .collect(),
            None => Vec::new(),
        };
        if validation_enabled {
            required_extensions.push(vk::EXT_DEBUG_UTILS_EXTENSION.name.as_ptr());
        }
        let enabled_layers: Vec<*const c_char> = if validation_enabled {
            vec![super::debug::VALIDATION_LAYER.as_ptr()]
        } else {
            Vec::new()
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
            .enabled_extension_names(&required_extensions)
            .enabled_layer_names(&enabled_layers);
        // SAFETY: app_info / instance_info / required_extensions / enabled_layers 在调用期间有效，
        // 入口为有效 Vulkan 实例入口；校验层与扩展均已通过 loader 查询。
        let instance = unsafe { entry.create_instance(&instance_info, None) }
            .map_err(|error| RenderInitError::Instance(error.to_string()))?;
        let debug_messenger = if validation_enabled {
            create_debug_messenger(&instance)
        } else {
            None
        };

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

        // 时间戳能力（R-05）：graphics 队列族的 timestampValidBits 与设备 limits.timestampPeriod。
        // 不支持时仅关闭 pass 耗时统计，不影响渲染路径。
        // SAFETY: physical_device 来自本实例的有效枚举结果。
        let queue_families =
            unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
        let timestamp_valid_bits = queue_families[graphics_family as usize].timestamp_valid_bits;
        // SAFETY: physical_device 有效。
        let timestamp_period = unsafe { instance.get_physical_device_properties(physical_device) }
            .limits
            .timestamp_period;

        // 查询并校验本渲染器依赖的两项可选特性：
        // - shaderDrawParameters：Slang 对 `SV_VertexID` 会声明 DrawParameters 能力；
        // - dynamicRendering：整个渲染路径都走 `vkCmdBeginRendering`，没有 render pass。
        // - synchronization2：RenderGraph 编译器生成的 barrier 全部走 `vkCmdPipelineBarrier2`（R-04）。
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
        if probe13.synchronization2 == 0 {
            return Err(RenderInitError::Device(
                "设备不支持 synchronization2（Vulkan 1.3 可选特性）".into(),
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
            .synchronization2(true)
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

        // 显存子分配器（R-08）：构造时自行查询 memory properties 与 bufferImageGranularity。
        // SAFETY: physical_device 有效。
        let limits = unsafe { instance.get_physical_device_properties(physical_device) }.limits;
        let allocator = MemoryAllocator::new(device.clone(), &instance, physical_device, limits);

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
            pipeline_cache: None,
            frames: Vec::new(),
            frame_index: 0,
            depth: None,
            depth_format: None,
            swapchain_outdated: false,
            image_views: Vec::new(),
            swapchain_images: Vec::new(),
            swapchain: None,
            surface,
            device,
            instance,
            debug_messenger,
            _entry: entry,
            physical_device,
            graphics_queue,
            present_queue,
            transfer_pool,
            transfer_command_buffer,
            meshes: Vec::new(),
            free_meshes: Vec::new(),
            materials: Vec::new(),
            free_materials: Vec::new(),
            default_material: None,
            deletions: Vec::new(),
            allocator,
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
            graph_dump: false,
            dump_warned: false,
            timestamp_valid_bits,
            timestamp_period,
            last_pass_timings: Vec::new(),
        })
    }
}
