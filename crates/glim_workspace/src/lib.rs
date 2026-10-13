//! 工作区：终端会话区域与状态栏。
//!
//! 当前为单会话工作区；标签页、分屏树与工作区恢复在后续阶段加入。

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

use glim_terminal_view::{TerminalStatus, TerminalView, TerminalViewEvent};
use glim_ui::{ThemeCapsule, ThemePreference};

/// 工作区外壳。
///
/// 持有主题偏好并应用到全局主题；承载终端视图，并在状态栏显示
/// 会话状态与主题胶囊。
pub struct WorkspaceView {
    theme_preference: ThemePreference,
    terminal: Entity<TerminalView>,
    _appearance_observer: Subscription,
    _terminal_events: Subscription,
}

impl WorkspaceView {
    /// 创建工作区：启动默认 shell 会话。
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let theme_preference = ThemePreference::default();
        let appearance_observer = cx.observe_window_appearance(window, |this, window, cx| {
            if this.theme_preference.is_system() {
                glim_ui::apply_theme_preference(this.theme_preference, Some(window), cx);
            }
        });

        let terminal = cx.new(|cx| TerminalView::new(window, cx));
        let terminal_events = cx.subscribe(
            &terminal,
            |_this, _terminal, _event: &TerminalViewEvent, cx| {
                cx.notify();
            },
        );

        Self {
            theme_preference,
            terminal,
            _appearance_observer: appearance_observer,
            _terminal_events: terminal_events,
        }
    }

    fn select_theme(
        &mut self,
        preference: &ThemePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.theme_preference == *preference {
            return;
        }
        self.theme_preference = *preference;
        glim_ui::apply_theme_preference(*preference, Some(window), cx);
        cx.notify();
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (dot, shell, status_text) = {
            let terminal = self.terminal.read(cx);
            let (dot, status_text) = match terminal.status() {
                TerminalStatus::Running => (cx.theme().success, "运行中".to_owned()),
                TerminalStatus::Exited { code } => (
                    cx.theme().muted_foreground,
                    match code {
                        Some(code) => format!("已退出（代码 {code}）"),
                        None => "已退出".to_owned(),
                    },
                ),
                TerminalStatus::Failed { .. } => (cx.theme().danger, "启动失败".to_owned()),
            };
            (dot, terminal.shell_label().to_owned(), status_text)
        };

        h_flex()
            .h_9()
            .px_4()
            .gap_2()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .border_t_1()
            .border_color(cx.theme().status_bar_border)
            .bg(cx.theme().status_bar)
            .child(div().size_2().rounded_full().bg(dot))
            .child(shell)
            .child(div().w(px(1.)).h_2p5().bg(cx.theme().border))
            .child(status_text)
            .child(div().flex_1())
            .child(
                ThemeCapsule::new(self.theme_preference).on_select(cx.listener(Self::select_theme)),
            )
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .child(self.terminal.clone()),
            )
            .child(self.render_status_bar(cx))
    }
}
