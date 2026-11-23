//! 帧数据：渲染层的纯数据输入。
//!
//! `mir-render` 不访问 ECS/场景；应用层在 extract 阶段把世界转换成这些类型（ADR-0004）。

use mir_math::Mat4;

use crate::MeshHandle;

/// 相机帧数据。
///
/// v0.2 只传 view-projection；M5 的 PBR 光照需要相机世界位置，届时在此扩展。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraFrame {
    pub view_proj: Mat4,
}

impl Default for CameraFrame {
    fn default() -> Self {
        Self {
            view_proj: Mat4::IDENTITY,
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

/// 一帧内的一次绘制请求（网格 + 实例数据）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DrawInstance {
    pub mesh: MeshHandle,
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
