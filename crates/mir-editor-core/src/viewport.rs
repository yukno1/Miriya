//! 视口、Gizmo 与拾取的 UI 无关数据契约。
//!
//! 本模块只描述数学量与请求/结果，不创建窗口、surface 或 GPU 资源；
//! 实际渲染由 `mir-render` 承担，UI 宿主只消费结果。

use mir_math::{Quat, Vec3};

use crate::document::ObjectId;

/// Gizmo 操作模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GizmoMode {
    #[default]
    None,
    Translate,
    Rotate,
    Scale,
}

/// Gizmo 轴向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GizmoAxis {
    #[default]
    None,
    X,
    Y,
    Z,
}

/// 视口相机（位置单位为厘米，Z-up，左手系）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportCamera {
    pub position: Vec3,
    pub rotation: Quat,
    /// 垂直视场角（度）。
    pub fov_y_degrees: f32,
    /// 近裁剪面（厘米）。
    pub near: f32,
    /// 远裁剪面（厘米）。
    pub far: f32,
}

impl Default for ViewportCamera {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, -500.0, 200.0),
            rotation: Quat::IDENTITY,
            fov_y_degrees: 60.0,
            near: 1.0,
            far: 100_000.0,
        }
    }
}

/// 拾取请求：视口内的像素坐标与视口尺寸。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickRequest {
    pub x: u32,
    pub y: u32,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

impl PickRequest {
    /// 请求坐标是否落在视口内。
    pub fn is_inside(&self) -> bool {
        self.viewport_width > 0
            && self.viewport_height > 0
            && self.x < self.viewport_width
            && self.y < self.viewport_height
    }
}

/// 拾取结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickResult {
    Miss,
    Hit(ObjectId),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_request_bounds() {
        let request = PickRequest {
            x: 10,
            y: 20,
            viewport_width: 100,
            viewport_height: 50,
        };
        assert!(request.is_inside());
        assert!(!PickRequest { x: 100, ..request }.is_inside());
    }

    #[test]
    fn camera_defaults_are_valid() {
        let camera = ViewportCamera::default();
        assert!(camera.fov_y_degrees > 0.0);
        assert!(camera.near > 0.0);
        assert!(camera.far > camera.near);
    }
}
