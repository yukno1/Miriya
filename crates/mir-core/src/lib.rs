//! 引擎核心：主循环与生命周期。

pub mod time;

use std::time::Instant;
use time::Time;

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
}

impl EngineContext {
    /// 请求退出引擎主循环。
    pub fn request_exit(&mut self) {
        self.renderer.exit_requested = true;
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

    /// 运行引擎，直到 `AppState` 请求退出或窗口关闭。
    pub fn run(&self, app: &mut dyn AppState) {
        let mut ctx = EngineContext {
            time: Time::new(),
            input: mir_input::InputState::default(),
            renderer: mir_render::RenderContext::new(self.config.width, self.config.height),
            assets: mir_asset::AssetServer::new(),
        };

        ctx.renderer.init();
        app.on_start(&mut ctx);

        let frame_target = if self.config.target_fps > 0 {
            Some(1.0 / self.config.target_fps as f64)
        } else {
            None
        };

        let mut frames = 0u64;
        while !ctx.renderer.exit_requested {
            let frame_start = Instant::now();

            ctx.input.end_frame(); // 清除上一帧的按键沿
            ctx.time.update();

            let dt = ctx.time.delta_secs();
            app.on_update(&mut ctx, dt);
            app.on_render(&mut ctx);

            frames += 1;
            if self.config.max_frames.is_some_and(|max| frames >= max) {
                break;
            }

            // 帧率限制
            if let Some(target) = frame_target {
                let elapsed = frame_start.elapsed().as_secs_f64();
                if elapsed < target {
                    std::thread::sleep(std::time::Duration::from_secs_f64(target - elapsed));
                }
            }
        }

        app.on_exit(&mut ctx);
    }
}
