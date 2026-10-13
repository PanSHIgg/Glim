//! 按键编码：把按键与终端模式转换为写入 PTY 的字节序列。
//!
//! 映射遵循 xterm 的常见约定，与 Zed 终端采用的 Alacritty 映射保持一致
//! （参考 `crates/terminal/src/mappings/keys.rs`）。

use bitflags::bitflags;

bitflags! {
    /// 由终端输出序列切换的应用模式；影响按键编码、鼠标报告与渲染。
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct TerminalModes: u16 {
        /// 应用光标模式（DECCKM）：方向键使用 SS3 序列。
        const APP_CURSOR = 1 << 0;
        /// 应用小键盘模式（DECKPAM）。
        const APP_KEYPAD = 1 << 1;
        /// 括号粘贴模式（bracketed paste）。
        const BRACKETED_PASTE = 1 << 2;
        /// 备用屏幕处于活动状态。
        const ALT_SCREEN = 1 << 3;
        /// 焦点进入 / 离开时报告。
        const FOCUS_IN_OUT = 1 << 4;
        /// 按钮按下 / 释放报告（`?1000`）。
        const MOUSE_REPORT_CLICK = 1 << 5;
        /// 拖动报告（`?1002`）。
        const MOUSE_DRAG = 1 << 6;
        /// 任意移动报告（`?1003`）。
        const MOUSE_MOTION = 1 << 7;
        /// SGR 鼠标编码（`?1006`）。
        const SGR_MOUSE = 1 << 8;
        /// UTF-8 鼠标编码（`?1005`）。
        const UTF8_MOUSE = 1 << 9;
        /// 备用屏幕上滚轮发送方向键（alternate scroll）。
        const ALTERNATE_SCROLL = 1 << 10;
    }
}

impl TerminalModes {
    /// 终端是否请求鼠标报告（三种报告模式的任意一种）。
    pub fn mouse_reporting(self) -> bool {
        self.intersects(Self::MOUSE_REPORT_CLICK | Self::MOUSE_DRAG | Self::MOUSE_MOTION)
    }
}

/// 一次按键输入，与具体 UI 框架无关。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInput {
    /// 按键名：字母为 "a" / "A"，命名键为小写（"enter"、"up"、"f1"）。
    pub key: String,
    /// Ctrl 是否按下。
    pub ctrl: bool,
    /// Alt 是否按下。
    pub alt: bool,
    /// Shift 是否按下。
    pub shift: bool,
    /// Cmd / Win 是否按下。
    pub meta: bool,
}

/// 归类的修饰键组合。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Modifiers {
    None,
    Alt,
    Ctrl,
    Shift,
    CtrlShift,
    Other,
}

impl Modifiers {
    fn any(self) -> bool {
        !matches!(self, Self::None)
    }
}

impl KeyInput {
    /// 编码为要写入 PTY 的字节；无需写入时返回 `None`。
    pub fn to_bytes(&self, modes: TerminalModes) -> Option<Vec<u8>> {
        let modifiers = self.modifier_kind();

        if let Some(sequence) = self.manual_sequence(modes, modifiers) {
            return Some(sequence.as_bytes().to_vec());
        }

        if modifiers.any()
            && let Some(sequence) = self.modified_sequence()
        {
            return Some(sequence.into_bytes());
        }

        self.alt_ascii_sequence(modifiers)
    }

    fn modifier_kind(&self) -> Modifiers {
        match (self.alt, self.ctrl, self.shift, self.meta) {
            (false, false, false, false) => Modifiers::None,
            (true, false, false, false) => Modifiers::Alt,
            (false, true, false, false) => Modifiers::Ctrl,
            (false, false, true, false) => Modifiers::Shift,
            (false, true, true, false) => Modifiers::CtrlShift,
            _ => Modifiers::Other,
        }
    }

    /// 基础特殊键与 Ctrl / Shift 组合的固定序列。
    fn manual_sequence(&self, modes: TerminalModes, modifiers: Modifiers) -> Option<&'static str> {
        use Modifiers as M;

        let key = self.key.as_str();
        let app_cursor = modes.contains(TerminalModes::APP_CURSOR);

        match (key, modifiers) {
            // 基础键
            ("tab", M::None) => Some("\x09"),
            ("escape", M::None) => Some("\x1b"),
            ("enter", M::None) => Some("\x0d"),
            ("enter", M::Shift) => Some("\x0a"),
            ("enter", M::Alt) => Some("\x1b\x0d"),
            ("backspace", M::None) => Some("\x7f"),
            ("tab", M::Shift) => Some("\x1b[Z"),
            ("backspace", M::Ctrl) => Some("\x08"),
            ("backspace", M::Alt) => Some("\x1b\x7f"),
            ("backspace", M::Shift) => Some("\x7f"),
            ("space", M::Ctrl) => Some("\x00"),
            // 方向与定位键
            ("home", M::None) if app_cursor => Some("\x1bOH"),
            ("home", M::None) => Some("\x1b[H"),
            ("end", M::None) if app_cursor => Some("\x1bOF"),
            ("end", M::None) => Some("\x1b[F"),
            ("up", M::None) if app_cursor => Some("\x1bOA"),
            ("up", M::None) => Some("\x1b[A"),
            ("down", M::None) if app_cursor => Some("\x1bOB"),
            ("down", M::None) => Some("\x1b[B"),
            ("right", M::None) if app_cursor => Some("\x1bOC"),
            ("right", M::None) => Some("\x1b[C"),
            ("left", M::None) if app_cursor => Some("\x1bOD"),
            ("left", M::None) => Some("\x1b[D"),
            // 编辑与翻页
            ("insert", M::None) => Some("\x1b[2~"),
            ("delete", M::None) => Some("\x1b[3~"),
            ("pageup", M::None) => Some("\x1b[5~"),
            ("pagedown", M::None) => Some("\x1b[6~"),
            // 功能键
            ("f1", M::None) => Some("\x1bOP"),
            ("f2", M::None) => Some("\x1bOQ"),
            ("f3", M::None) => Some("\x1bOR"),
            ("f4", M::None) => Some("\x1bOS"),
            ("f5", M::None) => Some("\x1b[15~"),
            ("f6", M::None) => Some("\x1b[17~"),
            ("f7", M::None) => Some("\x1b[18~"),
            ("f8", M::None) => Some("\x1b[19~"),
            ("f9", M::None) => Some("\x1b[20~"),
            ("f10", M::None) => Some("\x1b[21~"),
            ("f11", M::None) => Some("\x1b[23~"),
            ("f12", M::None) => Some("\x1b[24~"),
            ("f13", M::None) => Some("\x1b[25~"),
            ("f14", M::None) => Some("\x1b[26~"),
            ("f15", M::None) => Some("\x1b[28~"),
            ("f16", M::None) => Some("\x1b[29~"),
            ("f17", M::None) => Some("\x1b[31~"),
            ("f18", M::None) => Some("\x1b[32~"),
            ("f19", M::None) => Some("\x1b[33~"),
            ("f20", M::None) => Some("\x1b[34~"),
            // Ctrl 字母（大小写等价）
            ("a", M::Ctrl) | ("A", M::CtrlShift) => Some("\x01"),
            ("b", M::Ctrl) | ("B", M::CtrlShift) => Some("\x02"),
            ("c", M::Ctrl) | ("C", M::CtrlShift) => Some("\x03"),
            ("d", M::Ctrl) | ("D", M::CtrlShift) => Some("\x04"),
            ("e", M::Ctrl) | ("E", M::CtrlShift) => Some("\x05"),
            ("f", M::Ctrl) | ("F", M::CtrlShift) => Some("\x06"),
            ("g", M::Ctrl) | ("G", M::CtrlShift) => Some("\x07"),
            ("h", M::Ctrl) | ("H", M::CtrlShift) => Some("\x08"),
            ("i", M::Ctrl) | ("I", M::CtrlShift) => Some("\x09"),
            ("j", M::Ctrl) | ("J", M::CtrlShift) => Some("\x0a"),
            ("k", M::Ctrl) | ("K", M::CtrlShift) => Some("\x0b"),
            ("l", M::Ctrl) | ("L", M::CtrlShift) => Some("\x0c"),
            ("m", M::Ctrl) | ("M", M::CtrlShift) => Some("\x0d"),
            ("n", M::Ctrl) | ("N", M::CtrlShift) => Some("\x0e"),
            ("o", M::Ctrl) | ("O", M::CtrlShift) => Some("\x0f"),
            ("p", M::Ctrl) | ("P", M::CtrlShift) => Some("\x10"),
            ("q", M::Ctrl) | ("Q", M::CtrlShift) => Some("\x11"),
            ("r", M::Ctrl) | ("R", M::CtrlShift) => Some("\x12"),
            ("s", M::Ctrl) | ("S", M::CtrlShift) => Some("\x13"),
            ("t", M::Ctrl) | ("T", M::CtrlShift) => Some("\x14"),
            ("u", M::Ctrl) | ("U", M::CtrlShift) => Some("\x15"),
            ("v", M::Ctrl) | ("V", M::CtrlShift) => Some("\x16"),
            ("w", M::Ctrl) | ("W", M::CtrlShift) => Some("\x17"),
            ("x", M::Ctrl) | ("X", M::CtrlShift) => Some("\x18"),
            ("y", M::Ctrl) | ("Y", M::CtrlShift) => Some("\x19"),
            ("z", M::Ctrl) | ("Z", M::CtrlShift) => Some("\x1a"),
            // Ctrl 符号
            ("@", M::Ctrl) => Some("\x00"),
            ("[", M::Ctrl) => Some("\x1b"),
            ("\\", M::Ctrl) => Some("\x1c"),
            ("]", M::Ctrl) => Some("\x1d"),
            ("^", M::Ctrl) => Some("\x1e"),
            ("_", M::Ctrl) => Some("\x1f"),
            ("?", M::Ctrl) => Some("\x7f"),
            _ => None,
        }
    }

    /// 带修饰键的方向键、功能键等 CSI 序列。
    fn modified_sequence(&self) -> Option<String> {
        let code = self.modifier_code();
        let sequence = match self.key.as_str() {
            "up" => format!("\x1b[1;{code}A"),
            "down" => format!("\x1b[1;{code}B"),
            "right" => format!("\x1b[1;{code}C"),
            "left" => format!("\x1b[1;{code}D"),
            "f1" => format!("\x1b[1;{code}P"),
            "f2" => format!("\x1b[1;{code}Q"),
            "f3" => format!("\x1b[1;{code}R"),
            "f4" => format!("\x1b[1;{code}S"),
            "f5" => format!("\x1b[15;{code}~"),
            "f6" => format!("\x1b[17;{code}~"),
            "f7" => format!("\x1b[18;{code}~"),
            "f8" => format!("\x1b[19;{code}~"),
            "f9" => format!("\x1b[20;{code}~"),
            "f10" => format!("\x1b[21;{code}~"),
            "f11" => format!("\x1b[23;{code}~"),
            "f12" => format!("\x1b[24;{code}~"),
            "f13" => format!("\x1b[25;{code}~"),
            "f14" => format!("\x1b[26;{code}~"),
            "f15" => format!("\x1b[28;{code}~"),
            "f16" => format!("\x1b[29;{code}~"),
            "f17" => format!("\x1b[31;{code}~"),
            "f18" => format!("\x1b[32;{code}~"),
            "f19" => format!("\x1b[33;{code}~"),
            "f20" => format!("\x1b[34;{code}~"),
            "insert" => format!("\x1b[2;{code}~"),
            "pageup" => format!("\x1b[5;{code}~"),
            "pagedown" => format!("\x1b[6;{code}~"),
            "end" => format!("\x1b[1;{code}F"),
            "home" => format!("\x1b[1;{code}H"),
            _ => return None,
        };
        Some(sequence)
    }

    /// Alt 作为 Meta 前缀：Alt+字符、Alt+Shift+字符、Ctrl+Alt+字符。
    fn alt_ascii_sequence(&self, modifiers: Modifiers) -> Option<Vec<u8>> {
        if !self.alt || !self.key.is_ascii() || self.key.len() != 1 {
            return None;
        }
        let key = self.key.chars().next()?;

        if modifiers == Modifiers::Alt {
            return Some(format!("\x1b{key}").into_bytes());
        }
        if self.shift {
            return Some(format!("\x1b{}", key.to_ascii_uppercase()).into_bytes());
        }
        if self.ctrl && key.is_ascii_lowercase() {
            return Some(format!("\x1b{}", (key as u8 - b'a' + 1) as char).into_bytes());
        }
        None
    }

    /// xterm 修饰键编码：Shift=1、Alt=2、Ctrl=4，再 +1。
    ///
    /// 参考 <https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-PC-Style-Function-Keys>。
    fn modifier_code(&self) -> u32 {
        let mut code = 0;
        if self.shift {
            code |= 1;
        }
        if self.alt {
            code |= 1 << 1;
        }
        if self.ctrl {
            code |= 1 << 2;
        }
        code + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(key: &str, ctrl: bool, alt: bool, shift: bool) -> KeyInput {
        KeyInput {
            key: key.to_owned(),
            ctrl,
            alt,
            shift,
            meta: false,
        }
    }

    fn bytes(key: &KeyInput, modes: TerminalModes) -> Option<String> {
        key.to_bytes(modes)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    #[test]
    fn plain_special_keys() {
        let none = TerminalModes::empty();
        assert_eq!(
            bytes(&input("enter", false, false, false), none).as_deref(),
            Some("\r")
        );
        assert_eq!(
            bytes(&input("enter", false, false, true), none).as_deref(),
            Some("\n")
        );
        assert_eq!(
            bytes(&input("backspace", false, false, false), none).as_deref(),
            Some("\x7f")
        );
        assert_eq!(
            bytes(&input("tab", false, false, false), none).as_deref(),
            Some("\t")
        );
        assert_eq!(
            bytes(&input("escape", false, false, false), none).as_deref(),
            Some("\x1b")
        );
    }

    #[test]
    fn shift_tab_uses_named_key() {
        let none = TerminalModes::empty();
        assert_eq!(
            bytes(&input("tab", false, false, true), none).as_deref(),
            Some("\x1b[Z")
        );
    }

    #[test]
    fn application_cursor_mode() {
        let app_cursor = TerminalModes::APP_CURSOR;
        let none = TerminalModes::empty();

        assert_eq!(
            bytes(&input("up", false, false, false), none).as_deref(),
            Some("\x1b[A")
        );
        assert_eq!(
            bytes(&input("down", false, false, false), none).as_deref(),
            Some("\x1b[B")
        );
        assert_eq!(
            bytes(&input("right", false, false, false), none).as_deref(),
            Some("\x1b[C")
        );
        assert_eq!(
            bytes(&input("left", false, false, false), none).as_deref(),
            Some("\x1b[D")
        );

        assert_eq!(
            bytes(&input("up", false, false, false), app_cursor).as_deref(),
            Some("\x1bOA")
        );
        assert_eq!(
            bytes(&input("left", false, false, false), app_cursor).as_deref(),
            Some("\x1bOD")
        );

        assert_eq!(
            bytes(&input("home", false, false, false), none).as_deref(),
            Some("\x1b[H")
        );
        assert_eq!(
            bytes(&input("end", false, false, false), none).as_deref(),
            Some("\x1b[F")
        );
        assert_eq!(
            bytes(&input("home", false, false, false), app_cursor).as_deref(),
            Some("\x1bOH")
        );
    }

    #[test]
    fn ctrl_codes() {
        let none = TerminalModes::empty();
        let esc_codes: Vec<String> = ('\x01'..='\x1a').map(String::from).collect();
        let letters: Vec<char> = ('a'..='z').collect();

        for (letter, expected) in letters.iter().zip(esc_codes.iter()) {
            let lower = input(&letter.to_string(), true, false, false);
            assert_eq!(
                bytes(&lower, none).as_deref(),
                Some(expected.as_str()),
                "ctrl-{letter}"
            );

            let upper = input(&letter.to_ascii_uppercase().to_string(), true, false, true);
            assert_eq!(
                bytes(&upper, none).as_deref(),
                Some(expected.as_str()),
                "ctrl-shift-{letter}"
            );
        }
    }

    #[test]
    fn alt_is_meta() {
        let none = TerminalModes::empty();
        for character in ' '..='~' {
            let key = input(&character.to_string(), false, true, false);
            assert_eq!(
                bytes(&key, none).as_deref(),
                Some(format!("\x1b{character}").as_str()),
                "alt-{character}"
            );
        }
    }

    #[test]
    fn ctrl_alt_letters() {
        let none = TerminalModes::empty();
        for (offset, letter) in ('a'..='z').enumerate() {
            let key = input(&letter.to_string(), true, true, false);
            let escape = (offset as u8 + 1) as char;
            let expected = format!("\x1b{escape}");
            assert_eq!(
                bytes(&key, none).as_deref(),
                Some(expected.as_str()),
                "ctrl-alt-{letter}"
            );
        }
    }

    #[test]
    fn modified_arrows() {
        let none = TerminalModes::empty();
        assert_eq!(
            bytes(&input("up", false, false, true), none).as_deref(),
            Some("\x1b[1;2A")
        );
        assert_eq!(
            bytes(&input("home", false, false, true), none).as_deref(),
            Some("\x1b[1;2H")
        );
    }

    #[test]
    fn modifier_code_calculation() {
        assert_eq!(input("a", false, false, false).modifier_code(), 1);
        assert_eq!(input("a", false, false, true).modifier_code(), 2);
        assert_eq!(input("a", false, true, false).modifier_code(), 3);
        assert_eq!(input("a", true, false, false).modifier_code(), 5);
        assert_eq!(input("a", true, true, true).modifier_code(), 8);
    }

    #[test]
    fn ctrl_c_maps_to_interrupt() {
        let none = TerminalModes::empty();
        assert_eq!(
            bytes(&input("c", true, false, false), none).as_deref(),
            Some("\x03")
        );
    }
}
