//! Windows PTY 探针：验证 shell 启动、读写、调整尺寸、子进程退出与失败错误传递。
//!
//! 用法：`cargo run -p glim_terminal --example pty_probe [shell]`。
//! 未指定 shell 时，按 `pwsh.exe`、`powershell.exe`、`%COMSPEC%` 的顺序查找。
//! 全部检查通过时以退出码 0 结束；任一检查失败输出 `[fail]` 并返回非零退出码。

use std::io;
use std::path::PathBuf;
use std::process::{ExitCode, ExitStatus};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg, State};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty::{self, Pty};

/// 初始终端尺寸（行列）。
const INITIAL_LINES: usize = 24;
const INITIAL_COLUMNS: usize = 80;
/// 探针调整后的终端尺寸（行列）。
const RESIZED_LINES: usize = 30;
const RESIZED_COLUMNS: usize = 100;
/// 单元格像素尺寸，仅用于构造 `WindowSize`，不影响文本解析。
const CELL_WIDTH: u16 = 8;
const CELL_HEIGHT: u16 = 16;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const EXIT_TIMEOUT: Duration = Duration::from_secs(10);
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

fn main() -> ExitCode {
    let shell = resolve_shell();
    println!("glim PTY probe");
    println!("shell: {}", shell.describe());
    println!();

    let mut report = Report::default();
    check_spawn_failure(&mut report);
    run_session_checks(&shell, &mut report);

    println!();
    if report.failed == 0 {
        println!("all {} checks passed", report.passed);
        ExitCode::SUCCESS
    } else {
        println!("{} passed, {} failed", report.passed, report.failed);
        ExitCode::FAILURE
    }
}

/// 检查结果汇总。
#[derive(Default)]
struct Report {
    passed: usize,
    failed: usize,
}

impl Report {
    fn record(&mut self, name: &str, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.passed += 1;
                println!("[ok] {name}");
            }
            Err(detail) => {
                self.failed += 1;
                println!("[fail] {name}: {detail}");
            }
        }
    }
}

/// 启动不存在的程序必须同步返回错误（失败错误传递）。
fn check_spawn_failure(report: &mut Report) {
    let shell = tty::Shell::new("glim-missing-shell.exe".to_owned(), Vec::new());
    match ProbeSession::start_shell(shell, ProbeSize::initial()) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            report.record("spawn failure propagates as NotFound", Ok(()));
        }
        Err(error) => {
            report.record(
                "spawn failure propagates as NotFound",
                Err(format!("unexpected error: {error}")),
            );
        }
        Ok(session) => {
            session.request_shutdown();
            report.record(
                "spawn failure propagates as NotFound",
                Err("spawning a missing shell unexpectedly succeeded".to_owned()),
            );
        }
    }
}

fn run_session_checks(shell: &Shell, report: &mut Report) {
    let mut session = match ProbeSession::start(shell) {
        Ok(session) => session,
        Err(error) => {
            report.record("shell session starts", Err(format!("{error}")));
            return;
        }
    };
    report.record("shell session starts", Ok(()));

    report.record(
        "shell prints a prompt",
        describe_wait(session.wait_for_text(">", STARTUP_TIMEOUT), "prompt"),
    );

    session.write_line("Write-Output 'glim-probe-ok'");
    report.record(
        "shell echoes command output",
        describe_wait(
            session.wait_for_text("glim-probe-ok", COMMAND_TIMEOUT),
            "command output",
        ),
    );

    let resized = ProbeSize::new(RESIZED_LINES, RESIZED_COLUMNS);
    session.resize(resized);
    let grid_ok = {
        let term = session.term.lock();
        term.screen_lines() == RESIZED_LINES && term.columns() == RESIZED_COLUMNS
    };
    report.record(
        "terminal grid resizes",
        if grid_ok {
            Ok(())
        } else {
            Err("grid did not adopt the new size".to_owned())
        },
    );

    session.write_line(
        "Write-Output ('size=' + [Console]::WindowWidth + 'x' + [Console]::WindowHeight)",
    );
    let expected = format!("size={RESIZED_COLUMNS}x{RESIZED_LINES}");
    report.record(
        "child process observes the resize",
        describe_wait(
            session.wait_for_text(&expected, COMMAND_TIMEOUT),
            "resized child console",
        ),
    );

    session.write_line("exit");
    report.record(
        "child process exits with status 0",
        match session.wait_for_exit(EXIT_TIMEOUT) {
            Wait::Exited(status) if status.success() => Ok(()),
            Wait::Exited(status) => Err(format!("exit status: {status:?}")),
            Wait::TimedOut => Err("no child exit within timeout".to_owned()),
            Wait::Seen => Err("unexpected wait outcome".to_owned()),
        },
    );

    report.record(
        "io thread ends after child exit",
        if session.join_reader(JOIN_TIMEOUT) {
            Ok(())
        } else {
            Err("reader thread did not stop".to_owned())
        },
    );

    // 运行中的会话收到关闭请求后应尽快结束并回收读取线程。
    let session = match ProbeSession::start(shell) {
        Ok(session) => session,
        Err(error) => {
            report.record(
                "live session shuts down on request",
                Err(format!("{error}")),
            );
            return;
        }
    };
    session.request_shutdown();
    report.record(
        "live session shuts down on request",
        if session.join_reader(JOIN_TIMEOUT) {
            Ok(())
        } else {
            Err("reader thread did not stop after shutdown".to_owned())
        },
    );
}

fn describe_wait(wait: Wait, name: &str) -> Result<(), String> {
    match wait {
        Wait::Seen => Ok(()),
        Wait::Exited(status) => Err(format!("shell exited before {name}: {status:?}")),
        Wait::TimedOut => Err(format!("no {name} within timeout")),
    }
}

/// 等待结果。
enum Wait {
    /// 目标文本出现。
    Seen,
    /// 子进程已退出。
    Exited(ExitStatus),
    /// 超时。
    TimedOut,
}

/// 终端尺寸。
#[derive(Clone, Copy)]
struct ProbeSize {
    lines: usize,
    columns: usize,
}

impl ProbeSize {
    fn new(lines: usize, columns: usize) -> Self {
        Self { lines, columns }
    }

    fn initial() -> Self {
        Self::new(INITIAL_LINES, INITIAL_COLUMNS)
    }

    fn window_size(self) -> WindowSize {
        WindowSize {
            num_lines: self.lines as u16,
            num_cols: self.columns as u16,
            cell_width: CELL_WIDTH,
            cell_height: CELL_HEIGHT,
        }
    }
}

impl Dimensions for ProbeSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// 把解析事件送回探针线程。
#[derive(Clone)]
struct ProbeListener {
    events: Sender<Event>,
}

impl EventListener for ProbeListener {
    fn send_event(&self, event: Event) {
        let _ = self.events.send(event);
    }
}

type ProbeEventLoop = EventLoop<Pty, ProbeListener>;

/// 一个探针会话：PTY、终端状态与事件循环线程。
struct ProbeSession {
    term: Arc<FairMutex<Term<ProbeListener>>>,
    io: EventLoopSender,
    events: Receiver<Event>,
    reader_thread: JoinHandle<(ProbeEventLoop, State)>,
    size: ProbeSize,
    child_exit: Option<ExitStatus>,
}

impl ProbeSession {
    fn start(shell: &Shell) -> io::Result<Self> {
        let shell = tty::Shell::new(shell.program.clone(), shell.args.clone());
        Self::start_shell(shell, ProbeSize::initial())
    }

    fn start_shell(shell: tty::Shell, size: ProbeSize) -> io::Result<Self> {
        let options = tty::Options {
            shell: Some(shell),
            drain_on_exit: true,
            ..Default::default()
        };

        let pty = tty::new(&options, size.window_size(), 0)?;

        let (events_tx, events) = mpsc::channel();
        let listener = ProbeListener { events: events_tx };
        let term = Arc::new(FairMutex::new(Term::new(
            Config {
                scrolling_history: 10_000,
                ..Config::default()
            },
            &size,
            listener.clone(),
        )));

        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let io = event_loop.channel();
        let reader_thread = event_loop.spawn();

        Ok(Self {
            term,
            io,
            events,
            reader_thread,
            size,
            child_exit: None,
        })
    }

    fn write(&self, bytes: &[u8]) {
        let _ = self.io.send(Msg::Input(bytes.to_vec().into()));
    }

    fn write_line(&self, line: &str) {
        self.write(format!("{line}\r").as_bytes());
    }

    fn resize(&mut self, size: ProbeSize) {
        let _ = self.io.send(Msg::Resize(size.window_size()));
        self.term.lock().resize(size);
        self.size = size;
    }

    fn request_shutdown(&self) {
        let _ = self.io.send(Msg::Shutdown);
    }

    /// 显示区当前文本，按行拼接。
    fn text(&self) -> String {
        let term = self.term.lock();
        let content = term.renderable_content();
        let mut text = String::new();
        let mut current_line = None;
        for cell in content.display_iter {
            let line = cell.point.line.0;
            if current_line != Some(line) {
                if current_line.is_some() {
                    text.push('\n');
                }
                current_line = Some(line);
            }
            text.push(cell.cell.c);
        }
        text
    }

    /// 处理解析事件；终端请求的应答（如设备属性）写回 PTY。
    fn pump_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::PtyWrite(response) => self.write(response.as_bytes()),
                Event::TextAreaSizeRequest(format) => {
                    let response = format(self.size.window_size());
                    self.write(response.as_bytes());
                }
                Event::ChildExit(status) => self.child_exit = Some(status),
                _ => {}
            }
        }
    }

    fn wait_for_text(&mut self, needle: &str, timeout: Duration) -> Wait {
        let deadline = Instant::now() + timeout;
        loop {
            self.pump_events();
            if self.text().contains(needle) {
                return Wait::Seen;
            }
            if let Some(status) = self.child_exit {
                return Wait::Exited(status);
            }
            if Instant::now() >= deadline {
                return Wait::TimedOut;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Wait {
        let deadline = Instant::now() + timeout;
        loop {
            self.pump_events();
            if let Some(status) = self.child_exit {
                return Wait::Exited(status);
            }
            if Instant::now() >= deadline {
                return Wait::TimedOut;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// 在超时内等待事件循环线程结束；超时返回 `false`。
    fn join_reader(self, timeout: Duration) -> bool {
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = self.reader_thread.join();
            let _ = done_tx.send(());
        });
        done_rx.recv_timeout(timeout).is_ok()
    }
}

/// 要启动的 shell。
struct Shell {
    program: String,
    args: Vec<String>,
}

impl Shell {
    fn describe(&self) -> String {
        if self.args.is_empty() {
            self.program.clone()
        } else {
            format!("{} {}", self.program, self.args.join(" "))
        }
    }
}

fn resolve_shell() -> Shell {
    if let Some(program) = std::env::args().nth(1) {
        return Shell {
            program,
            args: Vec::new(),
        };
    }
    for candidate in ["pwsh.exe", "powershell.exe"] {
        if which(candidate).is_some() {
            return Shell {
                program: candidate.to_owned(),
                args: Vec::new(),
            };
        }
    }
    if let Some(comspec) = std::env::var_os("COMSPEC") {
        return Shell {
            program: comspec.to_string_lossy().into_owned(),
            args: Vec::new(),
        };
    }
    Shell {
        program: "cmd.exe".to_owned(),
        args: Vec::new(),
    }
}

fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}
