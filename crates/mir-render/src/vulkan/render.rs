//! 每帧录制、提交与呈现：RenderGraph 编译产物驱动的逐 pass 录制。

use vulkanalia::prelude::v1_0::*;
use vulkanalia::vk::{DeviceV1_3, KhrSwapchainExtensionDeviceCommands};

use crate::frame::{
    CAMERA_UBO_SIZE, CameraFrame, DrawInstance, MAX_FRAMES_IN_FLIGHT, PushConstants,
    push_constant_bytes,
};
use crate::graph;

use super::VulkanCore;
use super::frame::TIMESTAMP_QUERY_COUNT;
use super::frame_graph::{emit_image_barrier, read_pass_timings};

impl VulkanCore {
    /// 当前交换链尺寸是否与应用层已知的窗口尺寸不一致。
    fn target_differs(&self, width: u32, height: u32) -> bool {
        self.extent.width != width || self.extent.height != height
    }

    /// 录制一帧：清屏 → 逐实例绘制 → 提交 → 呈现。无交换链时为空操作（headless）。
    ///
    /// 交换链失效（`OUT_OF_DATE`）不视为帧错误：标记后本帧跳过，下一帧开头按当前窗口尺寸重建。
    /// `width` / `height` 是应用层已知的窗口尺寸，用于重建时取最新目标大小；
    /// `camera` 是本帧相机（view_proj + 世界位置），`draws` 是应用层提交的实例列表。
    ///
    /// 帧资源按 `frame_index` 环形复用（[`MAX_FRAMES_IN_FLIGHT`] 帧在飞行）：
    /// 复用前等待本帧 fence，保证上一轮该帧的 GPU 工作（含 UBO 读取）已完成。
    pub(crate) fn render_frame(
        &mut self,
        clear_color: [f32; 4],
        width: u32,
        height: u32,
        camera: CameraFrame,
        draws: &[DrawInstance],
    ) -> Result<(), String> {
        if self.surface.is_none() {
            return Ok(());
        }
        if self.swapchain_outdated {
            self.swapchain_outdated = false;
            self.recreate_swapchain(width.max(1), height.max(1))?;
        }

        let swapchain = match self.swapchain {
            Some(swapchain) => swapchain,
            None => return Ok(()),
        };
        // 交换链尺寸与窗口尺寸是否不一致；先算好，避免持有 `frame` 可变借用时再借 `self`。
        let target_differs = self.target_differs(width, height);
        if self.frames.len() != MAX_FRAMES_IN_FLIGHT {
            return Ok(());
        }
        let frame_index = self.frame_index;

        // SAFETY: fence/semaphore/命令缓冲均有效且由本对象独占；等待上一轮该帧完成后再复用。
        unsafe {
            let frame_fence = self.frames[frame_index].fence;
            self.device
                .wait_for_fences(&[frame_fence], true, u64::MAX)
                .map_err(|error| format!("等待 GPU 完成失败: {error}"))?;
            // 延迟回收（R-09）：本帧 fence 已信号，绑定它的销毁请求现在安全。
            self.flush_deletions(Some(frame_fence));
            let frame = &mut self.frames[frame_index];
            let (index, suboptimal) = match self.device.acquire_next_image_khr(
                swapchain,
                u64::MAX,
                frame.image_available,
                vk::Fence::null(),
            ) {
                Ok(acquired) => acquired,
                Err(vk::ErrorCode::OUT_OF_DATE_KHR) => {
                    // 提前返回时不能重置 fence，否则下一帧会永久等待一个未信号的 fence。
                    self.swapchain_outdated = true;
                    return Ok(());
                }
                Err(error) => return Err(format!("获取交换链图像失败: {error}")),
            };
            self.device
                .reset_fences(&[frame.fence])
                .map_err(|error| format!("重置 fence 失败: {error}"))?;
            if suboptimal == vk::SuccessCode::SUBOPTIMAL_KHR && target_differs {
                self.swapchain_outdated = true;
            }

            let begin_info = vk::CommandBufferBeginInfo::builder()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device
                .begin_command_buffer(frame.command_buffer, &begin_info)
                .map_err(|error| format!("开始命令缓冲录制失败: {error}"))?;

            // —— 图声明与编译（R-03/R-04）：不持有帧资源借用。 ——
            // 每帧声明 RenderGraph：导入交换链图像（UNDEFINED → PRESENT）、深度图像
            // （创建时已一次性转入 DEPTH_ATTACHMENT_OPTIMAL，静态布局）与本帧绘制
            // 引用的网格 buffer；单个 graphics pass。编译产物驱动录制与 barrier。
            let (frame_graph, graph_images) = self.build_frame_graph(draws, index);
            let compiled = frame_graph
                .compile()
                .map_err(|error| format!("RenderGraph 编译失败: {error}"))?;
            if self.graph_dump {
                self.write_graph_dump(&frame_graph, &compiled);
            }

            // —— GPU 时间戳（R-05）：fence 等待已保证上一轮该帧的查询完成，先回读再重录。 ——
            let pass_count = compiled.pass_order.len();
            let timestamp_pool = self.frames[frame_index].timestamp_pool;
            let timed = timestamp_pool.is_some()
                && self.timestamp_valid_bits > 0
                && 2 * pass_count <= TIMESTAMP_QUERY_COUNT as usize;
            if timed && self.frames[frame_index].timings_valid {
                self.last_pass_timings = read_pass_timings(
                    &self.device,
                    timestamp_pool.expect("时间戳查询池存在"),
                    &frame_graph,
                    &compiled,
                    self.timestamp_period,
                );
            }

            // 空切片需要显式类型，否则无法推导 `Cast` 目标。
            let no_memory_barriers: [vk::MemoryBarrier2; 0] = [];
            let no_buffer_barriers: [vk::BufferMemoryBarrier2; 0] = [];

            if timed {
                // SAFETY（外层 unsafe）：查询池属于本帧资源；复位发生在本轮录制开头，
                // 上一轮结果已回读完毕。
                self.device.cmd_reset_query_pool(
                    self.frames[frame_index].command_buffer,
                    timestamp_pool.expect("时间戳查询池存在"),
                    0,
                    TIMESTAMP_QUERY_COUNT,
                );
            }

            // —— 图编译产物驱动的逐 pass 录制（R-04）：barrier/layout 全部由编译器生成， ——
            // 替换 v0.2 的 acquire 后 / 呈现前两处手写 barrier；深度一次性转换并入
            // imported 资源初始化语义（初始布局即 DEPTH_ATTACHMENT_OPTIMAL，无每帧 barrier）。
            for (position, pass_id) in compiled.pass_order.iter().enumerate() {
                let pass = &frame_graph.passes()[pass_id.0];
                let command_buffer = self.frames[frame_index].command_buffer;
                for barrier in &compiled.pre_pass_barriers[position] {
                    emit_image_barrier(
                        &self.device,
                        command_buffer,
                        barrier,
                        &graph_images,
                        &no_memory_barriers,
                        &no_buffer_barriers,
                    );
                }
                if timed {
                    let pool = timestamp_pool.expect("时间戳查询池存在");
                    // SAFETY（外层 unsafe）：查询池已复位、下标在容量内；录制中写入，
                    // stage 为合法管线阶段。
                    self.device.cmd_write_timestamp2(
                        command_buffer,
                        vk::PipelineStageFlags2::TOP_OF_PIPE,
                        pool,
                        (2 * position) as u32,
                    );
                }
                match &pass.kind {
                    graph::PassKind::Graphics { .. } => {
                        let frame = &mut self.frames[frame_index];
                        let clear = vk::ClearValue {
                            color: vk::ClearColorValue {
                                float32: clear_color,
                            },
                        };
                        let attachment = vk::RenderingAttachmentInfo::builder()
                            .image_view(self.image_views[index as usize])
                            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                            .load_op(vk::AttachmentLoadOp::CLEAR)
                            .store_op(vk::AttachmentStoreOp::STORE)
                            .clear_value(clear);
                        let render_area = vk::Rect2D {
                            offset: vk::Offset2D { x: 0, y: 0 },
                            extent: frame.extent,
                        };
                        let depth = self
                            .depth
                            .as_ref()
                            .ok_or_else(|| "深度资源未初始化".to_string())?;
                        let depth_attachment = vk::RenderingAttachmentInfo::builder()
                            .image_view(depth.view)
                            .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                            .load_op(vk::AttachmentLoadOp::CLEAR)
                            .store_op(vk::AttachmentStoreOp::DONT_CARE)
                            .clear_value(vk::ClearValue {
                                depth_stencil: vk::ClearDepthStencilValue {
                                    depth: 1.0,
                                    stencil: 0,
                                },
                            })
                            .build();
                        let attachments = [attachment];
                        let rendering_info = vk::RenderingInfo::builder()
                            .render_area(render_area)
                            .layer_count(1)
                            .color_attachments(&attachments)
                            .depth_attachment(&depth_attachment);
                        self.device
                            .cmd_begin_rendering(frame.command_buffer, &rendering_info);
                        if let Some(resources) = &self.pipeline {
                            let viewport = vk::Viewport {
                                x: 0.0,
                                y: 0.0,
                                width: frame.extent.width as f32,
                                height: frame.extent.height as f32,
                                min_depth: 0.0,
                                max_depth: 1.0,
                            };
                            let viewports = [viewport];
                            let scissors = [render_area];
                            self.device
                                .cmd_set_viewport(frame.command_buffer, 0, &viewports);
                            self.device
                                .cmd_set_scissor(frame.command_buffer, 0, &scissors);
                            self.device.cmd_bind_pipeline(
                                frame.command_buffer,
                                vk::PipelineBindPoint::GRAPHICS,
                                resources.pipeline,
                            );

                            // 相机 UBO：写入当前帧的持久映射。
                            // 本帧 fence 等待已保证上一轮该帧的 GPU 读取结束，HOST_COHERENT 无需 flush。
                            // std140 布局：view_proj（64 B）+ camera_position（16 B，含 vec3 补齐）。
                            let camera_matrix = camera.view_proj.to_cols_array();
                            let mut camera_position = [0.0_f32; 4];
                            camera_position[..3].copy_from_slice(&camera.camera_position);
                            let mut camera_ubo = [0.0_f32; 20];
                            camera_ubo[..16].copy_from_slice(&camera_matrix);
                            camera_ubo[16..].copy_from_slice(&camera_position);
                            // SAFETY: camera_mapped 是创建时 map_memory 得到的持久映射指针，
                            // 有效范围 CAMERA_UBO_SIZE 字节，仅由本对象独占访问。
                            std::ptr::copy_nonoverlapping(
                                camera_ubo.as_ptr().cast::<u8>(),
                                frame.camera_mapped,
                                CAMERA_UBO_SIZE as usize,
                            );

                            let descriptor_set = resources.descriptor_sets[frame_index];
                            self.device.cmd_bind_descriptor_sets(
                                frame.command_buffer,
                                vk::PipelineBindPoint::GRAPHICS,
                                resources.layout,
                                0,
                                &[descriptor_set],
                                &[],
                            );

                            // set 1（材质）：按需换绑——材质句柄无效或未提供时回退默认材质。
                            let mut bound_material_set = vk::DescriptorSet::null();
                            for draw in draws {
                                let Some(mesh) =
                                    self.meshes.get(draw.mesh.index(), draw.mesh.generation())
                                else {
                                    continue;
                                };
                                let material_set = match draw.material.and_then(|handle| {
                                    self.materials.get(handle.index(), handle.generation())
                                }) {
                                    Some(material) => material.descriptor_set,
                                    None => {
                                        self.default_material
                                            .as_ref()
                                            .expect("默认材质随管线初始化创建")
                                            .descriptor_set
                                    }
                                };
                                if material_set != bound_material_set {
                                    self.device.cmd_bind_descriptor_sets(
                                        frame.command_buffer,
                                        vk::PipelineBindPoint::GRAPHICS,
                                        resources.layout,
                                        1,
                                        &[material_set],
                                        &[],
                                    );
                                    bound_material_set = material_set;
                                }
                                let push = PushConstants {
                                    model: draw.data.model.to_cols_array(),
                                    tint: draw.data.tint,
                                };
                                self.device.cmd_push_constants(
                                    frame.command_buffer,
                                    resources.layout,
                                    vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                                    0,
                                    push_constant_bytes(&push),
                                );
                                self.device.cmd_bind_vertex_buffers(
                                    frame.command_buffer,
                                    0,
                                    &[mesh.vertex_buffer],
                                    &[0],
                                );
                                self.device.cmd_bind_index_buffer(
                                    frame.command_buffer,
                                    mesh.index_buffer,
                                    0,
                                    vk::IndexType::UINT32,
                                );
                                self.device.cmd_draw_indexed(
                                    frame.command_buffer,
                                    mesh.index_count,
                                    1,
                                    0,
                                    0,
                                    0,
                                );
                            }
                            self.device.cmd_end_rendering(frame.command_buffer);
                        }
                    }
                    // 每帧图内不含 transfer pass；网格上传走独立阻塞路径（保留 staging barrier）。
                    graph::PassKind::Transfer { .. } => {}
                }
                if timed {
                    let pool = timestamp_pool.expect("时间戳查询池存在");
                    // SAFETY（外层 unsafe）：查询池已复位、下标在容量内；pass 结束处写入。
                    self.device.cmd_write_timestamp2(
                        command_buffer,
                        vk::PipelineStageFlags2::BOTTOM_OF_PIPE,
                        pool,
                        (2 * position + 1) as u32,
                    );
                }
            }
            // 帧末 barrier：编译器按 imported 资源的 final 布局生成（交换链 → PRESENT_SRC_KHR）。
            let command_buffer = self.frames[frame_index].command_buffer;
            for barrier in &compiled.final_barriers {
                emit_image_barrier(
                    &self.device,
                    command_buffer,
                    barrier,
                    &graph_images,
                    &no_memory_barriers,
                    &no_buffer_barriers,
                );
            }
            if timed {
                self.frames[frame_index].timings_valid = true;
            }

            self.device
                .end_command_buffer(self.frames[frame_index].command_buffer)
                .map_err(|error| format!("结束命令缓冲录制失败: {error}"))?;

            let frame = &mut self.frames[frame_index];
            let wait_semaphores = [frame.image_available];
            let stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
            let command_buffers = [frame.command_buffer];
            // 本帧与本张图像专属的呈现信号量，避免在图像未重新 acquire 前复用。
            let render_finished = frame.render_finished[index as usize];
            let signal_semaphores = [render_finished];
            let submit = vk::SubmitInfo::builder()
                .wait_semaphores(&wait_semaphores)
                .wait_dst_stage_mask(&stages)
                .command_buffers(&command_buffers)
                .signal_semaphores(&signal_semaphores);
            let submits = [submit];
            self.device
                .queue_submit(self.graphics_queue, &submits, frame.fence)
                .map_err(|error| format!("提交命令缓冲失败: {error}"))?;

            let present_wait = [render_finished];
            let swapchains = [swapchain];
            let image_indices = [index];
            let present = vk::PresentInfoKHR::builder()
                .wait_semaphores(&present_wait)
                .swapchains(&swapchains)
                .image_indices(&image_indices);
            match self.device.queue_present_khr(self.present_queue, &present) {
                Ok(suboptimal) => {
                    if suboptimal == vk::SuccessCode::SUBOPTIMAL_KHR && target_differs {
                        self.swapchain_outdated = true;
                    }
                }
                Err(vk::ErrorCode::OUT_OF_DATE_KHR) => {
                    // 呈现失败但提交已完成；下一帧重建交换链后重新呈现新帧。
                    self.swapchain_outdated = true;
                }
                Err(error) => return Err(format!("呈现失败: {error}")),
            }
        }
        // 本帧提交完成，推进到下一帧资源；acquire 提前返回的路径不推进，
        // 下次仍从同一帧资源开始（其 fence 尚未重置，状态一致）。
        self.frame_index = (frame_index + 1) % MAX_FRAMES_IN_FLIGHT;
        Ok(())
    }
}
