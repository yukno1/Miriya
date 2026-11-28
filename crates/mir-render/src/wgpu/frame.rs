//! wgpu 每帧相机 UBO 与 bind group（2 帧环形复用，同步由 wgpu 管理）。

use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindingResource, Buffer, BufferBinding,
    BufferDescriptor, BufferUsages,
};

use crate::frame::{CAMERA_UBO_SIZE, CameraFrame, MAX_FRAMES_IN_FLIGHT};

use super::WgpuCore;

pub(crate) struct CameraFrameResources {
    pub(crate) buffer: Buffer,
    pub(crate) bind_group: BindGroup,
}

impl WgpuCore {
    pub(crate) fn init_frame_resources(&mut self) -> Result<(), String> {
        let layout = self
            .pipeline
            .as_ref()
            .ok_or_else(|| "管线尚未初始化".to_string())?
            .camera_bgl
            .clone();
        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            let buffer = self.device.create_buffer(&BufferDescriptor {
                label: Some("wgpu camera ubo"),
                size: CAMERA_UBO_SIZE,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
                label: Some("wgpu camera bind group"),
                layout: &layout,
                entries: &[BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: None,
                    }),
                }],
            });
            self.track_alloc(CAMERA_UBO_SIZE);
            self.camera_frames
                .push(CameraFrameResources { buffer, bind_group });
        }
        Ok(())
    }

    /// 写入当前帧相机数据：view_proj（64 B）+ camera_position（16 B，含补齐）。
    pub(crate) fn write_camera(&mut self, camera: CameraFrame) {
        let mut data = [0.0_f32; 20];
        data[..16].copy_from_slice(&camera.view_proj.to_cols_array());
        data[16..19].copy_from_slice(&camera.camera_position);
        // SAFETY: f32 数组无填充，长度与 CAMERA_UBO_SIZE 一致。
        let bytes = unsafe {
            std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), CAMERA_UBO_SIZE as usize)
        };
        let buffer = &self.camera_frames[self.frame_index].buffer;
        self.queue.write_buffer(buffer, 0, bytes);
    }
}
