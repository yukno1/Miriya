//! RenderGraph 声明、DOT dump、barrier 翻译与 GPU pass 计时回读（R-03/R-04/R-05）。

use vulkanalia::Device;
use vulkanalia::prelude::v1_0::*;
use vulkanalia::vk::DeviceV1_3;

use crate::frame::DrawInstance;
use crate::graph;
use crate::graph::GRAPH_DUMP_PATH;

use super::VulkanCore;

impl VulkanCore {
    /// 声明本帧的 RenderGraph（R-03）：复用共享的纯数据声明，额外返回
    /// resource → (image, aspect) 的翻译表，供 barrier 生成时定位 `vk::Image`。
    pub(super) fn build_frame_graph(
        &self,
        draws: &[DrawInstance],
        image_index: u32,
    ) -> (
        graph::RenderGraph,
        Vec<(graph::ResourceId, vk::Image, vk::ImageAspectFlags)>,
    ) {
        let frame_graph = graph::build_frame_graph(draws);
        // 声明顺序固定：swapchain = 0，depth = 1（见 `graph::build_frame_graph`）。
        let mut images = Vec::with_capacity(2);
        images.push((
            graph::ResourceId(0),
            self.swapchain_images[image_index as usize],
            vk::ImageAspectFlags::COLOR,
        ));
        if let Some(depth_resources) = &self.depth {
            images.push((
                graph::ResourceId(1),
                depth_resources.image,
                vk::ImageAspectFlags::DEPTH,
            ));
        }
        (frame_graph, images)
    }

    /// 每帧把当前图与编译产物 dump 成 DOT 文件（R-05，`--graph-dump` 开启时）。
    /// 覆盖写同一文件（各帧 barrier 计划一致）；IO 失败仅告警一次，不影响渲染。
    pub(super) fn write_graph_dump(
        &mut self,
        frame_graph: &graph::RenderGraph,
        compiled: &graph::CompiledGraph,
    ) {
        let dot = frame_graph.dump_dot(compiled);
        if let Err(error) = std::fs::write(GRAPH_DUMP_PATH, dot)
            && !self.dump_warned
        {
            mir_log::warn!(
                target: "vulkan",
                "RenderGraph dump 写入 {} 失败: {error}",
                GRAPH_DUMP_PATH
            );
            self.dump_warned = true;
        }
    }
}

/// 把编译器产出的 barrier 翻译成 synchronization2 的图像 barrier 并录入命令缓冲（R-04）。
pub(super) fn emit_image_barrier(
    device: &Device,
    command_buffer: vk::CommandBuffer,
    barrier: &graph::Barrier,
    graph_images: &[(graph::ResourceId, vk::Image, vk::ImageAspectFlags)],
    memory_barriers: &[vk::MemoryBarrier2],
    buffer_barriers: &[vk::BufferMemoryBarrier2],
) {
    let Some((_, image, aspect)) = graph_images
        .iter()
        .find(|(resource, _, _)| *resource == barrier.resource)
    else {
        mir_log::warn!(target: "vulkan", "RenderGraph barrier 引用了未导入的图像资源，已跳过");
        return;
    };
    let subresource = vk::ImageSubresourceRange::builder()
        .aspect_mask(*aspect)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1)
        .build();
    // 每 pass 前置 barrier 独立成组：src/dst 阶段由编译器按资源最近一次写入推导。
    let image_barrier = vk::ImageMemoryBarrier2::builder()
        .src_stage_mask(map_stages(barrier.src_stage))
        .dst_stage_mask(map_stages(barrier.dst_stage))
        .src_access_mask(map_access(barrier.src_access))
        .dst_access_mask(map_access(barrier.dst_access))
        .old_layout(map_layout(barrier.src_layout))
        .new_layout(map_layout(barrier.dst_layout))
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(*image)
        .subresource_range(subresource)
        .build();
    let image_barriers = [image_barrier];
    let dependency_info = vk::DependencyInfo::builder()
        .memory_barriers(memory_barriers)
        .buffer_memory_barriers(buffer_barriers)
        .image_memory_barriers(&image_barriers);
    // SAFETY: command_buffer 处于录制中；图像有效且布局由编译器逐帧追踪。
    unsafe { device.cmd_pipeline_barrier2(command_buffer, &dependency_info) };
}

/// 回读上一轮该帧的 pass 时间戳并换算为毫秒（R-05）。
/// 调用前提：本帧 fence 已等待（上一轮提交完成）；查询池已按 pass 数写满。
pub(super) fn read_pass_timings(
    device: &Device,
    pool: vk::QueryPool,
    frame_graph: &graph::RenderGraph,
    compiled: &graph::CompiledGraph,
    period_ns: f32,
) -> Vec<(String, f32)> {
    let pass_count = compiled.pass_order.len();
    if pass_count == 0 {
        return Vec::new();
    }
    let query_count = 2 * pass_count;
    // vulkanalia 以字节切片暴露结果缓冲：u64 × 查询数，stride 8。
    let mut bytes = vec![0u8; 8 * query_count];
    let flags = vk::QueryResultFlags::_64 | vk::QueryResultFlags::WAIT;
    // SAFETY: pool 有效且调用方已通过 fence 确认上一轮提交完成；WAIT 阻塞至结果可用。
    let read =
        unsafe { device.get_query_pool_results(pool, 0, query_count as u32, &mut bytes, 8, flags) };
    if let Err(error) = read {
        mir_log::warn!(target: "vulkan", "读取 GPU pass 时间戳失败: {error}");
        return Vec::new();
    }
    // 时间戳为 64 位整型，小端序（x86/ARM 均为本机小端）。
    let results: Vec<u64> = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| u64::from_le_bytes(*chunk))
        .collect();
    compiled
        .pass_order
        .iter()
        .enumerate()
        .map(|(position, pass_id)| {
            let name = frame_graph.passes()[pass_id.0].name.clone();
            let nanos = (results[2 * position + 1].saturating_sub(results[2 * position])) as f64
                * period_ns as f64;
            (name, (nanos / 1_000_000.0) as f32)
        })
        .collect()
}

/// 纯数据布局 → Vulkan 布局。
fn map_layout(layout: graph::ImageLayout) -> vk::ImageLayout {
    match layout {
        graph::ImageLayout::Undefined => vk::ImageLayout::UNDEFINED,
        graph::ImageLayout::ColorAttachment => vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        graph::ImageLayout::DepthAttachment => vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL,
        graph::ImageLayout::Present => vk::ImageLayout::PRESENT_SRC_KHR,
    }
}

/// 纯数据阶段位集 → Vulkan PipelineStageFlags2。
fn map_stages(stages: graph::Stages) -> vk::PipelineStageFlags2 {
    let mut flags = vk::PipelineStageFlags2::empty();
    if stages.contains(graph::Stages::TOP_OF_PIPE) {
        flags |= vk::PipelineStageFlags2::TOP_OF_PIPE;
    }
    if stages.contains(graph::Stages::EARLY_FRAGMENT_TESTS) {
        flags |= vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS;
    }
    if stages.contains(graph::Stages::LATE_FRAGMENT_TESTS) {
        flags |= vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS;
    }
    if stages.contains(graph::Stages::COLOR_ATTACHMENT_OUTPUT) {
        flags |= vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT;
    }
    if stages.contains(graph::Stages::BOTTOM_OF_PIPE) {
        flags |= vk::PipelineStageFlags2::BOTTOM_OF_PIPE;
    }
    if stages.contains(graph::Stages::TRANSFER) {
        // VK_PIPELINE_STAGE_2_TRANSFER_BIT 在 vulkanalia 中命名为 ALL_TRANSFER。
        flags |= vk::PipelineStageFlags2::ALL_TRANSFER;
    }
    if stages.contains(graph::Stages::VERTEX_INPUT) {
        flags |= vk::PipelineStageFlags2::VERTEX_INPUT;
    }
    flags
}

/// 纯数据访问位集 → Vulkan AccessFlags2。
fn map_access(access: graph::Access) -> vk::AccessFlags2 {
    let mut flags = vk::AccessFlags2::empty();
    if access.contains(graph::Access::COLOR_ATTACHMENT_WRITE) {
        flags |= vk::AccessFlags2::COLOR_ATTACHMENT_WRITE;
    }
    if access.contains(graph::Access::DEPTH_STENCIL_ATTACHMENT_WRITE) {
        flags |= vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE;
    }
    if access.contains(graph::Access::SHADER_READ) {
        flags |= vk::AccessFlags2::SHADER_READ;
    }
    if access.contains(graph::Access::TRANSFER_WRITE) {
        flags |= vk::AccessFlags2::TRANSFER_WRITE;
    }
    if access.contains(graph::Access::VERTEX_ATTRIBUTE_READ) {
        flags |= vk::AccessFlags2::VERTEX_ATTRIBUTE_READ;
    }
    flags
}
