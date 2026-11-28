//! 每帧录制与呈现：RenderGraph 负责 pass 执行序/DOT，wgpu 负责同步与资源状态。

use wgpu::{
    Color, CommandEncoderDescriptor, CurrentSurfaceTexture, IndexFormat, LoadOp, Operations,
    RenderPassColorAttachment, RenderPassDepthStencilAttachment, RenderPassDescriptor, StoreOp,
    TextureViewDescriptor,
};

use crate::frame::{
    CameraFrame, DrawInstance, MAX_FRAMES_IN_FLIGHT, PushConstants, push_constant_bytes,
};
use crate::graph::{self, GRAPH_DUMP_PATH};

use super::WgpuCore;

impl WgpuCore {
    /// 录制一帧：获取交换链纹理 → 按 RenderGraph 的 pass 序绘制 → 提交并呈现。
    /// headless 或窗口被遮挡时为空操作。
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
        if self
            .config
            .as_ref()
            .is_some_and(|config| config.width != width || config.height != height)
        {
            self.resize(width, height)?;
        }

        let frame_graph = graph::build_frame_graph(draws);
        let compiled = frame_graph
            .compile()
            .map_err(|error| format!("RenderGraph 编译失败：{error}"))?;
        if self.graph_dump {
            self.write_graph_dump(&frame_graph, &compiled);
        }

        // surface 借用只到 acquire 结束；Outdated 分支需要 &mut self 重建配置。
        let acquisition = self
            .surface
            .as_ref()
            .expect("surface 存在")
            .get_current_texture();
        let surface_texture = match acquisition {
            CurrentSurfaceTexture::Success(texture)
            | CurrentSurfaceTexture::Suboptimal(texture) => texture,
            CurrentSurfaceTexture::Outdated => {
                self.resize(width, height)?;
                return Ok(());
            }
            CurrentSurfaceTexture::Timeout
            | CurrentSurfaceTexture::Occluded
            | CurrentSurfaceTexture::Lost
            | CurrentSurfaceTexture::Validation => return Ok(()),
        };

        self.write_camera(camera);
        let view = surface_texture
            .texture
            .create_view(&TextureViewDescriptor::default());
        let depth_view = self
            .depth_view
            .as_ref()
            .ok_or_else(|| "深度资源未初始化".to_string())?
            .clone();
        let pipeline = self
            .pipeline
            .as_ref()
            .ok_or_else(|| "管线未初始化".to_string())?
            .pipeline
            .clone();
        let camera_bind_group = self.camera_frames[self.frame_index].bind_group.clone();
        let clear = Color {
            r: f64::from(clear_color[0]),
            g: f64::from(clear_color[1]),
            b: f64::from(clear_color[2]),
            a: f64::from(clear_color[3]),
        };
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("wgpu frame encoder"),
            });

        for pass_id in &compiled.pass_order {
            let pass_desc = &frame_graph.passes()[pass_id.0];
            match &pass_desc.kind {
                graph::PassKind::Graphics { .. } => {
                    let color = RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(clear),
                            store: StoreOp::Store,
                        },
                    };
                    let depth = RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(Operations {
                            load: LoadOp::Clear(1.0),
                            store: StoreOp::Store,
                        }),
                        stencil_ops: None,
                    };
                    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                        label: Some("main_geometry"),
                        color_attachments: &[Some(color)],
                        depth_stencil_attachment: Some(depth),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &camera_bind_group, &[]);
                    for draw in draws {
                        let Some(mesh) = self.meshes.get(draw.mesh.index(), draw.mesh.generation())
                        else {
                            continue;
                        };
                        let material = draw
                            .material
                            .and_then(|handle| {
                                self.materials.get(handle.index(), handle.generation())
                            })
                            .or(self.default_material.as_ref());
                        let Some(material) = material else {
                            continue;
                        };
                        pass.set_bind_group(1, &material.bind_group, &[]);
                        let constants = PushConstants {
                            model: draw.data.model.to_cols_array(),
                            tint: draw.data.tint,
                        };
                        // wgpu 30 将 SPIR-V PushConstant 翻译为 immediate 数据。
                        pass.set_immediates(0, push_constant_bytes(&constants));
                        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(mesh.index_buffer.slice(..), IndexFormat::Uint32);
                        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                    }
                }
                // 每帧图内不含 transfer pass；资源上传走 device.queue 的同步路径。
                graph::PassKind::Transfer { .. } => {}
            }
        }

        self.queue.submit([encoder.finish()]);
        self.queue.present(surface_texture);
        self.frame_index = (self.frame_index + 1) % MAX_FRAMES_IN_FLIGHT;
        Ok(())
    }

    fn write_graph_dump(
        &mut self,
        frame_graph: &graph::RenderGraph,
        compiled: &graph::CompiledGraph,
    ) {
        let dot = frame_graph.dump_dot(compiled);
        if let Err(error) = std::fs::write(GRAPH_DUMP_PATH, dot)
            && !self.dump_warned
        {
            mir_log::warn!(
                target: "wgpu",
                "RenderGraph dump 写入 {} 失败：{error}",
                GRAPH_DUMP_PATH
            );
            self.dump_warned = true;
        }
    }
}
