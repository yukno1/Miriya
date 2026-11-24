//! `vulkanalia` 内部实现：实例、物理设备、逻辑设备与队列。
//!
//! 本模块是 crate 私有的；Vulkan 类型不得出现在 `mir-render` 的公共 API 中。

use std::ffi::{CStr, c_char, c_void};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use vulkanalia::loader::{LIBRARY, LibloadingLoader};
use vulkanalia::prelude::v1_0::*;
use vulkanalia::vk::{
    DeviceV1_3, EntryV1_0, ExtDebugUtilsExtensionInstanceCommands, InstanceV1_1,
    KhrSurfaceExtensionInstanceCommands, KhrSwapchainExtensionDeviceCommands,
};
use vulkanalia::window as vulkan_window;
use vulkanalia::{Device, Entry, Instance};

use crate::RenderInitError;
use crate::frame::DrawInstance;
use crate::mesh::{MeshData, MeshHandle, Vertex, indices_as_bytes, vertices_as_bytes};
use mir_math::Mat4;

const API_VERSION: u32 = vk::make_version(1, 3, 0);
const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

/// 查询 loader 是否提供 Khronos 校验层；缺失时调用方降级关闭而不是报错。
fn validation_layer_available(entry: &Entry) -> bool {
    // SAFETY: `entry` 由当前进程加载的 Vulkan loader 构造，查询不依赖实例。
    unsafe { entry.enumerate_instance_layer_properties() }
        .map(|layers| {
            layers
                .iter()
                .any(|layer| layer.layer_name.as_cstr() == VALIDATION_LAYER)
        })
        .unwrap_or(false)
}

/// debug utils 回调信息：错误/警告记入 `mir-log`，并带上类型、ID 与对象名。
fn debug_messenger_create_info() -> vk::DebugUtilsMessengerCreateInfoEXT {
    vk::DebugUtilsMessengerCreateInfoEXT::builder()
        .message_severity(
            vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
        )
        .message_type(
            vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
        )
        .user_callback(Some(debug_callback))
        .build()
}

/// 安全读取校验层回调中的 C 字符串；调用方需保证指针有效或以空指针表示缺省。
unsafe fn debug_cstr(ptr: *const c_char, fallback: &str) -> String {
    if ptr.is_null() {
        fallback.to_string()
    } else {
        // SAFETY: 调用方保证 `ptr` 为空指针或有效的 NUL 结尾字符串。
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

unsafe extern "system" fn debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_types: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _user_data: *mut c_void,
) -> vk::Bool32 {
    if data.is_null() {
        return vk::FALSE;
    }
    // SAFETY: 回调数据由 Vulkan 校验层提供，调用期间有效。
    let data = unsafe { &*data };
    let id = unsafe { debug_cstr(data.message_id_name, "unknown") };
    let message = unsafe { debug_cstr(data.message, "") };
    let mut object_names = Vec::new();
    if !data.objects.is_null() && data.object_count > 0 {
        // SAFETY: `object_count` 与 `objects` 由校验层保证一致且调用期间有效。
        let objects =
            unsafe { std::slice::from_raw_parts(data.objects, data.object_count as usize) };
        for object in objects.iter().take(4) {
            let name = unsafe { debug_cstr(object.object_name, "unnamed") };
            object_names.push(format!(
                "{:?}:{}:{}",
                object.object_type, object.object_handle, name
            ));
        }
        if data.object_count > 4 {
            object_names.push(format!("+{}", data.object_count - 4));
        }
    }
    let objects = if object_names.is_empty() {
        "-".to_string()
    } else {
        object_names.join(", ")
    };
    let payload = format!("[{message_types:?}/{id}] objects=[{objects}] {message}");

    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        mir_log::error!(target: "vulkan", "{payload}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
        mir_log::warn!(target: "vulkan", "{payload}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::INFO) {
        mir_log::info!(target: "vulkan", "{payload}");
    } else {
        mir_log::debug!(target: "vulkan", "{payload}");
    }

    // 返回 FALSE：校验层不应因为日志而中止 Vulkan 调用。
    vk::FALSE
}

fn create_debug_messenger(instance: &Instance) -> Option<vk::DebugUtilsMessengerEXT> {
    let create_info = debug_messenger_create_info();
    // SAFETY: instance 有效且创建时启用了 VK_EXT_debug_utils；回调为 `extern "system"` 且不逃逸数据。
    match unsafe { instance.create_debug_utils_messenger_ext(&create_info, None) } {
        Ok(messenger) => {
            mir_log::info!(target: "vulkan", "校验层已启用");
            Some(messenger)
        }
        Err(error) => {
            mir_log::warn!(target: "vulkan", "debug utils messenger 创建失败，校验层不可用：{error}");
            None
        }
    }
}

/// 帧在飞行数：CPU 可领先 GPU 的帧数。每帧拥有独立的命令资源、同步原语与相机 UBO。
const MAX_FRAMES_IN_FLIGHT: usize = 2;

/// 单帧录制与同步资源；共 [`MAX_FRAMES_IN_FLIGHT`] 份环形复用，
/// 复用前通过本帧 fence 等待上一轮该帧的 GPU 工作完成。
struct FrameResources {
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,
    /// 每帧一个 acquire 信号量：帧在飞行保证同一信号量不会被并发 acquire 复用。
    image_available: vk::Semaphore,
    /// 每张交换链图像一个呈现信号量（每帧各持一份）：
    /// 避免同一信号量在"已呈现但尚未重新 acquire"的图像间复用（v0.2 VUID-00067 修复），
    /// 每帧再各持一份以适配多帧在飞行。
    render_finished: Vec<vk::Semaphore>,
    /// 相机 UBO（每帧一份，HOST_VISIBLE|COHERENT，持久映射）。
    camera_buffer: vk::Buffer,
    camera_memory: vk::DeviceMemory,
    /// `camera_memory` 的持久映射；写入前由本帧 fence 保证 GPU 已读完上一轮内容。
    camera_mapped: *mut u8,
    extent: vk::Extent2D,
}

struct PipelineResources {
    vertex_module: vk::ShaderModule,
    fragment_module: vk::ShaderModule,
    descriptor_set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    /// 每帧一个相机 UBO 描述符集，按下标与 `VulkanCore::frames` 对齐。
    descriptor_sets: Vec<vk::DescriptorSet>,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
}

/// [`VulkanCore::init_pipeline`] 中途失败的回收守卫：任一步返回错误即按逆序销毁已创建对象；
/// 成功路径通过 [`PipelineGuard::defuse`] 移交所有权。
struct PipelineGuard<'a> {
    device: &'a Device,
    vertex_module: Option<vk::ShaderModule>,
    fragment_module: Option<vk::ShaderModule>,
    descriptor_set_layout: Option<vk::DescriptorSetLayout>,
    descriptor_pool: Option<vk::DescriptorPool>,
    pipeline_layout: Option<vk::PipelineLayout>,
    pipeline: Option<vk::Pipeline>,
}

impl PipelineGuard<'_> {
    /// 成功路径：移交全部所有权，此后 Drop 为空操作。
    #[allow(clippy::type_complexity)]
    fn defuse(
        mut self,
    ) -> (
        vk::ShaderModule,
        vk::ShaderModule,
        vk::DescriptorSetLayout,
        vk::DescriptorPool,
        vk::PipelineLayout,
        vk::Pipeline,
    ) {
        (
            self.vertex_module.take().expect("顶点着色器模块已创建"),
            self.fragment_module.take().expect("片元着色器模块已创建"),
            self.descriptor_set_layout.take().expect("描述符布局已创建"),
            self.descriptor_pool.take().expect("描述符池已创建"),
            self.pipeline_layout.take().expect("管线布局已创建"),
            self.pipeline.take().expect("图形管线已创建"),
        )
    }
}

impl Drop for PipelineGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: 各对象由本设备创建且尚未移交所有权；描述符集随池销毁一并释放。
        unsafe {
            if let Some(pipeline) = self.pipeline.take() {
                self.device.destroy_pipeline(pipeline, None);
            }
            if let Some(layout) = self.pipeline_layout.take() {
                self.device.destroy_pipeline_layout(layout, None);
            }
            if let Some(pool) = self.descriptor_pool.take() {
                self.device.destroy_descriptor_pool(pool, None);
            }
            if let Some(layout) = self.descriptor_set_layout.take() {
                self.device.destroy_descriptor_set_layout(layout, None);
            }
            if let Some(module) = self.fragment_module.take() {
                self.device.destroy_shader_module(module, None);
            }
            if let Some(module) = self.vertex_module.take() {
                self.device.destroy_shader_module(module, None);
            }
        }
    }
}

/// 相机 UBO（binding 0）：`view_proj`（4 × 4 f32）。
const CAMERA_UBO_SIZE: u64 = 64;

/// 逐实例 push constants：`model` + `tint`，共 80 字节（低于 128 字节下限）。
#[repr(C)]
#[derive(Clone, Copy)]
struct PushConstants {
    model: [f32; 16],
    tint: [f32; 4],
}

const PUSH_CONSTANT_SIZE: u32 = 80;

fn push_constant_bytes(pc: &PushConstants) -> &[u8] {
    // SAFETY: `PushConstants` 为 repr(C) 且只含 f32，无填充，大小 80。
    unsafe {
        std::slice::from_raw_parts(
            (pc as *const PushConstants).cast::<u8>(),
            std::mem::size_of::<PushConstants>(),
        )
    }
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

/// Vulkan 核心对象。按 frame → pipeline → image views → swapchain → surface → device → instance 的逆序销毁。
pub(super) struct VulkanCore {
    pipeline: Option<PipelineResources>,
    /// 每帧独立资源，环形复用；仅带 surface 路径创建。
    frames: Vec<FrameResources>,
    /// 当前使用的帧下标（环形递增）。
    frame_index: usize,
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
    /// 校验层 debug messenger；必须在 instance 之前销毁。
    debug_messenger: Option<vk::DebugUtilsMessengerEXT>,
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
    pub(super) fn new_headless(validation: bool) -> Result<Self, RenderInitError> {
        Self::create(None, None, 0, 0, validation)
    }

    /// 带窗口初始化：创建 surface 与交换链，并要求物理设备支持呈现。
    pub(super) fn with_surface(
        display: &dyn HasDisplayHandle,
        window: &dyn HasWindowHandle,
        width: u32,
        height: u32,
        validation: bool,
    ) -> Result<Self, RenderInitError> {
        let mut core = Self::create(Some(display), Some(window), width, height, validation)?;
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
            vec![VALIDATION_LAYER.as_ptr()]
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
        let semaphore_info = vk::SemaphoreCreateInfo::builder();
        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            // 单帧资源创建失败时先就地回收已创建的句柄，再向上传播错误。
            let frame = self.create_frame_sync(&semaphore_info)?;
            self.frames.push(frame);
        }
        Ok(())
    }

    /// 创建一帧的全部资源：命令池/缓冲、fence（创建即 SIGNALED）、信号量与相机 UBO（持久映射）。
    ///
    /// 失败时销毁本次已创建的对象，不产生泄漏。
    fn create_frame_sync(
        &self,
        semaphore_info: &vk::SemaphoreCreateInfoBuilder,
    ) -> Result<FrameResources, String> {
        let pool_info = vk::CommandPoolCreateInfo::builder()
            .queue_family_index(self.graphics_family)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        // SAFETY: graphics_family 是本设备支持的队列族。
        let command_pool = unsafe { self.device.create_command_pool(&pool_info, None) }
            .map_err(|error| format!("创建命令池失败: {error}"))?;

        // 任一步失败都经 `fail` 就地回收已创建的句柄。
        macro_rules! fail {
            ($error:expr) => {{
                let error: String = $error;
                // SAFETY: 以下句柄由本设备创建且尚未销毁。
                unsafe {
                    self.device.destroy_command_pool(command_pool, None);
                }
                return Err(error);
            }};
        }

        let allocate_info = vk::CommandBufferAllocateInfo::builder()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: command_pool 有效，请求一个主命令缓冲。
        let command_buffer = match unsafe { self.device.allocate_command_buffers(&allocate_info) }
            .map_err(|error| format!("分配命令缓冲失败: {error}"))?
            .into_iter()
            .next()
        {
            Some(buffer) => buffer,
            None => fail!("命令缓冲分配结果为空".to_string()),
        };

        let fence_info = vk::FenceCreateInfo::builder().flags(vk::FenceCreateFlags::SIGNALED);
        // SAFETY: fence_info 有效；创建即 SIGNALED 保证该帧首轮不被阻塞。
        let fence = match unsafe { self.device.create_fence(&fence_info, None) } {
            Ok(fence) => fence,
            Err(error) => fail!(format!("创建 fence 失败: {error}")),
        };

        // SAFETY: semaphore_info 有效。
        let image_available = match unsafe { self.device.create_semaphore(semaphore_info, None) } {
            Ok(semaphore) => semaphore,
            Err(error) => {
                // SAFETY: fence 由本设备创建且尚未销毁。
                unsafe { self.device.destroy_fence(fence, None) };
                fail!(format!("创建 semaphore 失败: {error}"))
            }
        };
        let mut render_finished = Vec::with_capacity(self.image_views.len());
        for _ in 0..self.image_views.len() {
            // SAFETY: semaphore_info 有效。
            let semaphore = match unsafe { self.device.create_semaphore(semaphore_info, None) } {
                Ok(semaphore) => semaphore,
                Err(error) => {
                    // SAFETY: 以上句柄由本设备创建且尚未销毁。
                    unsafe {
                        self.device.destroy_semaphore(image_available, None);
                        self.device.destroy_fence(fence, None);
                    }
                    fail!(format!("创建 semaphore 失败: {error}"))
                }
            };
            render_finished.push(semaphore);
        }

        let (camera_buffer, camera_memory) = match self.create_buffer(
            CAMERA_UBO_SIZE,
            vk::BufferUsageFlags::UNIFORM_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        ) {
            Ok(created) => created,
            Err(error) => {
                // SAFETY: 以上句柄由本设备创建且尚未销毁。
                unsafe {
                    for semaphore in render_finished.drain(..) {
                        self.device.destroy_semaphore(semaphore, None);
                    }
                    self.device.destroy_semaphore(image_available, None);
                    self.device.destroy_fence(fence, None);
                }
                fail!(error)
            }
        };
        // 持久映射：HOST_COHERENT 无需 flush，映射保持到内存释放。
        let mapped = unsafe {
            self.device.map_memory(
                camera_memory,
                0,
                CAMERA_UBO_SIZE,
                vk::MemoryMapFlags::empty(),
            )
        };
        let camera_mapped = match mapped {
            Ok(mapped) => mapped.cast::<u8>(),
            Err(error) => {
                // SAFETY: 以上句柄由本设备创建且尚未销毁。
                unsafe {
                    self.device.destroy_buffer(camera_buffer, None);
                    self.device.free_memory(camera_memory, None);
                    for semaphore in render_finished.drain(..) {
                        self.device.destroy_semaphore(semaphore, None);
                    }
                    self.device.destroy_semaphore(image_available, None);
                    self.device.destroy_fence(fence, None);
                }
                fail!(format!("映射相机 UBO 失败: {error}"))
            }
        };

        Ok(FrameResources {
            command_pool,
            command_buffer,
            fence,
            image_available,
            render_finished,
            camera_buffer,
            camera_memory,
            camera_mapped,
            extent: self.extent,
        })
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
            vulkanalia::include_shader_code!("../../../shaders/cube/cube.vert.spv");
        const FRAGMENT_CODE: &[u32] =
            vulkanalia::include_shader_code!("../../../shaders/cube/cube.frag.spv");

        // 失败回收守卫：任一步出错即销毁已创建对象，成功时 defuse 移交所有权。
        let mut guard = PipelineGuard {
            device: &self.device,
            vertex_module: None,
            fragment_module: None,
            descriptor_set_layout: None,
            descriptor_pool: None,
            pipeline_layout: None,
            pipeline: None,
        };

        let vertex_info = vk::ShaderModuleCreateInfo::builder()
            .code_size(VERTEX_CODE.len() * 4)
            .code(VERTEX_CODE);
        // SAFETY: VERTEX_CODE 是已校验的 SPIR-V 字节码，生命周期覆盖模块创建。
        let vertex_module = unsafe { self.device.create_shader_module(&vertex_info, None) }
            .map_err(|error| format!("创建顶点着色器模块失败: {error}"))?;
        guard.vertex_module = Some(vertex_module);
        let fragment_info = vk::ShaderModuleCreateInfo::builder()
            .code_size(FRAGMENT_CODE.len() * 4)
            .code(FRAGMENT_CODE);
        // SAFETY: FRAGMENT_CODE 是已校验的 SPIR-V 字节码，生命周期覆盖模块创建。
        let fragment_module = unsafe { self.device.create_shader_module(&fragment_info, None) }
            .map_err(|error| format!("创建片元着色器模块失败: {error}"))?;
        guard.fragment_module = Some(fragment_module);

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

        // 顶点布局：位置 @0、颜色 @1，stride 24（与 `Vertex` 一致）。
        let vertex_binding = vk::VertexInputBindingDescription::builder()
            .binding(0)
            .stride(Vertex::SIZE as u32)
            .input_rate(vk::VertexInputRate::VERTEX)
            .build();
        let vertex_attributes = [
            vk::VertexInputAttributeDescription::builder()
                .location(0)
                .binding(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(0)
                .build(),
            vk::VertexInputAttributeDescription::builder()
                .location(1)
                .binding(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(12)
                .build(),
        ];
        let vertex_bindings = [vertex_binding];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::builder()
            .vertex_binding_descriptions(&vertex_bindings)
            .vertex_attribute_descriptions(&vertex_attributes)
            .build();

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::builder()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false)
            .build();
        let viewport_state = vk::PipelineViewportStateCreateInfo::builder()
            .viewport_count(1)
            .scissor_count(1)
            .build();
        // 模型空间绕序为"从外部看逆时针"；Vulkan NDC 为 Y 向下，投影后按顺时针判定正面。
        let rasterization = vk::PipelineRasterizationStateCreateInfo::builder()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::CLOCKWISE)
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

        // 描述符集 0 / binding 0：相机 UBO。
        let ubo_binding = vk::DescriptorSetLayoutBinding::builder()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX);
        let descriptor_bindings = [ubo_binding];
        let descriptor_layout_info =
            vk::DescriptorSetLayoutCreateInfo::builder().bindings(&descriptor_bindings);
        // SAFETY: 绑定描述有效。
        let descriptor_set_layout = unsafe {
            self.device
                .create_descriptor_set_layout(&descriptor_layout_info, None)
        }
        .map_err(|error| format!("创建描述符布局失败: {error}"))?;
        guard.descriptor_set_layout = Some(descriptor_set_layout);

        let pool_sizes = [vk::DescriptorPoolSize {
            type_: vk::DescriptorType::UNIFORM_BUFFER,
            descriptor_count: MAX_FRAMES_IN_FLIGHT as u32,
        }];
        let pool_info = vk::DescriptorPoolCreateInfo::builder()
            .pool_sizes(&pool_sizes)
            .max_sets(MAX_FRAMES_IN_FLIGHT as u32);
        // SAFETY: 池大小与最大集合数有效。
        let descriptor_pool = unsafe { self.device.create_descriptor_pool(&pool_info, None) }
            .map_err(|error| format!("创建描述符池失败: {error}"))?;
        guard.descriptor_pool = Some(descriptor_pool);

        // 分配请求每帧一个集合；管线布局仍只声明 1 个 set。
        let allocate_layouts = [descriptor_set_layout; MAX_FRAMES_IN_FLIGHT];
        let allocate_info = vk::DescriptorSetAllocateInfo::builder()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&allocate_layouts);
        // SAFETY: 池与布局有效，请求每帧一个描述符集。
        let descriptor_sets = unsafe { self.device.allocate_descriptor_sets(&allocate_info) }
            .map_err(|error| format!("分配描述符集失败: {error}"))?;
        if descriptor_sets.len() != self.frames.len() {
            return Err(format!(
                "描述符集数量与帧数不一致: {} != {}",
                descriptor_sets.len(),
                self.frames.len()
            ));
        }

        // 相机 UBO 属于每帧资源（`frames`），这里只为各帧描述符集写入绑定。
        let buffer_infos: Vec<vk::DescriptorBufferInfo> = self
            .frames
            .iter()
            .map(|frame| {
                vk::DescriptorBufferInfo::builder()
                    .buffer(frame.camera_buffer)
                    .offset(0)
                    .range(CAMERA_UBO_SIZE)
                    .build()
            })
            .collect();
        // 必须命名中间数组：`WriteDescriptorSet` 保存的是指针，临时数组会在语句结束即失效
        // （debug 构建侥幸可用，release 优化后会读到已经被覆盖的栈内存）。
        let writes: Vec<vk::WriteDescriptorSet> = descriptor_sets
            .iter()
            .zip(&buffer_infos)
            .map(|(&set, info)| {
                vk::WriteDescriptorSet::builder()
                    .dst_set(set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(std::slice::from_ref(info))
                    .build()
            })
            .collect();
        // SAFETY: 描述符集与 buffer_infos（命名 Vec）在调用期间有效。
        unsafe {
            let no_copies: [vk::CopyDescriptorSet; 0] = [];
            self.device.update_descriptor_sets(&writes, &no_copies);
        }

        let push_range = vk::PushConstantRange::builder()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(PUSH_CONSTANT_SIZE)
            .build();
        let push_ranges = [push_range];
        let single_layouts = [descriptor_set_layout];
        let layout_info = vk::PipelineLayoutCreateInfo::builder()
            .set_layouts(&single_layouts)
            .push_constant_ranges(&push_ranges);
        // SAFETY: layout_info 有效：1 个描述符集 + 80 字节 push constants。
        let layout = unsafe { self.device.create_pipeline_layout(&layout_info, None) }
            .map_err(|error| format!("创建管线布局失败: {error}"))?;
        guard.pipeline_layout = Some(layout);

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
        guard.pipeline = Some(pipeline);

        // 全部对象创建成功：从守卫移交所有权。
        let (
            vertex_module,
            fragment_module,
            descriptor_set_layout,
            descriptor_pool,
            layout,
            pipeline,
        ) = guard.defuse();
        self.pipeline = Some(PipelineResources {
            vertex_module,
            fragment_module,
            descriptor_set_layout,
            descriptor_pool,
            descriptor_sets,
            layout,
            pipeline,
        });
        Ok(())
    }

    fn destroy_pipeline(&mut self) {
        if let Some(resources) = self.pipeline.take() {
            // SAFETY: 这些对象由本对象创建且尚未销毁；调用方保证 GPU 已空闲。
            // 相机 UBO 属于每帧资源，随帧资源销毁，不在这里释放。
            unsafe {
                self.device.destroy_pipeline(resources.pipeline, None);
                self.device.destroy_pipeline_layout(resources.layout, None);
                self.device
                    .destroy_descriptor_pool(resources.descriptor_pool, None);
                self.device
                    .destroy_descriptor_set_layout(resources.descriptor_set_layout, None);
                self.device
                    .destroy_shader_module(resources.fragment_module, None);
                self.device
                    .destroy_shader_module(resources.vertex_module, None);
            }
        }
    }

    /// 解析网格句柄；世代号不匹配（已被销毁/复用）时返回 `None`。
    fn mesh_lookup(meshes: &[MeshSlot], handle: MeshHandle) -> Option<&GpuMesh> {
        let slot = meshes.get(handle.index())?;
        if slot.generation != handle.generation() {
            return None;
        }
        slot.mesh.as_ref()
    }

    /// 当前交换链尺寸是否与应用层已知的窗口尺寸不一致。
    fn target_differs(&self, width: u32, height: u32) -> bool {
        self.extent.width != width || self.extent.height != height
    }

    /// 录制一帧：清屏 → 逐实例绘制 → 提交 → 呈现。无交换链时为空操作（headless）。
    ///
    /// 交换链失效（`OUT_OF_DATE`）不视为帧错误：标记后本帧跳过，下一帧开头按当前窗口尺寸重建。
    /// `width` / `height` 是应用层已知的窗口尺寸，用于重建时取最新目标大小；
    /// `view_proj` 是本帧相机，`draws` 是应用层提交的实例列表。
    ///
    /// 帧资源按 `frame_index` 环形复用（[`MAX_FRAMES_IN_FLIGHT`] 帧在飞行）：
    /// 复用前等待本帧 fence，保证上一轮该帧的 GPU 工作（含 UBO 读取）已完成。
    pub(super) fn render_frame(
        &mut self,
        clear_color: [f32; 4],
        width: u32,
        height: u32,
        view_proj: Mat4,
        draws: &[DrawInstance],
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
        if self.frames.len() != MAX_FRAMES_IN_FLIGHT {
            return Ok(());
        }
        let frame_index = self.frame_index;

        // SAFETY: fence/semaphore/命令缓冲均有效且由本对象独占；等待上一轮该帧完成后再复用。
        unsafe {
            let frame = &mut self.frames[frame_index];
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

            // 从"未定义"转到颜色附件：本帧首操作是 CLEAR，旧内容本就要丢弃，
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

                // 相机 UBO：写入当前帧的持久映射。
                // 本帧 fence 等待已保证上一轮该帧的 GPU 读取结束，HOST_COHERENT 无需 flush。
                let camera_matrix = view_proj.to_cols_array();
                // SAFETY: camera_mapped 是创建时 map_memory 得到的持久映射指针，
                // 有效范围 CAMERA_UBO_SIZE 字节，仅由本对象独占访问。
                std::ptr::copy_nonoverlapping(
                    camera_matrix.as_ptr().cast::<u8>(),
                    frame.camera_mapped,
                    CAMERA_UBO_SIZE as usize,
                );

                let descriptor_set = resources.descriptor_sets[frame_index];
                self.device.cmd_bind_descriptor_sets(
                    frame.command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    resources.layout,
                    0,
                    &[descriptor_set],
                    &[],
                );

                for draw in draws {
                    let Some(mesh) = Self::mesh_lookup(&self.meshes, draw.mesh) else {
                        continue;
                    };
                    let push = PushConstants {
                        model: draw.data.model.to_cols_array(),
                        tint: draw.data.tint,
                    };
                    self.device.cmd_push_constants(
                        frame.command_buffer,
                        resources.layout,
                        vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                        0,
                        push_constant_bytes(&push),
                    );
                    self.device.cmd_bind_vertex_buffers(
                        frame.command_buffer,
                        0,
                        &[mesh.vertex_buffer],
                        &[0],
                    );
                    self.device.cmd_bind_index_buffer(
                        frame.command_buffer,
                        mesh.index_buffer,
                        0,
                        vk::IndexType::UINT32,
                    );
                    self.device.cmd_draw_indexed(
                        frame.command_buffer,
                        mesh.index_count,
                        1,
                        0,
                        0,
                        0,
                    );
                }
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
            // 本帧与本张图像专属的呈现信号量，避免在图像未重新 acquire 前复用。
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
        // 本帧提交完成，推进到下一帧资源；acquire 提前返回的路径不推进，
        // 下次仍从同一帧资源开始（其 fence 尚未重置，状态一致）。
        self.frame_index = (frame_index + 1) % MAX_FRAMES_IN_FLIGHT;
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
        for frame in &mut self.frames {
            frame.extent = self.extent;
        }
        Ok(())
    }
}

impl Drop for VulkanCore {
    fn drop(&mut self) {
        // SAFETY: 本对象仍持有有效的 Vulkan 句柄；先等待 GPU 空闲，再按
        // frame → pipeline → image views → swapchain → surface → messenger → device → instance 逆序释放。
        unsafe {
            if !self.frames.is_empty() {
                let _ = self.device.device_wait_idle();
            }
            for frame in self.frames.drain(..) {
                for semaphore in frame.render_finished {
                    self.device.destroy_semaphore(semaphore, None);
                }
                self.device.destroy_semaphore(frame.image_available, None);
                self.device.destroy_fence(frame.fence, None);
                // 持久映射随内存释放自动解除。
                self.device.destroy_buffer(frame.camera_buffer, None);
                self.device.free_memory(frame.camera_memory, None);
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
            if let Some(messenger) = self.debug_messenger.take() {
                self.instance
                    .destroy_debug_utils_messenger_ext(messenger, None);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_constant_layout_is_80_bytes() {
        assert_eq!(
            std::mem::size_of::<PushConstants>(),
            PUSH_CONSTANT_SIZE as usize
        );
        assert_eq!(std::mem::align_of::<PushConstants>(), 4);
    }

    #[test]
    fn camera_ubo_size_is_64_bytes() {
        assert_eq!(CAMERA_UBO_SIZE, 64);
    }

    #[test]
    fn frames_in_flight_is_two() {
        // 批次 A 的决策：2 帧在飞行；修改时需同步核对信号量复用与 UBO 环形写入。
        assert_eq!(MAX_FRAMES_IN_FLIGHT, 2);
    }

    #[test]
    fn validation_layer_name_is_standard() {
        assert_eq!(VALIDATION_LAYER.to_bytes(), b"VK_LAYER_KHRONOS_validation");
    }

    #[test]
    fn debug_messenger_subscribes_to_errors_and_warnings() {
        let info = debug_messenger_create_info();
        assert!(info.message_severity.contains(
            vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
        ));
        assert!(
            info.message_type
                .contains(vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION)
        );
        assert!(info.user_callback.is_some());
    }
}
