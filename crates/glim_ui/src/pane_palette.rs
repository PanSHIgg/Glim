//! 窗格配色：窗格标题栏与次要前景。
//!
//! 取自设计稿的主题令牌（`design/styles.css`）：非活动窗格标题使用
//! `--header`，活动窗格使用 `--header-active`，图标与进程标签使用
//! `--subtle-foreground`。

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

/// 窗格相关的产品颜色。
#[derive(Clone, Copy, Debug)]
pub struct PanePalette {
    /// 窗格标题栏背景（非活动）。
    pub header: Hsla,
    /// 窗格标题栏背景（活动窗格）。
    pub header_active: Hsla,
    /// 次要前景：标题图标、进程标签、窗格操作按钮。
    pub subtle_foreground: Hsla,
}

/// 标题栏高度（rem；设计稿 2.75rem）。
pub const TITLE_BAR_HEIGHT_REMS: f32 = 2.75;
/// 窗格标题栏高度（rem；设计稿 2.125rem）。
pub const PANE_HEADER_HEIGHT_REMS: f32 = 2.125;

/// 读取当前主题对应的窗格配色。
pub fn pane_palette(cx: &App) -> PanePalette {
    if cx.theme().is_dark() {
        PanePalette {
            header: rgb(0x1e1f1f).into(),
            header_active: rgb(0x292926).into(),
            subtle_foreground: rgb(0x82857e).into(),
        }
    } else {
        PanePalette {
            header: rgb(0xf3f4ed).into(),
            header_active: rgb(0xe9ece1).into(),
            subtle_foreground: rgb(0x72786a).into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{TestAppContext, rgb};

    /// 窗格配色随主题深浅切换；活动窗格标题比非活动更亮。
    #[gpui_kit::test]
    fn pane_palette_follows_theme(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::init(cx);
        });
        cx.update(|cx| {
            let palette = crate::pane_palette(cx);
            assert_eq!(palette.header, rgb(0x1e1f1f).into());
            assert_eq!(palette.header_active, rgb(0x292926).into());

            crate::apply_theme_preference(crate::ThemePreference::Light, None, cx);
            let palette = crate::pane_palette(cx);
            assert_eq!(palette.header, rgb(0xf3f4ed).into());
            assert_eq!(palette.header_active, rgb(0xe9ece1).into());
        });
    }
}
