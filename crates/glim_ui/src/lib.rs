//! 产品界面基础：主题、字体、间距、图标与公共展示组件。
//!
//! 提供产品主题装配（[`init`]）、终端字体的解析与应用
//! （[`apply_appearance`]）、标题栏的主题选择胶囊（[`ThemeCapsule`]）
//! 与终端网格调色板（[`terminal_palette`]）。

mod font;
mod pane_palette;
mod terminal_palette;
mod theme;
mod theme_capsule;

use gpui_kit::App;

pub use font::{
    DEFAULT_TERMINAL_FONT_SIZE, FontApplication, MAX_TERMINAL_FONT_SIZE, MIN_TERMINAL_FONT_SIZE,
    apply_appearance, apply_mono_font, default_mono_font_family, default_mono_font_size,
};
pub use pane_palette::{PANE_HEADER_HEIGHT_REMS, PanePalette, TITLE_BAR_HEIGHT_REMS, pane_palette};
pub use terminal_palette::{TerminalPalette, terminal_palette};
pub use theme::{ThemePreference, apply_theme_preference};
pub use theme_capsule::ThemeCapsule;

/// 安装产品界面基础：注册主题并解析默认终端字体。
///
/// 必须在 `gpui_kit::init` 之后调用一次。
pub fn init(cx: &mut App) {
    theme::init(cx);
    font::init(cx);
}
