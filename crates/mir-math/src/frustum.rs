//! 视锥与包围盒：extract 侧视锥剔除（R-13）。
//!
//! [`Frustum`] 由 view-projection 矩阵按 Gribb–Hartmann 六平面提取，
//! 平面方程 `ax + by + cz + d >= 0` 表示半空间在内侧（平面已归一化）。
//! 深度约定为 Vulkan（NDC z ∈ 0…1）：近平面 = clip z 行，远平面 = w 行 − z 行。
//!
//! 剔除发生在应用层 extract（ADR-0004 决策 6）：渲染层不感知剔除。

use crate::mat::Mat4;
use crate::vec::Vec3;

/// 轴对齐包围盒。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// 由中心与半包围（half extent）构造。
    pub fn from_center_half_extent(center: Vec3, half_extent: Vec3) -> Self {
        Self {
            min: center - half_extent,
            max: center + half_extent,
        }
    }

    /// 用矩阵变换包围盒，并返回包围变换后 8 个角点的世界空间 AABB。
    ///
    /// 层级场景下局部 AABB 需先经世界矩阵变换，再交给视锥剔除（M3）。
    pub fn transformed(&self, matrix: Mat4) -> Self {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for corner in self.corners() {
            let point = matrix.transform_point(corner);
            min = min.min(point);
            max = max.max(point);
        }
        Self { min, max }
    }

    /// 包围盒的 8 个角点。
    pub fn corners(&self) -> [Vec3; 8] {
        [
            Vec3::new(self.min.x, self.min.y, self.min.z),
            Vec3::new(self.max.x, self.min.y, self.min.z),
            Vec3::new(self.min.x, self.max.y, self.min.z),
            Vec3::new(self.max.x, self.max.y, self.min.z),
            Vec3::new(self.min.x, self.min.y, self.max.z),
            Vec3::new(self.max.x, self.min.y, self.max.z),
            Vec3::new(self.min.x, self.max.y, self.max.z),
            Vec3::new(self.max.x, self.max.y, self.max.z),
        ]
    }
}

/// 视锥体（世界空间，6 个归一化平面，法线指向体内）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frustum {
    /// 平面方程系数 `[a, b, c, d]`：`a*x + b*y + c*z + d >= 0` 为内侧。
    planes: [[f32; 4]; 6],
}

impl Frustum {
    /// 从 view-projection 矩阵提取六平面（Vulkan 深度 0…1 约定）。
    pub fn from_view_proj(view_proj: Mat4) -> Self {
        // glam 为列主序；行 i 的元素为 [m[i], m[i+4], m[i+8], m[i+12]]。
        let m = view_proj.to_cols_array();
        let row = |i: usize| [m[i], m[i + 4], m[i + 8], m[i + 12]];
        let clip_z = row(2);
        let clip_w = row(3);
        let clip_x = row(0);
        let clip_y = row(1);

        let raw = [
            add(clip_w, clip_x), // 左
            sub(clip_w, clip_x), // 右
            add(clip_w, clip_y), // 下
            sub(clip_w, clip_y), // 上
            clip_z,              // 近（clip z >= 0）
            sub(clip_w, clip_z), // 远（clip z <= w）
        ];
        let planes = raw.map(normalize_plane);
        Self { planes }
    }

    /// 点是否在视锥内（含边界）。
    pub fn contains_point(&self, point: Vec3) -> bool {
        self.planes
            .iter()
            .all(|[a, b, c, d]| a * point.x + b * point.y + c * point.z + *d >= 0.0)
    }

    /// 包围盒是否与视锥相交（或完全在内）。
    ///
    /// 采用正顶点（p-vertex）测试：对每个平面取沿法线方向最远的角点，
    /// 若该点也在平面外侧则整个包围盒在外侧。
    pub fn intersects_aabb(&self, aabb: &Aabb) -> bool {
        self.planes.iter().all(|&[a, b, c, d]| {
            let px = if a >= 0.0 { aabb.max.x } else { aabb.min.x };
            let py = if b >= 0.0 { aabb.max.y } else { aabb.min.y };
            let pz = if c >= 0.0 { aabb.max.z } else { aabb.min.z };
            a * px + b * py + c * pz + d >= 0.0
        })
    }

    /// 六个平面（调试/单测用）。
    pub fn planes(&self) -> &[[f32; 4]; 6] {
        &self.planes
    }
}

fn add(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

fn sub(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
}

/// 归一化平面（法线长度归 1；法线退化为零时原样返回，只出现在退化矩阵中）。
fn normalize_plane(plane: [f32; 4]) -> [f32; 4] {
    let [a, b, c, d] = plane;
    let length = (a * a + b * b + c * c).sqrt();
    if length <= f32::EPSILON {
        return plane;
    }
    [a / length, b / length, c / length, d / length]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mat::Mat4;

    /// 与 miriya-app 相机同构的 LH 视锥：位于原点、看向 +X、fov 60°。
    fn frustum_at_origin() -> Frustum {
        // 组合顺序与 `Camera::view_projection` 一致：projection * view（列向量约定）。
        let view_proj = Mat4::perspective(60.0_f32.to_radians(), 1.0, 1.0, 100.0)
            * Mat4::look_to(Vec3::ZERO, Vec3::X, Vec3::Z);
        Frustum::from_view_proj(view_proj)
    }

    #[test]
    fn frustum_planes_are_unit_normals() {
        let frustum = frustum_at_origin();
        for [a, b, c, _] in frustum.planes() {
            let length = (a * a + b * b + c * c).sqrt();
            assert!((length - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn points_inside_and_outside_frustum() {
        let frustum = frustum_at_origin();
        // 视空间 +Z 向前：视锥沿 +X 张开，近处 (5, 0, 0) 在内部。
        assert!(frustum.contains_point(Vec3::new(5.0, 0.0, 0.0)));
        // 原点在近平面之前（近平面距离 1）。
        assert!(!frustum.contains_point(Vec3::ZERO));
        // 远平面之外。
        assert!(!frustum.contains_point(Vec3::new(200.0, 0.0, 0.0)));
        // 上方超出视锥（fov 60°，5 米处上边界 z ≈ 2.9）。
        assert!(!frustum.contains_point(Vec3::new(5.0, 0.0, 5.0)));
        // 侧向超出。
        assert!(!frustum.contains_point(Vec3::new(5.0, 5.0, 0.0)));
    }

    #[test]
    fn aabb_intersection_and_culling() {
        let frustum = frustum_at_origin();
        // 完全在内。
        let inside = Aabb::from_center_half_extent(Vec3::new(10.0, 0.0, 0.0), Vec3::splat(1.0));
        assert!(frustum.intersects_aabb(&inside));
        // 在相机后方（近平面前）。
        let behind = Aabb::from_center_half_extent(Vec3::new(-5.0, 0.0, 0.0), Vec3::splat(1.0));
        assert!(!frustum.intersects_aabb(&behind));
        // 与近平面相交（跨越 z=1 平面）。
        let crossing = Aabb::from_center_half_extent(Vec3::new(1.0, 0.0, 0.0), Vec3::splat(1.0));
        assert!(frustum.intersects_aabb(&crossing));
        // 侧向远处：AABB 角点测试剔除。
        let sideways = Aabb::from_center_half_extent(Vec3::new(10.0, 20.0, 0.0), Vec3::splat(1.0));
        assert!(!frustum.intersects_aabb(&sideways));
    }

    #[test]
    fn aabb_corners_cover_all_extremes() {
        let aabb =
            Aabb::from_center_half_extent(Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 1.0, 1.0));
        let corners = aabb.corners();
        assert_eq!(corners.len(), 8);
        assert!(corners.iter().any(|c| *c == Vec3::new(0.0, 1.0, 2.0)));
        assert!(corners.iter().any(|c| *c == Vec3::new(2.0, 3.0, 4.0)));
    }

    #[test]
    fn aabb_transformed_covers_rotated_extent() {
        use crate::{Quat, Transform};

        let local = Aabb::from_center_half_extent(Vec3::ZERO, Vec3::new(1.0, 2.0, 3.0));
        let world = Transform {
            position: Vec3::new(100.0, 0.0, 0.0),
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale: Vec3::splat(2.0),
        };
        let transformed = local.transformed(world.to_matrix());
        assert!((transformed.min.x - 96.0).abs() < 1e-4);
        assert!((transformed.max.x - 104.0).abs() < 1e-4);
        assert!((transformed.min.y + 2.0).abs() < 1e-4);
        assert!((transformed.max.y - 2.0).abs() < 1e-4);
        assert!((transformed.min.z + 6.0).abs() < 1e-4);
        assert!((transformed.max.z - 6.0).abs() < 1e-4);
    }
}
