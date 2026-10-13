//! 终端会话：ConPTY 生命周期、后台解析与快照读取。
//!
//! 每个会话拥有一个 PTY 与一个后台读取 / 解析线程；界面线程只做
//! 写入、调整尺寸与读取快照。

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, CursorShape as AnsiCursorShape, NamedColor};
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

use crate::keys::TerminalModes;
use crate::options::TerminalOptions;
use crate::shell::default_shell;
use crate::snapshot::{
    CellAttributes, CursorShape, CursorSnapshot, GridCell, GridLine, GridSnapshot, TermColor,
};

/// 会话事件；在后台线程产生、由界面消费。
#[derive(Clone, Debug)]
pub enum SessionEvent {
    /// 终端有新输出（多次输出可能合并为一次通知）。
    Output,
    /// 窗口标题变化。
    Title(String),
    /// 响铃。
    Bell,
    /// 子进程已退出；无法取得退出码时为 `None`。
    Exited { code: Option<i32> },
}

/// 会话事件流；通过 [`SessionEvents::next`] 逐个等待。
pub struct SessionEvents {
    rx: UnboundedReceiver<SessionEvent>,
}

impl SessionEvents {
    /// 等待下一个事件；会话完全关闭后返回 `None`。
    pub async fn next(&mut self) -> Option<SessionEvent> {
        self.rx.next().await
    }
}

/// 运行中的终端会话句柄。
///
/// 持有会话的写入通道与终端状态；丢弃时关闭会话。
pub struct SessionHandle {
    term: Arc<FairMutex<Term<SessionListener>>>,
    sender: EventLoopSender,
    size: Arc<Mutex<WindowSize>>,
}

impl SessionHandle {
    /// 启动会话：创建 ConPTY、启动 shell 并开始后台解析。
    ///
    /// 启动失败（程序不存在、PTY 创建失败等）同步返回错误。
    pub fn start(options: &TerminalOptions) -> io::Result<(Self, SessionEvents)> {
        let shell = options.shell.clone().or_else(default_shell);
        let tty_options = tty::Options {
            shell: shell
                .map(|spec| tty::Shell::new(spec.program().to_owned(), spec.args().to_vec())),
            working_directory: options.working_directory.clone(),
            drain_on_exit: true,
            env: options.env.clone(),
            ..tty::Options::default()
        };

        let size = WindowSize {
            num_lines: options.rows,
            num_cols: options.columns,
            cell_width: 8,
            cell_height: 16,
        };
        let pty = tty::new(&tty_options, size, 0)?;

        let (events, rx) = unbounded();
        let responses = Arc::new(OnceLock::new());
        let size_shared = Arc::new(Mutex::new(size));
        let listener = SessionListener {
            events,
            responses: responses.clone(),
            size: size_shared.clone(),
            exited: Arc::new(AtomicBool::new(false)),
        };

        let term = Arc::new(FairMutex::new(Term::new(
            Config {
                scrolling_history: options.scrollback,
                ..Config::default()
            },
            &TermDimensions::new(options.rows, options.columns),
            listener.clone(),
        )));

        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let sender = event_loop.channel();
        let _ = responses.set(sender.clone());

        let _thread = event_loop.spawn();

        Ok((
            Self {
                term,
                sender,
                size: size_shared,
            },
            SessionEvents { rx },
        ))
    }

    /// 写入 PTY。
    pub fn write(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let _ = self.sender.send(Msg::Input(bytes.to_vec().into()));
    }

    /// 调整网格与 PTY 尺寸；尺寸不变时不产生任何动作。
    pub fn resize(&self, columns: u16, rows: u16, cell_width: u16, cell_height: u16) {
        let next = WindowSize {
            num_lines: rows.max(1),
            num_cols: columns.max(1),
            cell_width: cell_width.max(1),
            cell_height: cell_height.max(1),
        };

        {
            let mut current = self.size.lock().expect("terminal size lock");
            if current.num_lines == next.num_lines
                && current.num_cols == next.num_cols
                && current.cell_width == next.cell_width
                && current.cell_height == next.cell_height
            {
                return;
            }
            *current = next;
        }

        let _ = self.sender.send(Msg::Resize(next));
        self.term
            .lock()
            .resize(TermDimensions::new(next.num_lines, next.num_cols));
    }

    /// 读取当前可视区域快照。
    pub fn snapshot(&self) -> GridSnapshot {
        let term = self.term.lock();
        let columns = term.columns();
        let rows = term.screen_lines();
        let display_offset = term.grid().display_offset() as i32;

        let content = term.renderable_content();
        let modes = terminal_modes(content.mode);

        let cursor = cursor_from_content(&content, display_offset, rows);
        let cursor = if content.mode.contains(TermMode::SHOW_CURSOR) {
            cursor
        } else {
            None
        };

        let mut lines: Vec<GridLine> = (0..rows)
            .map(|_| GridLine {
                cells: vec![GridCell::blank(); columns],
            })
            .collect();

        for indexed in content.display_iter {
            let row = indexed.point.line.0 + display_offset;
            if row < 0 || row as usize >= rows {
                continue;
            }
            let column = indexed.point.column.0;
            if column >= columns {
                continue;
            }
            lines[row as usize].cells[column] = grid_cell(indexed.cell);
        }

        GridSnapshot {
            columns,
            rows,
            cursor,
            lines,
            modes,
        }
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}

/// 把解析事件转成会话事件；终端请求的应答（设备属性、尺寸查询等）写回 PTY。
#[derive(Clone)]
struct SessionListener {
    events: UnboundedSender<SessionEvent>,
    responses: Arc<OnceLock<EventLoopSender>>,
    size: Arc<Mutex<WindowSize>>,
    exited: Arc<AtomicBool>,
}

impl SessionListener {
    fn respond(&self, bytes: Vec<u8>) {
        if let Some(sender) = self.responses.get() {
            let _ = sender.send(Msg::Input(bytes.into()));
        }
    }

    fn report_exit(&self, code: Option<i32>) {
        if !self.exited.swap(true, Ordering::SeqCst) {
            let _ = self.events.unbounded_send(SessionEvent::Exited { code });
        }
    }
}

impl EventListener for SessionListener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Wakeup => {
                let _ = self.events.unbounded_send(SessionEvent::Output);
            }
            Event::Title(title) => {
                let _ = self.events.unbounded_send(SessionEvent::Title(title));
            }
            Event::Bell => {
                let _ = self.events.unbounded_send(SessionEvent::Bell);
            }
            Event::ChildExit(status) => {
                self.report_exit(status.code());
            }
            Event::Exit => {
                self.report_exit(None);
            }
            Event::PtyWrite(response) => {
                self.respond(response.into_bytes());
            }
            Event::TextAreaSizeRequest(formatter) => {
                let size = *self.size.lock().expect("terminal size lock");
                self.respond(formatter(size).into_bytes());
            }
            _ => {}
        }
    }
}

/// 网格尺寸的 `Dimensions` 实现。
#[derive(Clone, Copy)]
struct TermDimensions {
    lines: usize,
    columns: usize,
}

impl TermDimensions {
    fn new(lines: u16, columns: u16) -> Self {
        Self {
            lines: lines.max(1) as usize,
            columns: columns.max(1) as usize,
        }
    }
}

impl Dimensions for TermDimensions {
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

fn terminal_modes(mode: TermMode) -> TerminalModes {
    let mut modes = TerminalModes::empty();
    modes.set(
        TerminalModes::APP_CURSOR,
        mode.contains(TermMode::APP_CURSOR),
    );
    modes.set(
        TerminalModes::APP_KEYPAD,
        mode.contains(TermMode::APP_KEYPAD),
    );
    modes.set(
        TerminalModes::BRACKETED_PASTE,
        mode.contains(TermMode::BRACKETED_PASTE),
    );
    modes.set(
        TerminalModes::ALT_SCREEN,
        mode.contains(TermMode::ALT_SCREEN),
    );
    modes.set(
        TerminalModes::FOCUS_IN_OUT,
        mode.contains(TermMode::FOCUS_IN_OUT),
    );
    modes
}

fn cursor_from_content(
    content: &alacritty_terminal::term::RenderableContent<'_>,
    display_offset: i32,
    rows: usize,
) -> Option<CursorSnapshot> {
    let shape = match content.cursor.shape {
        AnsiCursorShape::Block => CursorShape::Block,
        AnsiCursorShape::Underline => CursorShape::Underline,
        AnsiCursorShape::Beam => CursorShape::Beam,
        AnsiCursorShape::HollowBlock => CursorShape::HollowBlock,
        AnsiCursorShape::Hidden => return None,
    };

    let row = content.cursor.point.line.0 + display_offset;
    if row < 0 || row as usize >= rows {
        return None;
    }

    Some(CursorSnapshot {
        column: content.cursor.point.column.0,
        row: row as usize,
        shape,
    })
}

fn grid_cell(cell: &alacritty_terminal::term::cell::Cell) -> GridCell {
    let flags = cell.flags;
    GridCell {
        character: if flags.contains(Flags::HIDDEN) {
            ' '
        } else {
            cell.c
        },
        fg: term_color(cell.fg),
        bg: term_color(cell.bg),
        attributes: CellAttributes {
            bold: flags.contains(Flags::BOLD),
            dim: flags.contains(Flags::DIM),
            italic: flags.contains(Flags::ITALIC),
            underline: flags.intersects(Flags::ALL_UNDERLINES),
            strikeout: flags.contains(Flags::STRIKEOUT),
            inverse: flags.contains(Flags::INVERSE),
            wide_char_spacer: flags.contains(Flags::WIDE_CHAR_SPACER),
        },
    }
}

fn term_color(color: Color) -> TermColor {
    match color {
        Color::Named(NamedColor::Foreground) | Color::Named(NamedColor::BrightForeground) => {
            TermColor::Foreground
        }
        Color::Named(NamedColor::Background) => TermColor::Background,
        Color::Named(NamedColor::Cursor) => TermColor::Cursor,
        Color::Named(named) => TermColor::Indexed(named_index(named)),
        Color::Spec(rgb) => TermColor::Rgb(rgb.r, rgb.g, rgb.b),
        Color::Indexed(index) => TermColor::Indexed(index),
    }
}

/// 基础色与亮色映射到 0–15；暗色变体映射回同色相位（变暗由 DIM 标记表达）。
fn named_index(named: NamedColor) -> u8 {
    match named {
        NamedColor::Black | NamedColor::DimBlack => 0,
        NamedColor::Red | NamedColor::DimRed => 1,
        NamedColor::Green | NamedColor::DimGreen => 2,
        NamedColor::Yellow | NamedColor::DimYellow => 3,
        NamedColor::Blue | NamedColor::DimBlue => 4,
        NamedColor::Magenta | NamedColor::DimMagenta => 5,
        NamedColor::Cyan | NamedColor::DimCyan => 6,
        NamedColor::White | NamedColor::DimWhite => 7,
        NamedColor::BrightBlack => 8,
        NamedColor::BrightRed => 9,
        NamedColor::BrightGreen => 10,
        NamedColor::BrightYellow => 11,
        NamedColor::BrightBlue => 12,
        NamedColor::BrightMagenta => 13,
        NamedColor::BrightCyan => 14,
        NamedColor::BrightWhite => 15,
        // 前五类已在上层映射；此处仅为穷尽。
        _ => 7,
    }
}
