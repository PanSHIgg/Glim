//! 非按键形式的输入编码：粘贴、焦点报告与备用屏幕滚轮。

use crate::TerminalModes;

/// 括号粘贴序列的起始 / 结束标记。
const PASTE_START: &str = "\x1b[200~";
const PASTE_END: &str = "\x1b[201~";

/// 把要粘贴的文本编码为写入 PTY 的字节。
///
/// 括号粘贴模式下原文被包裹，且原始 ESC 会被剥离；否则把不同平台的
/// 行结束符归一为回车，直接交给终端的行编辑器。
pub fn paste_bytes(text: &str, modes: TerminalModes) -> Vec<u8> {
    if modes.contains(TerminalModes::BRACKETED_PASTE) {
        let mut bytes = Vec::with_capacity(text.len() + PASTE_START.len() + PASTE_END.len());
        bytes.extend_from_slice(PASTE_START.as_bytes());
        for character in text.chars() {
            if character != '\x1b' {
                let mut buffer = [0u8; 4];
                bytes.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
            }
        }
        bytes.extend_from_slice(PASTE_END.as_bytes());
        bytes
    } else {
        let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
        normalized.into_bytes()
    }
}

/// 焦点进入 / 离开的报告序列；模式未开启时返回 `None`。
pub fn focus_report(focused: bool, modes: TerminalModes) -> Option<&'static [u8]> {
    if !modes.contains(TerminalModes::FOCUS_IN_OUT) {
        return None;
    }
    Some(if focused { b"\x1b[I" } else { b"\x1b[O" })
}

/// 备用屏幕上的滚轮：按方向键序列发送 `lines` 次。
///
/// `lines` 为正表示向上（历史方向）。序列跟随应用光标模式
/// （DECCKM）：SS3（`\x1bOA`）或 CSI（`\x1b[A`）。
pub fn alternate_scroll_bytes(lines: i32, modes: TerminalModes) -> Vec<u8> {
    if lines == 0 {
        return Vec::new();
    }
    let app_cursor = modes.contains(TerminalModes::APP_CURSOR);
    let (up, down) = if app_cursor {
        ("\x1bOA", "\x1bOB")
    } else {
        ("\x1b[A", "\x1b[B")
    };
    let sequence = if lines > 0 { up } else { down };
    let mut bytes = Vec::with_capacity(sequence.len() * lines.unsigned_abs() as usize);
    for _ in 0..lines.unsigned_abs() {
        bytes.extend_from_slice(sequence.as_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bracketed_paste_wraps_and_strips_escape() {
        let modes = TerminalModes::BRACKETED_PASTE;
        let bytes = paste_bytes("echo \x1b[31m红色\x1b[0m", modes);
        assert_eq!(
            String::from_utf8_lossy(&bytes),
            "\x1b[200~echo [31m红色[0m\x1b[201~"
        );
    }

    #[test]
    fn plain_paste_normalizes_newlines() {
        let bytes = paste_bytes("a\r\nb\nc", TerminalModes::empty());
        assert_eq!(String::from_utf8_lossy(&bytes), "a\rb\rc");
    }

    #[test]
    fn focus_report_follows_mode() {
        assert_eq!(focus_report(true, TerminalModes::empty()), None);
        assert_eq!(
            focus_report(true, TerminalModes::FOCUS_IN_OUT),
            Some(b"\x1b[I".as_slice())
        );
        assert_eq!(
            focus_report(false, TerminalModes::FOCUS_IN_OUT),
            Some(b"\x1b[O".as_slice())
        );
    }

    #[test]
    fn alternate_scroll_respects_cursor_mode() {
        assert_eq!(
            String::from_utf8_lossy(&alternate_scroll_bytes(2, TerminalModes::empty())),
            "\x1b[A\x1b[A"
        );
        assert_eq!(
            String::from_utf8_lossy(&alternate_scroll_bytes(-1, TerminalModes::APP_CURSOR)),
            "\x1bOB"
        );
        assert!(alternate_scroll_bytes(0, TerminalModes::empty()).is_empty());
    }
}
