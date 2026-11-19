//! 渲染子系统：Vulkan 1.3 唯一后端（`vulkanalia`）。
//!
//! 公共 API 不暴露任何 Vulkan 类型；`VulkanCore` 完全封装在 crate 内部
//! （ADR 0005 / README §5.1 规则 2）。当前已接入实例、物理设备、逻辑设备、
//! 队列选择、surface 与交换链；RenderGraph / 绘制 / present 仍待后续里程碑。
//!
//! 无窗口（headless）路径不会创建 surface，也不会要求交换链扩展，
//! 因此 `mir-core` 的 `run_headless` 与 CI 仍然可用。

mod vulkan;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use thiserror::Error;
use vulkan::VulkanCore;

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
    frame_error: Option<String>,
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
            frame_error: None,
        }
    }

    /// 以 headless 方式初始化：尝试创建 Vulkan 实例与逻辑设备，但不创建 surface。
    ///
    /// 失败时记录 [`RendererStatus::Failed`] 而不 panic，保证 CI/无 GPU 环境可运行。
    pub fn init(&mut self) {
        if self.initialized {
            return;
        }
        match VulkanCore::new_headless() {
            Ok(backend) => {
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
        match VulkanCore::with_surface(display, window, self.width, self.height) {
            Ok(backend) => {
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

    /// 记录本帧清屏色（实际清屏在 `end_frame` 提交）。
    pub fn begin_frame(&mut self, clear_color: [f32; 4]) {
        debug_assert!(self.initialized);
        self.clear_color = clear_color;
    }

    /// 完成本帧：清屏 → 提交 → 呈现。headless 或窗口最小化时为空操作。
    pub fn end_frame(&mut self) {
        debug_assert!(self.initialized);
        // 最小化时窗口尺寸为 0：跳过 acquire/present，避免查询/提交到无效交换链。
        if self.width == 0 || self.height == 0 {
            return;
        }
        if let Some(backend) = self.backend.as_mut() {
            if let Err(error) = backend.render_frame(self.clear_color, self.width, self.height) {
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
