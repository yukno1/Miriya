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
                scale: Vec3::ONE,
            },
        );
        Self { world, player }
    }
}

impl AppState for Game {
    fn on_start(&mut self, _ctx: &mut EngineContext) {
        println!("游戏启动，当前实体数：{}", self.world.entities().len());
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

        if delta.length_sq() > 0.0 {
            if let Some(t) = self.world.get_mut::<Transform>(self.player) {
                t.position += delta.normalized() * 5.0 * dt;
            }
        }

        if ctx.input.just_pressed(KeyCode::Escape) {
            ctx.request_exit();
        }
    }

    fn on_render(&mut self, ctx: &mut EngineContext) {
        ctx.renderer.begin_frame([0.1, 0.1, 0.15, 1.0]);
        // TODO: 收集 world 中的渲染组件并提交绘制
        ctx.renderer.end_frame();
    }

    fn on_exit(&mut self, _ctx: &mut EngineContext) {
        println!("游戏退出");
    }
}

/// 解析 `--frames N` / `--frames=N`，用于限定演示运行帧数。
fn parse_max_frames() -> Option<u64> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    for (i, arg) in args.iter().enumerate() {
        if let Some(value) = arg.strip_prefix("--frames=") {
            return value.parse().ok();
        }
        if arg == "--frames" {
            return args.get(i + 1).and_then(|v| v.parse().ok());
        }
    }
    None
}

fn main() {
    let engine = miriya_engine::Engine::new(EngineConfig {
        title: "Miriya Demo".to_string(),
        max_frames: parse_max_frames(),
        ..Default::default()
    });
    engine.run(&mut Game::new());
}
