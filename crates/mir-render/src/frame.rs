//! 帧数据：渲染层的纯数据输入。
//!
//! `mir-render` 不访问 ECS/场景；应用层在 extract 阶段把世界转换成这些类型（ADR-0004）。

use mir_math::Mat4;

use crate::{MaterialHandle, MeshHandle};

/// 相机帧数据。
///
/// R-12 起 PBR 高光需要相机世界位置（视线方向 `V = normalize(camera_position - world_pos)`）；
/// UBO 布局为 std140：`view_proj`（64 B）+ `camera_position`（偏移 64，补齐到 80 B）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraFrame {
    pub view_proj: Mat4,
    /// 相机世界坐标（米/单位与场景一致）。
    pub camera_position: [f32; 3],
}

impl Default for CameraFrame {
    fn default() -> Self {
        Self {
            view_proj: Mat4::IDENTITY,
            camera_position: [0.0; 3],
        }
    }
}

/// 单个实例：模型矩阵 + 颜色 tint。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InstanceData {
    pub model: Mat4,
    pub tint: [f32; 4],
}

impl Default for InstanceData {
    fn default() -> Self {
        Self {
            model: Mat4::IDENTITY,
            tint: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// 一帧内的一次绘制请求（网格 + 实例数据 + 可选材质）。
///
/// `material = None` 表示使用渲染层内置的默认材质（白色 Lambert）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DrawInstance {
    pub mesh: MeshHandle,
    pub material: Option<MaterialHandle>,
    pub data: InstanceData,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_identity_and_white() {
        let camera = CameraFrame::default();
        assert_eq!(camera.view_proj, Mat4::IDENTITY);

        let instance = InstanceData::default();
        assert_eq!(instance.model, Mat4::IDENTITY);
        assert_eq!(instance.tint, [1.0, 1.0, 1.0, 1.0]);
    }
}
