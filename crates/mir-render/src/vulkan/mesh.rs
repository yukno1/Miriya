//! GPU 网格资源与世代句柄槽位。

use vulkanalia::prelude::v1_0::*;

use crate::allocator::Allocation;
use crate::mesh::{MeshData, MeshHandle, indices_as_bytes, vertices_as_bytes};

use super::VulkanCore;
use super::buffer::DeletionEntry;

/// 一个已上传到显存的网格（顶点/索引各一段子分配）。
pub(super) struct GpuMesh {
    pub(super) vertex_buffer: vk::Buffer,
    vertex_allocation: Allocation,
    pub(super) index_buffer: vk::Buffer,
    index_allocation: Allocation,
    /// 绘制时读取（V-13 提交 API）。
    pub(super) index_count: u32,
}

/// 网格槽位：世代号用于让销毁后的旧 [`MeshHandle`] 自动失效。
pub(super) struct MeshSlot {
    generation: u32,
    mesh: Option<GpuMesh>,
}

impl VulkanCore {
    /// 创建 GPU 网格：staging → device-local，阻塞式上传（v0.2 网格极少，成本可忽略）。
    pub(crate) fn create_mesh(&mut self, data: &MeshData) -> Result<MeshHandle, String> {
        if !data.is_valid() {
            return Err("网格数据无效：顶点/索引为空或索引越界".to_string());
        }
        let mesh = self.upload_mesh(data)?;

        let index = self.free_meshes.pop().unwrap_or(self.meshes.len() as u32);
        let generation = self
            .meshes
            .get(index as usize)
            .map(|slot| slot.generation)
            .unwrap_or(0);
        let slot = MeshSlot {
            generation,
            mesh: Some(mesh),
        };
        if index as usize == self.meshes.len() {
            self.meshes.push(slot);
        } else {
            self.meshes[index as usize] = slot;
        }
        Ok(MeshHandle::from_parts(index, generation))
    }

    /// 销毁网格；句柄无效或世代不匹配时返回 `false`（旧句柄不会误删新网格）。
    pub(crate) fn destroy_mesh(&mut self, handle: MeshHandle) -> bool {
        let index = handle.index();
        let Some(slot) = self.meshes.get(index) else {
            return false;
        };
        if slot.generation != handle.generation() {
            return false;
        }

        let mesh = self.meshes[index].mesh.take();
        self.meshes[index].generation = handle.generation().wrapping_add(1);
        self.free_meshes.push(index as u32);
        if let Some(mesh) = mesh {
            if self.frames.is_empty() {
                // headless：无帧在飞行，上传路径已同步等待队列，立即释放安全。
                self.release_mesh(mesh);
            } else {
                // 延迟回收（R-09）：绑定下一帧的 fence——其信号保证所有更早的
                // 在飞提交已完成（队列操作按提交序完成）。
                let fence = self.frames[self.frame_index].fence;
                self.deletions.push(DeletionEntry {
                    fence,
                    buffers: vec![
                        (mesh.vertex_buffer, mesh.vertex_allocation),
                        (mesh.index_buffer, mesh.index_allocation),
                    ],
                    descriptor_sets: Vec::new(),
                });
            }
        }
        true
    }

    /// 立即销毁网格的 GPU 缓冲并归还显存（Drop / headless 路径）。
    fn release_mesh(&mut self, mesh: GpuMesh) {
        // SAFETY: 网格缓冲由本设备创建，且调用方保证 GPU 侧不再使用。
        unsafe {
            self.device.destroy_buffer(mesh.vertex_buffer, None);
            self.device.destroy_buffer(mesh.index_buffer, None);
        }
        self.allocator.free(&mesh.vertex_allocation);
        self.allocator.free(&mesh.index_allocation);
    }

    /// 解析网格句柄；世代号不匹配（已被销毁/复用）时返回 `None`。
    pub(super) fn mesh_lookup(meshes: &[MeshSlot], handle: MeshHandle) -> Option<&GpuMesh> {
        let slot = meshes.get(handle.index())?;
        if slot.generation != handle.generation() {
            return None;
        }
        slot.mesh.as_ref()
    }

    fn upload_mesh(&mut self, data: &MeshData) -> Result<GpuMesh, String> {
        let vertex_bytes = vertices_as_bytes(&data.vertices);
        let index_bytes = indices_as_bytes(&data.indices);

        let vertex = self.upload_bytes(vertex_bytes, vk::BufferUsageFlags::VERTEX_BUFFER)?;
        let index = match self.upload_bytes(index_bytes, vk::BufferUsageFlags::INDEX_BUFFER) {
            Ok(created) => created,
            Err(error) => {
                self.release_buffer(vertex);
                return Err(error);
            }
        };

        Ok(GpuMesh {
            vertex_buffer: vertex.buffer,
            vertex_allocation: vertex.allocation,
            index_buffer: index.buffer,
            index_allocation: index.allocation,
            index_count: data.indices.len() as u32,
        })
    }

    /// Drop 收尾：释放全部存活网格的缓冲。
    pub(super) fn destroy_all_meshes(&mut self) {
        // 先收集再释放，避免同时借用 `self.meshes` 与 `self.device`。
        let meshes: Vec<GpuMesh> = self
            .meshes
            .iter_mut()
            .filter_map(|slot| slot.mesh.take())
            .collect();
        for mesh in meshes {
            self.release_mesh(mesh);
        }
    }
}
