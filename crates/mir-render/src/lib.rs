//! 渲染子系统：Vulkan 1.3 唯一后端（`vulkanalia`）。
//!
//! 公共 API 不暴露任何 Vulkan 类型；`VulkanCore` 完全封装在 crate 内部
//! （ADR 0005 / README §5.1 规则 2）。当前已接入实例、物理设备、逻辑设备、
//! 队列选择、surface、交换链、多帧在飞行（批次 A）与 RenderGraph v0.5
//! （批次 B：声明式 pass/资源 + 编译期 barrier 推导 + GPU pass 时间戳 + 依赖图 dump）。
//!
//! 无窗口（headless）路径不会创建 surface，也不会要求交换链扩展，
//! 因此 `mir-core` 的 `run_headless` 与 CI 仍然可用。

mod frame;
mod geometry;
mod graph;
mod mesh;
mod vulkan;

pub use frame::{CameraFrame, InstanceData};
pub use geometry::cube;
pub use mesh::{MeshData, MeshHandle, Vertex};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use thiserror::Error;
use vulkan::VulkanCore;

/// 校验层默认策略：debug 构建开启，单元测试关闭以避免噪音。
fn default_validation() -> bool {
    cfg!(debug_assertions) && !cfg!(test)
}

/// 渲染初始化失败。仅用于诊断；`RenderContext::init` 会记录失败并继续以占位模式运行。
#[derive(Debug, Clone, PartialEq, Error)]
pub enum RenderInitError {
    #[error("Vulkan loader unavailable: {0}")]
    Loader(String),
    #[error("Vulkan instance creation failed: {0}")]
    Instance(String),
    #[error("no suitable Vulkan 1.3 physical device")]
    NoDevice,
    #[error("Vulkan logical device creation failed: {0}")]
    Device(String),
    #[error("Vulkan surface creation failed: {0}")]
    Surface(String),
    #[error("Vulkan swapchain creation failed: {0}")]
    Swapchain(String),
    #[error("Vulkan frame resources initialization failed: {0}")]
    FrameResources(String),
    #[error("Vulkan depth resources initialization failed: {0}")]
    Depth(String),
    #[error("Vulkan transfer resources initialization failed: {0}")]
    Transfer(String),
    #[error("Vulkan mesh upload failed: {0}")]
    Mesh(String),
    #[error("renderer is not initialized")]
    NotInitialized,
    #[error("renderer is already initialized")]
    AlreadyInitialized,
}

/// 渲染后端当前状态（UI/应用层可显示，不含 Vulkan 类型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererStatus {
    /// 尚未调用初始化。
    Uninitialized,
    /// Vulkan 已就绪。
    Vulkan {
        device_name: String,
        api_version: String,
        has_surface: bool,
        graphics_family: u32,
        present_family: u32,
        surface_format: Option<String>,
        present_mode: Option<String>,
        swapchain_image_count: u32,
    },
    /// 初始化失败，附可读原因。
    Failed(String),
}

/// 渲染上下文：持有后端状态，由主循环持有。
pub struct RenderContext {
    pub exit_requested: bool,
    initialized: bool,
    width: u32,
    height: u32,
    backend: Option<VulkanCore>,
    status: RendererStatus,
    clear_color: [f32; 4],
    camera: CameraFrame,
    draws: Vec<frame::DrawInstance>,
    frame_error: Option<String>,
    validation: bool,
    /// 每帧把 RenderGraph 依赖图 dump 成 DOT 文件（批次 B R-05）。
    graph_dump: bool,
}

impl RenderContext {
    /// 创建渲染上下文。
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            exit_requested: false,
            initialized: false,
            width,
            height,
            backend: None,
            status: RendererStatus::Uninitialized,
            clear_color: [0.0, 0.0, 0.0, 1.0],
            camera: CameraFrame::default(),
            draws: Vec::new(),
            frame_error: None,
            validation: default_validation(),
            graph_dump: false,
        }
    }

    /// 覆盖校验层开关；需在 `init` / `init_with_window` 之前调用。
    pub fn with_validation(mut self, enabled: bool) -> Self {
        self.validation = enabled;
        self
    }

    /// 覆盖 RenderGraph 依赖图 dump 开关；需在 `init` / `init_with_window` 之前调用。
    ///
    /// 开启后每帧把依赖图 DOT 写入工作目录下的 `render_graph.dot`（调试用，默认关闭）。
    pub fn with_graph_dump(mut self, enabled: bool) -> Self {
        self.graph_dump = enabled;
        self
    }

    /// 当前校验层配置（实际是否启用取决于驱动是否提供 layer）。
    pub fn validation(&self) -> bool {
        self.validation
    }

    /// 以 headless 方式初始化：尝试创建 Vulkan 实例与逻辑设备，但不创建 surface。
    ///
    /// 失败时记录 [`RendererStatus::Failed`] 而不 panic，保证 CI/无 GPU 环境可运行。
    pub fn init(&mut self) {
        if self.initialized {
            return;
        }
        match VulkanCore::new_headless(self.validation) {
            Ok(mut backend) => {
                self.status = RendererStatus::Vulkan {
                    device_name: backend.device_name().to_string(),
                    api_version: backend.api_version().to_string(),
                    has_surface: backend.has_surface(),
                    graphics_family: backend.queue_families().0,
                    present_family: backend.queue_families().1,
                    surface_format: backend.surface_format().map(str::to_string),
                    present_mode: backend.present_mode().map(str::to_string),
                    swapchain_image_count: backend.swapchain_image_count(),
                };
                backend.set_graph_dump(self.graph_dump);
                self.backend = Some(backend);
            }
            Err(error) => {
                self.status = RendererStatus::Failed(error.to_string());
            }
        }
        self.initialized = true;
    }

    /// 使用窗口初始化：创建 Vulkan surface，并要求设备支持呈现。
    ///
    /// `display` / `window` 由 `mir-window` 的 raw-window-handle 提供；
    /// 窗口必须比 [`RenderContext`] 活得更久（surface 在 Drop 时才销毁）。
    pub fn init_with_window(
        &mut self,
        display: &dyn HasDisplayHandle,
        window: &dyn HasWindowHandle,
    ) -> Result<(), RenderInitError> {
        if self.initialized {
            return Err(RenderInitError::AlreadyInitialized);
        }
        match VulkanCore::with_surface(display, window, self.width, self.height, self.validation) {
            Ok(mut backend) => {
                self.status = RendererStatus::Vulkan {
                    device_name: backend.device_name().to_string(),
                    api_version: backend.api_version().to_string(),
                    has_surface: backend.has_surface(),
                    graphics_family: backend.queue_families().0,
                    present_family: backend.queue_families().1,
                    surface_format: backend.surface_format().map(str::to_string),
                    present_mode: backend.present_mode().map(str::to_string),
                    swapchain_image_count: backend.swapchain_image_count(),
                };
                backend.set_graph_dump(self.graph_dump);
                self.backend = Some(backend);
                self.initialized = true;
                Ok(())
            }
            Err(error) => {
                self.status = RendererStatus::Failed(error.to_string());
                self.initialized = true;
                Err(error)
            }
        }
    }

    /// 是否已完成初始化（含降级为占位模式的情况）。
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// 当前后端状态。
    pub fn status(&self) -> &RendererStatus {
        &self.status
    }

    /// 最近一帧的错误（无交换链/无后端时为 `None`）。
    pub fn frame_error(&self) -> Option<&str> {
        self.frame_error.as_deref()
    }

    /// 运行时开关 RenderGraph 依赖图 dump（初始化前后均可调用）。
    pub fn set_graph_dump(&mut self, enabled: bool) {
        self.graph_dump = enabled;
        if let Some(backend) = self.backend.as_mut() {
            backend.set_graph_dump(enabled);
        }
    }

    /// 上一帧各 GPU pass 的耗时（毫秒；headless / 不支持时间戳时为空）。
    pub fn pass_timings(&self) -> &[(String, f32)] {
        self.backend
            .as_ref()
            .map(|backend| backend.last_pass_timings())
            .unwrap_or(&[])
    }

    /// 当前渲染目标尺寸（随窗口缩放更新）。
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 窗口缩放时更新渲染目标尺寸，并重建交换链。
    ///
    /// 最小化（宽或高为 0）时只记录尺寸、保留旧交换链，恢复为非零尺寸后再重建。
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        if width == 0 || height == 0 {
            return;
        }
        if let Some(backend) = self.backend.as_mut()
            && let Err(error) = backend.recreate_swapchain(width, height)
        {
            self.frame_error = Some(error);
        }
    }

    /// 记录本帧清屏色（实际清屏在 `end_frame` 提交），并清空上一帧的绘制请求。
    pub fn begin_frame(&mut self, clear_color: [f32; 4]) {
        debug_assert!(self.initialized);
        self.clear_color = clear_color;
        self.draws.clear();
    }

    /// 本帧相机（view-projection）。
    pub fn set_camera(&mut self, camera: CameraFrame) {
        self.camera = camera;
    }

    /// 提交一次绘制：网格句柄 + 实例数据（同一网格可提交多次）。
    pub fn submit(&mut self, mesh: MeshHandle, instance: InstanceData) {
        self.draws.push(frame::DrawInstance {
            mesh,
            data: instance,
        });
    }

    /// 本帧收集到的绘制请求数量（测试/诊断）。
    pub fn submitted_instances(&self) -> usize {
        self.draws.len()
    }

    /// 创建 GPU 网格：需要先完成渲染初始化（headless 下也可上传，只是不会被绘制）。
    pub fn create_mesh(&mut self, data: &MeshData) -> Result<MeshHandle, RenderInitError> {
        let backend = self
            .backend
            .as_mut()
            .ok_or(RenderInitError::NotInitialized)?;
        backend.create_mesh(data).map_err(RenderInitError::Mesh)
    }

    /// 销毁网格；句柄无效或已销毁时为空操作。
    pub fn destroy_mesh(&mut self, handle: MeshHandle) {
        if let Some(backend) = self.backend.as_mut() {
            let _ = backend.destroy_mesh(handle);
        }
    }

    /// 完成本帧：清屏 → 提交 → 呈现。headless 或窗口最小化时为空操作。
    pub fn end_frame(&mut self) {
        debug_assert!(self.initialized);
        // 最小化时窗口尺寸为 0：跳过 acquire/present，避免查询/提交到无效交换链。
        if self.width == 0 || self.height == 0 {
            return;
        }
        if let Some(backend) = self.backend.as_mut() {
            if let Err(error) = backend.render_frame(
                self.clear_color,
                self.width,
                self.height,
                self.camera.view_proj,
                &self.draws,
            ) {
                self.frame_error = Some(error);
            } else {
                self.frame_error = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_context_is_uninitialized() {
        let context = RenderContext::new(1280, 720);
        assert!(!context.is_initialized());
        assert_eq!(context.status(), &RendererStatus::Uninitialized);
        assert_eq!(context.size(), (1280, 720));
        assert!(!context.exit_requested);
    }

    #[test]
    fn validation_setting_defaults_off_in_unit_tests_and_is_overridable() {
        let context = RenderContext::new(320, 200);
        assert_eq!(context.validation(), default_validation());
        assert!(!context.validation());

        let enabled = RenderContext::new(320, 200).with_validation(true);
        assert!(enabled.validation());
    }

    #[test]
    fn headless_init_never_panics() {
        let mut context = RenderContext::new(640, 480);
        context.init();
        assert!(context.is_initialized());
        assert!(matches!(
            context.status(),
            RendererStatus::Vulkan {
                has_surface: false,
                ..
            } | RendererStatus::Failed(_)
        ));
        context.begin_frame([0.0, 0.0, 0.0, 1.0]);
        context.end_frame();
    }

    #[test]
    fn resize_updates_render_target() {
        let mut context = RenderContext::new(640, 480);
        context.resize(1024, 768);
        assert_eq!(context.size(), (1024, 768));
    }

    #[test]
    fn submit_collects_and_begin_frame_clears() {
        let mut context = RenderContext::new(320, 200);
        context.init();

        let mesh = crate::mesh::MeshHandle::from_parts(0, 0);
        context.begin_frame([0.0, 0.0, 0.0, 1.0]);
        assert_eq!(context.submitted_instances(), 0);

        context.submit(mesh, InstanceData::default());
        context.submit(mesh, InstanceData::default());
        assert_eq!(context.submitted_instances(), 2);

        context.begin_frame([0.0, 0.0, 0.0, 1.0]);
        assert_eq!(context.submitted_instances(), 0);
    }

    #[test]
    fn create_mesh_requires_initialized_renderer() {
        let mut context = RenderContext::new(320, 200);
        let error = context.create_mesh(&crate::cube()).unwrap_err();
        assert_eq!(error, RenderInitError::NotInitialized);
    }

    #[test]
    fn headless_mesh_upload_roundtrip_when_vulkan_available() {
        let mut context = RenderContext::new(320, 240);
        context.init();

        // 无 Vulkan 设备（CI/无 GPU）的环境直接跳过，保持测试可移植。
        if !matches!(context.status(), RendererStatus::Vulkan { .. }) {
            return;
        }

        let mesh = context.create_mesh(&crate::cube()).expect("上传立方体网格");
        context.destroy_mesh(mesh);

        // 旧句柄重复销毁不应 panic（世代号不匹配即忽略）。
        context.destroy_mesh(mesh);

        // 槽位复用后世代号应变化，新旧句柄不相等。
        let reused = context
            .create_mesh(&crate::cube())
            .expect("复用槽位上传网格");
        assert_ne!(mesh, reused);
        context.destroy_mesh(reused);

        // 无效数据应返回明确错误而不是上传。
        assert!(matches!(
            context.create_mesh(&MeshData::default()),
            Err(RenderInitError::Mesh(_))
        ));
    }

    #[test]
    fn minimized_window_skips_frame() {
        let mut context = RenderContext::new(640, 480);
        context.init();

        // 最小化：尺寸归零，跳帧，不应产生错误。
        context.resize(0, 0);
        assert_eq!(context.size(), (0, 0));
        context.begin_frame([0.0, 0.0, 0.0, 1.0]);
        context.end_frame();
        assert!(context.frame_error().is_none());

        // 恢复：重新按新尺寸重建并继续出帧。
        context.resize(800, 600);
        assert_eq!(context.size(), (800, 600));
        context.begin_frame([0.1, 0.1, 0.1, 1.0]);
        context.end_frame();
        assert!(context.frame_error().is_none());
    }
}
