//! 时间系统：帧间隔、运行时长、帧统计。

use std::time::Instant;

/// 引擎时间信息，每帧由主循环更新。
#[derive(Debug, Clone)]
pub struct Time {
    start: Instant,
    last_frame: Instant,
    delta: f32,
    elapsed: f32,
}

impl Time {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            last_frame: now,
            delta: 0.0,
            elapsed: 0.0,
        }
    }

    /// 每帧开始时调用。
    pub fn update(&mut self) {
        let now = Instant::now();
        self.delta = (now - self.last_frame).as_secs_f32();
        self.elapsed = (now - self.start).as_secs_f32();
        self.last_frame = now;
    }

    /// 上一帧到本帧的时间间隔（秒），用于帧率无关的逻辑更新。
    pub fn delta_secs(&self) -> f32 {
        self.delta
    }

    /// 引擎启动至今的总时长（秒）。
    pub fn elapsed_secs(&self) -> f32 {
        self.elapsed
    }
}

impl Default for Time {
    fn default() -> Self {
        Self::new()
    }
}

/// 帧统计：帧数、平滑 FPS、上一帧耗时。
#[derive(Debug, Clone, Default)]
pub struct FrameStats {
    pub frames: u64,
    pub fps: f32,
    pub frame_time_ms: f32,
}

impl FrameStats {
    pub(crate) fn update(&mut self, dt: f32) {
        self.frames += 1;
        self.frame_time_ms = dt * 1000.0;
        if dt > 0.0 {
            let instant = 1.0 / dt;
            self.fps = if self.fps == 0.0 {
                instant
            } else {
                self.fps * 0.9 + instant * 0.1
            };
        }
    }
}
