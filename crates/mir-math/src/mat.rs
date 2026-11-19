//! 矩阵：对 `glam` 矩阵的 newtype 封装。

use crate::quat::Quat;
use crate::vec::Vec3;

use glam::{Mat3 as GMat3, Mat4 as GMat4};

macro_rules! impl_mat {
    ($name:ident, $glam:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Default)]
        #[repr(transparent)]
        pub struct $name(pub(crate) $glam);

        impl $name {
            /// 单位矩阵。
            pub const IDENTITY: Self = Self($glam::IDENTITY);
        }

        impl From<$glam> for $name {
            #[inline]
            fn from(m: $glam) -> Self {
                Self(m)
            }
        }

        impl From<$name> for $glam {
            #[inline]
            fn from(m: $name) -> Self {
                m.0
            }
        }

        impl std::ops::Deref for $name {
            type Target = $glam;
            #[inline]
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl std::ops::DerefMut for $name {
            #[inline]
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl std::ops::Mul<$name> for $name {
            type Output = Self;
            #[inline]
            fn mul(self, rhs: $name) -> Self {
                (self.0 * rhs.0).into()
            }
        }
    };
}

impl_mat!(Mat3, GMat3);
impl_mat!(Mat4, GMat4);

impl Mat4 {
    /// 平移矩阵。
    #[inline]
    pub fn from_translation(v: Vec3) -> Self {
        GMat4::from_translation(v.0).into()
    }

    /// 缩放矩阵。
    #[inline]
    pub fn from_scale(v: Vec3) -> Self {
        GMat4::from_scale(v.0).into()
    }

    /// 旋转矩阵。
    #[inline]
    pub fn from_quat(q: Quat) -> Self {
        GMat4::from_quat(q.0).into()
    }

    /// TRS 变换：先缩放，再旋转，后平移。
    #[inline]
    pub fn from_trs(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        GMat4::from_scale_rotation_translation(scale.0, rotation.0, translation.0).into()
    }

    /// 变换点（应用矩阵的线性部分；不做透视除法）。
    #[inline]
    pub fn transform_point(self, v: Vec3) -> Vec3 {
        self.0.transform_point3(v.0).into()
    }

    /// 变换方向/向量（忽略平移）。
    #[inline]
    pub fn transform_vector(self, v: Vec3) -> Vec3 {
        self.0.transform_vector3(v.0).into()
    }

    /// 透视投影矩阵（垂直 fov，弧度；**左手系世界 → Y-up 视空间 → Vulkan NDC**：
    /// 深度 0..1、NDC Y 向下，见 ADR-0003）。
    #[inline]
    pub fn perspective(fov_y: f32, aspect: f32, z_near: f32, z_far: f32) -> Self {
        glam::camera::lh::proj::vulkan::perspective(fov_y, aspect, z_near, z_far).into()
    }

    /// 左手系观察矩阵：由视点、注视点与世界上方向构造（要求 `up` 已归一化）。
    #[inline]
    pub fn look_at(eye: Vec3, center: Vec3, up: Vec3) -> Self {
        glam::camera::lh::view::look_at_mat4(eye.0, center.0, up.0).into()
    }

    /// 左手系观察矩阵：由视点、前方向与世界上方向构造（`dir` / `up` 需已归一化）。
    ///
    /// 视空间为 Y-up、+Z 向前，与 [`Mat4::perspective`] 配套。
    #[inline]
    pub fn look_to(eye: Vec3, dir: Vec3, up: Vec3) -> Self {
        glam::camera::lh::view::look_to_mat4(eye.0, dir.0, up.0).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trs_transform_point() {
        let m = Mat4::from_trs(
            Vec3::new(1.0, 2.0, 3.0),
            Quat::IDENTITY,
            Vec3::new(2.0, 2.0, 2.0),
        );
        let p = m.transform_point(Vec3::new(1.0, 0.0, 0.0));
        assert!((p.x - 3.0).abs() < 1e-5);
        assert!((p.y - 2.0).abs() < 1e-5);
        assert!((p.z - 3.0).abs() < 1e-5);
    }

    #[test]
    fn identity_mul() {
        assert_eq!(
            Mat4::IDENTITY * Mat4::from_translation(Vec3::new(4.0, 5.0, 6.0)),
            Mat4::from_translation(Vec3::new(4.0, 5.0, 6.0))
        );
    }

    #[test]
    fn quat_to_mat4() {
        let angle = std::f32::consts::FRAC_PI_2;
        let m = Mat4::from_quat(Quat::from_rotation_z(angle));
        let p = m.transform_vector(Vec3::new(1.0, 0.0, 0.0));
        assert!((p.y - 1.0).abs() < 1e-5);
    }

    /// 左手系（X 前 / Y 右 / Z 上，ADR-0003）：世界轴应映射到 Y-up 视空间
    /// 的 +Z 前、+X 右、+Y 上。
    #[test]
    fn look_to_is_left_handed() {
        let view = Mat4::look_to(Vec3::ZERO, Vec3::X, Vec3::Z);

        let ahead = view.transform_point(Vec3::X);
        assert!(ahead.x.abs() < 1e-5, "前方向不应有右分量: {ahead:?}");
        assert!(ahead.y.abs() < 1e-5, "前方向不应有上分量: {ahead:?}");
        assert!(
            (ahead.z - 1.0).abs() < 1e-5,
            "视空间 +Z 应为前方: {ahead:?}"
        );

        let right = view.transform_point(Vec3::Y);
        assert!(
            (right.x - 1.0).abs() < 1e-5,
            "世界 +Y 应落到视空间 +X: {right:?}"
        );

        let up = view.transform_point(Vec3::Z);
        assert!((up.y - 1.0).abs() < 1e-5, "世界 +Z 应落到视空间 +Y: {up:?}");
    }

    #[test]
    fn look_at_matches_look_to() {
        let at = Mat4::look_at(Vec3::ZERO, Vec3::X, Vec3::Z);
        let to = Mat4::look_to(Vec3::ZERO, Vec3::X, Vec3::Z);
        for (at_col, to_col) in at.to_cols_array().iter().zip(to.to_cols_array()) {
            assert!((*at_col - to_col).abs() < 1e-5);
        }
    }

    /// Vulkan 投影：左手系 Y-up 视空间 → NDC 深度 [0, 1]、Y 向下。
    #[test]
    fn perspective_maps_vulkan_ndc() {
        use glam::Vec4;

        let proj = Mat4::perspective(std::f32::consts::FRAC_PI_2, 1.0, 1.0, 100.0);
        let project = |x: f32, y: f32, z: f32| {
            let clip = proj.0 * Vec4::new(x, y, z, 1.0);
            clip.truncate() / clip.w
        };

        let near = project(0.0, 0.0, 1.0);
        assert!(near.z.abs() < 1e-4, "近平面应映射到 NDC z=0: {near:?}");
        let far = project(0.0, 0.0, 100.0);
        assert!(
            (far.z - 1.0).abs() < 1e-4,
            "远平面应映射到 NDC z=1: {far:?}"
        );

        // 视空间 +Y（世界上方）在 Vulkan NDC 中 Y 向下，落到屏幕上方即负 y。
        let above = project(0.0, 1.0, 10.0);
        assert!(above.y < 0.0, "世界上方应映射到 NDC y<0: {above:?}");
        assert!(above.x.abs() < 1e-4);
        assert!(above.z > 0.0 && above.z < 1.0);
    }
}
