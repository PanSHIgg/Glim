//! 鼠标事件到终端报告序列的编码。
//!
//! 支持三种编码格式：SGR（`?1006`）、UTF-8（`?1005`）与经典 X10 编码。
//! 坐标使用网格坐标（行可为负；视口上方的事件不报告）。
//! 映射与 Alacritty / Zed 的终端保持一致
//! （参考 `crates/terminal/src/mappings/mouse.rs`）。

use crate::TerminalModes;
use crate::snapshot::GridPoint;

/// 参与的鼠标按键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    /// 左键。
    Left,
    /// 中键。
    Middle,
    /// 右键。
    Right,
}

/// 鼠标事件类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEventKind {
    /// 按下。
    Press,
    /// 释放。
    Release,
    /// 移动；`button` 为当前按住的按键。
    Move {
        /// 按住的按键；`None` 表示仅移动。
        button: Option<MouseButton>,
    },
    /// 滚轮向上。
    WheelUp,
    /// 滚轮向下。
    WheelDown,
    /// 滚轮向左。
    WheelLeft,
    /// 滚轮向右。
    WheelRight,
}

/// 鼠标事件携带的修饰键。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseModifiers {
    /// Shift。
    pub shift: bool,
    /// Alt。
    pub alt: bool,
    /// Ctrl。
    pub ctrl: bool,
}

/// 编码格式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MouseFormat {
    Sgr,
    /// 经典编码；参数为是否使用 UTF-8 坐标。
    Normal(bool),
}

impl MouseFormat {
    fn from_modes(modes: TerminalModes) -> Self {
        if modes.contains(TerminalModes::SGR_MOUSE) {
            MouseFormat::Sgr
        } else if modes.contains(TerminalModes::UTF8_MOUSE) {
            MouseFormat::Normal(true)
        } else {
            MouseFormat::Normal(false)
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ButtonCode {
    Left = 0,
    Middle = 1,
    Right = 2,
    LeftMove = 32,
    MiddleMove = 33,
    RightMove = 34,
    NoButtonMove = 35,
    WheelUp = 64,
    WheelDown = 65,
    WheelLeft = 66,
    WheelRight = 67,
}

impl ButtonCode {
    fn is_wheel(self) -> bool {
        matches!(
            self,
            Self::WheelUp | Self::WheelDown | Self::WheelLeft | Self::WheelRight
        )
    }
}

/// 把一次鼠标事件编码为要写入 PTY 的字节。
///
/// 事件的坐标是网格坐标；返回 `None` 表示当前模式不报告该事件。
pub fn encode_mouse_event(
    kind: MouseEventKind,
    button: Option<MouseButton>,
    point: GridPoint,
    modifiers: MouseModifiers,
    modes: TerminalModes,
) -> Option<Vec<u8>> {
    let code = match kind {
        MouseEventKind::Press => button_code(button?)?,
        MouseEventKind::Release => button_code(button?)?,
        MouseEventKind::WheelUp => ButtonCode::WheelUp,
        MouseEventKind::WheelDown => ButtonCode::WheelDown,
        MouseEventKind::WheelLeft => ButtonCode::WheelLeft,
        MouseEventKind::WheelRight => ButtonCode::WheelRight,
        MouseEventKind::Move { button } => match button {
            Some(button) => move_button_code(button),
            None => ButtonCode::NoButtonMove,
        },
    };

    let reportable = match kind {
        MouseEventKind::Move { button } => match button {
            // 拖动模式只报告按住按键的移动；移动模式报告所有移动。
            Some(_) => modes.intersects(TerminalModes::MOUSE_DRAG | TerminalModes::MOUSE_MOTION),
            None => modes.contains(TerminalModes::MOUSE_MOTION),
        },
        MouseEventKind::Press | MouseEventKind::Release => modes.mouse_reporting(),
        MouseEventKind::WheelUp
        | MouseEventKind::WheelDown
        | MouseEventKind::WheelLeft
        | MouseEventKind::WheelRight => modes.mouse_reporting(),
    };
    if !reportable {
        return None;
    }

    let format = MouseFormat::from_modes(modes);
    if code.is_wheel() {
        // 滚轮只有“按下”形式。
        return mouse_report(point, code as u8 + modifier_bits(modifiers), true, format);
    }
    match kind {
        MouseEventKind::Press | MouseEventKind::Move { .. } => {
            mouse_report(point, code as u8 + modifier_bits(modifiers), true, format)
        }
        MouseEventKind::Release => {
            mouse_report(point, code as u8 + modifier_bits(modifiers), false, format)
        }
        _ => None,
    }
}

fn button_code(button: MouseButton) -> Option<ButtonCode> {
    Some(match button {
        MouseButton::Left => ButtonCode::Left,
        MouseButton::Middle => ButtonCode::Middle,
        MouseButton::Right => ButtonCode::Right,
    })
}

fn move_button_code(button: MouseButton) -> ButtonCode {
    match button {
        MouseButton::Left => ButtonCode::LeftMove,
        MouseButton::Middle => ButtonCode::MiddleMove,
        MouseButton::Right => ButtonCode::RightMove,
    }
}

fn modifier_bits(modifiers: MouseModifiers) -> u8 {
    let mut bits = 0;
    if modifiers.shift {
        bits += 4;
    }
    if modifiers.alt {
        bits += 8;
    }
    if modifiers.ctrl {
        bits += 16;
    }
    bits
}

fn mouse_report(
    point: GridPoint,
    button: u8,
    pressed: bool,
    format: MouseFormat,
) -> Option<Vec<u8>> {
    if point.line < 0 {
        return None;
    }

    match format {
        MouseFormat::Sgr => Some(sgr_mouse_report(point, button, pressed).into_bytes()),
        MouseFormat::Normal(utf8) => {
            if pressed {
                normal_mouse_report(point, button, utf8)
            } else {
                normal_mouse_report(point, 3 + button, utf8)
            }
        }
    }
}

fn normal_mouse_report(point: GridPoint, button: u8, utf8: bool) -> Option<Vec<u8>> {
    let max_point = if utf8 { 2015 } else { 223 };
    if point.line >= max_point || point.column >= max_point as usize {
        return None;
    }

    let mut message = vec![b'\x1b', b'[', b'M', 32 + button];
    if utf8 && point.column >= 95 {
        message.extend_from_slice(&utf8_coordinate(point.column));
    } else {
        message.push(32 + 1 + point.column as u8);
    }
    if utf8 && point.line >= 95 {
        message.extend_from_slice(&utf8_coordinate(point.line as usize));
    } else {
        message.push(32 + 1 + point.line as u8);
    }
    Some(message)
}

fn utf8_coordinate(position: usize) -> [u8; 2] {
    let position = 32 + 1 + position;
    [0xC0 + (position / 64) as u8, 0x80 + (position & 63) as u8]
}

fn sgr_mouse_report(point: GridPoint, button: u8, pressed: bool) -> String {
    let final_char = if pressed { 'M' } else { 'm' };
    format!(
        "\x1b[<{};{};{}{}",
        button,
        point.column + 1,
        point.line + 1,
        final_char
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(line: i32, column: usize) -> GridPoint {
        GridPoint::new(line, column)
    }

    fn encode(kind: MouseEventKind, modes: TerminalModes) -> Option<String> {
        encode_mouse_event(
            kind,
            Some(MouseButton::Left),
            point(3, 7),
            MouseModifiers::default(),
            modes,
        )
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    const CLICK: TerminalModes = TerminalModes::MOUSE_REPORT_CLICK;
    const SGR_CLICK: TerminalModes =
        TerminalModes::MOUSE_REPORT_CLICK.union(TerminalModes::SGR_MOUSE);

    #[test]
    fn sgr_press_and_release() {
        assert_eq!(
            encode(MouseEventKind::Press, SGR_CLICK).as_deref(),
            Some("\x1b[<0;8;4M")
        );
        assert_eq!(
            encode(MouseEventKind::Release, SGR_CLICK).as_deref(),
            Some("\x1b[<0;8;4m")
        );
    }

    #[test]
    fn classic_press_and_release() {
        assert_eq!(
            encode(MouseEventKind::Press, CLICK).as_deref(),
            Some("\x1b[M\u{20}\u{28}\u{24}")
        );
        // 释放使用按键码 3（无按键）。
        assert_eq!(
            encode(MouseEventKind::Release, CLICK).as_deref(),
            Some("\x1b[M\u{23}\u{28}\u{24}")
        );
    }

    #[test]
    fn modifiers_add_bits() {
        let modifiers = MouseModifiers {
            shift: true,
            alt: true,
            ctrl: true,
        };
        let bytes = encode_mouse_event(
            MouseEventKind::Press,
            Some(MouseButton::Right),
            point(0, 0),
            modifiers,
            SGR_CLICK,
        )
        .expect("mouse reporting enabled");
        let text = String::from_utf8_lossy(&bytes);
        // 右键 = 2；修饰位 4 + 8 + 16。
        assert_eq!(text, "\x1b[<30;1;1M");
    }

    #[test]
    fn wheel_repeats_only_in_mouse_mode() {
        assert_eq!(
            encode(MouseEventKind::WheelUp, CLICK).as_deref(),
            Some("\x1b[M\u{60}\u{28}\u{24}")
        );
        assert_eq!(
            encode(MouseEventKind::WheelUp, TerminalModes::empty()),
            None
        );
    }

    #[test]
    fn motion_respects_reporting_mode() {
        let motion = MouseEventKind::Move { button: None };
        assert_eq!(encode(motion, CLICK), None);
        let dragged = MouseEventKind::Move {
            button: Some(MouseButton::Left),
        };
        assert_eq!(encode(dragged, CLICK), None);
        assert_eq!(
            encode(dragged, TerminalModes::MOUSE_DRAG).as_deref(),
            Some("\x1b[M\u{40}\u{28}\u{24}")
        );
        assert_eq!(
            encode(motion, TerminalModes::MOUSE_MOTION).as_deref(),
            Some("\x1b[M\u{43}\u{28}\u{24}")
        );
    }

    #[test]
    fn utf8_encoding_for_large_coordinates() {
        let modes = TerminalModes::MOUSE_REPORT_CLICK.union(TerminalModes::UTF8_MOUSE);
        let bytes = encode_mouse_event(
            MouseEventKind::Press,
            Some(MouseButton::Left),
            point(100, 100),
            MouseModifiers::default(),
            modes,
        )
        .expect("mouse reporting enabled");
        assert_eq!(bytes, vec![0x1b, b'[', b'M', 32, 0xC2, 0x85, 0xC2, 0x85]);
    }

    #[test]
    fn negative_line_is_not_reported() {
        let bytes = encode_mouse_event(
            MouseEventKind::Press,
            Some(MouseButton::Left),
            point(-1, 4),
            MouseModifiers::default(),
            SGR_CLICK,
        );
        assert_eq!(bytes, None);
    }
}
