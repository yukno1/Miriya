//! 场景子系统：游戏对象的组织与管理。

use mir_ecs::World;
use mir_math::{Mat4, Quat, Vec3};

/// 场景：一个 World 加上场景级元数据。
pub struct Scene {
    pub name: String,
    pub world: World,
    ambient: Vec3,
}

impl Scene {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            world: World::new(),
            ambient: Vec3::ONE,
        }
    }

    /// 环境光颜色。
    pub fn ambient(&self) -> Vec3 {
        self.ambient
    }

    pub fn set_ambient(&mut self, color: Vec3) {
        self.ambient = color;
    }
}

/// 内置的变换组件，几乎所有对象都需要（ADR-0003：左手系、X 前 / Y 右 / Z 上）。
#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Transform {
    /// 组合为 4x4 变换矩阵：先缩放、再旋转、后平移。
    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_trs(self.position, self.rotation, self.scale)
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl mir_ecs::Component for Transform {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_to_matrix_applies_trs_order() {
        // 缩放 2 → 旋转 90°（+X 转向 +Y）→ 平移 (1, 2, 3)。
        let transform = Transform {
            position: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale: Vec3::new(2.0, 2.0, 2.0),
        };
        let p = transform
            .to_matrix()
            .transform_point(Vec3::new(1.0, 0.0, 0.0));

        assert!((p.x - 1.0).abs() < 1e-5, "缩放后旋转再平移: {p:?}");
        assert!((p.y - 4.0).abs() < 1e-5, "缩放后旋转再平移: {p:?}");
        assert!((p.z - 3.0).abs() < 1e-5, "缩放后旋转再平移: {p:?}");
    }

    #[test]
    fn default_transform_is_identity() {
        let transform = Transform::default();
        assert_eq!(transform.position, Vec3::ZERO);
        assert_eq!(transform.rotation, Quat::IDENTITY);
        assert_eq!(transform.scale, Vec3::ONE);
        for (expected, actual) in Mat4::IDENTITY
            .to_cols_array()
            .iter()
            .zip(transform.to_matrix().to_cols_array())
        {
            assert!((*expected - actual).abs() < 1e-5);
        }
    }
}
