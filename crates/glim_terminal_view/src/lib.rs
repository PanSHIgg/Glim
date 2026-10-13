//! 终端视图：终端绘制、焦点、输入、鼠标、选择与滚动。
//!
//! 视图持有一个终端会话：会话在后台运行，视图消费会话事件、
//! 把键盘输入编码后写入 PTY，并用自定义元素绘制当前快照。
//! 输入法、选择与滚动在后续阶段加入。

mod element;
mod input;

use std::sync::Arc;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

use glim_terminal::{GridSnapshot, SessionEvent, SessionEvents, SessionHandle, TerminalOptions};
use glim_ui::TerminalPalette;

use element::TerminalGridElement;
use input::keystroke_bytes;

/// 会话状态；工作区状态栏据此显示。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalStatus {
    /// 运行中。
    Running,
    /// 已退出；`code` 为退出码，无法取得时为 `None`。
    Exited {
        /// 子进程退出码。
        code: Option<i32>,
    },
    /// 启动失败。
    Failed {
        /// 失败原因。
        message: String,
    },
}

/// 终端视图事件。
#[derive(Clone, Copy, Debug)]
pub enum TerminalViewEvent {
    /// 会话状态变化（启动失败、退出等）。
    StatusChanged,
}

impl EventEmitter<TerminalViewEvent> for TerminalView {}

/// 终端视图实体。
pub struct TerminalView {
    session: Option<SessionHandle>,
    status: TerminalStatus,
    snapshot: Option<Arc<GridSnapshot>>,
    focus_handle: FocusHandle,
    shell_label: String,
    applied_size: Option<(u16, u16)>,
    _events_task: Option<Task<()>>,
}

impl TerminalView {
    /// 创建视图并启动默认 shell 会话。
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let options = TerminalOptions::default();
        let shell_label = options
            .shell()
            .cloned()
            .or_else(glim_terminal::default_shell)
            .map(|shell| shell.display_name())
            .unwrap_or_else(|| "shell".to_owned());

        let focus_handle = cx.focus_handle();

        match SessionHandle::start(&options) {
            Ok((session, events)) => {
                let events_task = cx.spawn(async move |this, cx| {
                    Self::consume_events(this, events, cx).await;
                });

                let mut view = Self {
                    session: Some(session),
                    status: TerminalStatus::Running,
                    snapshot: None,
                    focus_handle,
                    shell_label,
                    applied_size: None,
                    _events_task: Some(events_task),
                };
                view.refresh(cx);
                window.focus(&view.focus_handle, cx);
                view
            }
            Err(error) => Self {
                session: None,
                status: TerminalStatus::Failed {
                    message: error.to_string(),
                },
                snapshot: None,
                focus_handle,
                shell_label,
                applied_size: None,
                _events_task: None,
            },
        }
    }

    async fn consume_events(this: WeakEntity<Self>, mut events: SessionEvents, cx: &mut AsyncApp) {
        while let Some(event) = events.next().await {
            let updated = this.update(cx, |view, cx| view.handle_session_event(event, cx));
            if updated.is_err() {
                break;
            }
        }
    }

    fn handle_session_event(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        match event {
            SessionEvent::Output => self.refresh(cx),
            SessionEvent::Title(_) => {}
            SessionEvent::Bell => {}
            SessionEvent::Exited { code } => {
                self.status = TerminalStatus::Exited { code };
                self.refresh(cx);
                cx.emit(TerminalViewEvent::StatusChanged);
            }
        }
    }

    /// 工作区状态栏读取的会话状态。
    pub fn status(&self) -> &TerminalStatus {
        &self.status
    }

    /// 当前 shell 的展示名。
    pub fn shell_label(&self) -> &str {
        &self.shell_label
    }

    /// 元素测量后上报网格尺寸；尺寸变化时调整会话并刷新快照。
    pub(crate) fn apply_grid_size(
        &mut self,
        columns: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
        cx: &mut Context<Self>,
    ) {
        if self.applied_size == Some((columns, rows)) {
            return;
        }
        self.applied_size = Some((columns, rows));

        if let Some(session) = &self.session {
            session.resize(columns, rows, cell_width, cell_height);
        }
        self.refresh(cx);
    }

    /// 更新快照并请求重绘。
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            self.snapshot = Some(Arc::new(session.snapshot()));
        }
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            return;
        };
        let modes = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.modes)
            .unwrap_or_default();
        if let Some(bytes) = keystroke_bytes(&event.keystroke, event.prefer_character_input, modes)
        {
            session.write(&bytes);
            cx.stop_propagation();
        }
    }

    fn render_failed(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let message = match &self.status {
            TerminalStatus::Failed { message } => message.clone(),
            _ => String::new(),
        };

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(cx.theme().background)
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child("无法启动 shell"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(message),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.shell_label.clone()),
            )
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette: TerminalPalette = glim_ui::terminal_palette(cx);
        let focused = self.focus_handle.is_focused(window);

        let content = match (&self.session, &self.snapshot) {
            (Some(_), Some(snapshot)) => TerminalGridElement {
                view: cx.entity().downgrade(),
                snapshot: snapshot.clone(),
                palette,
                font_family: cx.theme().mono_font_family.clone(),
                font_size: cx.theme().mono_font_size,
                focused,
            }
            .into_any_element(),
            (Some(_), None) => div().into_any_element(),
            (None, _) => self.render_failed(cx).into_any_element(),
        };

        h_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context("Terminal")
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event, window, cx| {
                    window.focus(&this.focus_handle, cx);
                }),
            )
            .bg(palette.background)
            .text_color(palette.foreground)
            .child(content)
    }
}
