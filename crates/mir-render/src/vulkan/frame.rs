//! 每帧资源：命令池/缓冲、同步原语、相机 UBO 与 GPU 时间戳查询池。

use vulkanalia::prelude::v1_0::*;

use crate::allocator::Allocation;
use crate::frame::{CAMERA_UBO_SIZE, MAX_FRAMES_IN_FLIGHT};

use super::VulkanCore;

/// GPU 时间戳查询池容量：每 pass 2 个查询（pass 起/止），按 32 个 pass 上限预留。
pub(super) const TIMESTAMP_QUERY_COUNT: u32 = 64;

/// 单帧录制与同步资源；共 [`MAX_FRAMES_IN_FLIGHT`] 份环形复用，
/// 复用前通过本帧 fence 等待上一轮该帧的 GPU 工作完成。
pub(super) struct FrameResources {
    pub(super) command_pool: vk::CommandPool,
    pub(super) command_buffer: vk::CommandBuffer,
    pub(super) fence: vk::Fence,
    /// 每帧一个 acquire 信号量：帧在飞行保证同一信号量不会被并发 acquire 复用。
    pub(super) image_available: vk::Semaphore,
    /// 每张交换链图像一个呈现信号量（每帧各持一份）：
    /// 避免同一信号量在"已呈现但尚未重新 acquire"的图像间复用（v0.2 VUID-00067 修复），
    /// 每帧再各持一份以适配多帧在飞行。
    pub(super) render_finished: Vec<vk::Semaphore>,
    /// 相机 UBO（每帧一份，HOST_VISIBLE|COHERENT，持久映射；dedicated 整块分配，
    /// 避免 staging 的 map/unmap 影响同一块内存中的持久映射）。
    pub(super) camera_buffer: vk::Buffer,
    pub(super) camera_allocation: Allocation,
    /// `camera_allocation` 的持久映射；写入前由本帧 fence 保证 GPU 已读完上一轮内容。
    pub(super) camera_mapped: *mut u8,
    /// GPU 时间戳查询池（每 pass 2 个查询，R-05）；队列族不支持时间戳时为 `None`。
    pub(super) timestamp_pool: Option<vk::QueryPool>,
    /// 上一轮该帧是否成功写入了 pass 时间戳（首轮为 false，避免读到未初始化查询）。
    pub(super) timings_valid: bool,
    pub(super) extent: vk::Extent2D,
}

impl VulkanCore {
    pub(super) fn init_frame_resources(&mut self) -> Result<(), String> {
        let semaphore_info = vk::SemaphoreCreateInfo::builder();
        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            // 单帧资源创建失败时先就地回收已创建的句柄，再向上传播错误。
            let frame = self.create_frame_sync(&semaphore_info)?;
            self.frames.push(frame);
        }
        Ok(())
    }

    /// 创建一帧的全部资源：命令池/缓冲、fence（创建即 SIGNALED）、信号量与相机 UBO（持久映射）。
    ///
    /// 失败时销毁本次已创建的对象，不产生泄漏。
    fn create_frame_sync(
        &mut self,
        semaphore_info: &vk::SemaphoreCreateInfoBuilder,
    ) -> Result<FrameResources, String> {
        let pool_info = vk::CommandPoolCreateInfo::builder()
            .queue_family_index(self.graphics_family)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        // SAFETY: graphics_family 是本设备支持的队列族。
        let command_pool = unsafe { self.device.create_command_pool(&pool_info, None) }
            .map_err(|error| format!("创建命令池失败: {error}"))?;

        // 任一步失败都经 `fail` 就地回收已创建的句柄。
        macro_rules! fail {
            ($error:expr) => {{
                let error: String = $error;
                // SAFETY: 以下句柄由本设备创建且尚未销毁。
                unsafe {
                    self.device.destroy_command_pool(command_pool, None);
                }
                return Err(error);
            }};
        }

        let allocate_info = vk::CommandBufferAllocateInfo::builder()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: command_pool 有效，请求一个主命令缓冲。
        let command_buffer = match unsafe { self.device.allocate_command_buffers(&allocate_info) }
            .map_err(|error| format!("分配命令缓冲失败: {error}"))?
            .into_iter()
            .next()
        {
            Some(buffer) => buffer,
            None => fail!("命令缓冲分配结果为空".to_string()),
        };

        let fence_info = vk::FenceCreateInfo::builder().flags(vk::FenceCreateFlags::SIGNALED);
        // SAFETY: fence_info 有效；创建即 SIGNALED 保证该帧首轮不被阻塞。
        let fence = match unsafe { self.device.create_fence(&fence_info, None) } {
            Ok(fence) => fence,
            Err(error) => fail!(format!("创建 fence 失败: {error}")),
        };

        // SAFETY: semaphore_info 有效。
        let image_available = match unsafe { self.device.create_semaphore(semaphore_info, None) } {
            Ok(semaphore) => semaphore,
            Err(error) => {
                // SAFETY: fence 由本设备创建且尚未销毁。
                unsafe { self.device.destroy_fence(fence, None) };
                fail!(format!("创建 semaphore 失败: {error}"))
            }
        };
        let mut render_finished = Vec::with_capacity(self.image_views.len());
        for _ in 0..self.image_views.len() {
            // SAFETY: semaphore_info 有效。
            let semaphore = match unsafe { self.device.create_semaphore(semaphore_info, None) } {
                Ok(semaphore) => semaphore,
                Err(error) => {
                    // SAFETY: 以上句柄由本设备创建且尚未销毁。
                    unsafe {
                        self.device.destroy_semaphore(image_available, None);
                        self.device.destroy_fence(fence, None);
                    }
                    fail!(format!("创建 semaphore 失败: {error}"))
                }
            };
            render_finished.push(semaphore);
        }

        // 相机 UBO：dedicated 整块分配（持久映射不能与 staging 的 map/unmap 共享块）。
        let camera = match self.create_buffer(
            CAMERA_UBO_SIZE,
            vk::BufferUsageFlags::UNIFORM_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            true,
        ) {
            Ok(created) => created,
            Err(error) => {
                // SAFETY: 以上句柄由本设备创建且尚未销毁。
                unsafe {
                    for semaphore in render_finished.drain(..) {
                        self.device.destroy_semaphore(semaphore, None);
                    }
                    self.device.destroy_semaphore(image_available, None);
                    self.device.destroy_fence(fence, None);
                }
                fail!(error)
            }
        };
        // 持久映射：HOST_COHERENT 无需 flush，映射保持到内存释放。
        let mapped = unsafe {
            self.device.map_memory(
                camera.allocation.memory,
                camera.allocation.offset,
                CAMERA_UBO_SIZE,
                vk::MemoryMapFlags::empty(),
            )
        };
        let camera_mapped = match mapped {
            Ok(mapped) => mapped.cast::<u8>(),
            Err(error) => {
                // SAFETY: 以上句柄由本设备创建且尚未销毁。
                unsafe {
                    self.device.destroy_buffer(camera.buffer, None);
                    for semaphore in render_finished.drain(..) {
                        self.device.destroy_semaphore(semaphore, None);
                    }
                    self.device.destroy_semaphore(image_available, None);
                    self.device.destroy_fence(fence, None);
                }
                self.allocator.free(&camera.allocation);
                fail!(format!("映射相机 UBO 失败: {error}"))
            }
        };

        // GPU 时间戳查询池（R-05）：每 pass 2 个查询（起/止）。队列族不支持时为 None，
        // 仅关闭 pass 耗时统计。放在最后创建，之后的失败分支无需回收它。
        let timestamp_pool = if self.timestamp_valid_bits > 0 {
            let pool_info = vk::QueryPoolCreateInfo::builder()
                .query_type(vk::QueryType::TIMESTAMP)
                .query_count(TIMESTAMP_QUERY_COUNT);
            // SAFETY: query_type/query_count 合法；查询池由本设备创建并随帧资源销毁。
            match unsafe { self.device.create_query_pool(&pool_info, None) } {
                Ok(pool) => Some(pool),
                Err(error) => {
                    // SAFETY: 以上句柄由本设备创建且尚未销毁。
                    unsafe {
                        self.device.destroy_buffer(camera.buffer, None);
                        for semaphore in render_finished.drain(..) {
                            self.device.destroy_semaphore(semaphore, None);
                        }
                        self.device.destroy_semaphore(image_available, None);
                        self.device.destroy_fence(fence, None);
                    }
                    self.allocator.free(&camera.allocation);
                    fail!(format!("创建时间戳查询池失败: {error}"))
                }
            }
        } else {
            None
        };

        Ok(FrameResources {
            command_pool,
            command_buffer,
            fence,
            image_available,
            render_finished,
            camera_buffer: camera.buffer,
            camera_allocation: camera.allocation,
            camera_mapped,
            timestamp_pool,
            timings_valid: false,
            extent: self.extent,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_ubo_size_is_80_bytes() {
        // std140：mat4 view_proj（0…63）+ vec3 camera_position（64…75）+ 补齐（76…79）。
        assert_eq!(CAMERA_UBO_SIZE, 80);
    }

    #[test]
    fn frames_in_flight_is_two() {
        // 批次 A 的决策：2 帧在飞行；修改时需同步核对信号量复用与 UBO 环形写入。
        assert_eq!(MAX_FRAMES_IN_FLIGHT, 2);
    }
}
