//! 类型化配置文档：`config.toml` 的内存表示与默认值。
//!
//! 未知字段在解析时忽略；本类型序列化时只写出已知字段。数值范围在
//! [`Settings::sanitize`] 中收敛到产品约束，无效文件永远不会让应用
//! 无法启动。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use glim_ui::ThemePreference;

/// 默认滚动历史行数。
pub const DEFAULT_SCROLLBACK: usize = 10_000;
/// 滚动历史的允许范围。
pub const MIN_SCROLLBACK: usize = 100;
/// 滚动历史的允许上限。
pub const MAX_SCROLLBACK: usize = 1_000_000;
/// 终端字号范围（逻辑像素）；与界面层保持一致。
pub const MIN_FONT_SIZE: f32 = glim_ui::MIN_TERMINAL_FONT_SIZE;
/// 终端字号上限（逻辑像素）。
pub const MAX_FONT_SIZE: f32 = glim_ui::MAX_TERMINAL_FONT_SIZE;

/// 主题选择。
///
/// 与 [`glim_ui::ThemePreference`] 一一对应；配置文档中的取值为
/// `"light"`、`"dark"`、`"system"`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeSetting {
    /// 固定浅色。
    Light,
    /// 固定深色；产品默认。
    #[default]
    Dark,
    /// 跟随系统外观。
    System,
}

impl ThemeSetting {
    /// 转换为界面主题偏好。
    pub fn to_preference(self) -> ThemePreference {
        match self {
            Self::Light => ThemePreference::Light,
            Self::Dark => ThemePreference::Dark,
            Self::System => ThemePreference::System,
        }
    }
}

impl From<ThemePreference> for ThemeSetting {
    fn from(preference: ThemePreference) -> Self {
        match preference {
            ThemePreference::Light => Self::Light,
            ThemePreference::Dark => Self::Dark,
            ThemePreference::System => Self::System,
        }
    }
}

/// 默认 shell 配置。
///
/// 对应 `config.toml` 的 `[shell]` 段；界面只编辑程序与参数，
/// 环境变量属于高级设置，通过配置文件维护。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellConfig {
    program: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
}

impl ShellConfig {
    /// 以程序路径创建。
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            ..Self::default()
        }
    }

    /// 追加参数。
    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// 追加环境变量。
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// 程序路径。
    pub fn program(&self) -> &str {
        &self.program
    }

    /// 参数。
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// 环境变量。
    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }
}

/// 应用设置；与 `config.toml` 一一对应。
///
/// 默认值对应产品默认：深色主题、内置终端字体（Cascadia Mono →
/// Consolas）、10,000 行滚动历史、平台默认 shell。未知字段原样保留，
/// 保存时不会丢失（高级设置或更新版本写入的内容）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    theme: ThemeSetting,
    #[serde(skip_serializing_if = "Option::is_none")]
    font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    font_size: Option<f32>,
    scrollback: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    shell: Option<ShellConfig>,
    /// 未知字段；保存时按原样写回。
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeSetting::default(),
            font_family: None,
            font_size: None,
            scrollback: DEFAULT_SCROLLBACK,
            shell: None,
            extra: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// 创建默认设置。
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置主题。
    pub fn with_theme(mut self, theme: ThemeSetting) -> Self {
        self.theme = theme;
        self
    }

    /// 设置终端字体族；`None` 表示使用内置默认字体。
    pub fn with_font_family(mut self, family: Option<impl Into<String>>) -> Self {
        self.font_family = family.map(Into::into);
        self
    }

    /// 设置终端字号；`None` 表示使用内置默认字号。
    pub fn with_font_size(mut self, size: Option<f32>) -> Self {
        self.font_size = size;
        self
    }

    /// 设置滚动历史行数。
    pub fn with_scrollback(mut self, lines: usize) -> Self {
        self.scrollback = lines;
        self
    }

    /// 设置默认 shell。
    pub fn with_shell(mut self, shell: Option<ShellConfig>) -> Self {
        self.shell = shell;
        self
    }

    /// 主题。
    pub fn theme(&self) -> ThemeSetting {
        self.theme
    }

    /// 终端字体族；`None` 表示内置默认。
    pub fn font_family(&self) -> Option<&str> {
        self.font_family.as_deref()
    }

    /// 终端字号；`None` 表示内置默认。
    pub fn font_size(&self) -> Option<f32> {
        self.font_size
    }

    /// 滚动历史行数。
    pub fn scrollback(&self) -> usize {
        self.scrollback
    }

    /// 默认 shell；`None` 表示按平台默认顺序查找。
    pub fn shell(&self) -> Option<&ShellConfig> {
        self.shell.as_ref()
    }

    /// 把无效或越界的取值收敛到产品约束。
    ///
    /// 在解析之后与写入之前调用；界面与启动都只接触收敛后的值。
    pub fn sanitize(&mut self) {
        if !(MIN_SCROLLBACK..=MAX_SCROLLBACK).contains(&self.scrollback) {
            self.scrollback = DEFAULT_SCROLLBACK;
        }
        if let Some(size) = self.font_size
            && !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&size)
        {
            self.font_size = None;
        }
        if self
            .font_family
            .as_deref()
            .is_some_and(|s| s.trim().is_empty())
        {
            self.font_family = None;
        }
        if let Some(shell) = &mut self.shell
            && shell.program.trim().is_empty()
        {
            self.shell = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_product_defaults() {
        let settings = Settings::default();
        assert_eq!(settings.theme(), ThemeSetting::Dark);
        assert_eq!(settings.font_family(), None);
        assert_eq!(settings.font_size(), None);
        assert_eq!(settings.scrollback(), DEFAULT_SCROLLBACK);
        assert!(settings.shell().is_none());
    }

    #[test]
    fn theme_setting_round_trips_through_preference() {
        for preference in [
            ThemePreference::Light,
            ThemePreference::Dark,
            ThemePreference::System,
        ] {
            assert_eq!(ThemeSetting::from(preference).to_preference(), preference);
        }
    }

    #[test]
    fn sanitize_clamps_out_of_range_values() {
        let mut settings = Settings::default()
            .with_scrollback(0)
            .with_font_size(Some(100.))
            .with_font_family(Some("  "))
            .with_shell(Some(ShellConfig::new("  ")));
        settings.sanitize();
        assert_eq!(settings.scrollback(), DEFAULT_SCROLLBACK);
        assert_eq!(settings.font_size(), None);
        assert_eq!(settings.font_family(), None);
        assert!(settings.shell().is_none());

        let mut valid = Settings::default()
            .with_scrollback(500)
            .with_font_size(Some(14.))
            .with_font_family(Some("Consolas"))
            .with_shell(Some(ShellConfig::new("pwsh.exe").with_args(["-NoLogo"])));
        valid.sanitize();
        assert_eq!(valid.scrollback(), 500);
        assert_eq!(valid.font_size(), Some(14.));
        assert_eq!(valid.font_family(), Some("Consolas"));
        assert_eq!(valid.shell().expect("shell").program(), "pwsh.exe");
    }
}
