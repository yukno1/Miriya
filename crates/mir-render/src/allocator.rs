//! `mir-render` 内部显存子分配器（R-08，批次 E）。
//!
//! 设计（`docs/plans/m2-remaining.md` §3 决策 5）：
//! - 按 memory type index 分桶；桶内块大小分级（DEVICE_LOCAL：64/256 MB，
//!   HOST_VISIBLE：16/64 MB），按需开新块并设块数上限。
//! - 块内按空闲区间线性分配 + 相邻合并；全部子分配按
//!   `max(requirements.alignment, bufferImageGranularity)` 对齐，
//!   使 buffer/image 混布在块内天然满足粒度约束。
//! - 特殊资源（持久映射的相机 UBO、超过最大块的分配）走 dedicated 整块分配。
//! - 全部 `allocate_memory` / `free_memory` 调用集中在本模块；
//!   其余代码只接触 [`Allocation`]，不出现 `allocate_memory`。
//!
//! 本模块为 crate 私有；不向公共 API 泄漏任何 Vulkan 类型（ADR-0012）。

use vulkanalia::Device;
use vulkanalia::prelude::v1_0::*;

const MIB: u64 = 1024 * 1024;

/// DEVICE_LOCAL 桶的块大小分级（MiB）：小分配用小块，超过后用大块。
const DEVICE_LOCAL_BLOCK_MIB: [u64; 2] = [64, 256];
/// HOST_VISIBLE 桶的块大小分级（MiB）。可见内存通常较小，分级整体下调。
const HOST_VISIBLE_BLOCK_MIB: [u64; 2] = [16, 64];
/// 单个 memory type 的块数上限：16 × 256 MiB = 4 GiB，超出即报错而不是无限扩张。
const MAX_BLOCKS_PER_TYPE: usize = 16;
/// 单进程设备内存分配数上限的保守保护值（规范只保证 ≥ 4096，含 dedicated 与块）。
const MAX_DEVICE_ALLOCATIONS: usize = 1024;

/// 一个已生效的显存区间：调用方持有了 `[offset, offset + size)` 的使用权。
///
/// `free` 时凭 `memory + offset + type_index + block` 归还到对应块
/// （或 dedicated 列表）；区间内容在归还前必须保证 GPU 侧不再使用。
#[derive(Debug, Clone, Copy)]
pub(super) struct Allocation {
    pub(super) memory: vk::DeviceMemory,
    pub(super) offset: u64,
    pub(super) size: u64,
    /// 所属 memory type index（归还时定位桶）。
    type_index: u32,
    /// 所属块下标；`None` 表示 dedicated 整块分配。
    block: Option<usize>,
}

/// 空闲区间（块内按 offset 升序、互不重叠）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FreeRange {
    offset: u64,
    size: u64,
}

/// 块内空闲列表：线性分配 + 相邻合并。纯数据结构，可脱离 Vulkan 独立单测。
#[derive(Debug, Default)]
struct FreeList {
    ranges: Vec<FreeRange>,
}

impl FreeList {
    /// 按 alignment 向上对齐；溢出返回 `None`。
    fn align_up(value: u64, alignment: u64) -> Option<u64> {
        if alignment <= 1 {
            return Some(value);
        }
        let remainder = value % alignment;
        if remainder == 0 {
            Some(value)
        } else {
            value.checked_add(alignment - remainder)
        }
    }

    /// 在空闲区间内做一次带对齐的分配，返回分配到的 offset；成功后拆分区间。
    fn try_alloc(&mut self, size: u64, alignment: u64) -> Option<u64> {
        for index in 0..self.ranges.len() {
            let range = self.ranges[index];
            let Some(aligned) = Self::align_up(range.offset, alignment) else {
                continue;
            };
            let Some(end) = aligned.checked_add(size) else {
                continue;
            };
            let range_end = range.offset.checked_add(range.size)?;
            if end > range_end {
                continue;
            }
            // 命中：拆分空闲区间（前段 / 后段都可能为空）。
            if aligned == range.offset && end == range_end {
                self.ranges.remove(index);
            } else if aligned == range.offset {
                self.ranges[index] = FreeRange {
                    offset: end,
                    size: range_end - end,
                };
            } else if end == range_end {
                self.ranges[index] = FreeRange {
                    offset: range.offset,
                    size: aligned - range.offset,
                };
            } else {
                self.ranges[index] = FreeRange {
                    offset: end,
                    size: range_end - end,
                };
                self.ranges.insert(
                    index,
                    FreeRange {
                        offset: range.offset,
                        size: aligned - range.offset,
                    },
                );
            }
            return Some(aligned);
        }
        None
    }

    /// 归还一段区间并合并相邻空闲区间。
    fn free_at(&mut self, offset: u64, size: u64) {
        let position = self.ranges.partition_point(|range| range.offset < offset);
        self.ranges.insert(position, FreeRange { offset, size });
        self.normalize();
    }

    /// 合并物理相邻的空闲区间（调用后 `ranges` 仍按 offset 升序且互不重叠）。
    fn normalize(&mut self) {
        let mut merged: Vec<FreeRange> = Vec::with_capacity(self.ranges.len());
        for range in self.ranges.drain(..) {
            match merged.last_mut() {
                Some(last) if last.offset.checked_add(last.size) == Some(range.offset) => {
                    last.size += range.size;
                }
                _ => merged.push(range),
            }
        }
        self.ranges = merged;
    }
}

/// 一块从驱动分配的设备内存及其空闲列表。
struct MemoryBlock {
    memory: vk::DeviceMemory,
    free: FreeList,
}

/// 同一 memory type 的所有块（新块大小由 [`block_size_for`] 按类型分级决定）。
struct Bucket {
    blocks: Vec<MemoryBlock>,
}

impl Bucket {
    fn new() -> Self {
        Self { blocks: Vec::new() }
    }
}

/// 按请求大小选择新块大小：小块能装下用小块，否则用大块。
fn block_size_for(device_local: bool, size: u64) -> u64 {
    let tiers = if device_local {
        DEVICE_LOCAL_BLOCK_MIB.map(|mib| mib * MIB)
    } else {
        HOST_VISIBLE_BLOCK_MIB.map(|mib| mib * MIB)
    };
    if size <= tiers[0] { tiers[0] } else { tiers[1] }
}

/// 显存子分配器：集中管理全部 `allocate_memory` / `free_memory`。
pub(super) struct MemoryAllocator {
    device: Device,
    /// 下标即 memory type index；未用到的类型为 `None`。
    buckets: Vec<Option<Bucket>>,
    /// dedicated 整块分配（持久映射等特殊资源）。
    dedicated: Vec<Allocation>,
    /// 当前未释放的子分配字节数（泄漏守恒断言用）。
    allocated_bytes: u64,
    /// buffer/image 混布粒度：所有子分配按 `max(alignment, granularity)` 对齐。
    granularity: u64,
    /// 每个 memory type 是否 DEVICE_LOCAL（决定块大小分级）。
    device_local: Vec<bool>,
}

impl MemoryAllocator {
    /// 查询物理设备内存属性与限制，构造空分配器。
    pub(super) fn new(
        device: Device,
        instance: &Instance,
        physical_device: vk::PhysicalDevice,
        limits: vk::PhysicalDeviceLimits,
    ) -> Self {
        // SAFETY: physical_device 来自本实例的有效枚举结果。
        let properties = unsafe { instance.get_physical_device_memory_properties(physical_device) };
        let device_local = (0..properties.memory_type_count)
            .map(|index| {
                properties.memory_types[index as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .collect();
        Self {
            device,
            buckets: (0..properties.memory_type_count).map(|_| None).collect(),
            dedicated: Vec::new(),
            allocated_bytes: 0,
            granularity: limits.buffer_image_granularity.max(1),
            device_local,
        }
    }

    /// 分配一段显存。`dedicated = true`（或请求超过最大块、超过块容量）时整块分配，
    /// 否则从对应 memory type 的块内子分配。
    ///
    /// `size` / `alignment` 应传驱动的 memory requirements；
    /// 本函数额外保证对齐到 `bufferImageGranularity` 以支持块内混布。
    pub(super) fn alloc(
        &mut self,
        size: u64,
        alignment: u64,
        type_index: u32,
        dedicated: bool,
    ) -> Result<Allocation, String> {
        if size == 0 {
            return Err("显存分配大小为 0".to_string());
        }
        let type_index = type_index as usize;
        if type_index >= self.device_local.len() {
            return Err(format!("memory type index 越界: {type_index}"));
        }
        let alignment = alignment.max(self.granularity);
        let max_block = if self.device_local[type_index] {
            DEVICE_LOCAL_BLOCK_MIB[1] * MIB
        } else {
            HOST_VISIBLE_BLOCK_MIB[1] * MIB
        };
        if dedicated || size > max_block {
            // dedicated 整块从 0 开始，天然满足任何对齐。
            self.alloc_dedicated(size, type_index as u32)
        } else {
            self.alloc_from_block(size, alignment, type_index)
        }
    }

    /// dedicated 整块分配：一次 `allocate_memory`，`free` 时整体归还。
    fn alloc_dedicated(&mut self, size: u64, type_index: u32) -> Result<Allocation, String> {
        if self.dedicated.len() + self.bucket_allocation_count() >= MAX_DEVICE_ALLOCATIONS {
            return Err(format!(
                "设备内存分配数达到保护上限 {MAX_DEVICE_ALLOCATIONS}"
            ));
        }
        let allocation = self.allocate_memory(size, type_index)?;
        let allocation = Allocation {
            memory: allocation,
            offset: 0,
            size,
            type_index,
            block: None,
        };
        self.dedicated.push(allocation);
        self.allocated_bytes += size;
        Ok(allocation)
    }

    /// 从块内子分配：先在现有块中 first-fit，失败则按分级开新块。
    fn alloc_from_block(
        &mut self,
        size: u64,
        alignment: u64,
        type_index: usize,
    ) -> Result<Allocation, String> {
        let device_local = self.device_local[type_index];
        let block_size = block_size_for(device_local, size);

        /// 阶段一结果：命中现有块，或需要开新块。
        enum Stage {
            Hit { block: usize, offset: u64 },
            NewBlock,
        }

        // 阶段一（借用 buckets）：现有块 first-fit；借用在此作用域结束，
        // 让 `allocate_memory`（需要 `&self`）可以在其后调用。
        let stage = {
            let Some(slot) = self.buckets.get_mut(type_index) else {
                return Err(format!("memory type index 越界: {type_index}"));
            };
            let bucket = slot.get_or_insert_with(Bucket::new);
            let mut stage = Stage::NewBlock;
            for (index, block) in bucket.blocks.iter_mut().enumerate() {
                if let Some(offset) = block.free.try_alloc(size, alignment) {
                    stage = Stage::Hit {
                        block: index,
                        offset,
                    };
                    break;
                }
            }
            if matches!(stage, Stage::NewBlock) && bucket.blocks.len() >= MAX_BLOCKS_PER_TYPE {
                return Err(format!(
                    "memory type {type_index} 的块数达到上限 {MAX_BLOCKS_PER_TYPE}"
                ));
            }
            stage
        };

        match stage {
            Stage::Hit {
                block: index,
                offset,
            } => {
                let memory = self.buckets[type_index]
                    .as_ref()
                    .and_then(|bucket| bucket.blocks.get(index))
                    .map(|block| block.memory)
                    .ok_or_else(|| format!("memory type {type_index} 桶缺失"))?;
                self.allocated_bytes += size;
                Ok(Allocation {
                    memory,
                    offset,
                    size,
                    type_index: type_index as u32,
                    block: Some(index),
                })
            }
            Stage::NewBlock => {
                let memory = self.allocate_memory(block_size, type_index as u32)?;
                let slot = self
                    .buckets
                    .get_mut(type_index)
                    .ok_or_else(|| format!("memory type index 越界: {type_index}"))?;
                let bucket = slot
                    .as_mut()
                    .ok_or_else(|| format!("memory type {type_index} 桶缺失"))?;
                let mut block = MemoryBlock {
                    memory,
                    free: FreeList {
                        ranges: vec![FreeRange {
                            offset: 0,
                            size: block_size,
                        }],
                    },
                };
                let offset = block
                    .free
                    .try_alloc(size, alignment)
                    .ok_or_else(|| "新块容量不足以容纳请求".to_string())?;
                bucket.blocks.push(block);
                let index = bucket.blocks.len() - 1;
                self.allocated_bytes += size;
                Ok(Allocation {
                    memory,
                    offset,
                    size,
                    type_index: type_index as u32,
                    block: Some(index),
                })
            }
        }
    }

    /// 一次裸 `allocate_memory`（dedicated 与新块共用）。
    fn allocate_memory(&self, size: u64, type_index: u32) -> Result<vk::DeviceMemory, String> {
        let info = vk::MemoryAllocateInfo::builder()
            .allocation_size(size)
            .memory_type_index(type_index);
        // SAFETY: 分配参数已由调用方校验（大小非零、type index 越界检查）。
        unsafe { self.device.allocate_memory(&info, None) }
            .map_err(|error| format!("分配设备内存失败（{size} 字节，type {type_index}）: {error}"))
    }

    /// 归还一段显存；内容必须保证 GPU 侧已不再使用。
    pub(super) fn free(&mut self, allocation: &Allocation) {
        match allocation.block {
            Some(index) => {
                if let Some(Some(bucket)) = self.buckets.get_mut(allocation.type_index as usize)
                    && let Some(block) = bucket.blocks.get_mut(index)
                {
                    block.free.free_at(allocation.offset, allocation.size);
                }
            }
            None => {
                if let Some(position) = self.dedicated.iter().position(|candidate| {
                    candidate.memory == allocation.memory && candidate.offset == allocation.offset
                }) {
                    self.dedicated.remove(position);
                    // SAFETY: memory 由本分配器分配，且不在任何块中复用。
                    unsafe { self.device.free_memory(allocation.memory, None) };
                }
            }
        }
        self.allocated_bytes = self.allocated_bytes.saturating_sub(allocation.size);
    }

    /// 当前未释放的子分配字节数（泄漏守恒断言：销毁全部资源后应为 0）。
    pub(super) fn allocated_bytes(&self) -> u64 {
        self.allocated_bytes
    }

    fn bucket_allocation_count(&self) -> usize {
        self.buckets
            .iter()
            .flatten()
            .map(|bucket| bucket.blocks.len())
            .sum()
    }

    /// 释放全部块与 dedicated 内存；`VulkanCore::drop` 在销毁 device 前调用。
    /// 幂等：清空后再调用为空操作。
    pub(super) fn destroy_all(&mut self) {
        for bucket in self.buckets.drain(..).flatten() {
            for block in bucket.blocks {
                // SAFETY: 块内存由本分配器分配且尚未释放。
                unsafe { self.device.free_memory(block.memory, None) };
            }
        }
        for allocation in self.dedicated.drain(..) {
            // SAFETY: dedicated 内存由本分配器分配且尚未释放。
            unsafe { self.device.free_memory(allocation.memory, None) };
        }
        self.allocated_bytes = 0;
    }
}

impl Drop for MemoryAllocator {
    fn drop(&mut self) {
        // 兜底：正常路径已在 `VulkanCore::drop` 中 destroy_all（清空后此处为空操作）。
        self.destroy_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个覆盖单个空闲区间的列表。
    fn full_list(total: u64) -> FreeList {
        FreeList {
            ranges: vec![FreeRange {
                offset: 0,
                size: total,
            }],
        }
    }

    /// 已占用字节数 = 块总大小 − 空闲总量（测试辅助）。
    fn used_bytes(list: &FreeList, total: u64) -> u64 {
        let free: u64 = list.ranges.iter().map(|range| range.size).sum();
        total.saturating_sub(free)
    }

    #[test]
    fn align_up_handles_unaligned_and_overflow() {
        assert_eq!(FreeList::align_up(0, 4), Some(0));
        assert_eq!(FreeList::align_up(1, 4), Some(4));
        assert_eq!(FreeList::align_up(4, 4), Some(4));
        assert_eq!(FreeList::align_up(5, 3), Some(6));
        assert_eq!(FreeList::align_up(u64::MAX, 2), None);
    }

    #[test]
    fn alloc_respects_alignment_including_non_power_of_two() {
        let mut list = full_list(1024);
        // 非幂次对齐（驱动可能出现，如 6）：0 → 6。
        assert_eq!(list.try_alloc(1, 6), Some(0));
        // 下一个可用位置：1 对齐到 6。
        assert_eq!(list.try_alloc(1, 6), Some(6));
        // 大对齐跳过空隙。
        assert_eq!(list.try_alloc(1, 512), Some(512));
        assert_eq!(used_bytes(&list, 1024), 3);
    }

    #[test]
    fn alloc_splits_ranges_and_exhausts_block() {
        let mut list = full_list(64);
        assert_eq!(list.try_alloc(16, 1), Some(0));
        assert_eq!(list.try_alloc(16, 1), Some(16));
        assert_eq!(list.try_alloc(16, 1), Some(32));
        assert_eq!(list.try_alloc(16, 1), Some(48));
        assert_eq!(list.try_alloc(1, 1), None);
        assert_eq!(used_bytes(&list, 64), 64);
    }

    #[test]
    fn free_merges_adjacent_ranges_on_both_sides() {
        let mut list = full_list(64);
        let offsets: Vec<u64> = (0..4)
            .map(|_| list.try_alloc(16, 1).expect("分配成功"))
            .collect();
        assert_eq!(offsets, vec![0, 16, 32, 48]);
        // 中间先释放一段，再释放两侧，最终应合并回单个完整区间。
        list.free_at(16, 16);
        assert_eq!(list.ranges.len(), 1);
        list.free_at(0, 16);
        assert_eq!(list.ranges.len(), 1);
        list.free_at(32, 32);
        assert_eq!(list.ranges.len(), 1);
        assert_eq!(
            list.ranges[0],
            FreeRange {
                offset: 0,
                size: 64
            }
        );
        // 合并后的整块可再次完整分配。
        assert_eq!(list.try_alloc(64, 1), Some(0));
    }

    #[test]
    fn free_leaves_gap_when_not_adjacent() {
        let mut list = full_list(96);
        let offsets: Vec<u64> = (0..3)
            .map(|_| list.try_alloc(32, 1).expect("分配成功"))
            .collect();
        list.free_at(offsets[1], 32);
        // 未相邻，不能与两端合并。
        assert_eq!(list.ranges.len(), 1);
        assert_eq!(used_bytes(&list, 96), 64);
    }

    #[test]
    fn interleaved_alloc_free_cycle_is_exact() {
        let mut list = full_list(256);
        let mut live: Vec<(u64, u64)> = Vec::new();
        // 交错大小分配 + 按序释放（FIFO）200 轮，结束后必须回到整块空闲。
        let mut next_size = 1_u64;
        for round in 0..200 {
            let size = next_size * 3;
            next_size = next_size * 7 % 29 + 1;
            let offset = list.try_alloc(size, 8).unwrap_or_else(|| {
                panic!(
                    "第 {round} 轮分配失败（容量 256，size {size}）ranges={:?}",
                    list.ranges
                )
            });
            live.push((offset, size));
            // 奇数轮释放最早的两个请求，保持存活集合有界并制造碎片。
            if round % 2 == 1 {
                for _ in 0..2.min(live.len()) {
                    let (offset, size) = live.remove(0);
                    list.free_at(offset, size);
                }
            }
        }
        for (offset, size) in live {
            list.free_at(offset, size);
        }
        assert_eq!(list.ranges.len(), 1);
        assert_eq!(used_bytes(&list, 256), 0);
    }

    #[test]
    fn bucket_picks_tier_by_request_size() {
        assert_eq!(block_size_for(true, 1), DEVICE_LOCAL_BLOCK_MIB[0] * MIB);
        assert_eq!(
            block_size_for(true, DEVICE_LOCAL_BLOCK_MIB[0] * MIB),
            DEVICE_LOCAL_BLOCK_MIB[0] * MIB
        );
        assert_eq!(
            block_size_for(true, DEVICE_LOCAL_BLOCK_MIB[0] * MIB + 1),
            DEVICE_LOCAL_BLOCK_MIB[1] * MIB
        );

        assert_eq!(block_size_for(false, 1), HOST_VISIBLE_BLOCK_MIB[0] * MIB);
        assert_eq!(
            block_size_for(false, u64::MAX),
            HOST_VISIBLE_BLOCK_MIB[1] * MIB
        );
    }
}
