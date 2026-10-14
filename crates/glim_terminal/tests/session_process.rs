//! 真实 PTY 测试：运行中任务检测与工作目录查询。
//!
//! 关闭确认依赖子进程检测：空闲 shell 没有子进程，前台命令会作为
//! 子进程运行；分屏继承依赖 shell 的当前工作目录。

#![cfg(windows)]

use std::thread::sleep;
use std::time::{Duration, Instant};

use glim_terminal::{SessionHandle, TerminalOptions};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const BUSY_TIMEOUT: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// 空闲 shell 没有子进程；启动长命令后报告运行中，中断后恢复空闲。
#[test]
fn session_reports_running_task_while_child_process_exists() {
    let options = TerminalOptions::default().with_dimensions(100, 30);
    let (session, _events) = SessionHandle::start(&options).expect("start session");

    wait_for_text(&session, ">", STARTUP_TIMEOUT).expect("shell prints a prompt");
    assert!(
        !session.has_running_task(),
        "an idle shell must not report a running task"
    );

    // `ping` 是独立的子进程（`Start-Sleep` 等 PowerShell 内建命令不会产生子进程）。
    // 用较短次数保证测试在有限时间内观察到“运行中 → 空闲”的变化。
    session.write(b"cmd /c \"ping -n 10 127.0.0.1 > nul\"\r");

    wait_for(
        || session.has_running_task(),
        BUSY_TIMEOUT,
        "child process to be detected",
    )
    .expect("a running command spawns a child process");

    // 命令结束后 shell 回到空闲。
    wait_for(
        || !session.has_running_task(),
        BUSY_TIMEOUT,
        "child process to exit",
    )
    .expect("a finished command releases its child process");
}

/// 会话在指定目录启动后，可以查询到 shell 的工作目录。
#[test]
fn session_reports_working_directory() {
    let directory = std::env::temp_dir().join("glim-session-cwd");
    std::fs::create_dir_all(&directory).expect("create working directory");
    let expected = directory.canonicalize().expect("canonicalize directory");

    let options = TerminalOptions::default().with_working_directory(directory);
    let (session, _events) = SessionHandle::start(&options).expect("start session");

    wait_for(
        || {
            session
                .working_directory()
                .and_then(|path| path.canonicalize().ok())
                .is_some_and(|path| path == expected)
        },
        STARTUP_TIMEOUT,
        "working directory to be reported",
    )
    .expect("the shell process reports its working directory");
}

/// `cd` 之后会话上报的是新的工作目录；保存工作区取的就是这个值。
///
/// Windows PowerShell 的 `cd` 不会同步进程目录，靠启动时注入的提示符
/// 包装（见 `shell::integration_args`）同步；这条用例也守住那个行为。
#[test]
fn session_reports_directory_after_changing_it() {
    let first = std::env::temp_dir().join("glim-session-cwd-first");
    let second = std::env::temp_dir().join("glim-session-cwd-second");
    std::fs::create_dir_all(&first).expect("create first directory");
    std::fs::create_dir_all(&second).expect("create second directory");
    let expected = second
        .canonicalize()
        .expect("canonicalize second directory");

    let options = TerminalOptions::default().with_working_directory(first);
    let (session, _events) = SessionHandle::start(&options).expect("start session");
    wait_for_text(&session, ">", STARTUP_TIMEOUT).expect("shell prints a prompt");

    session.write(format!("cd \"{}\"\r", second.display()).as_bytes());
    wait_for(
        || {
            session
                .working_directory()
                .and_then(|path| path.canonicalize().ok())
                .is_some_and(|path| path == expected)
        },
        STARTUP_TIMEOUT,
        "the changed working directory to be reported",
    )
    .expect("the shell process reports the directory it changed to");
}

/// 轮询直到条件成立。
fn wait_for(condition: impl Fn() -> bool, timeout: Duration, what: &str) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return Ok(());
        }
        sleep(POLL_INTERVAL);
    }
    Err(format!("{what} not observed within {timeout:?}"))
}

/// 轮询网格文本，直到包含目标字符串。
fn wait_for_text(session: &SessionHandle, needle: &str, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if grid_text(session).contains(needle) {
            return Ok(());
        }
        sleep(POLL_INTERVAL);
    }
    Err(format!("{needle:?} not seen within {timeout:?}"))
}

/// 显示区当前文本，按行拼接。
fn grid_text(session: &SessionHandle) -> String {
    let snapshot = session.snapshot();
    let mut text = String::new();
    for line in snapshot.lines.iter() {
        for cell in line.cells.iter() {
            if cell.attributes.wide_char_spacer {
                continue;
            }
            text.push(cell.character);
        }
        text.push('\n');
    }
    text
}
