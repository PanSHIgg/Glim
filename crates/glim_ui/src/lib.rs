//! 产品界面基础：主题、字体、间距、图标与公共展示组件。
//!
//! 当前提供产品主题装配（[`init`]）与状态栏的主题选择胶囊
//! （[`ThemeCapsule`]）。字体、间距与更多展示组件随后续阶段加入。

mod theme;
mod theme_capsule;

pub use theme::{ThemePreference, apply_theme_preference, init};
pub use theme_capsule::ThemeCapsule;
