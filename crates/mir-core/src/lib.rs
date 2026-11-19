//! 引擎核心：主循环与生命周期。

pub mod time;

use std::time::Instant;

use mir_window::{ButtonState, WindowApp, WindowEvent};
use time::{FrameStats, Time};

/// 引擎配置。
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// 窗口标题
    pub title: String,
    /// 窗口尺寸
    pub width: u32,
    pub height: u32,
    /// 目标帧率（0 表示不限制）
    pub target_fps: u32,
    /// 最大运行帧数（`None` 表示不限；用于 headless 测试与演示自动退出）
    pub max_frames: Option<u64>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            title: "Miriya".to_string(),
            width: 1280,
            height: 720,
            target_fps: 60,
            max_frames: None,
        }
    }
}

/// 应用状态 trait，由 app 层实现，引擎在主循环中回调。
pub trait AppState {
    /// 引擎启动后调用一次。
    fn on_start(&mut self, _ctx: &mut EngineContext) {}
    /// 每帧逻辑更新。
    fn on_update(&mut self, _ctx: &mut EngineContext, _dt: f32) {}
    /// 每帧渲染（逻辑更新之后）。
    fn on_render(&mut self, _ctx: &mut EngineContext) {}
    /// 引擎退出前调用一次。
    fn on_exit(&mut self, _ctx: &mut EngineContext) {}
}

/// 传给应用层的引擎上下文，聚合各子系统。
pub struct EngineContext {
    pub time: Time,
    pub input: mir_input::InputState,
    pub renderer: mir_render::RenderContext,
    pub assets: mir_asset::AssetServer,
    pub stats: FrameStats,
    /// 待应用到窗口的光标捕获请求；由主循环消费（headless 下不会被消费）。
    cursor_capture_request: Option<bool>,
}

impl EngineContext {
    /// 请求退出引擎主循环。
    pub fn request_exit(&mut self) {
        self.renderer.exit_requested = true;
    }

    /// 请求锁定/释放鼠标光标；主循环在下一帧应用到窗口。
    ///
    /// 无窗口（headless）运行时该请求不会被消费，也不产生副作用。
    pub fn set_cursor_captured(&mut self, captured: bool) {
        self.cursor_capture_request = Some(captured);
    }

    /// 平台层：把窗口事件翻译成输入状态与退出请求。
    pub fn on_window_event(&mut self, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.request_exit(),
            WindowEvent::Resized { width, height } => self.renderer.resize(width, height),
            WindowEvent::Keyboard { key, state } => match state {
                ButtonState::Pressed => self.input.press(key),
                ButtonState::Released => self.input.release(key),
            },
            WindowEvent::CursorMoved { x, y } => {
                self.input.set_mouse_pos(mir_math::Vec2::new(x, y));
            }
            WindowEvent::MouseMotion { dx, dy } => self.input.set_mouse_delta(dx, dy),
            WindowEvent::MouseButton { button, state } => match state {
                ButtonState::Pressed => self.input.press_mouse(button),
                ButtonState::Released => self.input.release_mouse(button),
            },
            WindowEvent::Focused(false) => self.input.clear(),
            WindowEvent::Focused(true) => {}
        }
    }
}

fn frame_target(config: &EngineConfig) -> Option<f64> {
    if config.target_fps > 0 {
        Some(1.0 / config.target_fps as f64)
    } else {
        None
    }
}

fn make_context(config: &EngineConfig) -> EngineContext {
    EngineContext {
        time: Time::new(),
        input: mir_input::InputState::default(),
        renderer: mir_render::RenderContext::new(config.width, config.height),
        assets: mir_asset::AssetServer::new(),
        stats: FrameStats::default(),
        cursor_capture_request: None,
    }
}

/// 驱动一帧：时间推进 → 逻辑 → 渲染 → 帧统计 → 清理输入沿 → 帧率限制。
fn tick(
    ctx: &mut EngineContext,
    app: &mut dyn AppState,
    frame_start: Instant,
    target: Option<f64>,
) {
    ctx.time.update();
    let dt = ctx.time.delta_secs();
    app.on_update(ctx, dt);
    app.on_render(ctx);
    ctx.stats.update(dt);
    ctx.input.end_frame();

    if let Some(target) = target {
        let elapsed = frame_start.elapsed().as_secs_f64();
        if elapsed < target {
            std::thread::sleep(std::time::Duration::from_secs_f64(target - elapsed));
        }
    }
}

/// 把引擎主循环适配为窗口事件循环的回调。
struct FrameDriver<'a> {
    config: EngineConfig,
    ctx: EngineContext,
    app: &'a mut dyn AppState,
}

impl WindowApp for FrameDriver<'_> {
    fn on_window_created(&mut self, window: &mir_window::Window) {
        if let Err(error) = self.ctx.renderer.init_with_window(window, window) {
            eprintln!("渲染初始化失败：{error}");
            self.ctx.request_exit();
        }
    }

    fn on_event(&mut self, event: WindowEvent) {
        self.ctx.on_window_event(event);
    }

    fn on_frame(&mut self, window: &mir_window::Window) {
        let target = frame_target(&self.config);
        tick(&mut self.ctx, self.app, Instant::now(), target);

        // 应用本帧新产生的光标捕获请求（在 update 之后，使状态当帧生效）。
        if let Some(captured) = self.ctx.cursor_capture_request.take()
            && !window.set_cursor_captured(captured)
        {
            eprintln!("光标捕获失败：请求 {captured}");
        }

        if self
            .config
            .max_frames
            .is_some_and(|max| self.ctx.stats.frames >= max)
        {
            self.ctx.request_exit();
        }
    }

    fn should_exit(&self) -> bool {
        self.ctx.renderer.exit_requested
    }
}

/// 引擎本体，负责驱动主循环。
pub struct Engine {
    config: EngineConfig,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self { config }
    }

    /// 打开窗口并运行主循环，直到应用请求退出或窗口关闭。
    pub fn run(&self, app: &mut dyn AppState) {
        let mut driver = FrameDriver {
            config: self.config.clone(),
            ctx: make_context(&self.config),
            app,
        };
        driver.app.on_start(&mut driver.ctx);

        let window_config = mir_window::WindowConfig {
            title: self.config.title.clone(),
            width: self.config.width,
            height: self.config.height,
        };
        mir_window::run(window_config, &mut driver);

        driver.app.on_exit(&mut driver.ctx);
    }

    /// 无窗口运行指定帧数（headless 测试用）；不创建事件循环。
    pub fn run_headless(&self, app: &mut dyn AppState, frames: u64) {
        let mut ctx = make_context(&self.config);
        ctx.renderer.init();
        app.on_start(&mut ctx);

        let target = frame_target(&self.config);
        for _ in 0..frames {
            tick(&mut ctx, app, Instant::now(), target);
            if ctx.renderer.exit_requested {
                break;
            }
        }

        app.on_exit(&mut ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CountingApp {
        updates: u64,
        renders: u64,
        started: bool,
        exited: bool,
    }

    impl AppState for CountingApp {
        fn on_start(&mut self, _ctx: &mut EngineContext) {
            self.started = true;
        }
        fn on_update(&mut self, _ctx: &mut EngineContext, _dt: f32) {
            self.updates += 1;
        }
        fn on_render(&mut self, _ctx: &mut EngineContext) {
            self.renders += 1;
        }
        fn on_exit(&mut self, _ctx: &mut EngineContext) {
            self.exited = true;
        }
    }

    #[test]
    fn headless_1000_frames() {
        let engine = Engine::new(EngineConfig {
            title: "headless".to_string(),
            target_fps: 0,
            ..Default::default()
        });
        let mut app = CountingApp {
            updates: 0,
            renders: 0,
            started: false,
            exited: false,
        };
        engine.run_headless(&mut app, 1000);
        assert!(app.started);
        assert!(app.exited);
        assert_eq!(app.updates, 1000);
        assert_eq!(app.renders, 1000);
    }

    #[test]
    fn window_events_update_input_and_exit() {
        let mut ctx = make_context(&EngineConfig::default());
        ctx.on_window_event(WindowEvent::Keyboard {
            key: mir_input::KeyCode::W,
            state: ButtonState::Pressed,
        });
        assert!(ctx.input.is_down(mir_input::KeyCode::W));
        assert!(ctx.input.just_pressed(mir_input::KeyCode::W));

        ctx.on_window_event(WindowEvent::MouseMotion { dx: 4.0, dy: -3.0 });
        assert_eq!(ctx.input.mouse_delta(), mir_math::Vec2::new(4.0, -3.0));

        ctx.on_window_event(WindowEvent::Resized {
            width: 800,
            height: 600,
        });
        assert_eq!(ctx.renderer.size(), (800, 600));

        ctx.on_window_event(WindowEvent::CloseRequested);
        assert!(ctx.renderer.exit_requested);
    }

    #[test]
    fn cursor_capture_request_is_recorded_once() {
        let mut ctx = make_context(&EngineConfig::default());
        assert_eq!(ctx.cursor_capture_request, None);

        ctx.set_cursor_captured(true);
        assert_eq!(ctx.cursor_capture_request, Some(true));

        ctx.set_cursor_captured(false);
        assert_eq!(ctx.cursor_capture_request, Some(false));

        // 主循环消费后回到 None（headless 下不会被消费，仍保留请求）。
        assert_eq!(ctx.cursor_capture_request.take(), Some(false));
        assert_eq!(ctx.cursor_capture_request, None);
    }
}
