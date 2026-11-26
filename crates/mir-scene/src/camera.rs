//! 场景相机：UI/输入无关的相机数学与自由飞行控制。
//!
//! 世界约定采用 ADR-0003：厘米语义、X 前 / Y 右 / Z 上、左手系；
//! 投影使用 `glam::camera::lh::proj::vulkan`，只在本模块做坐标约定转换。

use mir_math::{Mat4, Quat, Vec2, Vec3};

const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_2 - 0.01;

/// 透视相机：位置 + yaw/pitch，无 roll。
///
/// `yaw` 绕世界 +Z，正值右转（+X → +Y）；`pitch` 正值抬头。
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// 垂直视场角（弧度）。
    pub fov_y: f32,
    pub z_near: f32,
    pub z_far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 60.0_f32.to_radians(),
            // ADR-0003 / M3：运行时单位为厘米，近远裁剪面同步校准为 1 cm / 1000 m。
            z_near: 1.0,
            z_far: 100_000.0,
        }
    }
}

impl Camera {
    /// 观察方向（已归一化，左手系 +X 为基准前向）。
    pub fn forward(&self) -> Vec3 {
        Quat::from_rotation_z(self.yaw) * Quat::from_rotation_y(-self.pitch) * Vec3::X
    }

    /// 右方向（世界 +Y 绕 +Z 旋转后的方向）。
    pub fn right(&self) -> Vec3 {
        Quat::from_rotation_z(self.yaw) * Vec3::Y
    }

    /// 改变朝向并使目标点落在画面中心。
    pub fn look_at(&mut self, target: Vec3) {
        let direction = (target - self.position).normalize();
        self.yaw = direction.y.atan2(direction.x);
        self.pitch = direction.z.asin().clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// 限制 pitch，避免翻转。
    pub fn clamp_pitch(&mut self) {
        self.pitch = self.pitch.clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// 左手系观察矩阵（ADR-0003）。
    pub fn view(&self) -> Mat4 {
        Mat4::look_to(self.position, self.forward(), Vec3::Z)
    }

    /// Vulkan 约定透视矩阵（深度 0..1、NDC Y 向下）。
    pub fn projection(&self, aspect: f32) -> Mat4 {
        Mat4::perspective(
            self.fov_y,
            aspect.max(f32::EPSILON),
            self.z_near,
            self.z_far,
        )
    }

    /// 提交给渲染层的 view-projection 矩阵。
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }
}

/// 一帧的相机输入：移动轴 + 鼠标增量 + 加速标志。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CameraInput {
    /// 前后（正值向前）。
    pub move_forward: f32,
    /// 左右（正值向右）。
    pub move_right: f32,
    /// 上下（正值向上）。
    pub move_up: f32,
    /// 本帧原始鼠标位移（像素，正值向右/向下）。
    pub look_delta: Vec2,
    pub boost: bool,
}

/// 帧率无关的自由飞行控制器；不依赖窗口或输入 crate，便于 headless 测试。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraController {
    /// 基础移动速度（厘米/秒）。
    pub speed: f32,
    /// 加速倍率。
    pub boost_multiplier: f32,
    /// 鼠标灵敏度（弧度/像素）。
    pub sensitivity: f32,
}

impl Default for CameraController {
    fn default() -> Self {
        Self {
            // 500 cm/s = 5 m/s，与 v0.2 演示的视觉移动速度一致。
            speed: 500.0,
            boost_multiplier: 3.0,
            sensitivity: 0.0022,
        }
    }
}

impl CameraController {
    /// 应用一帧输入：先转向、再按新朝向移动。
    pub fn update(&self, camera: &mut Camera, dt: f32, input: CameraInput) {
        let dt = dt.clamp(0.0, 0.1);
        camera.yaw += input.look_delta.x * self.sensitivity;
        camera.pitch -= input.look_delta.y * self.sensitivity;
        camera.clamp_pitch();

        let mut direction = camera.forward() * input.move_forward
            + camera.right() * input.move_right
            + Vec3::Z * input.move_up;
        if direction.length_sq() > 0.0 {
            direction = direction.normalized();
            let speed = self.speed
                * if input.boost {
                    self.boost_multiplier
                } else {
                    1.0
                };
            camera.position += direction * speed * dt;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(actual: f32, expected: f32) -> bool {
        (actual - expected).abs() < 1e-5
    }

    #[test]
    fn default_camera_looks_forward() {
        let camera = Camera::default();
        assert_eq!(camera.forward(), Vec3::X);
        assert_eq!(camera.right(), Vec3::Y);
    }

    #[test]
    fn yaw_and_pitch_follow_world_axes() {
        let camera = Camera {
            yaw: std::f32::consts::FRAC_PI_2,
            ..Default::default()
        };
        assert!(approx(camera.forward().y, 1.0));

        let camera = Camera {
            pitch: std::f32::consts::FRAC_PI_4,
            ..Default::default()
        };
        assert!(approx(camera.forward().z, std::f32::consts::FRAC_1_SQRT_2));
    }

    #[test]
    fn look_at_target_and_clamp_pitch() {
        let mut camera = Camera {
            position: Vec3::new(-1200.0, -1000.0, 800.0),
            ..Default::default()
        };
        camera.look_at(Vec3::ZERO);

        let direction = (Vec3::ZERO - camera.position).normalize();
        let actual = camera.forward();
        for (a, b) in actual.to_array().iter().zip(direction.to_array().iter()) {
            assert!(approx(*a, *b), "朝向与目标方向不一致: {actual:?}");
        }

        camera.pitch = 10.0;
        camera.clamp_pitch();
        assert!(camera.pitch < std::f32::consts::FRAC_PI_2);
        assert!(camera.forward().length_sq() > 0.0);
    }

    #[test]
    fn controller_moves_frame_rate_independently() {
        let controller = CameraController::default();
        let input = CameraInput {
            move_forward: 1.0,
            ..Default::default()
        };

        // 控制器按 README §5.2 把 dt 上限设为 100 ms。
        let mut once = Camera::default();
        controller.update(&mut once, 0.1, input);

        let mut twice = Camera::default();
        controller.update(&mut twice, 0.05, input);
        controller.update(&mut twice, 0.05, input);

        assert!(approx(once.position.x, 50.0));
        assert!(approx(twice.position.x, once.position.x));
    }

    #[test]
    fn controller_boost_and_look_delta() {
        let controller = CameraController::default();
        let mut camera = Camera::default();
        controller.update(
            &mut camera,
            0.1,
            CameraInput {
                move_forward: 1.0,
                boost: true,
                ..Default::default()
            },
        );
        assert!(approx(camera.position.length(), 150.0));

        controller.update(
            &mut camera,
            0.0,
            CameraInput {
                look_delta: Vec2::new(100.0, 1000.0),
                ..Default::default()
            },
        );
        assert!(camera.yaw > 0.0, "鼠标右移应右转: yaw={}", camera.yaw);
        assert!(camera.pitch < 0.0, "鼠标下移应低头: pitch={}", camera.pitch);
        assert!(camera.pitch >= -std::f32::consts::FRAC_PI_2);
    }
}
