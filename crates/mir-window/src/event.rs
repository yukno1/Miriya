//! 窗口事件：平台无关的事件类型与键位映射。
//!
//! winit 类型只在本文件内出现，引擎其他层只消费 [`WindowEvent`]。

use mir_input::{KeyCode, MouseButton};

/// 按键/按钮状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Pressed,
    Released,
}

/// 引擎侧的窗口事件（不含任何 winit 类型）。
#[derive(Debug, Clone)]
pub enum WindowEvent {
    /// 用户请求关闭窗口（点击关闭按钮 / Alt+F4）。
    CloseRequested,
    /// 窗口尺寸变化（物理像素）。
    Resized { width: u32, height: u32 },
    /// 键盘物理键状态变化。
    Keyboard { key: KeyCode, state: ButtonState },
    /// 鼠标移动（窗口坐标，左上角为原点）。
    CursorMoved { x: f32, y: f32 },
    /// 原始鼠标位移（不受窗口边界/系统加速度限制；单位由平台决定）。
    ///
    /// 用于视角控制；平台层会无条件转发，是否消费由应用层决定。
    MouseMotion { dx: f32, dy: f32 },
    /// 鼠标按钮状态变化。
    MouseButton {
        button: MouseButton,
        state: ButtonState,
    },
    /// 窗口获得/失去焦点。
    Focused(bool),
}

/// 把 winit 的物理键映射到引擎键位；不认识的键返回 [`KeyCode::Unknown`]。
pub(crate) fn map_physical_key(key: winit::keyboard::PhysicalKey) -> KeyCode {
    use winit::keyboard::KeyCode as Wkc;
    let winit::keyboard::PhysicalKey::Code(code) = key else {
        return KeyCode::Unknown;
    };
    match code {
        Wkc::KeyA => KeyCode::A,
        Wkc::KeyB => KeyCode::B,
        Wkc::KeyC => KeyCode::C,
        Wkc::KeyD => KeyCode::D,
        Wkc::KeyE => KeyCode::E,
        Wkc::KeyF => KeyCode::F,
        Wkc::KeyG => KeyCode::G,
        Wkc::KeyH => KeyCode::H,
        Wkc::KeyI => KeyCode::I,
        Wkc::KeyJ => KeyCode::J,
        Wkc::KeyK => KeyCode::K,
        Wkc::KeyL => KeyCode::L,
        Wkc::KeyM => KeyCode::M,
        Wkc::KeyN => KeyCode::N,
        Wkc::KeyO => KeyCode::O,
        Wkc::KeyP => KeyCode::P,
        Wkc::KeyQ => KeyCode::Q,
        Wkc::KeyR => KeyCode::R,
        Wkc::KeyS => KeyCode::S,
        Wkc::KeyT => KeyCode::T,
        Wkc::KeyU => KeyCode::U,
        Wkc::KeyV => KeyCode::V,
        Wkc::KeyW => KeyCode::W,
        Wkc::KeyX => KeyCode::X,
        Wkc::KeyY => KeyCode::Y,
        Wkc::KeyZ => KeyCode::Z,
        Wkc::Digit0 => KeyCode::Digit0,
        Wkc::Digit1 => KeyCode::Digit1,
        Wkc::Digit2 => KeyCode::Digit2,
        Wkc::Digit3 => KeyCode::Digit3,
        Wkc::Digit4 => KeyCode::Digit4,
        Wkc::Digit5 => KeyCode::Digit5,
        Wkc::Digit6 => KeyCode::Digit6,
        Wkc::Digit7 => KeyCode::Digit7,
        Wkc::Digit8 => KeyCode::Digit8,
        Wkc::Digit9 => KeyCode::Digit9,
        Wkc::F1 => KeyCode::F1,
        Wkc::F2 => KeyCode::F2,
        Wkc::F3 => KeyCode::F3,
        Wkc::F4 => KeyCode::F4,
        Wkc::F5 => KeyCode::F5,
        Wkc::F6 => KeyCode::F6,
        Wkc::F7 => KeyCode::F7,
        Wkc::F8 => KeyCode::F8,
        Wkc::F9 => KeyCode::F9,
        Wkc::F10 => KeyCode::F10,
        Wkc::F11 => KeyCode::F11,
        Wkc::F12 => KeyCode::F12,
        Wkc::Escape => KeyCode::Escape,
        Wkc::Space => KeyCode::Space,
        Wkc::Enter => KeyCode::Enter,
        Wkc::Tab => KeyCode::Tab,
        Wkc::Backspace => KeyCode::Backspace,
        Wkc::Delete => KeyCode::Delete,
        Wkc::Insert => KeyCode::Insert,
        Wkc::Home => KeyCode::Home,
        Wkc::End => KeyCode::End,
        Wkc::PageUp => KeyCode::PageUp,
        Wkc::PageDown => KeyCode::PageDown,
        Wkc::ShiftLeft => KeyCode::ShiftLeft,
        Wkc::ShiftRight => KeyCode::ShiftRight,
        Wkc::ControlLeft => KeyCode::ControlLeft,
        Wkc::ControlRight => KeyCode::ControlRight,
        Wkc::AltLeft => KeyCode::AltLeft,
        Wkc::AltRight => KeyCode::AltRight,
        Wkc::ArrowUp => KeyCode::ArrowUp,
        Wkc::ArrowDown => KeyCode::ArrowDown,
        Wkc::ArrowLeft => KeyCode::ArrowLeft,
        Wkc::ArrowRight => KeyCode::ArrowRight,
        _ => KeyCode::Unknown,
    }
}

pub(crate) fn map_mouse_button(button: winit::event::MouseButton) -> Option<MouseButton> {
    match button {
        winit::event::MouseButton::Left => Some(MouseButton::Left),
        winit::event::MouseButton::Right => Some(MouseButton::Right),
        winit::event::MouseButton::Middle => Some(MouseButton::Middle),
        _ => None,
    }
}

pub(crate) fn map_element_state(state: winit::event::ElementState) -> ButtonState {
    match state {
        winit::event::ElementState::Pressed => ButtonState::Pressed,
        winit::event::ElementState::Released => ButtonState::Released,
    }
}
