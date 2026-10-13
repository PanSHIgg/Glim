//! 真实 PTY 测试：会话尺寸必须与创建时给定的网格一致。
//!
//! ConPTY 在客户端控制台就绪前会丢弃尺寸调整，因此会话必须按实际网格尺寸
//! 创建；否则客户端会停留在默认尺寸，表现为内容错位、清屏不完整。

#![cfg(windows)]

use std::thread::sleep;
use std::time::{Duration, Instant};

use glim_terminal::{SessionHandle, TerminalOptions};

/// 期望的网格尺寸（列 × 行）。
const COLUMNS: u16 = 100;
const ROWS: u16 = 30;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const OUTPUT_TIMEOUT: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(40);

/// 会话按给定尺寸启动后，子进程应看到同样的控制台大小。
#[test]
fn session_starts_at_requested_size() {
    let options = TerminalOptions::default().with_dimensions(COLUMNS, ROWS);
    let (session, _events) = SessionHandle::start(&options).expect("start session");

    wait_for_text(&session, ">", STARTUP_TIMEOUT).expect("shell prints a prompt");

    session.write(
        "Write-Output ('SIZE=' + [Console]::WindowWidth + 'x' + [Console]::WindowHeight)\r"
            .as_bytes(),
    );

    let expected = format!("SIZE={COLUMNS}x{ROWS}");
    wait_for_text(&session, &expected, OUTPUT_TIMEOUT)
        .unwrap_or_else(|error| panic!("child console size mismatch: {error}"));
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
