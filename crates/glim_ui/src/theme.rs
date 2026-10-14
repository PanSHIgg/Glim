//! Glim 产品主题：定义浅色与深色配色，并提供主题偏好策略。
//!
//! 调色板字面量只属于本文件（主题定义层）；组件与视图一律读取语义令牌。

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig, ThemeConfigColors, ThemeMode};
use gpui_kit::*;

/// 用户的主题偏好。
///
/// 决定窗口使用固定浅色、固定深色，还是跟随系统外观。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemePreference {
    /// 固定浅色。
    Light,
    /// 固定深色；产品默认。
    #[default]
    Dark,
    /// 跟随系统外观。
    System,
}

impl ThemePreference {
    /// 是否跟随系统外观。
    pub fn is_system(self) -> bool {
        matches!(self, Self::System)
    }
}

/// 安装产品主题：注册浅色与深色两套配置，并应用默认偏好。
///
/// 必须在 `gpui_kit::init` 之后调用一次。
pub fn init(cx: &mut App) {
    let light = Rc::new(light_config());
    let dark = Rc::new(dark_config());
    Theme::update(cx, |theme| {
        theme.apply_config(&light);
        theme.apply_config(&dark);
    });
    apply_theme_preference(ThemePreference::default(), None, cx);
}

/// 应用主题偏好；[`ThemePreference::System`] 时读取窗口或应用的当前外观。
pub fn apply_theme_preference(
    preference: ThemePreference,
    window: Option<&mut Window>,
    cx: &mut App,
) {
    let mode = match preference {
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Dark => ThemeMode::Dark,
        ThemePreference::System => {
            let appearance = window
                .map(|window| window.appearance())
                .unwrap_or_else(|| cx.window_appearance());
            ThemeMode::from(appearance)
        }
    };
    Theme::change(mode, None, cx);
}

fn color(hex: &'static str) -> Option<SharedString> {
    Some(hex.into())
}

fn dark_config() -> ThemeConfig {
    ThemeConfig {
        name: "Glim Dark".into(),
        mode: ThemeMode::Dark,
        // 终端字族由 `font::init` 按已安装字体解析后应用。
        colors: dark_colors(),
        ..Default::default()
    }
}

fn light_config() -> ThemeConfig {
    ThemeConfig {
        name: "Glim Light".into(),
        mode: ThemeMode::Light,
        colors: light_colors(),
        ..Default::default()
    }
}

/// 深色配色，取自设计稿的主题令牌（`design/styles.css`）。
fn dark_colors() -> ThemeConfigColors {
    let mut colors = ThemeConfigColors::default();

    // 基础表面与文字
    colors.background = color("#1a1b1b");
    colors.foreground = color("#d9d9d2");
    colors.muted = color("#222323");
    colors.muted_foreground = color("#a0a19b");
    colors.border = color("#343633");
    colors.input = color("#343633");
    colors.ring = color("#d4c397");
    colors.caret = color("#c9ba91");

    // 强调与选择
    colors.accent = color("#30322d");
    colors.accent_foreground = color("#f0efe8");
    colors.selection = color("#454b3a");

    // 主 / 次操作
    colors.primary = color("#c9ba91");
    colors.primary_foreground = color("#141613");
    colors.secondary = color("#2a2c28");
    colors.secondary_foreground = color("#e6e5db");

    // 按钮
    colors.button = color("#222323");
    colors.button_hover = color("#30322d");
    colors.button_active = color("#373930");
    colors.button_foreground = color("#e6e5db");

    // 语义状态
    colors.danger = color("#df9690");
    colors.danger_foreground = color("#1f1312");
    colors.success = color("#a6bc92");
    colors.success_foreground = color("#141711");
    colors.warning = color("#d7b47d");
    colors.warning_foreground = color("#1d1710");
    colors.info = color("#a0b9c6");
    colors.info_foreground = color("#101619");

    // 浮层与窗口装饰
    colors.popover = color("#252724");
    colors.popover_foreground = color("#d9d9d2");
    colors.title_bar = color("#222323");
    colors.title_bar_border = color("#343633");
    colors.status_bar = color("#1e1f1f");
    colors.status_bar_border = color("#343633");

    // 滚动条与链接
    colors.scrollbar = color("#1a1b1b");
    colors.scrollbar_thumb = color("#343633");
    colors.scrollbar_thumb_hover = color("#4c4f47");
    colors.link = color("#a0b9c6");

    colors
}

/// 浅色配色，取自设计稿的主题令牌（`design/styles.css`）。
fn light_colors() -> ThemeConfigColors {
    let mut colors = ThemeConfigColors::default();

    // 基础表面与文字
    colors.background = color("#fafbf7");
    colors.foreground = color("#393c35");
    colors.muted = color("#eeefe9");
    colors.muted_foreground = color("#656a5e");
    colors.border = color("#d9ddd0");
    colors.input = color("#d9ddd0");
    colors.ring = color("#7b683f");
    colors.caret = color("#786442");

    // 强调与选择
    colors.accent = color("#e4e8da");
    colors.accent_foreground = color("#24271f");
    colors.selection = color("#dce6cc");

    // 主 / 次操作
    colors.primary = color("#786442");
    colors.primary_foreground = color("#f9faf6");
    colors.secondary = color("#e9ece1");
    colors.secondary_foreground = color("#24271f");

    // 按钮
    colors.button = color("#eeefe9");
    colors.button_hover = color("#e4e8da");
    colors.button_active = color("#dde3d2");
    colors.button_foreground = color("#393c35");

    // 语义状态
    colors.danger = color("#a63e3a");
    colors.danger_foreground = color("#fafbf7");
    colors.success = color("#496d34");
    colors.success_foreground = color("#fafbf7");
    colors.warning = color("#865b25");
    colors.warning_foreground = color("#fafbf7");
    colors.info = color("#3e687c");
    colors.info_foreground = color("#fafbf7");

    // 浮层与窗口装饰
    colors.popover = color("#f9faf6");
    colors.popover_foreground = color("#393c35");
    colors.title_bar = color("#eeefe9");
    colors.title_bar_border = color("#d9ddd0");
    colors.status_bar = color("#f3f4ed");
    colors.status_bar_border = color("#d9ddd0");

    // 滚动条与链接
    colors.scrollbar = color("#fafbf7");
    colors.scrollbar_thumb = color("#d9ddd0");
    colors.scrollbar_thumb_hover = color("#aeb6a0");
    colors.link = color("#3e687c");

    colors
}
