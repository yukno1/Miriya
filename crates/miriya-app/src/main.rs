//! Miriya 示例应用：展示如何基于 engine crate 搭建游戏。

use miriya_engine::core::{AppState, EngineConfig, EngineContext};
use miriya_engine::input::{KeyCode, MouseButton};
use miriya_engine::math::interop::{
    meters_to_centimeters as m_to_cm, meters_to_centimeters_vec3 as mvec_to_cm,
};
use miriya_engine::math::{Aabb, Frustum, Quat, Transform, Vec2, Vec3};
use miriya_engine::render::{CameraFrame, InstanceData, MaterialData, MaterialHandle, MeshHandle};
use miriya_engine::scene::{
    ActorId, Camera, CameraController, CameraInput, Level, WorldTransforms,
};

/// 演示物体使用的程序化几何（R-12）。
#[derive(Debug, Clone, Copy)]
enum Prop {
    Cube,
    Sphere,
    Torus,
}

/// 游戏状态：实现引擎的 `AppState` trait。
struct Game {
    level: Level,
    /// 演示物体：Actor + 几何类型 + 材质下标（`self.materials` 的下标）。
    props: Vec<(ActorId, Prop, usize)>,
    /// 阵列中心的 Actor（自转圆环），用于逐帧更新。
    center: ActorId,
    /// 网格句柄（渲染初始化后才能创建，故惰性创建）。
    meshes: Option<(MeshHandle, MeshHandle, MeshHandle)>,
    mesh_failed: bool,
    /// 材质句柄（R-12：metallic/roughness 梯度 + 金圆环 + 自发光球），惰性创建。
    materials: Vec<MaterialHandle>,
    materials_failed: bool,
    /// 自由飞行相机与控制器（输入无关，便于 headless 验证）。
    camera: Camera,
    camera_controller: CameraController,
    looking: bool,
    /// `--stats`：收集 GPU pass 耗时并定期输出。
    stats_enabled: bool,
    /// `--graph-dump`：每帧 dump RenderGraph 依赖图。
    graph_dump: bool,
    /// `--stats` 时累计各 pass 的 GPU 耗时（毫秒），退出时输出平均值。
    pass_time_total: Vec<(String, f64)>,
    /// 参与 GPU 耗时统计的帧数。
    pass_time_frames: u32,
    /// 上一帧剔除后实际提交的实例数（R-13 剔除生效的可观测诊断）。
    last_submitted: usize,
}

impl Game {
    fn new(stats_enabled: bool, graph_dump: bool) -> Self {
        let mut level = Level::new("demo");
        let mut props: Vec<(ActorId, Prop, usize)> = Vec::new();
        // 根 Actor 保持恒等变换：立方体挂到其下验证层级链路，但不改变 v0.2 画面。
        let root = level.spawn_actor("root").expect("创建根 Actor 失败");

        // 3×3 阵列（去掉中心位置留给自转圆环）：列向金属度梯度、行向粗糙度梯度。
        // M3 起所有世界位置/尺寸以厘米表示；下列字面量仍按米设计，统一经 interop 转换。
        for (row, x) in [-3.0_f32, 0.0, 3.0].into_iter().enumerate() {
            for (column, y) in [-3.0_f32, 0.0, 3.0].into_iter().enumerate() {
                if row == 1 && column == 1 {
                    continue; // 中心留给圆环
                }
                let actor = level
                    .spawn_actor_with_transform(
                        format!("cube-{row}-{column}"),
                        Transform {
                            position: mvec_to_cm(Vec3::new(x, y, 0.0)),
                            // 单位网格以厘米解释，缩放倍率按目标尺寸一次性校准。
                            scale: Vec3::splat(m_to_cm(2.0)),
                            ..Default::default()
                        },
                    )
                    .expect("创建立方体 Actor 失败");
                level
                    .set_parent(actor, root)
                    .expect("设置立方体父子关系失败");
                props.push((actor, Prop::Cube, props.len()));
            }
        }

        // 阵列中心：自转金属圆环。
        let center = level
            .spawn_actor_with_transform(
                "center-torus",
                Transform {
                    scale: Vec3::splat(m_to_cm(1.0)),
                    ..Default::default()
                },
            )
            .expect("创建圆环 Actor 失败");
        props.push((center, Prop::Torus, props.len()));

        // 两枚球：粗糙红塑料（左后）与自发光（右前），验证 emissive 与漫反射对照。
        let plastic_sphere = level
            .spawn_actor_with_transform(
                "plastic-sphere",
                Transform {
                    position: mvec_to_cm(Vec3::new(-8.0, 0.0, 0.0)),
                    scale: Vec3::splat(m_to_cm(1.5)),
                    ..Default::default()
                },
            )
            .expect("创建塑料球 Actor 失败");
        props.push((plastic_sphere, Prop::Sphere, props.len()));

        let emissive_sphere = level
            .spawn_actor_with_transform(
                "emissive-sphere",
                Transform {
                    position: mvec_to_cm(Vec3::new(0.0, 8.0, 0.0)),
                    scale: Vec3::splat(m_to_cm(1.2)),
                    ..Default::default()
                },
            )
            .expect("创建自发光球 Actor 失败");
        props.push((emissive_sphere, Prop::Sphere, props.len()));

        let mut camera = Camera {
            position: mvec_to_cm(Vec3::new(-12.0, -10.0, 8.0)),
            ..Default::default()
        };
        camera.look_at(Vec3::ZERO);

        Self {
            level,
            props,
            center,
            meshes: None,
            mesh_failed: false,
            materials: Vec::new(),
            materials_failed: false,
            camera,
            camera_controller: CameraController::default(),
            looking: false,
            stats_enabled,
            graph_dump,
            pass_time_total: Vec::new(),
            pass_time_frames: 0,
            last_submitted: 0,
        }
    }

    /// 惰性创建全部材质（顺序与 `props` 的材质下标对应）。
    fn ensure_materials(&mut self, ctx: &mut EngineContext) {
        if !self.materials.is_empty() || self.materials_failed {
            return;
        }
        let mut list = Vec::with_capacity(11);
        // 阵列立方体：metallic 列向 0→1、roughness 行向 0.15→1（8 个，中心留给圆环）。
        for (row, x) in [-3.0_f32, 0.0, 3.0].into_iter().enumerate() {
            for (column, y) in [-3.0_f32, 0.0, 3.0].into_iter().enumerate() {
                if row == 1 && column == 1 {
                    continue; // 中心留给圆环，跳过（与 props 生成顺序一致）
                }
                let _ = (x, y);
                list.push(MaterialData::new(
                    [0.85, 0.85, 0.9, 1.0],
                    column as f32 * 0.5,
                    0.15 + 0.425 * row as f32,
                    [0.0; 3],
                ));
            }
        }
        // 自转圆环：抛光金属。
        list.push(MaterialData::new(
            [1.0, 0.77, 0.34, 1.0],
            1.0,
            0.25,
            [0.0; 3],
        ));
        // 粗糙塑料球。
        list.push(MaterialData::new([0.8, 0.2, 0.2, 1.0], 0.0, 0.9, [0.0; 3]));
        // 自发光球。
        list.push(MaterialData::new(
            [0.05, 0.05, 0.05, 1.0],
            0.0,
            0.5,
            [0.1, 0.45, 0.4],
        ));

        for data in &list {
            match ctx.renderer.create_material(data) {
                Ok(handle) => self.materials.push(handle),
                Err(error) => {
                    eprintln!("材质创建失败：{error}");
                    self.materials_failed = true;
                    self.materials.clear();
                    return;
                }
            }
        }
    }
}
impl AppState for Game {
    fn on_start(&mut self, ctx: &mut EngineContext) {
        println!(
            "游戏启动，Actor 数：{}，窗口：{}x{}",
            self.level.actor_count(),
            ctx.renderer.size().0,
            ctx.renderer.size().1
        );
        // RenderGraph 依赖图 dump 初始化后仍可开关（每帧覆盖写 render_graph.dot）。
        if self.graph_dump {
            ctx.renderer.set_graph_dump(true);
        }
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

        // 中心圆环自转；模型矩阵在 on_render 的 extract 阶段读取。
        let elapsed = ctx.time.elapsed_secs();
        self.level
            .update_transform(self.center, |transform| {
                transform.rotation =
                    Quat::from_rotation_z(elapsed * 0.6) * Quat::from_rotation_y(elapsed * 0.4);
            })
            .expect("更新圆环 Actor 变换失败");

        // Esc 或点击关闭按钮均通过 request_exit 退出
        if ctx.input.just_pressed(KeyCode::Escape) {
            ctx.request_exit();
        }
    }

    fn on_render(&mut self, ctx: &mut EngineContext) {
        if self.meshes.is_none() && !self.mesh_failed {
            let cube = ctx.renderer.create_mesh(&miriya_engine::render::cube());
            let sphere = ctx
                .renderer
                .create_mesh(&miriya_engine::render::sphere(1.0, 32, 16));
            let torus = ctx
                .renderer
                .create_mesh(&miriya_engine::render::torus(1.6, 0.55, 48, 24));
            match (cube, sphere, torus) {
                (Ok(cube), Ok(sphere), Ok(torus)) => {
                    self.meshes = Some((cube, sphere, torus));
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    eprintln!("网格上传失败：{error}");
                    self.mesh_failed = true;
                }
            }
        }
        self.ensure_materials(ctx);

        // extract 边界：先烘焙层级世界矩阵，渲染层只接收纯数据。
        let world_transforms = match self.level.world_transforms() {
            Ok(transforms) => transforms,
            Err(error) => {
                eprintln!("层级世界变换失败：{error}");
                self.last_submitted = 0;
                WorldTransforms::default()
            }
        };

        ctx.renderer.begin_frame([0.02, 0.02, 0.04, 1.0]);

        let (width, height) = ctx.renderer.size();
        let aspect = width.max(1) as f32 / height.max(1) as f32;
        ctx.renderer.set_camera(CameraFrame {
            view_proj: self.camera.view_projection(aspect),
            camera_position: self.camera.position.to_array(),
        });

        if let Some((cube_mesh, sphere_mesh, torus_mesh)) = self.meshes {
            // 视锥剔除（R-13，ADR-0004 决策 6）：extract 侧用世界矩阵变换局部 AABB 后测试，
            // 渲染层不感知层级与剔除；自转圆环因此得到正确的世界空间包围盒。
            let frustum = Frustum::from_view_proj(self.camera.view_projection(aspect));
            let mut submitted = 0_usize;
            for (actor, prop, material_index) in self.props.iter().copied() {
                let Some(world_matrix) = world_transforms.get(actor) else {
                    continue;
                };
                let unit_half = match prop {
                    Prop::Cube => Vec3::splat(0.5),
                    Prop::Sphere => Vec3::splat(1.0),
                    // 圆环主半径 1.6 + 管半径 0.55（XY），Z 向厚 0.55。
                    Prop::Torus => Vec3::new(2.15, 2.15, 0.55),
                };
                let local_aabb = Aabb::from_center_half_extent(Vec3::ZERO, unit_half);
                let aabb = local_aabb.transformed(world_matrix);
                if !frustum.intersects_aabb(&aabb) {
                    continue;
                }
                let mesh = match prop {
                    Prop::Cube => cube_mesh,
                    Prop::Sphere => sphere_mesh,
                    Prop::Torus => torus_mesh,
                };
                let instance = InstanceData {
                    model: world_matrix,
                    tint: [1.0, 1.0, 1.0, 1.0],
                };
                if let Some(&material) = self.materials.get(material_index) {
                    ctx.renderer.submit_with_material(mesh, material, instance);
                } else {
                    ctx.renderer.submit(mesh, instance);
                }
                submitted += 1;
            }
            self.last_submitted = submitted;
        }

        ctx.renderer.end_frame();

        // --stats：回读 GPU pass 耗时进 FrameStats，累计并每 300 帧输出快照。
        if self.stats_enabled {
            let timings: Vec<(String, f32)> = ctx.renderer.pass_timings().to_vec();
            if !timings.is_empty() {
                self.pass_time_frames += 1;
                for (name, ms) in &timings {
                    match self
                        .pass_time_total
                        .iter_mut()
                        .find(|(total_name, _)| total_name == name)
                    {
                        Some((_, total)) => *total += f64::from(*ms),
                        None => self.pass_time_total.push((name.clone(), f64::from(*ms))),
                    }
                }
            }
            ctx.stats.gpu_pass_timings = timings;
            if ctx.stats.frames > 0 && ctx.stats.frames % 300 == 0 {
                let snapshot = if ctx.stats.gpu_pass_timings.is_empty() {
                    "GPU pass：无数据".to_string()
                } else {
                    let parts: Vec<String> = ctx
                        .stats
                        .gpu_pass_timings
                        .iter()
                        .map(|(name, ms)| format!("{name}: {ms:.3} ms"))
                        .collect();
                    format!("GPU pass：{}", parts.join("，"))
                };
                println!(
                    "[stats] 帧 {}，FPS {:.1}，提交实例 {}，{}",
                    ctx.stats.frames, ctx.stats.fps, self.last_submitted, snapshot
                );
            }
        }
    }

    fn on_exit(&mut self, ctx: &mut EngineContext) {
        println!(
            "游戏退出：共 {} 帧，平均 {:.1} FPS",
            ctx.stats.frames, ctx.stats.fps
        );
        if self.stats_enabled && self.pass_time_frames > 0 {
            println!("GPU pass 平均耗时（{} 帧统计）：", self.pass_time_frames);
            for (name, total) in &self.pass_time_total {
                println!(
                    "  {name}: {:.3} ms",
                    total / f64::from(self.pass_time_frames)
                );
            }
        }
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
    let stats_enabled = args.iter().any(|arg| arg == "--stats");
    let graph_dump = args.iter().any(|arg| arg == "--graph-dump");

    let config = EngineConfig {
        title: "Miriya Demo".to_string(),
        width,
        height,
        max_frames,
        target_fps: if headless { 0 } else { 60 },
        validation,
    };
    let engine = miriya_engine::Engine::new(config);
    let mut game = Game::new(stats_enabled, graph_dump);

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
