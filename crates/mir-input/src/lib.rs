//! 输入子系统：键盘与鼠标状态的统一管理。

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Unknown,
    W,
    A,
    S,
    D,
    Space,
    Escape,
    // TODO: 补全按键映射
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// 单帧输入状态，由平台层（窗口事件）写入、主循环每帧清理。
#[derive(Debug, Default)]
pub struct InputState {
    pressed: std::collections::HashSet<KeyCode>,
    released: std::collections::HashSet<KeyCode>,
    held: std::collections::HashSet<KeyCode>,
    mouse_pos: mir_math::Vec2,
    mouse_buttons: std::collections::HashSet<MouseButton>,
}

impl InputState {
    /// 平台层调用：记录按键按下。
    pub fn press(&mut self, key: KeyCode) {
        self.pressed.insert(key);
        self.held.insert(key);
    }

    /// 平台层调用：记录按键抬起。
    pub fn release(&mut self, key: KeyCode) {
        self.released.insert(key);
        self.held.remove(&key);
    }

    /// 本帧刚按下。
    pub fn just_pressed(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key)
    }

    /// 本帧刚抬起。
    pub fn just_released(&self, key: KeyCode) -> bool {
        self.released.contains(&key)
    }

    /// 按住不放。
    pub fn is_down(&self, key: KeyCode) -> bool {
        self.held.contains(&key)
    }

    /// 鼠标位置（窗口坐标）。
    pub fn mouse_pos(&self) -> mir_math::Vec2 {
        self.mouse_pos
    }

    /// 平台层调用：更新鼠标位置。
    pub fn set_mouse_pos(&mut self, pos: mir_math::Vec2) {
        self.mouse_pos = pos;
    }

    /// 主循环每帧结束时清理"按下沿/抬起沿"。
    pub fn end_frame(&mut self) {
        self.pressed.clear();
        self.released.clear();
    }
}
