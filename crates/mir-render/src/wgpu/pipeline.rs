//! PBR 图形管线：复用 Slang 编译的 SPIR-V，绑定布局与 Vulkan 侧一一对应。

use wgpu::{
    BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType,
    BufferBindingType, ColorTargetState, ColorWrites, CompareFunction, DepthBiasState,
    DepthStencilState, Face, FragmentState, FrontFace, MultisampleState, PipelineLayout,
    PipelineLayoutDescriptor, PrimitiveState, RenderPipeline, RenderPipelineDescriptor,
    ShaderStages, StencilState, TextureFormat, VertexAttribute, VertexBufferLayout, VertexFormat,
    VertexState, VertexStepMode,
};

use crate::frame::PUSH_CONSTANT_SIZE;
use crate::mesh::Vertex;

use super::WgpuCore;

/// 管线与描述符布局；相机/材质 bind group 由 frame.rs / resources.rs 创建。
pub(crate) struct PipelineResources {
    pub(crate) camera_bgl: BindGroupLayout,
    pub(crate) material_bgl: BindGroupLayout,
    #[allow(dead_code)] // 由 bind group 持有；保留布局以免提前释放。
    pub(crate) layout: PipelineLayout,
    pub(crate) pipeline: RenderPipeline,
    pub(crate) format: TextureFormat,
}

impl WgpuCore {
    pub(crate) fn init_pipeline(&mut self) -> Result<(), String> {
        let module = self
            .device
            .create_shader_module(wgpu::include_spirv!("../../../../shaders/pbr/pbr.spv"));

        let camera_bgl = self
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("wgpu camera bgl"),
                entries: &[BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let material_bgl = self
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("wgpu material bgl"),
                entries: &[BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let layout = self
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("wgpu pbr layout"),
                bind_group_layouts: &[Some(&camera_bgl), Some(&material_bgl)],
                immediate_size: PUSH_CONSTANT_SIZE,
            });

        // 位置 @0 / 颜色 @1 / 法线 @2，stride 36，与 `Vertex` 和 pbr.slang 一致。
        let vertex_buffers = [Some(VertexBufferLayout {
            array_stride: Vertex::SIZE as u64,
            step_mode: VertexStepMode::Vertex,
            attributes: &[
                VertexAttribute {
                    format: VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                VertexAttribute {
                    format: VertexFormat::Float32x3,
                    offset: 12,
                    shader_location: 1,
                },
                VertexAttribute {
                    format: VertexFormat::Float32x3,
                    offset: 24,
                    shader_location: 2,
                },
            ],
        })];
        let format = self
            .config
            .as_ref()
            .map(|config| config.format)
            .unwrap_or(TextureFormat::Bgra8Unorm);
        let targets = [Some(ColorTargetState {
            format,
            blend: None,
            write_mask: ColorWrites::ALL,
        })];
        let pipeline = self
            .device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("wgpu pbr"),
                layout: Some(&layout),
                vertex: VertexState {
                    module: &module,
                    entry_point: Some("vertexMain"),
                    compilation_options: Default::default(),
                    buffers: &vertex_buffers,
                },
                fragment: Some(FragmentState {
                    module: &module,
                    entry_point: Some("fragmentMain"),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                // 模型空间逆时针绕序 + Vulkan 投影的 y 翻转：framebuffer 中为 CW。
                primitive: PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: FrontFace::Cw,
                    cull_mode: Some(Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(DepthStencilState {
                    format: TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(CompareFunction::Less),
                    stencil: StencilState::default(),
                    bias: DepthBiasState::default(),
                }),
                multisample: MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        self.pipeline = Some(PipelineResources {
            camera_bgl,
            material_bgl,
            layout,
            pipeline,
            format,
        });
        // 默认材质（白色）随管线创建，供无效/缺失材质句柄回退。
        self.create_default_material()?;
        Ok(())
    }
}
