//! 图形管线：着色器模块、描述符布局/池、管线布局与 push constants。

use vulkanalia::prelude::v1_0::*;
use vulkanalia::Device;

use crate::mesh::Vertex;

use super::VulkanCore;
use super::frame::{CAMERA_UBO_SIZE, MAX_FRAMES_IN_FLIGHT};
use super::material::MAX_MATERIALS;
use super::pipeline_cache::pipeline_state_hash;

pub(super) struct PipelineResources {
    /// 单 `.spv` 多入口模块（`vertexMain`/`fragmentMain`），两个 stage 共用。
    shader_module: vk::ShaderModule,
    /// set 0：相机 UBO（每帧一个集合）。
    descriptor_set_layout: vk::DescriptorSetLayout,
    /// set 1：材质参数 UBO（R-11，每材质一个集合）。
    pub(super) material_set_layout: vk::DescriptorSetLayout,
    pub(super) descriptor_pool: vk::DescriptorPool,
    /// 每帧一个相机 UBO 描述符集，按下标与 `VulkanCore::frames` 对齐。
    pub(super) descriptor_sets: Vec<vk::DescriptorSet>,
    pub(super) layout: vk::PipelineLayout,
    pub(super) pipeline: vk::Pipeline,
}

/// [`VulkanCore::init_pipeline`] 中途失败的回收守卫：任一步返回错误即按逆序销毁已创建对象；
/// 成功路径通过 [`PipelineGuard::defuse`] 移交所有权。
struct PipelineGuard<'a> {
    device: &'a Device,
    shader_module: Option<vk::ShaderModule>,
    descriptor_set_layout: Option<vk::DescriptorSetLayout>,
    material_set_layout: Option<vk::DescriptorSetLayout>,
    descriptor_pool: Option<vk::DescriptorPool>,
    pipeline_layout: Option<vk::PipelineLayout>,
    pipeline: Option<vk::Pipeline>,
}

impl PipelineGuard<'_> {
    /// 成功路径：移交全部所有权，此后 Drop 为空操作。
    #[allow(clippy::type_complexity)]
    fn defuse(
        mut self,
    ) -> (
        vk::ShaderModule,
        vk::DescriptorSetLayout,
        vk::DescriptorSetLayout,
        vk::DescriptorPool,
        vk::PipelineLayout,
        vk::Pipeline,
    ) {
        (
            self.shader_module.take().expect("着色器模块已创建"),
            self.descriptor_set_layout.take().expect("描述符布局已创建"),
            self.material_set_layout
                .take()
                .expect("材质描述符布局已创建"),
            self.descriptor_pool.take().expect("描述符池已创建"),
            self.pipeline_layout.take().expect("管线布局已创建"),
            self.pipeline.take().expect("图形管线已创建"),
        )
    }
}

impl Drop for PipelineGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: 各对象由本设备创建且尚未移交所有权；描述符集随池销毁一并释放。
        unsafe {
            if let Some(pipeline) = self.pipeline.take() {
                self.device.destroy_pipeline(pipeline, None);
            }
            if let Some(layout) = self.pipeline_layout.take() {
                self.device.destroy_pipeline_layout(layout, None);
            }
            if let Some(pool) = self.descriptor_pool.take() {
                self.device.destroy_descriptor_pool(pool, None);
            }
            if let Some(layout) = self.descriptor_set_layout.take() {
                self.device.destroy_descriptor_set_layout(layout, None);
            }
            if let Some(layout) = self.material_set_layout.take() {
                self.device.destroy_descriptor_set_layout(layout, None);
            }
            if let Some(module) = self.shader_module.take() {
                self.device.destroy_shader_module(module, None);
            }
        }
    }
}

/// 逐实例 push constants：`model` + `tint`，共 80 字节（低于 128 字节下限）。
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct PushConstants {
    pub(super) model: [f32; 16],
    pub(super) tint: [f32; 4],
}

pub(super) const PUSH_CONSTANT_SIZE: u32 = 80;

pub(super) fn push_constant_bytes(pc: &PushConstants) -> &[u8] {
    // SAFETY: `PushConstants` 为 repr(C) 且只含 f32，无填充，大小 80。
    unsafe {
        std::slice::from_raw_parts(
            (pc as *const PushConstants).cast::<u8>(),
            std::mem::size_of::<PushConstants>(),
        )
    }
}

impl VulkanCore {
    pub(super) fn init_pipeline(&mut self) -> Result<(), String> {
        // 单 .spv 多入口（R-06/R-12）：`pbr.slang` 经 `-fvk-use-entrypoint-name` 编译，
        // 同时包含 vertexMain/fragmentMain 两个入口，两个 stage 共用同一模块；
        // 材质 UBO（set 1）与法线输入自 R-12 起启用。
        const SHADER_CODE: &[u32] =
            vulkanalia::include_shader_code!("../../../../shaders/pbr/pbr.spv");

        // 管线缓存（R-07）：常驻 VkPipelineCache + 磁盘持久化；全部失败路径仅告警降级。
        let state_hash = pipeline_state_hash(SHADER_CODE);
        let (cache_key, cache_path) = self.pipeline_cache_key_and_path(state_hash);
        let pipeline_cache = self.ensure_pipeline_cache(cache_key, cache_path.as_deref());

        // 失败回收守卫：任一步出错即销毁已创建对象，成功时 defuse 移交所有权。
        let mut guard = PipelineGuard {
            device: &self.device,
            shader_module: None,
            descriptor_set_layout: None,
            material_set_layout: None,
            descriptor_pool: None,
            pipeline_layout: None,
            pipeline: None,
        };

        let module_info = vk::ShaderModuleCreateInfo::builder()
            .code_size(SHADER_CODE.len() * 4)
            .code(SHADER_CODE);
        // SAFETY: SHADER_CODE 是已校验的 SPIR-V 字节码，生命周期覆盖模块创建。
        let shader_module = unsafe { self.device.create_shader_module(&module_info, None) }
            .map_err(|error| format!("创建着色器模块失败: {error}"))?;
        guard.shader_module = Some(shader_module);

        let stages = [
            vk::PipelineShaderStageCreateInfo::builder()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(shader_module)
                .name(b"vertexMain\0")
                .build(),
            vk::PipelineShaderStageCreateInfo::builder()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(shader_module)
                .name(b"fragmentMain\0")
                .build(),
        ];

        // 顶点布局：位置 @0、颜色 @1、法线 @2（R-12），stride 36（与 `Vertex` 一致）。
        let vertex_binding = vk::VertexInputBindingDescription::builder()
            .binding(0)
            .stride(Vertex::SIZE as u32)
            .input_rate(vk::VertexInputRate::VERTEX)
            .build();
        let vertex_attributes = [
            vk::VertexInputAttributeDescription::builder()
                .location(0)
                .binding(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(0)
                .build(),
            vk::VertexInputAttributeDescription::builder()
                .location(1)
                .binding(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(12)
                .build(),
            vk::VertexInputAttributeDescription::builder()
                .location(2)
                .binding(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(24)
                .build(),
        ];
        let vertex_bindings = [vertex_binding];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::builder()
            .vertex_binding_descriptions(&vertex_bindings)
            .vertex_attribute_descriptions(&vertex_attributes)
            .build();

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::builder()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false)
            .build();
        let viewport_state = vk::PipelineViewportStateCreateInfo::builder()
            .viewport_count(1)
            .scissor_count(1)
            .build();
        // 模型空间绕序为"从外部看逆时针"；Vulkan NDC 为 Y 向下，投影后按顺时针判定正面。
        let rasterization = vk::PipelineRasterizationStateCreateInfo::builder()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::CLOCKWISE)
            .line_width(1.0)
            .build();
        let multisample = vk::PipelineMultisampleStateCreateInfo::builder()
            .rasterization_samples(vk::SampleCountFlags::_1)
            .build();
        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::builder()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS)
            .depth_bounds_test_enable(false)
            .stencil_test_enable(false)
            .build();
        let blend_attachment = vk::PipelineColorBlendAttachmentState::builder()
            .blend_enable(false)
            .color_write_mask(
                vk::ColorComponentFlags::R
                    | vk::ColorComponentFlags::G
                    | vk::ColorComponentFlags::B
                    | vk::ColorComponentFlags::A,
            )
            .build();
        let attachments = [blend_attachment];
        let color_blend = vk::PipelineColorBlendStateCreateInfo::builder()
            .attachments(&attachments)
            .build();
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state = vk::PipelineDynamicStateCreateInfo::builder()
            .dynamic_states(&dynamic_states)
            .build();

        // 描述符集 0 / binding 0：相机 UBO（片元阶段读 camera_position 做 PBR 高光）。
        let ubo_binding = vk::DescriptorSetLayoutBinding::builder()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT);
        let descriptor_bindings = [ubo_binding];
        let descriptor_layout_info =
            vk::DescriptorSetLayoutCreateInfo::builder().bindings(&descriptor_bindings);
        // SAFETY: 绑定描述有效。
        let descriptor_set_layout = unsafe {
            self.device
                .create_descriptor_set_layout(&descriptor_layout_info, None)
        }
        .map_err(|error| format!("创建描述符布局失败: {error}"))?;
        guard.descriptor_set_layout = Some(descriptor_set_layout);

        // 描述符集 1 / binding 0：材质参数 UBO（R-11），片元阶段读取。
        let material_binding = vk::DescriptorSetLayoutBinding::builder()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let material_bindings = [material_binding];
        let material_layout_info =
            vk::DescriptorSetLayoutCreateInfo::builder().bindings(&material_bindings);
        // SAFETY: 绑定描述有效。
        let material_set_layout = unsafe {
            self.device
                .create_descriptor_set_layout(&material_layout_info, None)
        }
        .map_err(|error| format!("创建材质描述符布局失败: {error}"))?;
        guard.material_set_layout = Some(material_set_layout);

        // 池容量：每帧 1 个相机集合 + 每材质 1 个材质集合。
        // 带 FREE_DESCRIPTOR_SET_BIT：材质销毁后描述符集经 DeletionQueue 归还复用。
        let pool_sizes = [vk::DescriptorPoolSize {
            type_: vk::DescriptorType::UNIFORM_BUFFER,
            descriptor_count: (MAX_FRAMES_IN_FLIGHT + MAX_MATERIALS) as u32,
        }];
        let pool_info = vk::DescriptorPoolCreateInfo::builder()
            .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
            .pool_sizes(&pool_sizes)
            .max_sets((MAX_FRAMES_IN_FLIGHT + MAX_MATERIALS) as u32);
        // SAFETY: 池大小与最大集合数有效。
        let descriptor_pool = unsafe { self.device.create_descriptor_pool(&pool_info, None) }
            .map_err(|error| format!("创建描述符池失败: {error}"))?;
        guard.descriptor_pool = Some(descriptor_pool);

        // 分配请求每帧一个集合；管线布局仍只声明 1 个 set。
        let allocate_layouts = [descriptor_set_layout; MAX_FRAMES_IN_FLIGHT];
        let allocate_info = vk::DescriptorSetAllocateInfo::builder()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&allocate_layouts);
        // SAFETY: 池与布局有效，请求每帧一个描述符集。
        let descriptor_sets = unsafe { self.device.allocate_descriptor_sets(&allocate_info) }
            .map_err(|error| format!("分配描述符集失败: {error}"))?;
        if descriptor_sets.len() != self.frames.len() {
            return Err(format!(
                "描述符集数量与帧数不一致: {} != {}",
                descriptor_sets.len(),
                self.frames.len()
            ));
        }

        // 相机 UBO 属于每帧资源（`frames`），这里只为各帧描述符集写入绑定。
        let buffer_infos: Vec<vk::DescriptorBufferInfo> = self
            .frames
            .iter()
            .map(|frame| {
                vk::DescriptorBufferInfo::builder()
                    .buffer(frame.camera_buffer)
                    .offset(0)
                    .range(CAMERA_UBO_SIZE)
                    .build()
            })
            .collect();
        // 必须命名中间数组：`WriteDescriptorSet` 保存的是指针，临时数组会在语句结束即失效
        // （debug 构建侥幸可用，release 优化后会读到已经被覆盖的栈内存）。
        let writes: Vec<vk::WriteDescriptorSet> = descriptor_sets
            .iter()
            .zip(&buffer_infos)
            .map(|(&set, info)| {
                vk::WriteDescriptorSet::builder()
                    .dst_set(set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(std::slice::from_ref(info))
                    .build()
            })
            .collect();
        // SAFETY: 描述符集与 buffer_infos（命名 Vec）在调用期间有效。
        unsafe {
            let no_copies: [vk::CopyDescriptorSet; 0] = [];
            self.device.update_descriptor_sets(&writes, &no_copies);
        }

        let push_range = vk::PushConstantRange::builder()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(PUSH_CONSTANT_SIZE)
            .build();
        let push_ranges = [push_range];
        let set_layouts = [descriptor_set_layout, material_set_layout];
        let layout_info = vk::PipelineLayoutCreateInfo::builder()
            .set_layouts(&set_layouts)
            .push_constant_ranges(&push_ranges);
        // SAFETY: layout_info 有效：2 个描述符集（相机 + 材质）+ 80 字节 push constants。
        let layout = unsafe { self.device.create_pipeline_layout(&layout_info, None) }
            .map_err(|error| format!("创建管线布局失败: {error}"))?;
        guard.pipeline_layout = Some(layout);

        let format = self
            .format_value
            .ok_or_else(|| "缺少 surface 格式".to_string())?;
        let formats = [format.format];
        let depth_format = self
            .depth_format
            .ok_or_else(|| "缺少深度格式".to_string())?;
        let mut rendering_info = vk::PipelineRenderingCreateInfo::builder()
            .color_attachment_formats(&formats)
            .depth_attachment_format(depth_format)
            .build();
        let create_info = vk::GraphicsPipelineCreateInfo::builder()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blend)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .push_next(&mut rendering_info)
            .build();
        // SAFETY: 所有状态结构与着色器模块均有效；使用动态渲染无需 render pass。
        // pipeline_cache 可能是 null（缓存创建失败时降级），对驱动同样合法。
        let pipelines = unsafe {
            self.device
                .create_graphics_pipelines(pipeline_cache, &[create_info], None)
        }
        .map_err(|error| format!("创建图形管线失败: {error:?}"))?
        .0;
        let pipeline = pipelines
            .into_iter()
            .next()
            .ok_or_else(|| "图形管线创建结果为空".to_string())?;
        guard.pipeline = Some(pipeline);

        // 全部对象创建成功：从守卫移交所有权。
        let (
            shader_module,
            descriptor_set_layout,
            material_set_layout,
            descriptor_pool,
            layout,
            pipeline,
        ) = guard.defuse();
        self.pipeline = Some(PipelineResources {
            shader_module,
            descriptor_set_layout,
            material_set_layout,
            descriptor_pool,
            descriptor_sets,
            layout,
            pipeline,
        });
        // 默认材质（白色 Lambert）：绘制路径的兜底集合。失败时管线本身已可用，
        // 只在首次绘制时才会缺 set 1——直接报错让调用方走整体回收更简单。
        self.create_default_material()?;
        // 管线创建成功后写回缓存（R-07）；IO 失败仅告警，不阻塞。
        if self.pipeline_cache.is_some() {
            self.save_pipeline_cache(cache_key, cache_path.as_deref());
        }
        Ok(())
    }

    pub(super) fn destroy_pipeline(&mut self) {
        // 默认材质：缓冲随本处销毁（描述符集随下方描述符池销毁一并释放）。
        if let Some(material) = self.default_material.take() {
            self.release_material(material);
        }
        if let Some(resources) = self.pipeline.take() {
            // SAFETY: 这些对象由本对象创建且尚未销毁；调用方保证 GPU 已空闲。
            // 相机 UBO 属于每帧资源，随帧资源销毁，不在这里释放。
            unsafe {
                self.device.destroy_pipeline(resources.pipeline, None);
                self.device.destroy_pipeline_layout(resources.layout, None);
                self.device
                    .destroy_descriptor_pool(resources.descriptor_pool, None);
                self.device
                    .destroy_descriptor_set_layout(resources.descriptor_set_layout, None);
                self.device
                    .destroy_descriptor_set_layout(resources.material_set_layout, None);
                self.device
                    .destroy_shader_module(resources.shader_module, None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_constant_layout_is_80_bytes() {
        assert_eq!(
            std::mem::size_of::<PushConstants>(),
            PUSH_CONSTANT_SIZE as usize
        );
        assert_eq!(std::mem::align_of::<PushConstants>(), 4);
    }
}
