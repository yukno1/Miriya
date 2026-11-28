//! wgpu 后端实现：公共 API 与 Vulkan 共用同一套纯数据帧契约。
//!
//! wgpu 自身负责资源状态跟踪与同步，因此 RenderGraph 在本后端只用于
//! 图校验、pass 执行序与 DOT dump，不生成显式 barrier（见 ADR-0014）。

mod frame;
mod init;
mod pipeline;
mod render;
mod resources;
mod surface;
mod timing;

use wgpu::{Adapter, Device, Instance, Queue, Surface, SurfaceConfiguration, Texture, TextureView};

use crate::resource_table::SlotTable;

use frame::CameraFrameResources;
use pipeline::PipelineResources;
use resources::{GpuMaterial, GpuMesh, GpuTexture};
use timing::TimestampCapture;

/// wgpu 核心对象；`Instance` 必须比 `Surface` / `Device` 活得更久。
pub(crate) struct WgpuCore {
    /// 仅用于保证 Instance 比 Surface / Device 活得更久。
    #[allow(dead_code)]
    pub(crate) instance: Instance,
    pub(crate) adapter: Adapter,
    pub(crate) device: Device,
    pub(crate) queue: Queue,
    pub(crate) surface: Option<Surface<'static>>,
    pub(crate) config: Option<SurfaceConfiguration>,
    pub(crate) depth: Option<Texture>,
    pub(crate) depth_view: Option<TextureView>,
    pub(crate) depth_bytes: u64,
    pub(crate) pipeline: Option<PipelineResources>,
    pub(crate) camera_frames: Vec<CameraFrameResources>,
    pub(crate) frame_index: usize,
    pub(crate) meshes: SlotTable<GpuMesh>,
    pub(crate) materials: SlotTable<GpuMaterial>,
    pub(crate) textures: SlotTable<GpuTexture>,
    pub(crate) default_material: Option<GpuMaterial>,
    /// 渲染器主动请求的字节数（buffer/texture/UBO/诊断缓冲）；不等于驱动实际分配量。
    pub(crate) allocated: u64,
    /// 设备支持时间戳且 period 有效时才存在。
    pub(crate) timing: Option<TimestampCapture>,
    pub(crate) graph_dump: bool,
    pub(crate) dump_warned: bool,
    pub(crate) adapter_name: String,
    pub(crate) backend_name: String,
    pub(crate) surface_format: Option<String>,
    pub(crate) present_mode: Option<String>,
    pub(crate) swapchain_image_count: u32,
}

impl WgpuCore {
    pub(crate) fn device_name(&self) -> &str {
        &self.adapter_name
    }

    pub(crate) fn backend_name(&self) -> &str {
        &self.backend_name
    }

    pub(crate) fn has_surface(&self) -> bool {
        self.surface.is_some()
    }

    pub(crate) fn surface_format(&self) -> Option<&str> {
        self.surface_format.as_deref()
    }

    pub(crate) fn present_mode(&self) -> Option<&str> {
        self.present_mode.as_deref()
    }

    pub(crate) fn swapchain_image_count(&self) -> u32 {
        self.swapchain_image_count
    }

    pub(crate) fn set_graph_dump(&mut self, enabled: bool) {
        self.graph_dump = enabled;
    }

    /// 上一帧各 GPU pass 的耗时（毫秒）；不支持/未采集到时间戳时为空。
    pub(crate) fn last_pass_timings(&self) -> &[(String, f32)] {
        self.timing
            .as_ref()
            .map_or(&[], TimestampCapture::last_pass_timings)
    }

    pub(crate) fn allocated_bytes(&self) -> u64 {
        self.allocated
    }

    pub(crate) fn track_alloc(&mut self, bytes: u64) {
        self.allocated = self.allocated.saturating_add(bytes);
    }

    pub(crate) fn track_free(&mut self, bytes: u64) {
        self.allocated = self.allocated.saturating_sub(bytes);
    }
}
