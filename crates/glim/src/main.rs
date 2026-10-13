//! Glim 应用入口：窗口创建、初始化与退出协调。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use gpui_kit::*;

use glim_workspace::WorkspaceView;

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
                cx.new(|cx| WorkspaceView::new(window, cx))
            })
            .expect("failed to open window");
        });
}
