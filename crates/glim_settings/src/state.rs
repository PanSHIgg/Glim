//! 设置状态：内存中的当前设置、持久化与变化通知。
//!
//! [`SettingsState`] 是应用级设置服务：读取 `config.toml`，把界面与
//! 配置文件的修改合并到同一份内存值，并在每次变化时尝试安全写入。
//! 保存失败不回滚内存值（当前会话仍然可用），错误留给界面展示。

use std::path::PathBuf;

use gpui_kit::{App, AppContext, Context, Entity, EventEmitter, Global};

use crate::settings::Settings;
use crate::store::{self, StoreError};

/// 设置变化事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsEvent {
    /// 设置已变化；`saved` 表示是否已写入配置文件。
    Changed {
        /// 变化是否已持久化。
        saved: bool,
    },
}

impl EventEmitter<SettingsEvent> for SettingsState {}

/// 应用级设置服务。
///
/// 状态由实体持有；界面通过 reader 读取当前值，通过 [`Self::apply`] 或
/// [`Self::set_theme`] 提交变化。`path` 为 `None` 时不持久化（测试或
/// 平台没有配置目录的环境），其余行为一致。
pub struct SettingsState {
    settings: Settings,
    path: Option<PathBuf>,
    /// 读取配置时的问题（已回退到默认值）；成功保存后清除。
    load_warning: Option<String>,
    /// 最近一次保存失败的原因；成功保存后清除。
    save_error: Option<String>,
}

impl SettingsState {
    /// 读取配置并创建状态；文件缺失、无效或不可读都不会失败。
    pub fn load(path: Option<PathBuf>) -> Self {
        let mut load_warning = None;
        let settings = match &path {
            Some(path) => match store::load_toml::<Settings>(path) {
                Ok(Some(mut settings)) => {
                    settings.sanitize();
                    settings
                }
                Ok(None) => Settings::default(),
                Err(error) => {
                    load_warning = Some(format!("{error}；已使用默认设置"));
                    Settings::default()
                }
            },
            None => Settings::default(),
        };
        Self {
            settings,
            path,
            load_warning,
            save_error: None,
        }
    }

    /// 当前设置。
    pub fn current(&self) -> &Settings {
        &self.settings
    }

    /// 配置文件路径；`None` 表示本次运行不持久化。
    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    /// 读取配置时的问题；成功保存后清除。
    pub fn load_warning(&self) -> Option<&str> {
        self.load_warning.as_deref()
    }

    /// 最近一次保存失败的原因；成功保存后清除。
    pub fn save_error(&self) -> Option<&str> {
        self.save_error.as_deref()
    }

    /// 提交一整套设置：收敛取值、尝试保存并通知变化。
    ///
    /// 取值与当前一致时不触发任何事件（避免无变化的写入）。
    pub fn apply(&mut self, next: Settings, cx: &mut Context<Self>) {
        let mut next = next;
        next.sanitize();
        if self.settings == next {
            return;
        }
        self.settings = next;
        self.persist(cx);
    }

    /// 只切换主题；状态栏的主题胶囊使用它。
    pub fn set_theme(&mut self, theme: crate::settings::ThemeSetting, cx: &mut Context<Self>) {
        if self.settings.theme() == theme {
            return;
        }
        self.settings = self.settings.clone().with_theme(theme);
        self.persist(cx);
    }

    /// 保存当前设置并广播变化；保存失败保留内存值并记录错误。
    fn persist(&mut self, cx: &mut Context<Self>) {
        let mut saved = true;
        if let Some(path) = &self.path {
            match store::save_toml(path, &self.settings) {
                Ok(()) => {
                    self.load_warning = None;
                    self.save_error = None;
                }
                Err(error) => {
                    self.save_error = Some(error_text(&error));
                    saved = false;
                }
            }
        }
        cx.emit(SettingsEvent::Changed { saved });
        cx.notify();
    }
}

/// 面向界面的错误文本；保留路径信息。
fn error_text(error: &StoreError) -> String {
    format!("保存设置失败：{error}")
}

/// 应用级设置全局：登记 [`SettingsState`]，供任意视图读取。
pub struct SettingsGlobal(pub(crate) Entity<SettingsState>);

impl Global for SettingsGlobal {}

/// 读取并安装应用设置；`config.toml` 位于 [`crate::data_directory`]。
pub fn init(cx: &mut App) -> Entity<SettingsState> {
    let path = crate::data_directory().map(|directory| crate::config_path(&directory));
    init_with_path(cx, path)
}

/// 以指定路径安装设置（测试与嵌入场景）。
pub fn init_with_path(cx: &mut App, path: Option<PathBuf>) -> Entity<SettingsState> {
    let state = cx.new(|_| SettingsState::load(path));
    cx.set_global(SettingsGlobal(state.clone()));
    state
}

/// 应用设置实体；未安装时为 `None`。
pub fn handle(cx: &App) -> Option<Entity<SettingsState>> {
    cx.try_global::<SettingsGlobal>()
        .map(|global| global.0.clone())
}

/// 当前设置；未安装时返回默认值。
pub fn current(cx: &App) -> Settings {
    handle(cx)
        .map(|state| state.read(cx).current().clone())
        .unwrap_or_default()
}

/// 启动时的配置读取问题，用于通知。
pub fn load_warning(cx: &App) -> Option<String> {
    handle(cx).and_then(|state| state.read(cx).load_warning().map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{DEFAULT_SCROLLBACK, ThemeSetting};
    use gpui_kit::TestAppContext;
    use std::fs;

    fn test_path(name: &str) -> PathBuf {
        let directory = std::env::temp_dir()
            .join("glim-settings-state-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("create test directory");
        directory.join("config.toml")
    }

    #[gpui_kit::test]
    fn apply_persists_and_emits_once(cx: &mut TestAppContext) {
        let path = test_path("apply");
        let state = cx.update(|cx| init_with_path(cx, Some(path.clone())));

        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let recorded = events.clone();
        let subscription = cx.update(|cx| {
            cx.subscribe(&state, move |_state, event: &SettingsEvent, _| {
                recorded.borrow_mut().push(*event);
            })
        });

        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.apply(state.current().clone().with_scrollback(1_234), cx);
            });
            // 相同取值不产生事件。
            state.update(cx, |state, cx| {
                state.apply(state.current().clone(), cx);
            });
        });

        assert_eq!(
            *events.borrow(),
            vec![SettingsEvent::Changed { saved: true }],
            "一次有效变化只广播一次"
        );
        let loaded: Settings = store::load_toml(&path).expect("load").expect("file");
        assert_eq!(loaded.scrollback(), 1_234);
        assert!(state.read_with(cx, |state, _| state.save_error().is_none()));
        drop(subscription);
    }

    #[gpui_kit::test]
    fn save_failure_keeps_memory_value_and_reports_error(cx: &mut TestAppContext) {
        let directory = test_path("save-failure");
        // 目标路径的父级是一个文件：创建目录失败，保存必然失败。
        let blocker = directory.parent().expect("parent").join("blocker");
        fs::write(&blocker, "not a directory").expect("write blocker");
        let path = blocker.join("config.toml");

        let state = cx.update(|cx| init_with_path(cx, Some(path.clone())));
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.apply(state.current().clone().with_scrollback(2_000), cx);
            });
        });

        let (value, error) = state.read_with(cx, |state, _| {
            (
                state.current().scrollback(),
                state.save_error().map(str::to_owned),
            )
        });
        assert_eq!(value, 2_000, "保存失败不回滚内存值");
        assert!(error.is_some_and(|text| text.contains("保存设置失败")));
    }

    #[gpui_kit::test]
    fn invalid_document_starts_with_defaults_and_keeps_warning(cx: &mut TestAppContext) {
        let path = test_path("invalid");
        fs::write(&path, "theme = \"不存在的主题\"").expect("write invalid file");

        let state = cx.update(|cx| init_with_path(cx, Some(path.clone())));
        let (scrollback, warning) = state.read_with(cx, |state, _| {
            (
                state.current().scrollback(),
                state.load_warning().map(str::to_owned),
            )
        });
        assert_eq!(scrollback, DEFAULT_SCROLLBACK);
        assert!(warning.is_some_and(|text| text.contains("已使用默认设置")));

        // 成功保存后清除读取警告。
        cx.update(|cx| {
            state.update(cx, |state, cx| state.set_theme(ThemeSetting::Light, cx));
        });
        assert!(
            state.read_with(cx, |state, _| state.load_warning().is_none()),
            "成功保存后清除读取警告"
        );
    }

    #[gpui_kit::test]
    fn without_path_state_still_notifies(cx: &mut TestAppContext) {
        let state = cx.update(|cx| init_with_path(cx, None));
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let recorded = events.clone();
        let subscription = cx.update(|cx| {
            cx.subscribe(&state, move |_state, event: &SettingsEvent, _| {
                recorded.borrow_mut().push(*event);
            })
        });
        cx.update(|cx| {
            state.update(cx, |state, cx| state.set_theme(ThemeSetting::System, cx));
        });
        assert_eq!(
            *events.borrow(),
            vec![SettingsEvent::Changed { saved: true }]
        );
        drop(subscription);
    }
}
