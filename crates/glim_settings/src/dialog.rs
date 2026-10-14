//! 基础设置界面：常用设置的对话框（`Ctrl+,` 或工具栏入口）。
//!
//! 对话框编辑一份草稿，`保存` 才写回 [`SettingsState`]；`取消`、`Esc`
//! 或点击遮罩放弃修改。校验失败或保存失败时对话框保持打开，并在底部
//! 显示原因。字体与默认 shell 从已安装字族、探测到的 shell 中选择，
//! 当前值不在候选中时也会列出（标注未安装 / 作为当前设置保留）；
//! 字号、参数与滚动历史仍然直接读 [`InputState`]，主题的草稿选择保存在
//! [`SettingsDraft`] 实体中，供内容重建时读取。

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::IndexPath;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _, Toggle};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputState, NumberInput};
use gpui_kit::component::label::Label;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{SearchableVec, Select, SelectItem, SelectState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use glim_terminal::{ShellSpec, available_shells};

use crate::settings::{
    MAX_FONT_SIZE, MAX_SCROLLBACK, MIN_FONT_SIZE, MIN_SCROLLBACK, Settings, ShellConfig,
    ThemeSetting,
};
use crate::state::{SettingsState, handle};

actions!(
    glim_settings,
    [
        /// 打开基础设置对话框。
        OpenSettings,
    ]
);

/// 对话框草稿：主题选择与当前错误提示。
struct SettingsDraft {
    theme: ThemeSetting,
    error: Option<String>,
}

/// 对话框字段的原始文本。
#[derive(Clone, Debug, Default)]
struct FieldValues {
    font_family: Option<String>,
    font_size: String,
    shell_program: Option<String>,
    shell_args: String,
    scrollback: String,
}

/// 字体下拉项：`None` 表示内置默认字族。
#[derive(Clone, Debug)]
struct FontChoice {
    label: SharedString,
    family: Option<SharedString>,
}

impl SelectItem for FontChoice {
    type Value = Option<SharedString>;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    /// 默认项只按「默认」匹配：搜索字族时不会挤到结果最前面。
    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        match &self.family {
            Some(family) => family.to_lowercase().contains(&query),
            None => query.is_empty() || "默认".contains(&query),
        }
    }

    fn value(&self) -> &Option<SharedString> {
        &self.family
    }
}

/// shell 下拉项：`None` 表示按默认顺序查找；`program` 为会话要启动的程序。
#[derive(Clone, Debug)]
struct ShellChoice {
    label: SharedString,
    program: Option<String>,
}

impl SelectItem for ShellChoice {
    type Value = Option<String>;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    /// 名称与完整路径都可以搜索。
    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.label.to_lowercase().contains(&query)
            || self
                .program
                .as_deref()
                .is_some_and(|program| program.to_lowercase().contains(&query))
    }

    fn render(&self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        // 下拉行显示完整程序路径；触发器只显示名称。
        v_flex()
            .child(self.label.clone())
            .when_some(self.program.clone(), |this, program| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(program),
                )
            })
    }

    fn value(&self) -> &Option<String> {
        &self.program
    }
}

/// 组装字体下拉项与初始选中行：内置默认、已安装字族、配置里缺失的字族。
///
/// 缺失的字族排在最后并标注「未安装」，选中它会保留配置值（保存时由
/// 字体应用层回退到默认并提示），不会静默改掉用户的设置。
fn font_choices(
    installed: &[String],
    default_family: &str,
    current: Option<&str>,
) -> (Vec<FontChoice>, Option<IndexPath>) {
    let mut items = vec![FontChoice {
        label: format!("默认（{default_family}）").into(),
        family: None,
    }];
    items.extend(installed.iter().map(|family| FontChoice {
        label: family.clone().into(),
        family: Some(family.clone().into()),
    }));

    let Some(current) = current.map(str::trim).filter(|family| !family.is_empty()) else {
        return (items, Some(IndexPath::new(0)));
    };
    if let Some(index) = items.iter().position(|item| {
        item.family
            .as_deref()
            .is_some_and(|family| family.eq_ignore_ascii_case(current))
    }) {
        return (items, Some(IndexPath::new(index)));
    }

    items.push(FontChoice {
        label: format!("{current}（未安装）").into(),
        family: Some(current.into()),
    });
    let selected = items.len() - 1;
    (items, Some(IndexPath::new(selected)))
}

/// 组装 shell 下拉项与初始选中行：默认、探测到的 shell、配置里的自定义程序。
///
/// 程序路径不同但文件名相同时视为同一个 shell（例如配置里写
/// `powershell.exe`、候选是完整路径），选中候选中的条目。
fn shell_choices(
    available: &[ShellSpec],
    current: Option<&str>,
) -> (Vec<ShellChoice>, Option<IndexPath>) {
    let mut items = vec![ShellChoice {
        label: "默认（按顺序查找）".into(),
        program: None,
    }];
    items.extend(available.iter().map(|shell| ShellChoice {
        label: shell_label(shell.program()),
        program: Some(shell.program().to_owned()),
    }));

    let Some(current) = current.map(str::trim).filter(|program| !program.is_empty()) else {
        return (items, Some(IndexPath::new(0)));
    };
    if let Some(index) = items.iter().position(|item| {
        item.program.as_deref().is_some_and(|program| {
            program.eq_ignore_ascii_case(current) || same_file_name(program, current)
        })
    }) {
        return (items, Some(IndexPath::new(index)));
    }

    items.push(ShellChoice {
        label: shell_label(current),
        program: Some(current.to_owned()),
    });
    let selected = items.len() - 1;
    (items, Some(IndexPath::new(selected)))
}

/// shell 程序 → 下拉展示名；未知程序用文件名。
fn shell_label(program: &str) -> SharedString {
    let name = PathBuf::from(program)
        .file_stem()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match name.as_str() {
        "pwsh" => "PowerShell 7（pwsh.exe）".into(),
        "powershell" => "Windows PowerShell".into(),
        "cmd" => "命令提示符（cmd.exe）".into(),
        _ => PathBuf::from(program)
            .file_name()
            .map(|name| SharedString::from(name.to_string_lossy().into_owned()))
            .unwrap_or_else(|| SharedString::from(program.to_owned())),
    }
}

/// 两个程序路径的文件名是否相同（忽略大小写）。
fn same_file_name(left: &str, right: &str) -> bool {
    let file_name = |program: &str| {
        PathBuf::from(program)
            .file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
    };
    match (file_name(left), file_name(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// 字段下方的说明文字。
fn field_hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// 下拉列表的空状态文字。
fn empty_hint(text: impl Into<SharedString>, cx: &App) -> Div {
    h_flex()
        .h_24()
        .justify_center()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// 校验字段并组装设置；错误文本直接展示在对话框底部。
///
/// 以 `current` 为底：基础界面未出现的字段（shell 环境变量、未知的
/// 高级设置）原样保留。
fn settings_from(
    fields: &FieldValues,
    theme: ThemeSetting,
    current: &Settings,
) -> Result<Settings, String> {
    let font_family = fields
        .font_family
        .as_deref()
        .map(str::trim)
        .filter(|family| !family.is_empty())
        .map(str::to_owned);
    let font_size = match fields.font_size.trim() {
        "" => None,
        text => {
            let size = text
                .parse::<f32>()
                .map_err(|_| format!("字号必须是数字：{text}"))?;
            if !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&size) {
                return Err(format!("字号需要在 {MIN_FONT_SIZE}–{MAX_FONT_SIZE} 之间"));
            }
            Some(size)
        }
    };
    let scrollback = match fields.scrollback.trim() {
        "" => crate::settings::DEFAULT_SCROLLBACK,
        text => {
            let lines = text
                .parse::<usize>()
                .map_err(|_| format!("滚动历史必须是正整数：{text}"))?;
            if !(MIN_SCROLLBACK..=MAX_SCROLLBACK).contains(&lines) {
                return Err(format!(
                    "滚动历史需要在 {MIN_SCROLLBACK}–{MAX_SCROLLBACK} 行之间"
                ));
            }
            lines
        }
    };

    let shell = match fields
        .shell_program
        .as_deref()
        .map(str::trim)
        .filter(|program| !program.is_empty())
    {
        None => None,
        Some(program) => {
            let args = fields
                .shell_args
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let mut shell = ShellConfig::new(program).with_args(args);
            if let Some(current) = current.shell() {
                // 环境变量属于高级设置，只通过配置文件维护：保留现值。
                for (key, value) in current.env() {
                    shell = shell.with_env(key.clone(), value.clone());
                }
            }
            Some(shell)
        }
    };

    Ok(current
        .clone()
        .with_theme(theme)
        .with_font_family(font_family)
        .with_font_size(font_size)
        .with_scrollback(scrollback)
        .with_shell(shell))
}

/// 打开基础设置对话框。
///
/// 未安装设置服务（[`crate::init`] 未被调用）时不打开任何界面。
pub fn open_settings_dialog(window: &mut Window, cx: &mut App) {
    let Some(state) = handle(cx) else {
        return;
    };
    let current = state.read(cx).current().clone();

    let default_family = glim_ui::default_mono_font_family(cx);
    let installed_fonts = cx.text_system().all_font_names();
    let (font_items, font_selected) =
        font_choices(&installed_fonts, &default_family, current.font_family());
    let font_family = cx.new(|cx| {
        SelectState::new(SearchableVec::new(font_items), font_selected, window, cx).searchable(true)
    });
    let font_size = cx.new(|cx| {
        let mut input = InputState::new(window, cx)
            .placeholder("默认")
            .min(MIN_FONT_SIZE as f64)
            .max(MAX_FONT_SIZE as f64);
        if let Some(size) = current.font_size() {
            input = input.default_value(format!("{size:.0}"));
        }
        input
    });
    let (shell_items, shell_selected) = shell_choices(
        &available_shells(),
        current.shell().map(ShellConfig::program),
    );
    let shell_program =
        cx.new(|cx| SelectState::new(SearchableVec::new(shell_items), shell_selected, window, cx));
    let shell_args = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder("例如 -NoLogo")
            .default_value(
                current
                    .shell()
                    .map(|shell| shell.args().join(" "))
                    .unwrap_or_default(),
            )
    });
    let scrollback = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(current.scrollback().to_string())
            .min(MIN_SCROLLBACK as f64)
            .max(MAX_SCROLLBACK as f64)
    });

    let initial_error = state
        .read(cx)
        .save_error()
        .or_else(|| state.read(cx).load_warning())
        .map(str::to_owned);
    let draft = cx.new(|_| SettingsDraft {
        theme: current.theme(),
        error: initial_error,
    });

    let commit = {
        let state = state.clone();
        let draft = draft.clone();
        let font_family = font_family.clone();
        let font_size = font_size.clone();
        let shell_program = shell_program.clone();
        let shell_args = shell_args.clone();
        let scrollback = scrollback.clone();
        Rc::new(move |cx: &mut App| -> bool {
            let fields = FieldValues {
                font_family: font_family
                    .read(cx)
                    .selected_value()
                    .cloned()
                    .flatten()
                    .map(|family| family.to_string()),
                font_size: font_size.read(cx).value().to_string(),
                shell_program: shell_program.read(cx).selected_value().cloned().flatten(),
                shell_args: shell_args.read(cx).value().to_string(),
                scrollback: scrollback.read(cx).value().to_string(),
            };
            let theme = draft.read(cx).theme;
            let current = state.read(cx).current().clone();
            let next = match settings_from(&fields, theme, &current) {
                Ok(next) => next,
                Err(message) => {
                    draft.update(cx, |draft, cx| {
                        draft.error = Some(message);
                        cx.notify();
                    });
                    return false;
                }
            };

            state.update(cx, |state, cx| state.apply(next, cx));
            let error = state
                .read(cx)
                .save_error()
                .map(str::to_owned)
                .or_else(|| state.read(cx).load_warning().map(str::to_owned));
            match error {
                Some(message) => {
                    draft.update(cx, |draft, cx| {
                        draft.error = Some(message);
                        cx.notify();
                    });
                    false
                }
                None => true,
            }
        })
    };

    let cancel = {
        let draft = draft.clone();
        Rc::new(move |cx: &mut App| {
            draft.update(cx, |draft, cx| {
                if draft.error.take().is_some() {
                    cx.notify();
                }
            });
        })
    };

    window.open_dialog(cx, move |dialog, _window, cx| {
        let commit_ok = commit.clone();
        let commit_enter = commit.clone();
        let cancel_cancel = cancel.clone();
        let cancel_button = cancel.clone();
        let state_for_note = state.clone();
        let state_for_open = state.clone();
        let draft_view = draft.clone();
        let theme_radio = draft.clone();
        // 「默认」按未选择处理：候选项里永远包含默认项。
        let shell_is_default =
            matches!(shell_program.read(cx).selected_value(), None | Some(None));

        dialog
            .title("设置")
            .w(px(560.))
            .overlay_closable(true)
            .child(
                v_flex()
                    .gap_5()
                    .py_2()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(Label::new("主题"))
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(theme_segment(
                                        "settings-theme-light",
                                        "浅色",
                                        ThemeSetting::Light,
                                        &theme_radio,
                                        cx,
                                    ))
                                    .child(theme_segment(
                                        "settings-theme-dark",
                                        "深色",
                                        ThemeSetting::Dark,
                                        &theme_radio,
                                        cx,
                                    ))
                                    .child(theme_segment(
                                        "settings-theme-system",
                                        "跟随系统",
                                        ThemeSetting::System,
                                        &theme_radio,
                                        cx,
                                    )),
                            )
                            .child(field_hint(
                                "跟随系统会随系统外观在浅色与深色间切换",
                                cx,
                            )),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(Label::new("终端字体"))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_start()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                Select::new(&font_family)
                                                    .id("settings-font-family")
                                                    .accessibility_label("终端字体")
                                                    .search_placeholder("搜索字体")
                                                    .empty(|_, cx| {
                                                        empty_hint("没有匹配的字体", cx)
                                                    }),
                                            ),
                                    )
                                    .child(div().w_32().child(NumberInput::new(&font_size))),
                            )
                            .child(field_hint(
                                "列表为已安装字体；字号留空使用默认 13",
                                cx,
                            )),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(Label::new("默认 shell"))
                            .child(
                                Select::new(&shell_program)
                                    .id("settings-shell-program")
                                    .accessibility_label("默认 shell")
                                    .menu_width(px(420.))
                                    .empty(|_, cx| empty_hint("没有匹配的 shell", cx)),
                            )
                            .child(
                                Input::new(&shell_args)
                                    .id("settings-shell-args")
                                    .disabled(shell_is_default),
                            )
                            .child(field_hint(
                                if shell_is_default {
                                    "「默认」按 pwsh、Windows PowerShell、COMSPEC 顺序查找；启动参数需先选择一个具体 shell"
                                } else {
                                    "启动参数按空格拆分；未列出的程序可在配置文件中指定"
                                },
                                cx,
                            )),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(Label::new("滚动历史"))
                            .child(div().w_32().child(NumberInput::new(&scrollback)))
                            .child(field_hint("单位：行，作用于新会话", cx)),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .pt_3()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(field_hint(
                                        "高级设置（环境变量、快捷键）见配置文件",
                                        cx,
                                    ))
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("settings-open-config")
                                            .ghost()
                                            .xsmall()
                                            .label("打开配置文件位置…")
                                            .on_click(move |_, window, cx| {
                                                reveal_config_location(&state_for_open, window, cx);
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .text_color(cx.theme().muted_foreground)
                                    .child(settings_path_label(&state_for_note, cx)),
                            ),
                    )
                    .when_some(draft_view.read(cx).error.clone(), |this, error| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(error),
                        )
                    }),
            )
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("settings-cancel")
                            .label("取消")
                            .on_click(move |_, window, cx| {
                                cancel_button(cx);
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("settings-save")
                            .primary()
                            .label("保存")
                            .on_click(move |_, window, cx| {
                                if commit_ok(cx) {
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            )
            .on_ok(move |_, _window, cx| commit_enter(cx))
            .on_cancel(move |_, _window, cx| {
                cancel_cancel(cx);
                true
            })
    });
}

/// 主题分段按钮：选中态来自草稿，点击更新草稿。
fn theme_segment(
    id: &'static str,
    label: &'static str,
    setting: ThemeSetting,
    draft: &Entity<SettingsDraft>,
    cx: &App,
) -> impl IntoElement {
    let selected = draft.read(cx).theme == setting;
    let draft = draft.clone();
    Toggle::new(id)
        .small()
        .checked(selected)
        .label(label)
        .on_click(move |_, _window, cx| {
            draft.update(cx, |draft, cx| {
                draft.theme = setting;
                cx.notify();
            });
        })
}

/// 配置文件路径的显示文本；未启用持久化时说明原因。
fn settings_path_label(state: &Entity<SettingsState>, cx: &App) -> String {
    match state.read(cx).path() {
        Some(path) => path.display().to_string(),
        None => "本次运行未启用配置文件".to_owned(),
    }
}

/// 在系统文件管理器中定位配置文件；出错时通知。
fn reveal_config_location(state: &Entity<SettingsState>, window: &mut Window, cx: &mut App) {
    let Some(path) = state.read(cx).path().map(Path::to_path_buf) else {
        window.push_notification(
            Notification::warning("本次运行未启用配置文件").title("设置"),
            cx,
        );
        return;
    };
    let directory = path.parent().map(Path::to_path_buf).unwrap_or(path);
    if let Err(error) = open_directory(&directory) {
        window.push_notification(
            Notification::error(format!("无法打开 {}：{error}", directory.display())).title("设置"),
            cx,
        );
    }
}

/// 用系统默认方式打开目录（平台边界集中在此）。
fn open_directory(directory: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let command = std::process::Command::new("explorer")
        .arg(directory)
        .spawn();
    #[cfg(target_os = "macos")]
    let command = std::process::Command::new("open").arg(directory).spawn();
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let command = std::process::Command::new("xdg-open")
        .arg(directory)
        .spawn();
    command.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::{
        FieldValues, font_choices, same_file_name, settings_from, shell_choices, shell_label,
    };
    use crate::settings::{Settings, ShellConfig, ThemeSetting};
    use glim_terminal::ShellSpec;
    use gpui_kit::SharedString;
    use gpui_kit::component::IndexPath;
    use gpui_kit::component::select::SelectItem as _;

    fn fields() -> FieldValues {
        FieldValues {
            font_family: None,
            font_size: String::new(),
            shell_program: None,
            shell_args: String::new(),
            scrollback: String::new(),
        }
    }

    fn installed(families: &[&str]) -> Vec<String> {
        families.iter().map(|family| (*family).to_owned()).collect()
    }

    #[test]
    fn empty_fields_fall_back_to_defaults() {
        let settings =
            settings_from(&fields(), ThemeSetting::System, &Settings::default()).expect("valid");
        assert_eq!(settings.theme(), ThemeSetting::System);
        assert_eq!(settings.font_family(), None);
        assert_eq!(settings.font_size(), None);
        assert_eq!(settings.scrollback(), crate::settings::DEFAULT_SCROLLBACK);
        assert!(settings.shell().is_none());
    }

    #[test]
    fn filled_fields_map_to_settings() {
        let values = FieldValues {
            font_family: Some(" JetBrains Mono ".to_owned()),
            font_size: "14.5".to_owned(),
            shell_program: Some("C:\\Program Files\\PowerShell\\7\\pwsh.exe".to_owned()),
            shell_args: "-NoLogo -NoProfile".to_owned(),
            scrollback: "25000".to_owned(),
        };
        let settings = settings_from(&values, ThemeSetting::Light, &Settings::default())
            .expect("valid settings");
        assert_eq!(settings.font_family(), Some("JetBrains Mono"));
        assert_eq!(settings.font_size(), Some(14.5));
        assert_eq!(settings.scrollback(), 25_000);
        let shell = settings.shell().expect("shell");
        assert_eq!(
            shell.program(),
            "C:\\Program Files\\PowerShell\\7\\pwsh.exe"
        );
        assert_eq!(shell.args(), ["-NoLogo", "-NoProfile"]);
    }

    #[test]
    fn shell_environment_is_preserved_from_current_settings() {
        let current = Settings::default().with_shell(Some(
            ShellConfig::new("pwsh.exe").with_env("TERM", "xterm-256color"),
        ));
        let values = FieldValues {
            shell_program: Some("pwsh.exe".to_owned()),
            shell_args: "-NoLogo".to_owned(),
            ..fields()
        };
        let settings = settings_from(&values, ThemeSetting::Dark, &current).expect("valid");
        let shell = settings.shell().expect("shell");
        assert_eq!(shell.args(), ["-NoLogo"]);
        assert_eq!(
            shell.env().get("TERM").map(String::as_str),
            Some("xterm-256color")
        );
    }

    #[test]
    fn advanced_keys_survive_the_dialog() {
        // 高级设置写进配置文件后，经对话框保存不应丢失。
        let path = std::env::temp_dir()
            .join("glim-dialog-tests")
            .join(format!("advanced-{}.toml", std::process::id()));
        std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
        std::fs::write(&path, "theme = \"dark\"\n[keys]\nnew_tab = \"ctrl-t\"\n")
            .expect("write config");
        let current: Settings = crate::load_toml(&path).expect("load").expect("file exists");

        let settings = settings_from(
            &FieldValues {
                scrollback: "5000".to_owned(),
                ..fields()
            },
            ThemeSetting::Light,
            &current,
        )
        .expect("valid");
        crate::save_toml(&path, &settings).expect("save");

        let text = std::fs::read_to_string(&path).expect("read back");
        assert!(text.contains("[keys]"), "高级设置保留: {text}");
        assert!(
            text.contains("new_tab = \"ctrl-t\""),
            "高级设置保留: {text}"
        );
        assert!(text.contains("theme = \"light\""), "基础设置更新: {text}");
    }

    #[test]
    fn invalid_numbers_are_rejected_with_reason() {
        let error = settings_from(
            &FieldValues {
                font_size: "大".to_owned(),
                ..fields()
            },
            ThemeSetting::Dark,
            &Settings::default(),
        )
        .expect_err("invalid size");
        assert!(error.contains("字号必须是数字"));

        let error = settings_from(
            &FieldValues {
                scrollback: "10".to_owned(),
                ..fields()
            },
            ThemeSetting::Dark,
            &Settings::default(),
        )
        .expect_err("out of range");
        assert!(error.contains("滚动历史"));

        let error = settings_from(
            &FieldValues {
                scrollback: "abc".to_owned(),
                ..fields()
            },
            ThemeSetting::Dark,
            &Settings::default(),
        )
        .expect_err("not a number");
        assert!(error.contains("滚动历史必须是正整数"));
    }

    #[test]
    fn out_of_range_font_size_is_rejected() {
        let error = settings_from(
            &FieldValues {
                font_size: "100".to_owned(),
                ..fields()
            },
            ThemeSetting::Dark,
            &Settings::default(),
        )
        .expect_err("out of range");
        assert!(error.contains("字号需要在"));
    }

    /// 字体候选项：默认项在首位，缺失字族排在最后并标注未安装。
    #[test]
    fn font_choices_list_default_installed_and_missing() {
        let installed = installed(&["Cascadia Mono", "Consolas"]);
        let (items, selected) = font_choices(&installed, "Consolas", None);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].title(), SharedString::from("默认（Consolas）"));
        assert_eq!(selected, Some(IndexPath::new(0)), "默认时选中默认项");

        let (_items, selected) = font_choices(&installed, "Consolas", Some("consolas"));
        assert_eq!(
            selected,
            Some(IndexPath::new(2)),
            "忽略大小写匹配已安装字族"
        );

        let (items, selected) = font_choices(&installed, "Consolas", Some("JetBrains Mono"));
        assert_eq!(items.len(), 4);
        assert_eq!(
            items[3].title(),
            SharedString::from("JetBrains Mono（未安装）"),
            "缺失字族保留在候选中并标注"
        );
        assert_eq!(selected, Some(IndexPath::new(3)));
        let value: Option<SharedString> = Some("JetBrains Mono".into());
        assert_eq!(items[3].value(), &value);
    }

    /// shell 候选项：默认项在首位，自定义程序保留且按文件名匹配候选。
    #[test]
    fn shell_choices_list_default_available_and_custom() {
        let available = [
            ShellSpec::new("C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
            ShellSpec::new("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
            ShellSpec::new("C:\\Windows\\System32\\cmd.exe"),
        ];

        let (items, selected) = shell_choices(&available, None);
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].title(), SharedString::from("默认（按顺序查找）"));
        assert_eq!(selected, Some(IndexPath::new(0)));
        assert_eq!(
            items[1].title(),
            SharedString::from("PowerShell 7（pwsh.exe）")
        );
        assert_eq!(items[2].title(), SharedString::from("Windows PowerShell"));

        // 配置里写相对程序名时命中候选中同名的完整路径。
        let (_, selected) = shell_choices(&available, Some("powershell.exe"));
        assert_eq!(selected, Some(IndexPath::new(2)));

        // 未在候选中的程序作为最后一项保留。
        let (items, selected) = shell_choices(&available, Some("C:\\msys64\\usr\\bin\\bash.exe"));
        assert_eq!(items.len(), 5);
        assert_eq!(selected, Some(IndexPath::new(4)));
        assert_eq!(items[4].title(), SharedString::from("bash.exe"));
        assert_eq!(
            items[4].value(),
            &Some("C:\\msys64\\usr\\bin\\bash.exe".to_owned())
        );
    }

    /// 展示名与文件名比较。
    #[test]
    fn shell_labels_and_file_name_matching() {
        assert_eq!(
            shell_label("pwsh"),
            SharedString::from("PowerShell 7（pwsh.exe）")
        );
        assert_eq!(
            shell_label("cmd.exe"),
            SharedString::from("命令提示符（cmd.exe）")
        );
        assert_eq!(shell_label("bash.exe"), SharedString::from("bash.exe"));

        assert!(same_file_name("powershell.exe", "C:\\x\\PowerShell.EXE"));
        assert!(!same_file_name("pwsh.exe", "powershell.exe"));
    }

    /// 搜索匹配：默认项不被字族搜索命中；shell 支持按路径搜索。
    #[test]
    fn search_matching_keeps_the_default_item_out_of_the_way() {
        let (items, _) = font_choices(&installed(&["Cascadia Mono", "Consolas"]), "Consolas", None);
        let matched = items
            .iter()
            .filter(|item| item.matches("consolas"))
            .map(|item| item.title())
            .collect::<Vec<_>>();
        assert_eq!(matched, vec![SharedString::from("Consolas")]);

        let matched = items
            .iter()
            .filter(|item| item.matches("默认"))
            .map(|item| item.title())
            .collect::<Vec<_>>();
        assert_eq!(matched, vec![SharedString::from("默认（Consolas）")]);

        let available = [ShellSpec::new("C:\\Program Files\\PowerShell\\7\\pwsh.exe")];
        let (items, _) = shell_choices(&available, None);
        let matched = items
            .iter()
            .filter(|item| item.matches("program files"))
            .count();
        assert_eq!(matched, 1, "按路径搜索命中候选");
    }
}
