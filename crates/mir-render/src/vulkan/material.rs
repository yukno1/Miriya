//! GPU 材质资源：参数 UBO（set 1 / binding 0）与世代槽位（R-11）。

use vulkanalia::prelude::v1_0::*;

use crate::allocator::Allocation;
use crate::material::{MaterialData, MaterialHandle, material_data_bytes};

use super::VulkanCore;
use super::buffer::DeletionEntry;

/// 材质 UBO（set 1 / binding 0）：std140 布局的 [`MaterialData`]（R-11）。
const MATERIAL_UBO_SIZE: u64 = std::mem::size_of::<MaterialData>() as u64;

/// 材质槽位上限：决定描述符池容量（每材质 1 个 set 1 描述符集）。
/// 超出上限后 `create_material` 返回错误；材质销毁会把槽位与描述符集归还复用。
pub(super) const MAX_MATERIALS: usize = 64;

/// 一个已就绪的 GPU 材质：参数 UBO（HOST_VISIBLE|COHERENT，dedicated 整块分配，
/// 因为创建时要 map/unmap，不能与 staging 共享同一 DeviceMemory）+ set 1 描述符集。
pub(super) struct GpuMaterial {
    buffer: vk::Buffer,
    allocation: Allocation,
    /// set 1 描述符集；headless（无描述符池）时为 null，且不会被绘制路径引用。
    pub(super) descriptor_set: vk::DescriptorSet,
}

/// 材质槽位：世代号用于让销毁后的旧 [`MaterialHandle`] 自动失效（风格对齐 [`super::mesh::MeshSlot`]）。
pub(super) struct MaterialSlot {
    generation: u32,
    material: Option<GpuMaterial>,
}

impl VulkanCore {
    /// 创建 GPU 材质（R-11）：参数写入专用 UBO（dedicated 分配，创建时 map/unmap），
    /// 管线就绪时分配 set 1 描述符集。
    ///
    /// headless（无管线）同样可创建：句柄有效但不分配描述符集，不会被绘制。
    pub(crate) fn create_material(
        &mut self,
        data: &MaterialData,
    ) -> Result<MaterialHandle, String> {
        let buffer = self.create_buffer(
            MATERIAL_UBO_SIZE,
            vk::BufferUsageFlags::UNIFORM_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            true,
        )?;
        let bytes = material_data_bytes(data);
        // 写入参数：HOST_COHERENT 无需 flush；dedicated 整块保证 map/unmap 不影响
        // 同块内的 staging 等其他子分配（unmap_memory 按整个 DeviceMemory 生效）。
        let mapped = unsafe {
            self.device.map_memory(
                buffer.allocation.memory,
                buffer.allocation.offset,
                MATERIAL_UBO_SIZE,
                vk::MemoryMapFlags::empty(),
            )
        };
        match mapped {
            Ok(mapped) => unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast::<u8>(), bytes.len());
                self.device.unmap_memory(buffer.allocation.memory);
            },
            Err(error) => {
                // SAFETY: buffer 刚创建且 GPU 侧从未使用，立即释放安全。
                unsafe { self.device.destroy_buffer(buffer.buffer, None) };
                self.allocator.free(&buffer.allocation);
                return Err(format!("映射材质 UBO 失败: {error}"));
            }
        }

        // 管线就绪时分配 set 1 描述符集并写入绑定（命名数组模式，见相机 UBO 写入）。
        let descriptor_set = match &self.pipeline {
            Some(resources) => {
                let allocate_layouts = [resources.material_set_layout];
                let allocate_info = vk::DescriptorSetAllocateInfo::builder()
                    .descriptor_pool(resources.descriptor_pool)
                    .set_layouts(&allocate_layouts);
                // SAFETY: 池与材质布局有效。
                let sets = match unsafe { self.device.allocate_descriptor_sets(&allocate_info) } {
                    Ok(sets) => sets,
                    Err(error) => {
                        // SAFETY: buffer 刚创建且 GPU 侧从未使用，立即释放安全。
                        unsafe { self.device.destroy_buffer(buffer.buffer, None) };
                        self.allocator.free(&buffer.allocation);
                        return Err(format!("分配材质描述符集失败: {error}"));
                    }
                };
                let set = sets
                    .into_iter()
                    .next()
                    .ok_or_else(|| "材质描述符集分配结果为空".to_string())?;
                let material_infos = [vk::DescriptorBufferInfo::builder()
                    .buffer(buffer.buffer)
                    .offset(0)
                    .range(MATERIAL_UBO_SIZE)
                    .build()];
                // 必须命名中间数组：`WriteDescriptorSet` 保存的是指针，临时数组会在语句结束即失效。
                let material_write = vk::WriteDescriptorSet::builder()
                    .dst_set(set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(&material_infos)
                    .build();
                let material_writes = [material_write];
                // SAFETY: material_writes / material_infos 为命名数组，调用期间有效。
                unsafe {
                    let no_copies: [vk::CopyDescriptorSet; 0] = [];
                    self.device
                        .update_descriptor_sets(&material_writes, &no_copies);
                }
                set
            }
            // headless：无描述符池，null 集合不会被绘制路径引用。
            None => vk::DescriptorSet::null(),
        };

        let gpu = GpuMaterial {
            buffer: buffer.buffer,
            allocation: buffer.allocation,
            descriptor_set,
        };

        let index = self
            .free_materials
            .pop()
            .unwrap_or(self.materials.len() as u32);
        let generation = self
            .materials
            .get(index as usize)
            .map(|slot| slot.generation)
            .unwrap_or(0);
        let slot = MaterialSlot {
            generation,
            material: Some(gpu),
        };
        if index as usize == self.materials.len() {
            self.materials.push(slot);
        } else {
            self.materials[index as usize] = slot;
        }
        Ok(MaterialHandle::from_parts(index, generation))
    }

    /// 创建默认材质（白色 Lambert），绘制路径的兜底。随 `init_pipeline` 调用，
    /// 此时描述符池已就绪。
    pub(super) fn create_default_material(&mut self) -> Result<(), String> {
        let handle = self.create_material(&MaterialData::default())?;
        let index = handle.index();
        self.default_material = Some(
            self.materials[index]
                .material
                .take()
                .expect("默认材质槽位刚创建"),
        );
        // 该槽位永久保留（世代号不递增），不会再被复用——默认材质不走句柄表。
        Ok(())
    }

    /// 销毁材质；句柄无效或世代不匹配时返回 `false`（旧句柄不会误删新材质）。
    ///
    /// 缓冲经 DeletionQueue 延迟回收；描述符集在 fence 信号后归还描述符池。
    pub(crate) fn destroy_material(&mut self, handle: MaterialHandle) -> bool {
        let index = handle.index();
        let Some(slot) = self.materials.get(index) else {
            return false;
        };
        if slot.generation != handle.generation() {
            return false;
        }

        let material = self.materials[index].material.take();
        self.materials[index].generation = handle.generation().wrapping_add(1);
        self.free_materials.push(index as u32);
        if let Some(material) = material {
            if self.frames.is_empty() {
                // headless：无帧在飞行，参数 UBO 创建后未被 GPU 引用，立即释放安全。
                self.release_material(material);
            } else {
                // 延迟回收（R-09）：绑定下一帧的 fence——其信号保证所有更早的
                // 在飞提交已完成（队列操作按提交序完成）。
                let fence = self.frames[self.frame_index].fence;
                self.deletions.push(DeletionEntry {
                    fence,
                    buffers: vec![(material.buffer, material.allocation)],
                    descriptor_sets: vec![material.descriptor_set],
                });
            }
        }
        true
    }

    /// 立即销毁材质的 GPU 缓冲并归还显存（Drop / headless 路径）。
    /// 描述符集随描述符池销毁一并释放（Drop 前 `flush_deletions` 会先归还可达的集合）。
    pub(super) fn release_material(&mut self, material: GpuMaterial) {
        // SAFETY: 材质缓冲由本设备创建，且调用方保证 GPU 侧不再使用。
        unsafe {
            self.device.destroy_buffer(material.buffer, None);
        }
        self.allocator.free(&material.allocation);
    }

    /// 解析材质句柄；世代号不匹配（已被销毁/复用）时返回 `None`。
    pub(super) fn material_lookup(
        materials: &[MaterialSlot],
        handle: MaterialHandle,
    ) -> Option<&GpuMaterial> {
        let slot = materials.get(handle.index())?;
        if slot.generation != handle.generation() {
            return None;
        }
        slot.material.as_ref()
    }

    /// Drop 收尾：释放全部存活材质的缓冲（描述符集随池销毁）。
    pub(super) fn destroy_all_materials(&mut self) {
        let materials: Vec<GpuMaterial> = self
            .materials
            .iter_mut()
            .filter_map(|slot| slot.material.take())
            .collect();
        for material in materials {
            self.release_material(material);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_ubo_size_matches_data_layout() {
        // 材质 UBO 大小必须精确等于 std140 对齐后的 MaterialData（R-11）。
        assert_eq!(MATERIAL_UBO_SIZE, 48);
        assert_eq!(MATERIAL_UBO_SIZE as usize, MaterialData::SIZE);
    }
}
