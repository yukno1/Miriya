//! wgpu surface 配置、呈现模式选择与深度附件重建。

use wgpu::{
    CompositeAlphaMode, Extent3d, PresentMode, SurfaceConfiguration, TextureDescriptor,
    TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
};

use crate::frame::MAX_FRAMES_IN_FLIGHT;

use super::WgpuCore;

impl WgpuCore {
    /// 首次配置 surface：优先 sRGB 格式、Immediate 呈现模式与 Opaque alpha。
    pub(crate) fn configure_surface(&mut self, width: u32, height: u32) -> Result<(), String> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        let surface = self
            .surface
            .as_ref()
            .ok_or_else(|| "surface 尚未创建".to_string())?;
        let caps = surface.get_capabilities(&self.adapter);
        let mut config: SurfaceConfiguration = surface
            .get_default_config(&self.adapter, width, height)
            .ok_or_else(|| "适配器不支持该 surface".to_string())?;
        config.format = caps
            .formats
            .iter()
            .copied()
            .find(TextureFormat::is_srgb)
            .unwrap_or(config.format);
        config.present_mode = if caps.present_modes.contains(&PresentMode::Immediate) {
            PresentMode::Immediate
        } else {
            PresentMode::Fifo
        };
        config.alpha_mode = if caps.alpha_modes.contains(&CompositeAlphaMode::Opaque) {
            CompositeAlphaMode::Opaque
        } else {
            CompositeAlphaMode::Auto
        };
        config.desired_maximum_frame_latency = MAX_FRAMES_IN_FLIGHT as u32;
        surface.configure(&self.device, &config);

        self.surface_format = Some(format!("{:?}", config.format));
        self.present_mode = Some(format!("{:?}", config.present_mode));
        self.swapchain_image_count = 0; // wgpu 不暴露交换链图像数。
        self.config = Some(config);
        self.recreate_depth(width, height)?;
        if self
            .pipeline
            .as_ref()
            .is_some_and(|pipeline| pipeline.format != self.config.as_ref().expect("刚配置").format)
        {
            self.init_pipeline()?;
        }
        Ok(())
    }

    /// 窗口尺寸变化后重新配置交换链并重建深度附件。
    pub(crate) fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        if self.surface.is_none() || width == 0 || height == 0 {
            return Ok(());
        }
        let mut config = self
            .config
            .clone()
            .ok_or_else(|| "surface 尚未配置".to_string())?;
        if config.width == width && config.height == height {
            return Ok(());
        }
        config.width = width;
        config.height = height;
        self.surface
            .as_ref()
            .expect("surface 存在")
            .configure(&self.device, &config);
        self.config = Some(config);
        self.recreate_depth(width, height)
    }

    fn recreate_depth(&mut self, width: u32, height: u32) -> Result<(), String> {
        // 先置空引用再让旧纹理随作用域结束释放，避免新旧附件同时存活。
        self.depth_view = None;
        self.depth = None;
        if self.depth_bytes > 0 {
            self.track_free(self.depth_bytes);
            self.depth_bytes = 0;
        }
        let texture = self.device.create_texture(&TextureDescriptor {
            label: Some("wgpu depth"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        self.depth_bytes = u64::from(width) * u64::from(height) * 4;
        self.track_alloc(self.depth_bytes);
        self.depth = Some(texture);
        self.depth_view = Some(view);
        Ok(())
    }
}
