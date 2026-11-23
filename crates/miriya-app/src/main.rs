//! Miriya 示例应用：展示如何基于 engine crate 搭建游戏。

use miriya_engine::core::{AppState, EngineConfig, EngineContext};
use miriya_engine::ecs::{Entity, World};
use miriya_engine::input::{KeyCode, MouseButton};
use miriya_engine::math::{Quat, Vec2, Vec3};
use miriya_engine::render::{CameraFrame, InstanceData, MeshHandle};
use miriya_engine::scene::{Camera, CameraController, CameraInput, Transform};

/// 游戏状态：实现引擎的 `AppState` trait。
struct Game {
    world: World,
    /// 演示立方体（3×3 阵列 + 中心自转），实体持有变换，tint 随实例提交。
    cubes: Vec<(Entity, [f32; 4])>,
    /// 阵列中心的实体，用于逐帧更新自转。
    center: Entity,
    /// 立方体网格句柄（渲染初始化后才能创建，故惰性创建）。
    cube_mesh: Option<MeshHandle>,
    mesh_failed: bool,
    /// 自由飞行相机与控制器（输入无关，便于 headless 验证）。
    camera: Camera,
    camera_controller: CameraController,
    looking: bool,
}

impl Game {
    fn new() -> Self {
        let mut world = World::new();
        let mut cubes = Vec::with_capacity(10);

        // 3×3 阵列：1 单位 = 1 米，边长 2、间距 3（ADR-0012 记录单位偏差）。
        for (row, x) in [-3.0_f32, 0.0, 3.0].into_iter().enumerate() {
            for (column, y) in [-3.0_f32, 0.0, 3.0].into_iter().enumerate() {
                let entity = world.spawn();
                world.insert(
                    entity,
                    Transform {
                        position: Vec3::new(x, y, 0.0),
                        scale: Vec3::splat(2.0),
                        ..Default::default()
                    },
                );
                let tint = [
                    0.55 + 0.15 * row as f32,
                    0.55 + 0.15 * column as f32,
                    0.9,
                    1.0,
                ];
                cubes.push((entity, tint));
            }
        }

        let center = world.spawn();
        world.insert(
            center,
            Transform {
                scale: Vec3::splat(2.0),
                ..Default::default()
            },
        );
        cubes.push((center, [1.0, 1.0, 1.0, 1.0]));

        let mut camera = Camera {
            position: Vec3::new(-12.0, -10.0, 8.0),
            ..Default::default()
        };
        camera.look_at(Vec3::ZERO);

        Self {
            world,
            cubes,
            center,
            cube_mesh: None,
            mesh_failed: false,
            camera,
            camera_controller: CameraController::default(),
            looking: false,
        }
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
        // 右键按住捕获光标并控制视角；失焦时平台层会清空鼠标状态，自动释放。
        if ctx.input.just_mouse_pressed(MouseButton::Right) {
            self.looking = true;
            ctx.set_cursor_captured(true);
        }
        if self.looking && !ctx.input.is_mouse_down(MouseButton::Right) {
            self.looking = false;
            ctx.set_cursor_captured(false);
        }

        let look_delta = if self.looking {
            ctx.input.mouse_delta()
        } else {
            Vec2::ZERO
        };
        let move_forward = axis(ctx, KeyCode::W, KeyCode::S);
        let move_right = axis(ctx, KeyCode::D, KeyCode::A);
        let move_up = axis(ctx, KeyCode::E, KeyCode::Q);
        let boost = ctx.input.is_down(KeyCode::ShiftLeft) || ctx.input.is_down(KeyCode::ShiftRight);
        self.camera_controller.update(
            &mut self.camera,
            dt,
            CameraInput {
                move_forward,
                move_right,
                move_up,
                look_delta,
                boost,
            },
        );

        // 中心立方体自转；模型矩阵在 on_render 的 extract 阶段读取。
        let elapsed = ctx.time.elapsed_secs();
        if let Some(transform) = self.world.get_mut::<Transform>(self.center) {
            transform.rotation =
                Quat::from_rotation_z(elapsed * 0.6) * Quat::from_rotation_y(elapsed * 0.4);
        }

        // Esc 或点击关闭按钮均通过 request_exit 退出
        if ctx.input.just_pressed(KeyCode::Escape) {
            ctx.request_exit();
        }
    }

    fn on_render(&mut self, ctx: &mut EngineContext) {
        if self.cube_mesh.is_none() && !self.mesh_failed {
            match ctx.renderer.create_mesh(&miriya_engine::render::cube()) {
                Ok(handle) => self.cube_mesh = Some(handle),
                Err(error) => {
                    eprintln!("立方体网格上传失败：{error}");
                    self.mesh_failed = true;
                }
            }
        }

        ctx.renderer.begin_frame([0.1, 0.1, 0.15, 1.0]);

        let (width, height) = ctx.renderer.size();
        let aspect = width.max(1) as f32 / height.max(1) as f32;
        ctx.renderer.set_camera(CameraFrame {
            view_proj: self.camera.view_projection(aspect),
        });

        if let Some(mesh) = self.cube_mesh {
            for (entity, tint) in self.cubes.iter().copied() {
                let Some(transform) = self.world.get::<Transform>(entity) else {
                    continue;
                };
                ctx.renderer.submit(
                    mesh,
                    InstanceData {
                        model: transform.to_matrix(),
                        tint,
                    },
                );
            }
        }

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
    let width = parse_arg(&args, "--width")
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(1280);
    let height = parse_arg(&args, "--height")
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(720);
    let validation = if args.iter().any(|arg| arg == "--no-validation") {
        false
    } else if args.iter().any(|arg| arg == "--validation") {
        true
    } else {
        EngineConfig::default().validation
    };

    let config = EngineConfig {
        title: "Miriya Demo".to_string(),
        width,
        height,
        max_frames,
        target_fps: if headless { 0 } else { 60 },
        validation,
    };
    let engine = miriya_engine::Engine::new(config);
    let mut game = Game::new();

    if headless {
        engine.run_headless(&mut game, max_frames.unwrap_or(1000));
    } else {
        engine.run(&mut game);
    }
}

/// 正负轴输入：正键返回 +1，负键返回 -1，同时按下则抵消。
fn axis(ctx: &EngineContext, positive: KeyCode, negative: KeyCode) -> f32 {
    let positive = i8::from(ctx.input.is_down(positive));
    let negative = i8::from(ctx.input.is_down(negative));
    f32::from(positive - negative)
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
