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

    /// 变换点（含平移与投影除法）。
    #[inline]
    pub fn transform_point(self, v: Vec3) -> Vec3 {
        self.0.transform_point3(v.0).into()
    }

    /// 变换方向/向量（忽略平移）。
    #[inline]
    pub fn transform_vector(self, v: Vec3) -> Vec3 {
        self.0.transform_vector3(v.0).into()
    }

    /// 透视投影矩阵（垂直 fov，弧度；右手系，深度范围 0..1，适合 Vulkan/wgpu）。
    #[inline]
    pub fn perspective(fov_y: f32, aspect: f32, z_near: f32, z_far: f32) -> Self {
        glam::camera::rh::proj::vulkan::perspective(fov_y, aspect, z_near, z_far).into()
    }

    /// 右手观察矩阵。
    #[inline]
    pub fn look_at(eye: Vec3, center: Vec3, up: Vec3) -> Self {
        glam::camera::rh::view::look_at_mat4(eye.0, center.0, up.0).into()
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
}
