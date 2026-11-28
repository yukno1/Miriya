//! 后端分发：crate 内部用 enum + match 做静态分发，公共 API 不暴露任何图形 API 类型。

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::frame::{CameraFrame, DrawInstance};
use crate::vulkan::VulkanCore;
use crate::wgpu::WgpuCore;
use crate::{
    MaterialData, MaterialHandle, MeshData, MeshHandle, RenderInitError, RendererStatus,
    TextureData, TextureHandle,
};

/// 实际生效的渲染后端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Wgpu,
    Vulkan,
}

/// 后端偏好：`Auto` 先 wgpu，失败后回退 Vulkan。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendPreference {
    #[default]
    Auto,
    Wgpu,
    Vulkan,
}

impl BackendPreference {
    /// 按优先级返回候选后端。
    pub(crate) fn candidates(self) -> &'static [BackendKind] {
        match self {
            Self::Auto => &[BackendKind::Wgpu, BackendKind::Vulkan],
            Self::Wgpu => &[BackendKind::Wgpu],
            Self::Vulkan => &[BackendKind::Vulkan],
        }
    }
}

/// 内部统一后端：所有资源与帧操作经此委派到具体实现。
pub(crate) enum ActiveBackend {
    Wgpu(Box<WgpuCore>),
    Vulkan(Box<VulkanCore>),
}

impl ActiveBackend {
    pub(crate) fn new_headless(
        kind: BackendKind,
        validation: bool,
    ) -> Result<Self, RenderInitError> {
        match kind {
            BackendKind::Wgpu => Ok(Self::Wgpu(Box::new(WgpuCore::new_headless(validation)?))),
            BackendKind::Vulkan => Ok(Self::Vulkan(Box::new(VulkanCore::new_headless(
                validation,
            )?))),
        }
    }

    pub(crate) fn with_window(
        kind: BackendKind,
        display: &dyn HasDisplayHandle,
        window: &dyn HasWindowHandle,
        width: u32,
        height: u32,
        validation: bool,
    ) -> Result<Self, RenderInitError> {
        match kind {
            BackendKind::Wgpu => Ok(Self::Wgpu(Box::new(WgpuCore::with_surface(
                display, window, width, height, validation,
            )?))),
            BackendKind::Vulkan => Ok(Self::Vulkan(Box::new(VulkanCore::with_surface(
                display, window, width, height, validation,
            )?))),
        }
    }

    pub(crate) fn kind(&self) -> BackendKind {
        match self {
            Self::Wgpu(_) => BackendKind::Wgpu,
            Self::Vulkan(_) => BackendKind::Vulkan,
        }
    }

    pub(crate) fn describe(&self) -> RendererStatus {
        match self {
            Self::Wgpu(core) => RendererStatus::Wgpu {
                adapter_name: core.device_name().to_string(),
                backend: core.backend_name().to_string(),
                has_surface: core.has_surface(),
                surface_format: core.surface_format().map(str::to_string),
                present_mode: core.present_mode().map(str::to_string),
                swapchain_image_count: core.swapchain_image_count(),
            },
            Self::Vulkan(core) => RendererStatus::Vulkan {
                device_name: core.device_name().to_string(),
                api_version: core.api_version().to_string(),
                has_surface: core.has_surface(),
                graphics_family: core.queue_families().0,
                present_family: core.queue_families().1,
                surface_format: core.surface_format().map(str::to_string),
                present_mode: core.present_mode().map(str::to_string),
                swapchain_image_count: core.swapchain_image_count(),
            },
        }
    }

    pub(crate) fn set_graph_dump(&mut self, enabled: bool) {
        match self {
            Self::Wgpu(core) => core.set_graph_dump(enabled),
            Self::Vulkan(core) => core.set_graph_dump(enabled),
        }
    }

    pub(crate) fn last_pass_timings(&self) -> &[(String, f32)] {
        match self {
            Self::Wgpu(core) => core.last_pass_timings(),
            Self::Vulkan(core) => core.last_pass_timings(),
        }
    }

    pub(crate) fn allocated_bytes(&self) -> Option<u64> {
        match self {
            Self::Wgpu(core) => Some(core.allocated_bytes()),
            Self::Vulkan(core) => Some(core.allocated_bytes()),
        }
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        match self {
            Self::Wgpu(core) => core.resize(width, height),
            Self::Vulkan(core) => core.recreate_swapchain(width, height),
        }
    }

    pub(crate) fn create_mesh(&mut self, data: &MeshData) -> Result<MeshHandle, String> {
        match self {
            Self::Wgpu(core) => core.create_mesh(data),
            Self::Vulkan(core) => core.create_mesh(data),
        }
    }

    pub(crate) fn destroy_mesh(&mut self, handle: MeshHandle) {
        match self {
            Self::Wgpu(core) => {
                let _ = core.destroy_mesh(handle);
            }
            Self::Vulkan(core) => {
                let _ = core.destroy_mesh(handle);
            }
        }
    }

    pub(crate) fn create_material(
        &mut self,
        data: &MaterialData,
    ) -> Result<MaterialHandle, String> {
        match self {
            Self::Wgpu(core) => core.create_material(data),
            Self::Vulkan(core) => core.create_material(data),
        }
    }

    pub(crate) fn destroy_material(&mut self, handle: MaterialHandle) {
        match self {
            Self::Wgpu(core) => {
                let _ = core.destroy_material(handle);
            }
            Self::Vulkan(core) => {
                let _ = core.destroy_material(handle);
            }
        }
    }

    pub(crate) fn create_texture(&mut self, data: &TextureData) -> Result<TextureHandle, String> {
        match self {
            Self::Wgpu(core) => core.create_texture(data),
            Self::Vulkan(core) => core.create_texture(data),
        }
    }

    pub(crate) fn destroy_texture(&mut self, handle: TextureHandle) {
        match self {
            Self::Wgpu(core) => {
                let _ = core.destroy_texture(handle);
            }
            Self::Vulkan(core) => {
                let _ = core.destroy_texture(handle);
            }
        }
    }

    pub(crate) fn render_frame(
        &mut self,
        clear_color: [f32; 4],
        width: u32,
        height: u32,
        camera: CameraFrame,
        draws: &[DrawInstance],
    ) -> Result<(), String> {
        match self {
            Self::Wgpu(core) => core.render_frame(clear_color, width, height, camera, draws),
            Self::Vulkan(core) => core.render_frame(clear_color, width, height, camera, draws),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_selects_candidate_order() {
        assert_eq!(
            BackendPreference::Auto.candidates(),
            &[BackendKind::Wgpu, BackendKind::Vulkan]
        );
        assert_eq!(BackendPreference::Wgpu.candidates(), &[BackendKind::Wgpu]);
        assert_eq!(
            BackendPreference::Vulkan.candidates(),
            &[BackendKind::Vulkan]
        );
    }
}
