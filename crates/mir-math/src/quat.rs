//! 四元数：对 `glam::Quat` 的 newtype 封装。

use crate::vec::Vec3;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(transparent)]
pub struct Quat(pub(crate) glam::Quat);

impl Quat {
    /// 单位四元数（不旋转）。
    pub const IDENTITY: Self = Self(glam::Quat::IDENTITY);

    #[inline]
    pub const fn from_xyzw(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self(glam::Quat::from_xyzw(x, y, z, w))
    }

    #[inline]
    pub fn from_axis_angle(axis: Vec3, angle: f32) -> Self {
        Self(glam::Quat::from_axis_angle(axis.0, angle))
    }

    #[inline]
    pub fn from_rotation_x(angle: f32) -> Self {
        Self(glam::Quat::from_rotation_x(angle))
    }

    #[inline]
    pub fn from_rotation_y(angle: f32) -> Self {
        Self(glam::Quat::from_rotation_y(angle))
    }

    #[inline]
    pub fn from_rotation_z(angle: f32) -> Self {
        Self(glam::Quat::from_rotation_z(angle))
    }

    /// 欧拉角（弧度）构造，按 ZYX 顺序应用。
    #[inline]
    pub fn from_euler(x: f32, y: f32, z: f32) -> Self {
        Self(glam::Quat::from_euler(glam::EulerRot::ZYX, x, y, z))
    }

    /// 旋转向量 `v`。
    #[inline]
    pub fn rotate_vec3(self, v: Vec3) -> Vec3 {
        self.0.mul_vec3(v.0).into()
    }

    #[inline]
    pub fn conjugate(self) -> Self {
        Self(self.0.conjugate())
    }

    /// 球面插值。
    #[inline]
    pub fn slerp(self, other: Self, s: f32) -> Self {
        Self(self.0.slerp(other.0, s))
    }

    #[inline]
    pub fn to_mat4(self) -> crate::mat::Mat4 {
        crate::mat::Mat4::from_quat(self)
    }
}

impl From<glam::Quat> for Quat {
    #[inline]
    fn from(q: glam::Quat) -> Self {
        Self(q)
    }
}

impl From<Quat> for glam::Quat {
    #[inline]
    fn from(q: Quat) -> Self {
        q.0
    }
}

impl std::ops::Deref for Quat {
    type Target = glam::Quat;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Quat {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl std::ops::Mul<Quat> for Quat {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Quat) -> Self {
        (self.0 * rhs.0).into()
    }
}

impl std::ops::Mul<Vec3> for Quat {
    type Output = Vec3;
    #[inline]
    fn mul(self, rhs: Vec3) -> Vec3 {
        (self.0 * rhs.0).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_axis() {
        let q = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let r = q * Vec3::new(1.0, 0.0, 0.0);
        assert!((r.x - 0.0).abs() < 1e-5);
        assert!((r.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn identity() {
        assert_eq!(
            Quat::IDENTITY * Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(1.0, 2.0, 3.0)
        );
    }
}
