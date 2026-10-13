//! 产品界面基础：主题、字体、间距、图标与公共展示组件。
//!
//! 提供产品主题装配（[`init`]）、状态栏的主题选择胶囊
//! （[`ThemeCapsule`]）与终端网格调色板（[`terminal_palette`]）。

mod terminal_palette;
mod theme;
mod theme_capsule;

pub use terminal_palette::{TerminalPalette, terminal_palette};
pub use theme::{ThemePreference, apply_theme_preference, init};
pub use theme_capsule::ThemeCapsule;
