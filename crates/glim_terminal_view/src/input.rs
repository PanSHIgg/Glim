//! 键盘输入到终端字节的转换。

use gpui_kit::Keystroke;

use glim_terminal::{KeyInput, TerminalModes};

/// 把一次按键转换为要写入 PTY 的字节；无需写入时返回 `None`。
pub(crate) fn keystroke_bytes(
    keystroke: &Keystroke,
    prefer_character_input: bool,
    modes: TerminalModes,
) -> Option<Vec<u8>> {
    // 平台首选字符输入（如 Windows AltGr）时优先按字符处理。
    if prefer_character_input && let Some(text) = &keystroke.key_char {
        return Some(text.clone().into_bytes());
    }

    let key = KeyInput {
        key: keystroke.key.clone(),
        ctrl: keystroke.modifiers.control,
        alt: keystroke.modifiers.alt,
        shift: keystroke.modifiers.shift,
        meta: keystroke.modifiers.platform,
    };

    if let Some(bytes) = key.to_bytes(modes) {
        return Some(bytes);
    }

    // 可打印字符。
    if !keystroke.modifiers.control
        && !keystroke.modifiers.alt
        && !keystroke.modifiers.platform
        && let Some(text) = &keystroke.key_char
    {
        return Some(text.clone().into_bytes());
    }

    None
}
