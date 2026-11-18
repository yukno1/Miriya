//! 渲染子系统。
//!
//! 目前是占位实现；接入 vulkanalia 时在这里定义 `RenderBackend`
//! trait，把具体图形 API 隔离在引擎内部。

/// 渲染上下文：持有后端状态，由主循环持有。
pub struct RenderContext {
    pub exit_requested: bool,
    initialized: bool,
}

impl RenderContext {
    pub fn new(_width: u32, _height: u32) -> Self {
        Self {
            exit_requested: false,
            initialized: false,
        }
    }

    /// 初始化渲染后端（创建窗口、图形设备等）。
    pub fn init(&mut self) {
        self.initialized = true;
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// 清屏并开始本帧渲染。
    pub fn begin_frame(&mut self, clear_color: [f32; 4]) {
        debug_assert!(self.initialized);
        // TODO: 接入 vulkanalia 后在这里执行 pass 开始逻辑
        let _ = clear_color;
    }

    /// 结束本帧渲染并呈现。
    pub fn end_frame(&mut self) {
        debug_assert!(self.initialized);
        // TODO: 提交命令缓冲、swap chain present
    }
}
