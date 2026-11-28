//! GPU pass 时间戳采集与异步回读（wgpu 30）。
//!
//! 设备支持 `Features::TIMESTAMP_QUERY` 时，每个 graphics pass 在
//! `RenderPassDescriptor::timestamp_writes` 写入一对查询；帧末解析到常驻缓冲，
//! 再复制到双缓冲读回槽并 `map_async`。回读回调由 `Device::poll(PollType::Poll)`
//! 驱动，因此不会阻塞渲染线程；没有空闲槽时跳过该帧采样。
//!
//! 时间戳原始值须乘 `Queue::get_timestamp_period()` 才是纳秒（见 wgpu 文档）。

use std::sync::mpsc::{self, Receiver, Sender};

use wgpu::{
    Buffer, BufferAddress, BufferDescriptor, BufferUsages, CommandEncoder, Device, MapMode,
    PollType, QUERY_RESOLVE_BUFFER_ALIGNMENT, QUERY_SIZE, QuerySet, QuerySetDescriptor, QueryType,
    Queue, RenderPassTimestampWrites,
};

/// 每帧最多计时的 pass 数（与 Vulkan 的 `TIMESTAMP_QUERY_COUNT = 64` 同量级）。
pub(crate) const MAX_TIMED_PASSES: usize = 32;

const QUERY_COUNT: u32 = (2 * MAX_TIMED_PASSES) as u32;
const READBACK_SLOTS: usize = 2;

/// 读回槽状态；仅 `Idle` 槽可承接新一帧的复制与映射。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotState {
    Idle,
    Pending,
}

type ReadbackResult = Result<Vec<u8>, ()>;

/// wgpu 时间戳采集器。`None` 表示设备不支持或 period 无效。
pub(crate) struct TimestampCapture {
    query_set: QuerySet,
    resolve_buffer: Buffer,
    readback: Vec<Buffer>,
    states: Vec<SlotState>,
    pass_names: Vec<Vec<String>>,
    tx: Sender<(usize, ReadbackResult)>,
    rx: Receiver<(usize, ReadbackResult)>,
    timestamp_period: f32,
    current_names: Vec<String>,
    last_pass_timings: Vec<(String, f32)>,
}

impl TimestampCapture {
    /// 创建查询集与常驻缓冲。调用方需保证设备已启用 `Features::TIMESTAMP_QUERY`
    /// 且 `period > 0`。
    pub(crate) fn new(device: &Device, timestamp_period: f32) -> Self {
        let buffer_size = aligned_buffer_size();
        let query_set = device.create_query_set(&QuerySetDescriptor {
            label: Some("wgpu pass timestamps"),
            ty: QueryType::Timestamp,
            count: QUERY_COUNT,
        });
        let resolve_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("wgpu timestamp resolve"),
            size: buffer_size,
            usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let mut readback = Vec::with_capacity(READBACK_SLOTS);
        for _ in 0..READBACK_SLOTS {
            readback.push(device.create_buffer(&BufferDescriptor {
                label: Some("wgpu timestamp readback"),
                size: buffer_size,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let (tx, rx) = mpsc::channel();
        Self {
            query_set,
            resolve_buffer,
            readback,
            states: vec![SlotState::Idle; READBACK_SLOTS],
            pass_names: vec![Vec::new(); READBACK_SLOTS],
            tx,
            rx,
            timestamp_period,
            current_names: Vec::new(),
            last_pass_timings: Vec::new(),
        }
    }

    /// 诊断缓冲的常驻字节数，纳入 `RenderContext::allocated_bytes()` 口径。
    pub(crate) fn allocated_bytes(&self) -> u64 {
        (1 + self.readback.len()) as u64 * aligned_buffer_size()
    }

    pub(crate) fn last_pass_timings(&self) -> &[(String, f32)] {
        &self.last_pass_timings
    }

    /// 本帧 pass 数在查询容量内时才可计时。
    pub(crate) fn can_time(&self, pass_count: usize) -> bool {
        pass_count > 0 && pass_count <= MAX_TIMED_PASSES
    }

    /// 单个 pass 的时间戳写入点；超出容量或计数为 0 时返回 `None`。
    pub(crate) fn timestamp_writes(&self, index: usize) -> Option<RenderPassTimestampWrites<'_>> {
        if index >= MAX_TIMED_PASSES {
            return None;
        }
        Some(RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some((2 * index) as u32),
            end_of_pass_write_index: Some((2 * index + 1) as u32),
        })
    }

    /// 记住本帧 pass 名，供回读结果与 `FrameStats` 对齐。
    pub(crate) fn set_pass_names(&mut self, names: Vec<String>) {
        self.current_names = names;
    }

    /// 帧首调用：非阻塞地执行已完成的映射回调，并把最近一次结果换算为毫秒。
    pub(crate) fn poll_results(&mut self, device: &Device) {
        let _ = device.poll(PollType::Poll);
        while let Ok((slot, result)) = self.rx.try_recv() {
            if self.states.get(slot) != Some(&SlotState::Pending) {
                continue;
            }
            self.states[slot] = SlotState::Idle;
            match result {
                Ok(bytes) => {
                    let names = std::mem::take(&mut self.pass_names[slot]);
                    if let Some(timings) = decode_timings(&bytes, &names, self.timestamp_period) {
                        self.last_pass_timings = timings;
                    }
                }
                Err(()) => {
                    mir_log::warn!(target: "wgpu", "读取 GPU pass 时间戳失败，本次采样已丢弃");
                }
            }
        }
    }

    /// 解析查询、复制到空闲读回槽、提交命令并启动异步映射。
    /// `timed` 为 false 时只做普通提交，行为与未启用时间戳时一致。
    pub(crate) fn finish_and_submit(
        &mut self,
        encoder: CommandEncoder,
        queue: &Queue,
        pass_count: usize,
        timed: bool,
    ) {
        let mut encoder = encoder;
        let slot = if timed && self.can_time(pass_count) {
            next_free_slot(&self.states)
        } else {
            None
        };
        if let Some(slot) = slot {
            encoder.resolve_query_set(
                &self.query_set,
                0..(2 * pass_count) as u32,
                &self.resolve_buffer,
                0,
            );
            encoder.copy_buffer_to_buffer(
                &self.resolve_buffer,
                0,
                &self.readback[slot],
                0,
                aligned_buffer_size(),
            );
            self.states[slot] = SlotState::Pending;
            self.pass_names[slot] = self.current_names.clone();
        }
        queue.submit([encoder.finish()]);
        if let Some(slot) = slot {
            let buffer = self.readback[slot].clone();
            let mapped_buffer = buffer.clone();
            let tx = self.tx.clone();
            buffer.map_async(MapMode::Read, .., move |result| {
                let payload = result.map_err(|_| ()).and_then(|_| {
                    let view = mapped_buffer.get_mapped_range(..).map_err(|_| ())?;
                    let bytes = view.to_vec();
                    drop(view);
                    mapped_buffer.unmap();
                    Ok(bytes)
                });
                let _ = tx.send((slot, payload));
            });
        }
    }
}

/// Resolve 缓冲按 `QUERY_RESOLVE_BUFFER_ALIGNMENT` 对齐。
fn aligned_buffer_size() -> BufferAddress {
    let raw = u64::from(QUERY_SIZE) * u64::from(QUERY_COUNT);
    raw.div_ceil(QUERY_RESOLVE_BUFFER_ALIGNMENT) * QUERY_RESOLVE_BUFFER_ALIGNMENT
}

fn next_free_slot(states: &[SlotState]) -> Option<usize> {
    states.iter().position(|state| *state == SlotState::Idle)
}

/// 原始 tick 差 → 毫秒；period 为每 tick 的纳秒数。
fn ticks_to_ms(start: u64, end: u64, period: f32) -> f32 {
    (end.saturating_sub(start) as f64 * f64::from(period) / 1_000_000.0) as f32
}

fn decode_timings(bytes: &[u8], names: &[String], period: f32) -> Option<Vec<(String, f32)>> {
    if bytes.len() < QUERY_SIZE as usize * 2 * names.len() {
        return None;
    }
    let ticks: Vec<u64> = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| u64::from_le_bytes(*chunk))
        .collect();
    Some(
        names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                (
                    name.clone(),
                    ticks_to_ms(ticks[2 * index], ticks[2 * index + 1], period),
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_to_ms_applies_period_and_saturates() {
        assert_eq!(ticks_to_ms(100, 2_000_100, 0.5), 1.0);
        // 结束值异常小于起始值时按 0 处理，避免出现负耗时。
        assert_eq!(ticks_to_ms(2_000_100, 100, 0.5), 0.0);
    }

    #[test]
    fn decode_timings_pairs_pass_names_with_durations() {
        let names = vec!["main_geometry".to_string(), "tonemap".to_string()];
        let ticks = [0_u64, 2_000_000, 10_000_000, 13_000_000];
        let bytes: Vec<u8> = ticks.iter().flat_map(|tick| tick.to_le_bytes()).collect();
        let timings = decode_timings(&bytes, &names, 0.5).expect("字节数足够");
        assert_eq!(timings.len(), 2);
        assert_eq!(timings[0].0, "main_geometry");
        assert_eq!(timings[0].1, 1.0);
        assert_eq!(timings[1].0, "tonemap");
        assert_eq!(timings[1].1, 1.5);
    }

    #[test]
    fn decode_timings_rejects_truncated_payload() {
        assert!(decode_timings(&[0_u8; 8], &["a".to_string()], 1.0).is_none());
    }

    #[test]
    fn slot_rotation_stops_when_both_are_pending() {
        let mut states = vec![SlotState::Idle; READBACK_SLOTS];
        assert_eq!(next_free_slot(&states), Some(0));
        states[0] = SlotState::Pending;
        assert_eq!(next_free_slot(&states), Some(1));
        states[1] = SlotState::Pending;
        assert_eq!(next_free_slot(&states), None);
    }

    #[test]
    fn resolve_buffer_is_256_byte_aligned() {
        assert_eq!(aligned_buffer_size() % QUERY_RESOLVE_BUFFER_ALIGNMENT, 0);
        assert!(aligned_buffer_size() >= u64::from(QUERY_SIZE) * u64::from(QUERY_COUNT));
    }
}
