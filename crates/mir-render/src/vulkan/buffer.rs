//! GPU 缓冲分配、staging 上传与延迟销毁队列（R-09）。

use vulkanalia::prelude::v1_0::*;

use crate::allocator::Allocation;

use super::VulkanCore;

/// 分配好的 buffer 及其显存区间（批次 E 起全部缓冲资源统一经分配器）。
pub(super) struct GpuBuffer {
    pub(super) buffer: vk::Buffer,
    pub(super) allocation: Allocation,
}

/// 延迟销毁条目（R-09）：GPU 缓冲 + 显存区间 + 关联帧 fence。
///
/// fence 信号后（所有引用该资源的提交已完成）才真正 `destroy_buffer` 并归还分配器；
/// 队列按 fence 提交顺序天然有序，绑定「下一帧」的 fence 即可覆盖全部更早的在飞提交。
pub(super) struct DeletionEntry {
    pub(super) fence: vk::Fence,
    pub(super) buffers: Vec<(vk::Buffer, Allocation)>,
    /// 材质描述符集（R-11）：同 fence 信号后归还描述符池
    /// （池创建时带 FREE_DESCRIPTOR_SET_BIT）。
    pub(super) descriptor_sets: Vec<vk::DescriptorSet>,
    /// 图像资源（M5 批次 A）：image + 显存 + 视图 + 采样器，同 fence 信号后销毁。
    pub(super) images: Vec<DeletionImage>,
}

/// 待延迟销毁的完整 GPU 图像资源（M5 批次 A）。
pub(super) struct DeletionImage {
    pub(super) image: vk::Image,
    pub(super) allocation: Allocation,
    pub(super) view: vk::ImageView,
    pub(super) sampler: vk::Sampler,
}

impl VulkanCore {
    /// 分配 buffer 并绑定分配器子分配的设备内存（R-09：staging/UBO/网格统一入口）。
    ///
    /// `dedicated = true` 用于持久映射等特殊资源（相机 UBO）。
    pub(super) fn create_buffer(
        &mut self,
        size: u64,
        usage: vk::BufferUsageFlags,
        properties: vk::MemoryPropertyFlags,
        dedicated: bool,
    ) -> Result<GpuBuffer, String> {
        let info = vk::BufferCreateInfo::builder()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: BufferCreateInfo 有效。
        let buffer = unsafe { self.device.create_buffer(&info, None) }
            .map_err(|error| format!("创建缓冲失败: {error}"))?;
        // SAFETY: buffer 刚由本设备创建。
        let requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        let memory_type = match self.find_memory_type(requirements.memory_type_bits, properties) {
            Ok(memory_type) => memory_type,
            Err(error) => {
                // SAFETY: buffer 由本设备创建且尚未绑定。
                unsafe { self.device.destroy_buffer(buffer, None) };
                return Err(error);
            }
        };
        let allocation = match self.allocator.alloc(
            requirements.size,
            requirements.alignment,
            memory_type,
            dedicated,
        ) {
            Ok(allocation) => allocation,
            Err(error) => {
                // SAFETY: buffer 由本设备创建且尚未绑定。
                unsafe { self.device.destroy_buffer(buffer, None) };
                return Err(format!("分配缓冲内存失败: {error}"));
            }
        };
        // SAFETY: buffer 与 allocation.memory 由本设备创建，偏移在分配范围内。
        if let Err(error) = unsafe {
            self.device
                .bind_buffer_memory(buffer, allocation.memory, allocation.offset)
        }
        .map_err(|error| format!("绑定缓冲内存失败: {error}"))
        {
            // SAFETY: buffer 由本设备创建且已解绑/绑定失败，不再使用。
            unsafe { self.device.destroy_buffer(buffer, None) };
            self.allocator.free(&allocation);
            return Err(error);
        }
        Ok(GpuBuffer { buffer, allocation })
    }

    /// 立即销毁 buffer 并归还显存。仅用于调用点能证明 GPU 不再使用该缓冲的路径
    /// （刚创建未使用 / 阻塞上传后 `queue_wait_idle` 已返回）；其余走 `DeletionQueue`。
    pub(super) fn release_buffer(&mut self, gpu: GpuBuffer) {
        // SAFETY: buffer 由本设备创建且调用方保证 GPU 侧不再使用。
        unsafe { self.device.destroy_buffer(gpu.buffer, None) };
        self.allocator.free(&gpu.allocation);
    }

    /// 把字节上传到 device-local 缓冲（staging → transfer copy → 同步等待）。
    pub(super) fn upload_bytes(
        &mut self,
        bytes: &[u8],
        usage: vk::BufferUsageFlags,
    ) -> Result<GpuBuffer, String> {
        let size = bytes.len() as u64;
        let staging = self.create_buffer(
            size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            false,
        )?;
        // SAFETY: staging 内存为 HOST_VISIBLE|HOST_COHERENT，映射区间与 bytes 匹配；
        // 该分配不在持久映射块中（相机 UBO 为 dedicated），unmap 不影响其他映射。
        unsafe {
            let mapped = self
                .device
                .map_memory(
                    staging.allocation.memory,
                    staging.allocation.offset,
                    size,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(|error| format!("映射 staging 内存失败: {error}"))?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast::<u8>(), bytes.len());
            self.device.unmap_memory(staging.allocation.memory);
        }

        let buffer = match self.create_buffer(
            size,
            usage | vk::BufferUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            false,
        ) {
            Ok(created) => created,
            Err(error) => {
                self.release_buffer(staging);
                return Err(error);
            }
        };

        let result = self.copy_buffer_blocking(staging.buffer, buffer.buffer, size);
        // staging 只在拷贝期间需要；拷贝路径已等待队列空闲，可立即释放。
        self.release_buffer(staging);
        match result {
            Ok(()) => Ok(buffer),
            Err(error) => {
                self.release_buffer(buffer);
                Err(error)
            }
        }
    }

    /// 用传输命令缓冲做一次同步缓冲拷贝（提交后等待队列空闲）。
    fn copy_buffer_blocking(
        &mut self,
        source: vk::Buffer,
        destination: vk::Buffer,
        size: u64,
    ) -> Result<(), String> {
        let begin_info = vk::CommandBufferBeginInfo::builder()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: 传输命令缓冲只在同步上传路径使用，提交后立即等待完成。
        unsafe {
            self.device
                .begin_command_buffer(self.transfer_command_buffer, &begin_info)
                .map_err(|error| format!("开始缓冲拷贝失败: {error}"))?;
            let region = vk::BufferCopy {
                src_offset: 0,
                dst_offset: 0,
                size,
            };
            self.device.cmd_copy_buffer(
                self.transfer_command_buffer,
                source,
                destination,
                &[region],
            );
            // 传输写入 → 顶点/索引读取的可见性依赖；不依赖后续提交顺序自动保证。
            let no_memory_barriers: [vk::MemoryBarrier; 0] = [];
            let no_image_barriers: [vk::ImageMemoryBarrier; 0] = [];
            let buffer_barrier = vk::BufferMemoryBarrier::builder()
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(
                    vk::AccessFlags::INDEX_READ | vk::AccessFlags::VERTEX_ATTRIBUTE_READ,
                )
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .buffer(destination)
                .offset(0)
                .size(size)
                .build();
            self.device.cmd_pipeline_barrier(
                self.transfer_command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::VERTEX_INPUT,
                vk::DependencyFlags::empty(),
                &no_memory_barriers,
                &[buffer_barrier],
                &no_image_barriers,
            );
            self.device
                .end_command_buffer(self.transfer_command_buffer)
                .map_err(|error| format!("结束缓冲拷贝失败: {error}"))?;
            let command_buffers = [self.transfer_command_buffer];
            let submit = vk::SubmitInfo::builder().command_buffers(&command_buffers);
            self.device
                .queue_submit(self.graphics_queue, &[submit], vk::Fence::null())
                .map_err(|error| format!("提交缓冲拷贝失败: {error}"))?;
            self.device
                .queue_wait_idle(self.graphics_queue)
                .map_err(|error| format!("等待缓冲拷贝完成失败: {error}"))?;
        }
        Ok(())
    }

    /// 冲销队列：销毁所有 fence 已匹配（或 `fence = None` 即全部）的延迟销毁条目。
    ///
    /// 帧循环在本帧 fence 等待返回后调用；`Drop` / 重建交换链在 `wait_idle` 后以
    /// `None` 调用（此时全部提交已完成）。
    pub(super) fn flush_deletions(&mut self, fence: Option<vk::Fence>) {
        let mut remaining = Vec::new();
        for entry in self.deletions.drain(..) {
            let due = fence.is_none_or(|fence| fence == entry.fence);
            if due {
                for (buffer, allocation) in entry.buffers {
                    // SAFETY: 关联 fence 已信号，GPU 侧不再使用该缓冲。
                    unsafe { self.device.destroy_buffer(buffer, None) };
                    self.allocator.free(&allocation);
                }
                for image in entry.images {
                    // SAFETY: 关联 fence 已信号，GPU 侧不再使用该图像；
                    // sampler/view/image 由本设备创建，显存归分配器回收。
                    unsafe {
                        self.device.destroy_sampler(image.sampler, None);
                        self.device.destroy_image_view(image.view, None);
                        self.device.destroy_image(image.image, None);
                    }
                    self.allocator.free(&image.allocation);
                }
                // 材质描述符集（R-11）：GPU 已不再引用，归还描述符池复用。
                // 池在 destroy_pipeline 中先于本调用（Drop 顺序：flush_deletions 在前）。
                if !entry.descriptor_sets.is_empty()
                    && let Some(resources) = &self.pipeline
                {
                    // SAFETY: 池创建时带 FREE_DESCRIPTOR_SET_BIT，集合均属于该池且不再被引用。
                    unsafe {
                        let _ = self.device.free_descriptor_sets(
                            resources.descriptor_pool,
                            &entry.descriptor_sets,
                        );
                    }
                }
            } else {
                remaining.push(entry);
            }
        }
        self.deletions = remaining;
    }
}
