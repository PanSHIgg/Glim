//! Glim 应用入口：窗口创建、初始化与退出协调。
//!
//! 当前为阶段 0 的窗口外壳：原生窗口、产品主题与状态栏右下角的
//! 主题胶囊；终端会话在后续阶段接入。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

use glim_ui::{ThemeCapsule, ThemePreference};

/// 工作区外壳。
///
/// 持有主题偏好并把它应用到全局主题；[`ThemePreference::System`]
/// 时观察窗口外观变化并实时同步。
struct GlimWorkspace {
    theme_preference: ThemePreference,
    _appearance_observer: Subscription,
}

impl GlimWorkspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let theme_preference = ThemePreference::default();
        let appearance_observer = cx.observe_window_appearance(window, |this, window, cx| {
            if this.theme_preference.is_system() {
                glim_ui::apply_theme_preference(this.theme_preference, Some(window), cx);
            }
        });
        Self {
            theme_preference,
            _appearance_observer: appearance_observer,
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
        h_flex()
            .h_9()
            .px_4()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .justify_end()
            .border_t_1()
            .border_color(cx.theme().status_bar_border)
            .bg(cx.theme().status_bar)
            .child(
                ThemeCapsule::new(self.theme_preference).on_select(cx.listener(Self::select_theme)),
            )
    }
}

impl Render for GlimWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(div().flex_1())
            .child(self.render_status_bar(cx))
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            glim_ui::init(cx);

            let window_options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1200.), px(750.)), cx)),
                window_min_size: Some(size(px(800.), px(600.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Glim".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };

            gpui_kit::open_window(window_options, cx, |window, cx| {
                cx.new(|cx| GlimWorkspace::new(window, cx))
            })
            .expect("failed to open window");
        });
}
