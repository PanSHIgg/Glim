//! Glim 应用入口：窗口创建、初始化与退出协调。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use gpui_kit::AppContext as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use glim_workspace::WorkspaceView;

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            glim_ui::init(cx);

            // 设置先于窗口：主题与字体在首帧前生效，工作区恢复数据
            // 决定窗口尺寸与首个布局。
            let settings = glim_settings::init(cx);
            let mut warnings = Vec::new();
            {
                let current = settings.read(cx).current().clone();
                if let Some(warning) = glim_ui::apply_appearance(
                    current.theme().to_preference(),
                    current.font_family(),
                    current.font_size(),
                    None,
                    cx,
                ) {
                    warnings.push(warning);
                }
                if let Some(warning) = glim_settings::load_warning(cx) {
                    warnings.push(warning);
                }
            }
            glim_workspace::init(cx, glim_settings::data_directory());

            let window_options = WindowOptions {
                window_bounds: Some(
                    glim_workspace::window_bounds(cx)
                        .unwrap_or_else(|| WindowBounds::centered(size(px(1200.), px(750.)), cx)),
                ),
                window_min_size: Some(size(px(800.), px(600.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Glim".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };

            let (handle, _) = gpui_kit::open_window(window_options, cx, |window, cx| {
                cx.new(|cx| WorkspaceView::new(window, cx))
            })
            .expect("failed to open window");

            // 启动提示在窗口就绪后通知：配置读取问题、目录回退等。
            warnings.extend(glim_workspace::take_startup_warnings(cx));
            for warning in warnings {
                let _ = cx.update_window(handle, |_, window, cx| {
                    window.push_notification(Notification::warning(warning).title("Glim"), cx);
                });
            }
        });
}
