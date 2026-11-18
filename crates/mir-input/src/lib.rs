//! 输入子系统：键盘与鼠标状态的统一管理。
//!
//! 平台层（mir-window）把窗口事件翻译成 [`InputState`] 更新；
//! 主循环每帧调用 [`InputState::end_frame`] 清理"按下/抬起沿"。

use std::collections::HashSet;

/// 物理键位（按键盘位置，不随布局变化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Unknown,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Escape,
    Space,
    Enter,
    Tab,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// 单帧输入状态，由平台层（窗口事件）写入、主循环每帧清理沿。
#[derive(Debug, Clone, Default)]
pub struct InputState {
    pressed: HashSet<KeyCode>,
    released: HashSet<KeyCode>,
    held: HashSet<KeyCode>,
    mouse_pos: mir_math::Vec2,
    mouse_buttons: HashSet<MouseButton>,
    pressed_mouse: HashSet<MouseButton>,
    released_mouse: HashSet<MouseButton>,
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

    /// 鼠标位置（窗口坐标，左上角为原点）。
    pub fn mouse_pos(&self) -> mir_math::Vec2 {
        self.mouse_pos
    }

    /// 平台层调用：更新鼠标位置。
    pub fn set_mouse_pos(&mut self, pos: mir_math::Vec2) {
        self.mouse_pos = pos;
    }

    /// 平台层调用：记录鼠标按钮按下。
    pub fn press_mouse(&mut self, button: MouseButton) {
        self.pressed_mouse.insert(button);
        self.mouse_buttons.insert(button);
    }

    /// 平台层调用：记录鼠标按钮抬起。
    pub fn release_mouse(&mut self, button: MouseButton) {
        self.released_mouse.insert(button);
        self.mouse_buttons.remove(&button);
    }

    /// 鼠标按钮按住不放。
    pub fn is_mouse_down(&self, button: MouseButton) -> bool {
        self.mouse_buttons.contains(&button)
    }

    /// 本帧鼠标按钮刚按下。
    pub fn just_mouse_pressed(&self, button: MouseButton) -> bool {
        self.pressed_mouse.contains(&button)
    }

    /// 本帧鼠标按钮刚抬起。
    pub fn just_mouse_released(&self, button: MouseButton) -> bool {
        self.released_mouse.contains(&button)
    }

    /// 主循环每帧结束时清理"按下沿/抬起沿"。
    pub fn end_frame(&mut self) {
        self.pressed.clear();
        self.released.clear();
        self.pressed_mouse.clear();
        self.released_mouse.clear();
    }

    /// 丢弃所有状态（窗口失焦时调用，避免按键卡住）。
    pub fn clear(&mut self) {
        *self = Self {
            mouse_pos: self.mouse_pos,
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_edges_and_held() {
        let mut input = InputState::default();
        input.press(KeyCode::W);
        assert!(input.just_pressed(KeyCode::W));
        assert!(input.is_down(KeyCode::W));
        input.end_frame();
        assert!(!input.just_pressed(KeyCode::W));
        assert!(input.is_down(KeyCode::W));
        input.release(KeyCode::W);
        assert!(input.just_released(KeyCode::W));
        assert!(!input.is_down(KeyCode::W));
    }

    #[test]
    fn mouse_edges() {
        let mut input = InputState::default();
        input.press_mouse(MouseButton::Left);
        assert!(input.is_mouse_down(MouseButton::Left));
        assert!(input.just_mouse_pressed(MouseButton::Left));
        input.end_frame();
        assert!(!input.just_mouse_pressed(MouseButton::Left));
        input.release_mouse(MouseButton::Left);
        assert!(input.just_mouse_released(MouseButton::Left));
        assert!(!input.is_mouse_down(MouseButton::Left));
    }

    #[test]
    fn clear_drops_held_keys() {
        let mut input = InputState::default();
        input.press(KeyCode::Space);
        input.clear();
        assert!(!input.is_down(KeyCode::Space));
    }
}
