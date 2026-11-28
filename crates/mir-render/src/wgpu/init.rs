//! wgpu 实例 / surface / 适配器 / 设备创建。

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use wgpu::{
    Backends, DeviceDescriptor, ExperimentalFeatures, Features, Instance, InstanceDescriptor,
    InstanceFlags, Limits, MemoryHints, PowerPreference, RequestAdapterOptions, Surface,
    SurfaceTargetUnsafe, Trace,
};

use crate::RenderInitError;
use crate::frame::PUSH_CONSTANT_SIZE;
use crate::resource_table::SlotTable;

use super::WgpuCore;

impl WgpuCore {
    /// headless：无 surface、无交换链，但仍创建设备与管线以支持离线资源上传。
    pub(crate) fn new_headless(validation: bool) -> Result<Self, RenderInitError> {
        Self::create(validation, |_| Ok(None))
    }

    /// 带窗口：surface 必须在请求适配器前创建，以便选择可呈现的适配器。
    pub(crate) fn with_surface(
        display: &dyn HasDisplayHandle,
        window: &dyn HasWindowHandle,
        width: u32,
        height: u32,
        validation: bool,
    ) -> Result<Self, RenderInitError> {
        let mut core = Self::create(validation, |instance| {
            Self::create_surface(instance, display, window).map(Some)
        })?;
        core.configure_surface(width, height)
            .map_err(RenderInitError::Wgpu)?;
        Ok(core)
    }

    fn create<F>(validation: bool, make_surface: F) -> Result<Self, RenderInitError>
    where
        F: FnOnce(&Instance) -> Result<Option<Surface<'static>>, RenderInitError>,
    {
        let flags = if validation {
            InstanceFlags::VALIDATION | InstanceFlags::DEBUG
        } else {
            InstanceFlags::empty()
        };
        let mut descriptor = InstanceDescriptor::new_without_display_handle();
        descriptor.backends = Backends::PRIMARY;
        descriptor.flags = flags;
        let instance = Instance::new(descriptor);
        let surface = make_surface(&instance)?;

        let mut options = RequestAdapterOptions {
            power_preference: PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: surface.as_ref(),
            apply_limit_buckets: false,
        };
        let adapter = match pollster::block_on(instance.request_adapter(&options)) {
            Ok(adapter) => adapter,
            Err(_) => {
                options.force_fallback_adapter = true;
                pollster::block_on(instance.request_adapter(&options))
                    .map_err(|error| RenderInitError::Wgpu(format!("无可用适配器：{error}")))?
            }
        };
        // pbr.spv 的 PushConstant 由 naga 翻译为 wgpu IMMEDIATES（wgpu 30 起）。
        if !adapter.features().contains(Features::IMMEDIATES) {
            return Err(RenderInitError::Wgpu(
                "适配器不支持 IMMEDIATES（对应 Vulkan push constants）".to_string(),
            ));
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            label: None,
            required_features: Features::IMMEDIATES,
            required_limits: Limits {
                max_immediate_size: PUSH_CONSTANT_SIZE,
                ..Limits::default()
            },
            experimental_features: ExperimentalFeatures::default(),
            memory_hints: MemoryHints::default(),
            trace: Trace::Off,
        }))
        .map_err(|error| RenderInitError::Wgpu(format!("创建设备失败：{error}")))?;

        let info = adapter.get_info();
        let adapter_name = info.name;
        let backend_name = format!("{:?}", info.backend);
        let mut core = Self {
            instance,
            adapter,
            device,
            queue,
            surface,
            config: None,
            depth: None,
            depth_view: None,
            depth_bytes: 0,
            pipeline: None,
            camera_frames: Vec::new(),
            frame_index: 0,
            meshes: SlotTable::new(),
            materials: SlotTable::new(),
            textures: SlotTable::new(),
            default_material: None,
            allocated: 0,
            graph_dump: false,
            dump_warned: false,
            adapter_name,
            backend_name,
            surface_format: None,
            present_mode: None,
            swapchain_image_count: 0,
        };
        core.init_pipeline().map_err(RenderInitError::Wgpu)?;
        core.init_frame_resources().map_err(RenderInitError::Wgpu)?;
        Ok(core)
    }

    fn create_surface(
        instance: &Instance,
        display: &dyn HasDisplayHandle,
        window: &dyn HasWindowHandle,
    ) -> Result<Surface<'static>, RenderInitError> {
        let display_handle = display
            .display_handle()
            .map_err(|error| RenderInitError::Surface(error.to_string()))?;
        let window_handle = window
            .window_handle()
            .map_err(|error| RenderInitError::Surface(error.to_string()))?;
        // SAFETY: 调用方保证窗口与 display 至少存活到 WgpuCore 被销毁；
        // raw handle 仅用于创建 surface，wgpu 不会在此处取得所有权。
        let surface = unsafe {
            instance.create_surface_unsafe(SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(display_handle.as_raw()),
                raw_window_handle: window_handle.as_raw(),
            })
        }
        .map_err(|error| RenderInitError::Surface(error.to_string()))?;
        Ok(surface)
    }
}
