//! 基础设置界面：常用设置的对话框（`Ctrl+,` 或工具栏入口）。
//!
//! 对话框编辑一份草稿，`保存` 才写回 [`SettingsState`]；`取消`、`Esc`
//! 或点击遮罩放弃修改。校验失败或保存失败时对话框保持打开，并在底部
//! 显示原因。文字类字段直接读 [`InputState`]；主题的草稿选择保存在
//! [`SettingsDraft`] 实体中，供内容重建时读取。

use std::path::Path;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _, Toggle};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputState, NumberInput};
use gpui_kit::component::label::Label;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

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
    font_family: String,
    font_size: String,
    shell_program: String,
    shell_args: String,
    scrollback: String,
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
    let font_family = match fields.font_family.trim() {
        "" => None,
        family => Some(family.to_owned()),
    };
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

    let shell = match fields.shell_program.trim() {
        "" => None,
        program => {
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

    let font_family = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder("默认（Cascadia Mono / Consolas）")
            .default_value(current.font_family().unwrap_or_default())
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
    let shell_program = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder("默认（Platform shell）")
            .default_value(
                current
                    .shell()
                    .map(|shell| shell.program().to_owned())
                    .unwrap_or_default(),
            )
    });
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
                font_family: font_family.read(cx).value().to_string(),
                font_size: font_size.read(cx).value().to_string(),
                shell_program: shell_program.read(cx).value().to_string(),
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
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("跟随系统会随系统外观在浅色与深色间切换"),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(Label::new("终端字体"))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_start()
                                    .child(div().flex_1().min_w_0().child(Input::new(&font_family)))
                                    .child(div().w_32().child(NumberInput::new(&font_size))),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("字体族留空使用内置默认；字号留空使用默认 13"),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(Label::new("默认 shell"))
                            .child(Input::new(&shell_program))
                            .child(Input::new(&shell_args))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("留空按 pwsh、Windows PowerShell、COMSPEC 顺序查找；变更作用于新会话"),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(Label::new("滚动历史"))
                            .child(div().w_32().child(NumberInput::new(&scrollback)))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("单位：行，作用于新会话"),
                            ),
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
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("高级设置（环境变量、快捷键）见配置文件"),
                                    )
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
    use super::{FieldValues, settings_from};
    use crate::settings::{Settings, ShellConfig, ThemeSetting};

    fn fields() -> FieldValues {
        FieldValues {
            font_family: String::new(),
            font_size: String::new(),
            shell_program: String::new(),
            shell_args: String::new(),
            scrollback: String::new(),
        }
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
            font_family: " JetBrains Mono ".to_owned(),
            font_size: "14.5".to_owned(),
            shell_program: "C:\\Program Files\\PowerShell\\7\\pwsh.exe".to_owned(),
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
            shell_program: "pwsh.exe".to_owned(),
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
}
