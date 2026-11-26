//! 空间变换（M3）。
//!
//! `Transform` 是纯数学类型，归 `mir-math` 所有；位置使用 **厘米**（ADR-0003），
//! 缩放无量纲，旋转使用四元数。组件标记由 `mir-ecs` 在该 crate 外实现，
//! 以免数学层反向依赖 ECS。

use serde::{Deserialize, Serialize};

use crate::{Mat4, Quat, Vec3};

/// 局部空间变换：平移（厘米）→ 旋转 → 缩放。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    /// 世界/父空间中的位置，单位厘米。
    pub position: Vec3,
    /// 相对旋转。
    pub rotation: Quat,
    /// 各轴缩放，无量纲。
    pub scale: Vec3,
}

impl Transform {
    /// 恒等变换：位置为零厘米、无旋转、单位缩放。
    pub const IDENTITY: Self = Self {
        position: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// 由 TRS 构造变换。
    #[inline]
    pub const fn new(position: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self {
            position,
            rotation,
            scale,
        }
    }

    /// 仅指定位置的变换，旋转与缩放取默认值。
    #[inline]
    pub const fn from_position(position: Vec3) -> Self {
        Self {
            position,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    /// 组合为 4×4 矩阵：先缩放、再旋转、后平移。
    #[inline]
    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_trs(self.position, self.rotation, self.scale)
    }

    /// 从矩阵还原 TRS（glam 的列主序分解）。
    #[inline]
    pub fn from_matrix(matrix: Mat4) -> Self {
        let (scale, rotation, translation) = matrix.0.to_scale_rotation_translation();
        Self {
            position: translation.into(),
            rotation: rotation.into(),
            scale: scale.into(),
        }
    }

    /// `self` 为父变换、`child` 为局部变换时，返回组合后的变换。
    ///
    /// 非均匀缩放与旋转组合的 TRS 分解可能与原矩阵有舍入差异；
    /// 层级世界矩阵计算应直接使用 [`Transform::to_matrix`] 相乘。
    #[inline]
    pub fn compose(&self, child: &Self) -> Self {
        Self::from_matrix(self.to_matrix() * child.to_matrix())
    }

    /// 逆变换；行列式接近零（奇异矩阵）时返回 `None`。
    #[inline]
    pub fn inverse(&self) -> Option<Self> {
        let matrix = self.to_matrix().0;
        if matrix.determinant().abs() <= f32::EPSILON {
            return None;
        }
        Some(Self::from_matrix(matrix.inverse().into()))
    }
}

impl Default for Transform {
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    #[test]
    fn default_is_identity() {
        let t = Transform::default();
        assert_eq!(t.position, Vec3::ZERO);
        assert_eq!(t.rotation, Quat::IDENTITY);
        assert_eq!(t.scale, Vec3::ONE);
        assert_eq!(t.to_matrix(), Mat4::IDENTITY);
    }

    #[test]
    fn to_matrix_applies_trs_order() {
        let t = Transform {
            position: Vec3::new(100.0, 200.0, 300.0),
            rotation: Quat::from_rotation_z(FRAC_PI_2),
            scale: Vec3::splat(2.0),
        };
        let p = t.to_matrix().transform_point(Vec3::new(100.0, 0.0, 0.0));
        assert!((p.x - 100.0).abs() < 1e-4, "TRS 顺序错误: {p:?}");
        assert!((p.y - 400.0).abs() < 1e-4, "TRS 顺序错误: {p:?}");
        assert!((p.z - 300.0).abs() < 1e-4, "TRS 顺序错误: {p:?}");
    }

    #[test]
    fn compose_matches_matrix_multiplication() {
        let parent = Transform {
            position: Vec3::new(100.0, 0.0, 0.0),
            rotation: Quat::from_rotation_z(FRAC_PI_2),
            scale: Vec3::ONE,
        };
        let child = Transform::from_position(Vec3::new(200.0, 0.0, 0.0));
        let combined = parent.compose(&child);
        let expected = parent.to_matrix() * child.to_matrix();
        for (a, b) in combined
            .to_matrix()
            .to_cols_array()
            .iter()
            .zip(expected.to_cols_array())
        {
            assert!((*a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn inverse_roundtrip() {
        let t = Transform {
            position: Vec3::new(100.0, -200.0, 300.0),
            rotation: Quat::from_rotation_y(FRAC_PI_2),
            scale: Vec3::splat(2.0),
        };
        let inverse = t.inverse().expect("非奇异矩阵可逆");
        let product = t.to_matrix() * inverse.to_matrix();
        for (a, b) in product
            .to_cols_array()
            .iter()
            .zip(Mat4::IDENTITY.to_cols_array())
        {
            assert!((*a - b).abs() < 1e-4);
        }
    }
}
