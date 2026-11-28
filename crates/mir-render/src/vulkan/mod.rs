//! `vulkanalia` 内部实现：实例、物理设备、逻辑设备与队列。
//!
//! 本模块是 crate 私有的；Vulkan 类型不得出现在 `mir-render` 的公共 API 中。
//! 按功能拆分为子模块：
//! - [`debug`]：Khronos 校验层与 debug utils 回调；
//! - [`init`]：实例 / surface / 物理设备 / 逻辑设备创建；
//! - [`swapchain`]：交换链创建与重建；
//! - [`frame`]：每帧命令与同步资源、相机 UBO、时间戳查询池；
//! - [`depth`]：深度附件资源；
//! - [`buffer`]：GPU 缓冲分配、staging 上传与延迟销毁队列；
//! - [`mesh`]：网格资源与世代槽位；
//! - [`material`]：材质资源与世代槽位；
//! - [`texture`]：纹理资源（图像 + 视图 + 采样器 + staging 上传，M5 批次 A）；
//! - [`pipeline`]：图形管线与描述符布局；
//! - [`pipeline_cache`]：管线缓存与磁盘持久化（R-07）；
//! - [`frame_graph`]：RenderGraph 声明、dump 与 barrier 翻译；
//! - [`render`]：每帧录制、提交与呈现。

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use vulkanalia::prelude::v1_0::*;
use vulkanalia::vk::{
    ExtDebugUtilsExtensionInstanceCommands, KhrSurfaceExtensionInstanceCommands,
    KhrSwapchainExtensionDeviceCommands,
};
use vulkanalia::{Device, Entry, Instance};

use crate::RenderInitError;
use crate::allocator::MemoryAllocator;
use crate::resource_table::SlotTable;

mod buffer;
mod debug;
mod depth;
mod frame;
mod frame_graph;
mod init;
mod material;
mod mesh;
mod pipeline;
mod pipeline_cache;
mod render;
mod swapchain;
mod texture;

use buffer::DeletionEntry;
use depth::DepthResources;
use frame::FrameResources;
use material::GpuMaterial;
use mesh::GpuMesh;
use pipeline::PipelineResources;
use texture::GpuTexture;

/// Vulkan 核心对象。按 frame → pipeline → image views → swapchain → surface → device → instance 的逆序销毁。
pub(super) struct VulkanCore {
    pipeline: Option<PipelineResources>,
    /// 常驻管线缓存（R-07）；仅带 surface 路径创建，`Drop` 在 wait_idle 后销毁。
    pipeline_cache: Option<vk::PipelineCache>,
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
    /// 网格槽位表（世代号 + 空闲栈，两后端共享实现）。
    meshes: SlotTable<GpuMesh>,
    /// 材质槽位表（R-11）。
    materials: SlotTable<GpuMaterial>,
    /// 纹理槽位表（M5 批次 A）。
    textures: SlotTable<GpuTexture>,
    /// 默认材质（白色 Lambert）：随 `init_pipeline` 创建，绘制路径在材质句柄
    /// 无效/未提供时回退到它。
    default_material: Option<GpuMaterial>,
    /// 待销毁的 GPU 缓冲（R-09）：绑定帧 fence 延迟回收。
    deletions: Vec<DeletionEntry>,
    /// 显存子分配器（R-08）：声明在 `device` 之前，保证字段 Drop 顺序安全。
    allocator: MemoryAllocator,
    graphics_family: u32,
    present_family: u32,
    device_name: String,
    api_version: String,
    surface_format: Option<String>,
    present_mode: Option<String>,
    format_value: Option<vk::SurfaceFormatKHR>,
    mode_value: Option<vk::PresentModeKHR>,
    extent: vk::Extent2D,
    /// 每帧把 RenderGraph 编译产物 dump 成 DOT 文件（R-05，`--graph-dump` 控制）。
    graph_dump: bool,
    /// dump 落盘失败只告警一次。
    dump_warned: bool,
    /// graphics 队列族的 timestampValidBits（R-05；0 表示不支持时间戳）。
    timestamp_valid_bits: u32,
    /// 设备 limits.timestampPeriod（每个时间戳 tick 的纳秒数）。
    timestamp_period: f32,
    /// 上一帧各 GPU pass 的耗时（毫秒），由 `read_pass_timings` 回读填充。
    last_pass_timings: Vec<(String, f32)>,
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

    /// 开关每帧 RenderGraph 依赖图 dump（R-05）。
    pub(super) fn set_graph_dump(&mut self, enabled: bool) {
        self.graph_dump = enabled;
    }

    /// 上一帧各 GPU pass 的耗时（毫秒；headless/不支持时间戳时为空）。
    pub(super) fn last_pass_timings(&self) -> &[(String, f32)] {
        &self.last_pass_timings
    }

    /// 分配器未释放的子分配字节数（R-10 泄漏守恒断言用）。
    pub(super) fn allocated_bytes(&self) -> u64 {
        self.allocator.allocated_bytes()
    }
}

impl Drop for VulkanCore {
    fn drop(&mut self) {
        // SAFETY: 本对象仍持有有效的 Vulkan 句柄；先等待 GPU 空闲，再按
        // deletion → frame → pipeline → image views → swapchain → surface → messenger
        // → allocator → device → instance 逆序释放。
        unsafe {
            if !self.frames.is_empty() {
                let _ = self.device.device_wait_idle();
            }
            // 设备已空闲：延迟回收队列中的缓冲现在可以安全销毁（R-09）。
            self.flush_deletions(None);
            for frame in self.frames.drain(..) {
                for semaphore in frame.render_finished {
                    self.device.destroy_semaphore(semaphore, None);
                }
                self.device.destroy_semaphore(frame.image_available, None);
                self.device.destroy_fence(frame.fence, None);
                // 持久映射随内存释放自动解除。
                self.device.destroy_buffer(frame.camera_buffer, None);
                self.allocator.free(&frame.camera_allocation);
                if let Some(pool) = frame.timestamp_pool {
                    self.device.destroy_query_pool(pool, None);
                }
                self.device.destroy_command_pool(frame.command_pool, None);
            }
            // 管线销毁必须发生在命令缓冲执行完毕之后。
            self.destroy_pipeline();
            // 管线缓存随管线之后销毁（wait_idle 已在上方执行，R-07）。
            if let Some(cache) = self.pipeline_cache.take() {
                self.device.destroy_pipeline_cache(cache, None);
            }
            self.destroy_depth_resources();
            self.destroy_all_meshes();
            // 存活材质的缓冲在 device 销毁前释放（描述符集随上面的池销毁，R-11）。
            self.destroy_all_materials();
            // 存活纹理在 device 销毁前释放（M5 批次 A）。
            self.destroy_all_textures();
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
            // 分配器持有的全部块/dedicated 内存必须在销毁 device 之前释放（R-08）。
            self.allocator.destroy_all();
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}
