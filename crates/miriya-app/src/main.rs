//! Miriya 示例应用：展示如何基于 engine crate 搭建游戏。

use miriya_engine::core::{AppState, EngineConfig, EngineContext};
use miriya_engine::ecs::World;
use miriya_engine::input::KeyCode;
use miriya_engine::math::Vec3;
use miriya_engine::scene::Transform;

/// 游戏状态：实现引擎的 `AppState` trait。
struct Game {
    world: World,
    player: miriya_engine::ecs::Entity,
}

impl Game {
    fn new() -> Self {
        let mut world = World::new();
        let player = world.spawn();
        world.insert(
            player,
            Transform {
                position: Vec3::new(0.0, 0.0, 0.0),
                ..Default::default()
            },
        );
        Self { world, player }
    }
}

impl AppState for Game {
    fn on_start(&mut self, ctx: &mut EngineContext) {
        println!(
            "游戏启动，实体数：{}，窗口：{}x{}",
            self.world.entities().len(),
            ctx.renderer.size().0,
            ctx.renderer.size().1
        );
    }

    fn on_update(&mut self, ctx: &mut EngineContext, dt: f32) {
        // WASD 移动玩家
        let mut delta = Vec3::ZERO;
        if ctx.input.is_down(KeyCode::W) {
            delta.y += 1.0;
        }
        if ctx.input.is_down(KeyCode::S) {
            delta.y -= 1.0;
        }
        if ctx.input.is_down(KeyCode::A) {
            delta.x -= 1.0;
        }
        if ctx.input.is_down(KeyCode::D) {
            delta.x += 1.0;
        }

        if delta.length_sq() > 0.0
            && let Some(t) = self.world.get_mut::<Transform>(self.player)
        {
            t.position += delta.normalized() * 5.0 * dt;
        }

        // Esc 或点击关闭按钮均通过 request_exit 退出
        if ctx.input.just_pressed(KeyCode::Escape) {
            ctx.request_exit();
        }
    }

    fn on_render(&mut self, ctx: &mut EngineContext) {
        ctx.renderer.begin_frame([0.1, 0.1, 0.15, 1.0]);
        // TODO: 收集 world 中的渲染组件并提交绘制（M2）
        ctx.renderer.end_frame();
    }

    fn on_exit(&mut self, ctx: &mut EngineContext) {
        println!(
            "游戏退出：共 {} 帧，平均 {:.1} FPS",
            ctx.stats.frames, ctx.stats.fps
        );
        if let Some(error) = ctx.renderer.frame_error() {
            eprintln!("渲染帧错误：{error}");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let headless = args.iter().any(|arg| arg == "--headless");
    let max_frames = parse_arg(&args, "--frames");

    let config = EngineConfig {
        title: "Miriya Demo".to_string(),
        max_frames,
        target_fps: if headless { 0 } else { 60 },
        ..Default::default()
    };
    let engine = miriya_engine::Engine::new(config);
    let mut game = Game::new();

    if headless {
        engine.run_headless(&mut game, max_frames.unwrap_or(1000));
    } else {
        engine.run(&mut game);
    }
}

/// 解析 `--frames N` / `--frames=N`。
fn parse_arg(args: &[String], name: &str) -> Option<u64> {
    let prefix = format!("{name}=");
    for (i, arg) in args.iter().enumerate() {
        if let Some(value) = arg.strip_prefix(&prefix) {
            return value.parse().ok();
        }
        if arg == name {
            return args.get(i + 1).and_then(|v| v.parse().ok());
        }
    }
    None
}
