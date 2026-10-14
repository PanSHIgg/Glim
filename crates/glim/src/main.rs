//! Glim 应用入口：窗口创建、初始化与退出协调。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::Write as _;
use std::path::Path;

use gpui_kit::AppContext as _;
use gpui_kit::component::TitleBar;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use glim_workspace::WorkspaceView;

/// 运行日志（数据目录内）：每次启动一行，启动阶段的 panic 追加一行。
const LOG_FILE_NAME: &str = "glim.log";
/// 日志增长上限（字节）；超过后从空文件重新开始。
const LOG_LIMIT_BYTES: u64 = 256 * 1024;

fn main() {
    let log = glim_settings::data_directory().map(|directory| directory.join(LOG_FILE_NAME));
    if let Some(log) = log.as_deref() {
        prepare_log(log);
        append_log(
            log,
            &format!(
                "glim {} 启动 pid={}",
                env!("CARGO_PKG_VERSION"),
                std::process::id()
            ),
        );
        install_panic_log(log);
    }

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
                // 自定义标题栏：品牌、标签与窗口控件都由 `TitleBar` 承担。
                titlebar: Some(TitlebarOptions {
                    title: Some("Glim".into()),
                    ..TitleBar::title_bar_options()
                }),
                ..TitleBar::window_options()
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

/// 安装 panic 钩子：先把 panic 写进运行日志，再交给默认钩子（控制台输出）。
///
/// 发行版没有控制台（`windows_subsystem = "windows"`），启动阶段的 panic
/// 只表现为窗口不出现；日志让「换一台机器一运行就退出」留下可读的原因。
fn install_panic_log(path: &Path) {
    let path = path.to_owned();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|location| location.to_string())
            .unwrap_or_else(|| "未知位置".to_owned());
        append_log(&path, &format!("panic {location}: {}", panic_message(info)));
        previous(info);
    }));
}

/// panic 的可读消息：载荷通常是 `&str` 或 `String`。
fn panic_message<'a>(info: &'a std::panic::PanicHookInfo<'_>) -> &'a str {
    payload_message(info.payload())
}

/// 从 panic 载荷取消息；非字符串载荷给出说明。
fn payload_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<非字符串 panic>")
}

/// 追加一行到日志；数据目录不存在时先创建，仍失败则忽略。
fn append_log(path: &Path, line: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(file, "{line}");
}

/// 日志超过上限时清空重来，避免无限增长。
fn prepare_log(path: &Path) {
    if std::fs::metadata(path).is_ok_and(|meta| meta.len() > LOG_LIMIT_BYTES) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::{LOG_LIMIT_BYTES, append_log, payload_message, prepare_log};
    use std::path::PathBuf;

    /// 测试用目录：`${TEMP}/glim-log-tests/<name>/`。
    fn test_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir()
            .join("glim-log-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create test directory");
        directory
    }

    #[test]
    fn append_log_creates_file_and_keeps_previous_lines() {
        let log = test_directory("append").join("glim.log");
        append_log(&log, "第一行");
        append_log(&log, "第二行");
        assert_eq!(
            std::fs::read_to_string(&log).expect("read log"),
            "第一行\n第二行\n"
        );
    }

    #[test]
    fn append_log_creates_missing_data_directory() {
        // 首次启动时数据目录还不存在：日志要能自己建目录写下去。
        let log = test_directory("missing-directory")
            .join("Glim")
            .join("glim.log");
        append_log(&log, "启动");
        assert_eq!(std::fs::read_to_string(&log).expect("read log"), "启动\n");
    }

    #[test]
    fn append_log_ignores_unwritable_path() {
        // 父路径上是文件而不是目录：写入失败被忽略，不影响启动。
        let directory = test_directory("unwritable");
        let blocker = directory.join("blocker");
        std::fs::write(&blocker, "not a directory").expect("write blocker");
        let log = blocker.join("glim.log");
        append_log(&log, "不会写入");
        assert!(!log.exists());
    }

    #[test]
    fn prepare_log_removes_oversized_log_only() {
        let directory = test_directory("oversized");
        let oversized = directory.join("oversized.log");
        std::fs::write(&oversized, vec![b'x'; LOG_LIMIT_BYTES as usize + 1]).expect("write log");
        prepare_log(&oversized);
        assert!(!oversized.exists(), "超限日志被删除，下次启动从空文件开始");

        let kept = directory.join("kept.log");
        std::fs::write(&kept, "glim 0.1.0 启动\n").expect("write log");
        prepare_log(&kept);
        assert!(kept.exists(), "未超限的日志保留");
    }

    #[test]
    fn payload_message_reads_common_panic_payloads() {
        let owned = String::from("文本载荷");
        assert_eq!(payload_message(&owned), "文本载荷");
        assert_eq!(payload_message(&"静态载荷"), "静态载荷");
        assert_eq!(payload_message(&42_u32), "<非字符串 panic>");
    }

    #[test]
    fn panic_log_records_message_and_location() {
        let log = test_directory("panic").join("glim.log");
        let previous = std::panic::take_hook();
        super::install_panic_log(&log);
        let _ = std::panic::catch_unwind(|| panic!("启动失败示例"));
        std::panic::set_hook(previous);

        let text = std::fs::read_to_string(&log).expect("read log");
        assert!(text.contains("panic"), "记录 panic 位置：{text}");
        assert!(text.contains("启动失败示例"), "记录 panic 消息：{text}");
    }
}
