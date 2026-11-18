//! 窗口抽象。
//!
//! 接入 winit 后，窗口事件在这里翻译成引擎
//! 的 `InputState` 更新与退出请求。

mod event;

/// 平台窗口的薄封装。
pub struct Window {
    title: String,
    width: u32,
    height: u32,
}

impl Window {
    pub fn new(title: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            title: title.into(),
            width,
            height,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
