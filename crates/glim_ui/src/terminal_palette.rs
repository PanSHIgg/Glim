//! 终端调色板：终端网格使用的语义颜色。
//!
//! 调色板字面量属于主题定义层；渲染代码只读取 [`TerminalPalette`]。

use gpui_kit::component::{ActiveTheme as _, Theme};
use gpui_kit::*;

/// 终端渲染所需的颜色集合。
#[derive(Clone, Copy, Debug)]
pub struct TerminalPalette {
    /// 默认背景色。
    pub background: Hsla,
    /// 默认前景色。
    pub foreground: Hsla,
    /// 光标颜色。
    pub cursor: Hsla,
    /// ANSI 调色板：0–7 基础色，8–15 亮色。
    pub ansi: [Hsla; 16],
}

/// 读取当前主题对应的终端调色板。
pub fn terminal_palette(cx: &App) -> TerminalPalette {
    let theme = cx.theme();
    if theme.is_dark() {
        dark_palette(theme)
    } else {
        light_palette(theme)
    }
}

fn dark_palette(theme: &Theme) -> TerminalPalette {
    TerminalPalette {
        background: theme.background,
        foreground: rgb(0xe6e5db).into(),
        cursor: theme.caret,
        ansi: [
            rgb(0x2a2c28).into(),
            rgb(0xdf9690).into(),
            rgb(0xa6bc92).into(),
            rgb(0xd7b47d).into(),
            rgb(0xa0b9c6).into(),
            rgb(0xc1a3bd).into(),
            rgb(0x93b8ae).into(),
            rgb(0xd9d9d2).into(),
            rgb(0x6f7268).into(),
            rgb(0xecaba5).into(),
            rgb(0xb9cf9f).into(),
            rgb(0xe5c48f).into(),
            rgb(0xb3ccd8).into(),
            rgb(0xd0b5cc).into(),
            rgb(0xa5c9c0).into(),
            rgb(0xf0efe8).into(),
        ],
    }
}

fn light_palette(theme: &Theme) -> TerminalPalette {
    TerminalPalette {
        background: theme.background,
        foreground: rgb(0x30352a).into(),
        cursor: theme.caret,
        ansi: [
            rgb(0x2b2e28).into(),
            rgb(0xa63e3a).into(),
            rgb(0x496d34).into(),
            rgb(0x865b25).into(),
            rgb(0x3e687c).into(),
            rgb(0x7a5175).into(),
            rgb(0x3f6d68).into(),
            rgb(0xd9ddd0).into(),
            rgb(0x72786a).into(),
            rgb(0xc0524e).into(),
            rgb(0x5f8a45).into(),
            rgb(0xa5762f).into(),
            rgb(0x5286a0).into(),
            rgb(0x96688f).into(),
            rgb(0x4f8a83).into(),
            rgb(0xf4f5ef).into(),
        ],
    }
}
