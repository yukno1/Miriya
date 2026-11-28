//! 后端共享的世代槽位表：网格 / 材质 / 纹理句柄的索引与失效语义。
//!
//! 句柄编码沿用 `MeshHandle` / `MaterialHandle` / `TextureHandle` 的
//! `(generation << 32) | index`。销毁后槽位回到空闲栈、世代号递增，
//! 因此旧句柄不会误引用被复用的新资源（Vulkan 与 wgpu 行为一致）。

/// 单个槽位：世代号 + 可选资源值。
#[derive(Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// 世代槽位表。
#[derive(Debug)]
pub(crate) struct SlotTable<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
}

impl<T> Default for SlotTable<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SlotTable<T> {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    /// 已分配的槽位数（含已销毁、待复用的槽位）。
    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }

    /// 是否有空闲槽位可复用。
    pub(crate) fn has_free(&self) -> bool {
        !self.free.is_empty()
    }

    /// 插入资源并分配句柄分量 `(index, generation)`。
    pub(crate) fn insert(&mut self, value: T) -> (usize, u32) {
        let index = self.free.pop().unwrap_or(self.slots.len() as u32) as usize;
        let generation = self
            .slots
            .get(index)
            .map(|slot| slot.generation)
            .unwrap_or(0);
        let slot = Slot {
            generation,
            value: Some(value),
        };
        if index == self.slots.len() {
            self.slots.push(slot);
        } else {
            self.slots[index] = slot;
        }
        (index, generation)
    }

    /// 按句柄分量读取资源；索引越界、世代号过期或槽位为空均返回 `None`。
    pub(crate) fn get(&self, index: usize, generation: u32) -> Option<&T> {
        let slot = self.slots.get(index)?;
        if slot.generation != generation {
            return None;
        }
        slot.value.as_ref()
    }

    /// 取出并移除资源：世代号递增、槽位进入空闲栈，旧句柄随即失效。
    pub(crate) fn remove(&mut self, index: usize, generation: u32) -> Option<T> {
        let slot = self.slots.get_mut(index)?;
        if slot.generation != generation {
            return None;
        }
        let value = slot.value.take();
        if value.is_some() {
            slot.generation = generation.wrapping_add(1);
            self.free.push(index as u32);
        }
        value
    }

    /// 排空全部存活资源（Drop / 设备销毁路径）。
    pub(crate) fn drain(&mut self) -> Vec<T> {
        self.slots
            .iter_mut()
            .filter_map(|slot| slot.value.take())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_handle_is_invalid_and_slot_reuse_bumps_generation() {
        let mut table = SlotTable::new();
        let (index, generation) = table.insert("first");
        assert_eq!(table.get(index, generation), Some(&"first"));

        assert_eq!(table.remove(index, generation), Some("first"));
        assert_eq!(table.get(index, generation), None);
        assert!(table.has_free());

        let (reused_index, reused_generation) = table.insert("second");
        assert_eq!(reused_index, index);
        assert_eq!(reused_generation, generation.wrapping_add(1));
        assert_eq!(table.get(index, generation), None);
        assert_eq!(table.get(index, reused_generation), Some(&"second"));
    }

    #[test]
    fn stale_and_unknown_handles_are_noops() {
        let mut table = SlotTable::new();
        let (index, generation) = table.insert(7);
        assert_eq!(table.remove(index, generation.wrapping_add(1)), None);
        assert_eq!(table.remove(index + 10, generation), None);
        assert_eq!(table.get(index, generation), Some(&7));
        assert_eq!(table.drain(), vec![7]);
        assert_eq!(table.get(index, generation), None);
    }
}
