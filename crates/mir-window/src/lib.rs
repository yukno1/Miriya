//! 窗口抽象：基于 winit 的窗口与事件循环。
//!
//! winit 类型被隔离在 crate 内部；引擎层只消费 [`event::WindowEvent`]，
//! 并通过 [`WindowApp`] / [`run`] 驱动帧循环。

mod event;

pub use event::{ButtonState, WindowEvent};

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent as WinitDeviceEvent, WindowEvent as WinitWindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use winit::window::{CursorGrabMode, Window as WinitWindow, WindowAttributes};

use event::{map_element_state, map_mouse_button, map_physical_key};

/// 窗口配置。
#[derive(Debug, Clone)]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "Miriya".to_string(),
            width: 1280,
            height: 720,
        }
    }
}

/// 窗口应用回调：由引擎层实现，接收事件并在每帧被调用。
pub trait WindowApp {
    /// 窗口创建完成后调用一次，供渲染层创建 surface/交换链。
    fn on_window_created(&mut self, _window: &Window) {}
    /// 处理一个窗口事件。
    fn on_event(&mut self, event: WindowEvent);
    /// 每帧更新与渲染（由 `about_to_wait` 驱动）；`window` 供光标捕获等平台操作使用。
    fn on_frame(&mut self, window: &Window);
    /// 返回 `true` 请求退出事件循环。
    fn should_exit(&self) -> bool;
}

/// 平台窗口的薄封装，持有 winit 窗口；为 M2 的 Vulkan surface 提供 raw handle。
pub struct Window {
    inner: WinitWindow,
}

impl Window {
    /// 当前窗口的物理像素尺寸。
    pub fn size(&self) -> (u32, u32) {
        let size = self.inner.inner_size();
        (size.width, size.height)
    }

    /// 标题。
    pub fn title(&self) -> String {
        self.inner.title()
    }

    /// 供渲染后端创建 surface 使用的 raw-window-handle 0.6（窗口句柄）。
    pub fn raw_handle(&self) -> winit::raw_window_handle::RawWindowHandle {
        let handle = self.window_handle().expect("窗口应提供 raw handle");
        handle.as_raw()
    }

    /// 供渲染后端创建 surface 使用的 raw-window-handle 0.6（显示句柄）。
    pub fn raw_display_handle(&self) -> winit::raw_window_handle::RawDisplayHandle {
        let handle = self
            .display_handle()
            .expect("窗口应提供 raw display handle");
        handle.as_raw()
    }

    /// 锁定/释放鼠标光标：锁定优先 `Locked`，失败回退 `Confined`；成功时隐藏光标。
    ///
    /// 返回是否按请求进入了锁定状态（释放时总是返回 `true`）。
    /// 失去焦点时应调用 `set_cursor_captured(false)`，避免光标卡死。
    pub fn set_cursor_captured(&self, captured: bool) -> bool {
        if captured {
            let grabbed = self
                .inner
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.inner.set_cursor_grab(CursorGrabMode::Confined));
            let ok = grabbed.is_ok();
            if ok {
                self.inner.set_cursor_visible(false);
            }
            ok
        } else {
            let _ = self.inner.set_cursor_grab(CursorGrabMode::None);
            self.inner.set_cursor_visible(true);
            true
        }
    }
}

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.inner.window_handle()
    }
}

impl HasDisplayHandle for Window {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.inner.display_handle()
    }
}

struct Runner<'a, A: WindowApp> {
    config: WindowConfig,
    app: &'a mut A,
    window: Option<Window>,
    exit: bool,
}

impl<A: WindowApp> ApplicationHandler for Runner<'_, A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title(self.config.title.clone())
            .with_inner_size(PhysicalSize::new(self.config.width, self.config.height));
        let inner = event_loop
            .create_window(attrs)
            .expect("创建窗口失败（当前仅支持 Windows）");
        self.window = Some(Window { inner });
        if let Some(window) = self.window.as_ref() {
            self.app.on_window_created(window);
        }
        if self.app.should_exit() {
            self.exit = true;
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        event: WinitWindowEvent,
    ) {
        let translated = match event {
            WinitWindowEvent::CloseRequested => Some(WindowEvent::CloseRequested),
            WinitWindowEvent::Resized(size) => Some(WindowEvent::Resized {
                width: size.width,
                height: size.height,
            }),
            WinitWindowEvent::KeyboardInput { event, .. } => Some(WindowEvent::Keyboard {
                key: map_physical_key(event.physical_key),
                state: map_element_state(event.state),
            }),
            WinitWindowEvent::CursorMoved { position, .. } => Some(WindowEvent::CursorMoved {
                x: position.x as f32,
                y: position.y as f32,
            }),
            WinitWindowEvent::MouseInput { state, button, .. } => {
                map_mouse_button(button).map(|button| WindowEvent::MouseButton {
                    button,
                    state: map_element_state(state),
                })
            }
            WinitWindowEvent::Focused(focused) => Some(WindowEvent::Focused(focused)),
            _ => None,
        };

        // 失焦时强制释放光标，避免用户切走窗口后鼠标被锁死；应用层收到 Focused(false) 后自行改状态。
        if let Some(WindowEvent::Focused(false)) = translated.as_ref()
            && let Some(window) = self.window.as_ref()
        {
            window.set_cursor_captured(false);
        }

        if let Some(event) = translated {
            self.app.on_event(event);
        }

        if self.app.should_exit() {
            self.exit = true;
            event_loop.exit();
        }
    }

    /// 转发原始鼠标位移：视角控制需要不受窗口边界限制的增量。
    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: WinitDeviceEvent,
    ) {
        if let WinitDeviceEvent::MouseMotion { delta } = event {
            self.app.on_event(WindowEvent::MouseMotion {
                dx: delta.0 as f32,
                dy: delta.1 as f32,
            });
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.exit {
            return;
        }
        if let Some(window) = self.window.as_ref() {
            self.app.on_frame(window);
        }
        if self.app.should_exit() {
            self.exit = true;
            event_loop.exit();
        } else {
            event_loop.set_control_flow(ControlFlow::Poll);
            if let Some(window) = &self.window {
                window.inner.request_redraw();
            }
        }
    }
}

/// 创建窗口并进入事件循环，直到 [`WindowApp::should_exit`] 返回 `true`。
pub fn run<A: WindowApp>(config: WindowConfig, app: &mut A) {
    let event_loop = EventLoop::new().expect("创建事件循环失败");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut runner = Runner {
        config,
        app,
        window: None,
        exit: false,
    };
    event_loop.run_app(&mut runner).expect("事件循环异常退出");
}
