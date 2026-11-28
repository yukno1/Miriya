//! 渲染子系统：wgpu 首选、Vulkan 1.3（`vulkanalia`）保留的双后端（ADR-0014）。
//!
//! 公共 API 不暴露任何图形 API 类型；`WgpuCore` / `VulkanCore` 完全封装在 crate 内部。
//! 两个后端共用纯数据帧契约、RenderGraph 声明与世代句柄表；Vulkan 侧保留完整
//! barrier 推导、GPU pass 时间戳、管线缓存与自研显存子分配器。
//!
//! 无窗口（headless）路径不会创建 surface，也不会要求交换链扩展，
//! 因此 `mir-core` 的 `run_headless` 与 CI 仍然可用。

mod allocator;
mod backend;
mod frame;
mod geometry;
mod graph;
mod material;
mod mesh;
mod resource_table;
mod texture;
mod vulkan;
mod wgpu;

pub use backend::{BackendKind, BackendPreference};
pub use frame::{CameraFrame, InstanceData};
pub use geometry::{cube, sphere, torus};
pub use material::{MaterialData, MaterialHandle};
pub use mesh::{MeshData, MeshHandle, Vertex};
pub use texture::{ImageFormat, MipRange, TextureData, TextureDataError, TextureHandle};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use thiserror::Error;

use crate::backend::ActiveBackend;

/// 校验层默认策略：debug 构建开启，单元测试关闭以避免噪音。
fn default_validation() -> bool {
    cfg!(debug_assertions) && !cfg!(test)
}

/// 把各候选后端的失败原因拼成一条可读诊断。
fn describe_attempts(attempts: &[(BackendKind, String)]) -> String {
    if attempts.is_empty() {
        return "没有可用的渲染后端候选".to_string();
    }
    attempts
        .iter()
        .map(|(kind, error)| format!("{kind:?}: {error}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// 渲染初始化失败。仅用于诊断；`RenderContext::init` 会记录失败并继续以占位模式运行。
#[derive(Debug, Clone, PartialEq, Error)]
pub enum RenderInitError {
    #[error("Vulkan loader unavailable: {0}")]
    Loader(String),
    #[error("Vulkan instance creation failed: {0}")]
    Instance(String),
    #[error("no suitable Vulkan 1.3 physical device")]
    NoDevice,
    #[error("Vulkan logical device creation failed: {0}")]
    Device(String),
    #[error("Vulkan surface creation failed: {0}")]
    Surface(String),
    #[error("Vulkan swapchain creation failed: {0}")]
    Swapchain(String),
    #[error("Vulkan frame resources initialization failed: {0}")]
    FrameResources(String),
    #[error("Vulkan depth resources initialization failed: {0}")]
    Depth(String),
    #[error("Vulkan transfer resources initialization failed: {0}")]
    Transfer(String),
    #[error("Vulkan mesh upload failed: {0}")]
    Mesh(String),
    #[error("Vulkan material creation failed: {0}")]
    Material(String),
    #[error("Vulkan texture creation failed: {0}")]
    Texture(String),
    #[error("wgpu backend initialization failed: {0}")]
    Wgpu(String),
    #[error("all rendering backends failed: {0:?}")]
    AllBackendsFailed(Vec<(BackendKind, String)>),
    #[error("renderer is not initialized")]
    NotInitialized,
    #[error("renderer is already initialized")]
    AlreadyInitialized,
}

/// 渲染后端当前状态（UI/应用层可显示，不含 Vulkan 类型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererStatus {
    /// 尚未调用初始化。
    Uninitialized,
    /// Vulkan 已就绪。
    Vulkan {
        device_name: String,
        api_version: String,
        has_surface: bool,
        graphics_family: u32,
        present_family: u32,
        surface_format: Option<String>,
        present_mode: Option<String>,
        swapchain_image_count: u32,
    },
    /// wgpu 已就绪（`backend` 为 wgpu 实际选择的底层 API）。
    Wgpu {
        adapter_name: String,
        backend: String,
        has_surface: bool,
        surface_format: Option<String>,
        present_mode: Option<String>,
        swapchain_image_count: u32,
    },
    /// 初始化失败，附可读原因。
    Failed(String),
}

/// 渲染上下文：持有后端状态，由主循环持有。
pub struct RenderContext {
    pub exit_requested: bool,
    initialized: bool,
    width: u32,
    height: u32,
    backend: Option<ActiveBackend>,
    preference: BackendPreference,
    status: RendererStatus,
    clear_color: [f32; 4],
    camera: CameraFrame,
    draws: Vec<frame::DrawInstance>,
    frame_error: Option<String>,
    validation: bool,
    /// 每帧把 RenderGraph 依赖图 dump 成 DOT 文件（批次 B R-05）。
    graph_dump: bool,
}

impl RenderContext {
    /// 创建渲染上下文。
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            exit_requested: false,
            initialized: false,
            width,
            height,
            backend: None,
            preference: BackendPreference::default(),
            status: RendererStatus::Uninitialized,
            clear_color: [0.0, 0.0, 0.0, 1.0],
            camera: CameraFrame::default(),
            draws: Vec::new(),
            frame_error: None,
            validation: default_validation(),
            graph_dump: false,
        }
    }

    /// 覆盖校验层开关；需在 `init` / `init_with_window` 之前调用。
    pub fn with_validation(mut self, enabled: bool) -> Self {
        self.validation = enabled;
        self
    }

    /// 覆盖后端偏好；需在 `init` / `init_with_window` 之前调用。
    pub fn with_backend(mut self, preference: BackendPreference) -> Self {
        self.preference = preference;
        self
    }

    /// 覆盖 RenderGraph 依赖图 dump 开关；需在 `init` / `init_with_window` 之前调用。
    ///
    /// 开启后每帧把依赖图 DOT 写入工作目录下的 `render_graph.dot`（调试用，默认关闭）。
    pub fn with_graph_dump(mut self, enabled: bool) -> Self {
        self.graph_dump = enabled;
        self
    }

    /// 当前校验层配置（实际是否启用取决于驱动是否提供 layer）。
    pub fn validation(&self) -> bool {
        self.validation
    }

    /// 以 headless 方式初始化：按偏好候选顺序创建后端，不创建 surface。
    ///
    /// 全部失败时记录 [`RendererStatus::Failed`] 而不 panic，保证 CI/无 GPU 环境可运行。
    pub fn init(&mut self) {
        if self.initialized {
            return;
        }
        let mut attempts = Vec::new();
        for kind in self.preference.candidates() {
            match ActiveBackend::new_headless(*kind, self.validation) {
                Ok(mut backend) => {
                    self.status = backend.describe();
                    backend.set_graph_dump(self.graph_dump);
                    self.backend = Some(backend);
                    break;
                }
                Err(error) => attempts.push((*kind, error.to_string())),
            }
        }
        if self.backend.is_none() {
            self.status = RendererStatus::Failed(describe_attempts(&attempts));
        }
        self.initialized = true;
    }

    /// 使用窗口初始化：按偏好候选顺序创建后端与 surface/交换链。
    ///
    /// `display` / `window` 由 `mir-window` 的 raw-window-handle 提供；
    /// 窗口必须比 [`RenderContext`] 活得更久（surface 在 Drop 时才销毁）。
    pub fn init_with_window(
        &mut self,
        display: &dyn HasDisplayHandle,
        window: &dyn HasWindowHandle,
    ) -> Result<(), RenderInitError> {
        if self.initialized {
            return Err(RenderInitError::AlreadyInitialized);
        }
        let mut attempts = Vec::new();
        for kind in self.preference.candidates() {
            match ActiveBackend::with_window(
                *kind,
                display,
                window,
                self.width,
                self.height,
                self.validation,
            ) {
                Ok(mut backend) => {
                    self.status = backend.describe();
                    backend.set_graph_dump(self.graph_dump);
                    self.backend = Some(backend);
                    self.initialized = true;
                    return Ok(());
                }
                Err(error) => attempts.push((*kind, error.to_string())),
            }
        }
        self.status = RendererStatus::Failed(describe_attempts(&attempts));
        self.initialized = true;
        Err(RenderInitError::AllBackendsFailed(attempts))
    }

    /// 当前实际生效的后端（初始化成功后为 `Some`）。
    pub fn backend_kind(&self) -> Option<BackendKind> {
        self.backend.as_ref().map(ActiveBackend::kind)
    }

    /// 是否已完成初始化（含降级为占位模式的情况）。
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// 当前后端状态。
    pub fn status(&self) -> &RendererStatus {
        &self.status
    }

    /// 最近一帧的错误（无交换链/无后端时为 `None`）。
    pub fn frame_error(&self) -> Option<&str> {
        self.frame_error.as_deref()
    }

    /// 运行时开关 RenderGraph 依赖图 dump（初始化前后均可调用）。
    pub fn set_graph_dump(&mut self, enabled: bool) {
        self.graph_dump = enabled;
        if let Some(backend) = self.backend.as_mut() {
            backend.set_graph_dump(enabled);
        }
    }

    /// 上一帧各 GPU pass 的耗时（毫秒；headless / 不支持时间戳时为空）。
    pub fn pass_timings(&self) -> &[(String, f32)] {
        self.backend
            .as_ref()
            .map(|backend| backend.last_pass_timings())
            .unwrap_or(&[])
    }

    /// 当前后端未释放的资源字节数（泄漏守恒诊断；无后端时为 `None`）。
    /// Vulkan 是自研子分配器的未释放字节；wgpu 是渲染器主动请求的 buffer/texture 字节数，
    /// 不等于驱动实际显存分配（ADR-0014）。
    pub fn allocated_bytes(&self) -> Option<u64> {
        self.backend
            .as_ref()
            .and_then(ActiveBackend::allocated_bytes)
    }

    /// 当前渲染目标尺寸（随窗口缩放更新）。
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 窗口缩放时更新渲染目标尺寸，并重建交换链。
    ///
    /// 最小化（宽或高为 0）时只记录尺寸、保留旧交换链，恢复为非零尺寸后再重建。
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        if width == 0 || height == 0 {
            return;
        }
        if let Some(backend) = self.backend.as_mut()
            && let Err(error) = backend.resize(width, height)
        {
            self.frame_error = Some(error);
        }
    }

    /// 记录本帧清屏色（实际清屏在 `end_frame` 提交），并清空上一帧的绘制请求。
    pub fn begin_frame(&mut self, clear_color: [f32; 4]) {
        debug_assert!(self.initialized);
        self.clear_color = clear_color;
        self.draws.clear();
    }

    /// 本帧相机（view-projection）。
    pub fn set_camera(&mut self, camera: CameraFrame) {
        self.camera = camera;
    }

    /// 提交一次绘制：网格句柄 + 实例数据（同一网格可提交多次）。
    ///
    /// 使用渲染层内置默认材质（白色 Lambert）；带材质的提交见 [`RenderContext::submit_with_material`]。
    pub fn submit(&mut self, mesh: MeshHandle, instance: InstanceData) {
        self.draws.push(frame::DrawInstance {
            mesh,
            material: None,
            data: instance,
        });
    }

    /// 提交一次带材质的绘制：网格句柄 + 材质句柄 + 实例数据（R-11）。
    ///
    /// 材质句柄无效（已销毁/世代过期）时绘制回退到默认材质。
    pub fn submit_with_material(
        &mut self,
        mesh: MeshHandle,
        material: MaterialHandle,
        instance: InstanceData,
    ) {
        self.draws.push(frame::DrawInstance {
            mesh,
            material: Some(material),
            data: instance,
        });
    }

    /// 本帧收集到的绘制请求数量（测试/诊断）。
    pub fn submitted_instances(&self) -> usize {
        self.draws.len()
    }

    /// 创建材质：写入材质参数 UBO 并分配描述符（R-11）。
    ///
    /// headless 下同样可创建（不分配描述符，不会被绘制）。
    pub fn create_material(
        &mut self,
        data: &MaterialData,
    ) -> Result<MaterialHandle, RenderInitError> {
        let backend = self
            .backend
            .as_mut()
            .ok_or(RenderInitError::NotInitialized)?;
        backend
            .create_material(data)
            .map_err(RenderInitError::Material)
    }

    /// 销毁材质；句柄无效或已销毁时为空操作（对齐 `destroy_mesh` 语义）。
    pub fn destroy_material(&mut self, handle: MaterialHandle) {
        if let Some(backend) = self.backend.as_mut() {
            backend.destroy_material(handle);
        }
    }

    /// 创建 GPU 网格：需要先完成渲染初始化（headless 下也可上传，只是不会被绘制）。
    pub fn create_mesh(&mut self, data: &MeshData) -> Result<MeshHandle, RenderInitError> {
        let backend = self
            .backend
            .as_mut()
            .ok_or(RenderInitError::NotInitialized)?;
        backend.create_mesh(data).map_err(RenderInitError::Mesh)
    }

    /// 销毁网格；句柄无效或已销毁时为空操作。
    pub fn destroy_mesh(&mut self, handle: MeshHandle) {
        if let Some(backend) = self.backend.as_mut() {
            backend.destroy_mesh(handle);
        }
    }

    /// 创建 GPU 纹理（M5 批次 A）：mip 链图像 + 视图 + 采样器 + staging 上传。
    ///
    /// headless 下同样可创建（设备与分配器可用，只是不会被绘制引用）。
    /// 纹理槽位上限 128（`MAX_TEXTURES`），超限返回 `Texture` 错误。
    pub fn create_texture(&mut self, data: &TextureData) -> Result<TextureHandle, RenderInitError> {
        let backend = self
            .backend
            .as_mut()
            .ok_or(RenderInitError::NotInitialized)?;
        backend
            .create_texture(data)
            .map_err(RenderInitError::Texture)
    }

    /// 销毁纹理；句柄无效或已销毁时为空操作（对齐 `destroy_mesh` 语义）。
    pub fn destroy_texture(&mut self, handle: TextureHandle) {
        if let Some(backend) = self.backend.as_mut() {
            backend.destroy_texture(handle);
        }
    }

    /// 完成本帧：清屏 → 提交 → 呈现。headless 或窗口最小化时为空操作。
    pub fn end_frame(&mut self) {
        debug_assert!(self.initialized);
        // 最小化时窗口尺寸为 0：跳过 acquire/present，避免查询/提交到无效交换链。
        if self.width == 0 || self.height == 0 {
            return;
        }
        if let Some(backend) = self.backend.as_mut() {
            if let Err(error) = backend.render_frame(
                self.clear_color,
                self.width,
                self.height,
                self.camera,
                &self.draws,
            ) {
                self.frame_error = Some(error);
            } else {
                self.frame_error = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spike/回归：wgpu 经 naga 消费现有 Slang→SPIR-V 产物（ADR-0014）。
    #[test]
    fn pbr_spirv_is_accepted_by_naga_with_both_entry_points() {
        const SPIRV: &[u8] = include_bytes!("../../../shaders/pbr/pbr.spv");
        let module = ::wgpu::naga::front::spv::parse_u8_slice(SPIRV, &Default::default())
            .expect("naga 应能解析 pbr.spv");
        let entries: Vec<_> = module
            .entry_points
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert!(entries.contains(&"vertexMain"), "入口：{entries:?}");
        assert!(entries.contains(&"fragmentMain"), "入口：{entries:?}");
        ::wgpu::naga::valid::Validator::new(
            ::wgpu::naga::valid::ValidationFlags::all(),
            ::wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("naga 应能校验 pbr.spv");
    }

    #[test]
    fn new_context_is_uninitialized() {
        let context = RenderContext::new(1280, 720);
        assert!(!context.is_initialized());
        assert_eq!(context.status(), &RendererStatus::Uninitialized);
        assert_eq!(context.size(), (1280, 720));
        assert!(!context.exit_requested);
    }

    #[test]
    fn validation_setting_defaults_off_in_unit_tests_and_is_overridable() {
        let context = RenderContext::new(320, 200);
        assert_eq!(context.validation(), default_validation());
        assert!(!context.validation());

        let enabled = RenderContext::new(320, 200).with_validation(true);
        assert!(enabled.validation());
    }

    #[test]
    fn headless_init_never_panics() {
        let mut context = RenderContext::new(640, 480);
        context.init();
        assert!(context.is_initialized());
        assert!(matches!(
            context.status(),
            RendererStatus::Wgpu {
                has_surface: false,
                ..
            } | RendererStatus::Vulkan {
                has_surface: false,
                ..
            } | RendererStatus::Failed(_)
        ));
        context.begin_frame([0.0, 0.0, 0.0, 1.0]);
        context.end_frame();
    }

    #[test]
    fn backend_preference_defaults_to_auto_and_can_be_forced() {
        let context = RenderContext::new(64, 64);
        assert_eq!(context.preference, BackendPreference::Auto);
        assert_eq!(context.backend_kind(), None);

        let mut context = RenderContext::new(64, 64).with_backend(BackendPreference::Vulkan);
        context.init();
        if !matches!(context.status(), RendererStatus::Failed(_)) {
            assert_eq!(context.backend_kind(), Some(BackendKind::Vulkan));
        }
    }

    #[test]
    fn resize_updates_render_target() {
        let mut context = RenderContext::new(640, 480);
        context.resize(1024, 768);
        assert_eq!(context.size(), (1024, 768));
    }

    #[test]
    fn submit_collects_and_begin_frame_clears() {
        let mut context = RenderContext::new(320, 200);
        context.init();

        let mesh = crate::mesh::MeshHandle::from_parts(0, 0);
        context.begin_frame([0.0, 0.0, 0.0, 1.0]);
        assert_eq!(context.submitted_instances(), 0);

        context.submit(mesh, InstanceData::default());
        context.submit(mesh, InstanceData::default());
        assert_eq!(context.submitted_instances(), 2);

        context.begin_frame([0.0, 0.0, 0.0, 1.0]);
        assert_eq!(context.submitted_instances(), 0);
    }

    #[test]
    fn create_mesh_requires_initialized_renderer() {
        let mut context = RenderContext::new(320, 200);
        let error = context.create_mesh(&crate::cube()).unwrap_err();
        assert_eq!(error, RenderInitError::NotInitialized);
    }

    #[test]
    fn headless_mesh_upload_roundtrip_when_vulkan_available() {
        let mut context = RenderContext::new(320, 240);
        context.init();

        // 无 Vulkan 设备（CI/无 GPU）的环境直接跳过，保持测试可移植。
        if !matches!(
            context.status(),
            RendererStatus::Wgpu { .. } | RendererStatus::Vulkan { .. }
        ) {
            return;
        }

        let mesh = context.create_mesh(&crate::cube()).expect("上传立方体网格");
        context.destroy_mesh(mesh);

        // 旧句柄重复销毁不应 panic（世代号不匹配即忽略）。
        context.destroy_mesh(mesh);

        // 槽位复用后世代号应变化，新旧句柄不相等。
        let reused = context
            .create_mesh(&crate::cube())
            .expect("复用槽位上传网格");
        assert_ne!(mesh, reused);
        context.destroy_mesh(reused);

        // 无效数据应返回明确错误而不是上传。
        assert!(matches!(
            context.create_mesh(&MeshData::default()),
            Err(RenderInitError::Mesh(_))
        ));
    }

    #[test]
    fn create_texture_requires_initialized_renderer() {
        let mut context = RenderContext::new(320, 200);
        let data = TextureData::from_rgba8(4, 4, vec![0u8; 64]).expect("纹理数据");
        let error = context.create_texture(&data).unwrap_err();
        assert_eq!(error, RenderInitError::NotInitialized);
    }

    #[test]
    fn headless_texture_upload_roundtrip_when_vulkan_available() {
        let mut context = RenderContext::new(320, 240);
        context.init();

        // 无 Vulkan 设备（CI/无 GPU）的环境直接跳过，保持测试可移植。
        if !matches!(
            context.status(),
            RendererStatus::Wgpu { .. } | RendererStatus::Vulkan { .. }
        ) {
            return;
        }

        let baseline = context.allocated_bytes();
        let texture = context
            .create_texture(&test_texture(64, 64))
            .expect("上传多 mip 纹理");
        context.destroy_texture(texture);
        // 旧句柄重复销毁不应 panic（世代号不匹配即忽略）。
        context.destroy_texture(texture);

        // 槽位复用后世代号应变化，新旧句柄不相等。
        let reused = context
            .create_texture(&test_texture(4, 4))
            .expect("复用槽位上传纹理");
        assert_ne!(texture, reused);
        context.destroy_texture(reused);

        // 上传完成后分配器应无残留（M5 批次 A 泄漏守恒）。
        assert_eq!(context.allocated_bytes(), baseline);
    }

    /// 带校验层的纹理上传专项验证（R-03）：默认 ignored，CI 手动触发
    /// `cargo test -p mir-render texture_upload_validation -- --ignored --nocapture`。
    #[test]
    #[ignore = "需要 Vulkan 校验层环境，手动触发"]
    fn texture_upload_validation_layer_when_vulkan_available() {
        let mut context = RenderContext::new(320, 240).with_validation(true);
        context.init();
        if !matches!(
            context.status(),
            RendererStatus::Wgpu { .. } | RendererStatus::Vulkan { .. }
        ) {
            return;
        }

        let baseline = context.allocated_bytes();
        for size in [64, 33, 1] {
            let texture = context
                .create_texture(&test_texture(size, size))
                .expect("校验层纹理上传");
            context.destroy_texture(texture);
        }
        // 分配器守恒：全部销毁后无残留。
        assert_eq!(context.allocated_bytes(), baseline);
    }

    #[test]
    fn headless_texture_create_destroy_stress_when_vulkan_available() {
        let mut context = RenderContext::new(320, 240);
        context.init();

        // 无 Vulkan 设备（CI/无 GPU）的环境直接跳过，保持测试可移植。
        if !matches!(
            context.status(),
            RendererStatus::Wgpu { .. } | RendererStatus::Vulkan { .. }
        ) {
            return;
        }

        let data = test_texture(32, 32);
        // 上传前基线（headless 初始化本身会占用相机 UBO 等分配）。
        let baseline = context.allocated_bytes();
        for _ in 0..1000 {
            let handle = context.create_texture(&data).expect("压力循环创建纹理");
            // 门面销毁与 destroy_mesh 语义一致：句柄无效时为空操作。
            context.destroy_texture(handle);
        }
        assert_eq!(
            context.allocated_bytes(),
            baseline,
            "1000 次创建/销毁后无显存残留"
        );
    }

    /// 构造完整 mip 链的测试纹理（RGBA8，各层填充递增字节）。
    fn test_texture(width: u32, height: u32) -> TextureData {
        let mut bytes = Vec::new();
        let mut mips = Vec::new();
        let mut level = 0;
        loop {
            let w = crate::texture::mip_dimension(width, level);
            let h = crate::texture::mip_dimension(height, level);
            let size = (w * h * 4) as usize;
            let offset = bytes.len();
            bytes.extend(std::iter::repeat_n(level as u8 + 1, size));
            mips.push(MipRange {
                offset,
                length: size,
            });
            if w == 1 && h == 1 {
                break;
            }
            level += 1;
        }
        TextureData::new(ImageFormat::R8G8B8A8Unorm, width, height, mips, bytes)
            .expect("测试纹理数据合法")
    }

    #[test]
    fn minimized_window_skips_frame() {
        let mut context = RenderContext::new(640, 480);
        context.init();

        // 最小化：尺寸归零，跳帧，不应产生错误。
        context.resize(0, 0);
        assert_eq!(context.size(), (0, 0));
        context.begin_frame([0.0, 0.0, 0.0, 1.0]);
        context.end_frame();
        assert!(context.frame_error().is_none());

        // 恢复：重新按新尺寸重建并继续出帧。
        context.resize(800, 600);
        assert_eq!(context.size(), (800, 600));
        context.begin_frame([0.1, 0.1, 0.1, 1.0]);
        context.end_frame();
        assert!(context.frame_error().is_none());
    }

    /// 生成 n 个三角形的交错大小网格（R-10：交错大小分配）。
    fn sized_mesh(triangles: usize) -> MeshData {
        let mut vertices = Vec::with_capacity(triangles * 3);
        let mut indices = Vec::with_capacity(triangles * 3);
        for triangle in 0..triangles {
            let base = triangle as f32 * 0.01;
            let first = vertices.len() as u32;
            for corner in 0..3_u32 {
                vertices.push(Vertex::new(
                    [base + corner as f32 * 0.1, base, 0.0],
                    [0.2, 0.4, 0.8],
                    [0.0, 0.0, 1.0],
                ));
            }
            indices.extend_from_slice(&[first, first + 1, first + 2]);
        }
        MeshData::new(vertices, indices)
    }

    #[test]
    fn mesh_create_destroy_loop_conserves_allocations_when_gpu_available() {
        // R-10：1000 次 create/destroy 循环后分配计数必须守恒（回到 0）。
        let mut context = RenderContext::new(320, 240);
        context.init();
        // 无 Vulkan 设备（CI/无 GPU）的环境显式跳过，保持测试可移植。
        if !matches!(
            context.status(),
            RendererStatus::Wgpu { .. } | RendererStatus::Vulkan { .. }
        ) {
            return;
        }

        let baseline = context.allocated_bytes();
        let mesh = crate::cube();
        for _ in 0..1000 {
            let handle = context.create_mesh(&mesh).expect("上传网格");
            context.destroy_mesh(handle);
        }
        assert_eq!(context.allocated_bytes(), baseline, "循环后应回到基线");
    }

    #[test]
    fn interleaved_mesh_sizes_leave_no_residual_allocation_when_gpu_available() {
        // R-10：交错大小的网格反复创建/部分销毁，最终全部销毁后应回到 0。
        let mut context = RenderContext::new(320, 240);
        context.init();
        if !matches!(
            context.status(),
            RendererStatus::Wgpu { .. } | RendererStatus::Vulkan { .. }
        ) {
            return;
        }

        let baseline = context.allocated_bytes().expect("有设备时统计应存在");
        let mut handles = Vec::new();
        for triangles in [1, 64, 7, 256, 13, 128, 3] {
            handles.push(
                context
                    .create_mesh(&sized_mesh(triangles))
                    .expect("上传网格"),
            );
        }
        let allocated = context.allocated_bytes().expect("有设备时分配器统计应存在");
        assert!(allocated > baseline, "存活网格应占用显存统计");

        // 交错销毁一半（先取出另一半，避免 drain 丢句柄），统计应单调下降。
        let survivors: Vec<_> = handles
            .iter()
            .enumerate()
            .filter(|(position, _)| position % 2 == 1)
            .map(|(_, handle)| *handle)
            .collect();
        for handle in handles.drain(..).step_by(2) {
            context.destroy_mesh(handle);
        }
        let after_half = context.allocated_bytes().expect("统计应存在");
        assert!(after_half < allocated, "部分销毁后统计应下降");

        for handle in survivors {
            context.destroy_mesh(handle);
        }
        assert_eq!(
            context.allocated_bytes(),
            Some(baseline),
            "全部销毁后应回到基线"
        );
    }

    #[test]
    fn materials_submit_and_destroy_without_panic_when_gpu_available() {
        // R-11：headless 下多材质创建/提交/销毁不 panic，且分配守恒（Gate 6 验收路径）。
        let mut context = RenderContext::new(320, 240);
        context.init();
        if !matches!(
            context.status(),
            RendererStatus::Wgpu { .. } | RendererStatus::Vulkan { .. }
        ) {
            return;
        }

        let baseline = context.allocated_bytes();
        // 三种不同参数的材质（金属度/粗糙度对照，供 R-12 演示复用）。
        let gold = MaterialData::new([1.0, 0.77, 0.34, 1.0], 1.0, 0.25, [0.0; 3]);
        let plastic = MaterialData::new([0.8, 0.2, 0.2, 1.0], 0.0, 0.9, [0.0; 3]);
        let glowing = MaterialData::new([0.1, 0.1, 0.1, 1.0], 0.0, 0.5, [0.2, 0.15, 0.1]);
        let handles = [
            context.create_material(&gold).expect("创建金属材质"),
            context.create_material(&plastic).expect("创建塑料材质"),
            context.create_material(&glowing).expect("创建自发光材质"),
        ];
        assert_ne!(handles[0], handles[1]);

        let mesh = context.create_mesh(&crate::cube()).expect("上传立方体网格");
        context.begin_frame([0.0, 0.0, 0.0, 1.0]);
        context.set_camera(CameraFrame::default());
        for handle in handles {
            context.submit_with_material(mesh, handle, InstanceData::default());
        }
        context.submit(mesh, InstanceData::default()); // 默认材质路径
        assert_eq!(context.submitted_instances(), 4);
        // headless 下 end_frame 为空操作；窗口路径由验收（R-14）覆盖。
        context.end_frame();
        assert!(context.frame_error().is_none());

        // 旧句柄重复销毁为空操作；槽位复用后世代号递增。
        for handle in handles {
            context.destroy_material(handle);
        }
        context.destroy_material(handles[0]);
        let reused = context.create_material(&plastic).expect("复用槽位创建材质");
        assert_ne!(reused, handles[0], "世代号应使旧句柄失效");
        context.destroy_material(reused);
        context.destroy_mesh(mesh);
        assert_eq!(context.allocated_bytes(), baseline, "全部销毁后应回到基线");
    }
}
