//! 终端会话：ConPTY 生命周期、后台解析与快照读取。
//!
//! 每个会话拥有一个 PTY 与一个后台读取 / 解析线程；界面线程只做
//! 写入、调整尺寸、滚动、选择与读取快照。

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::RegexSearch;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, CursorShape as AnsiCursorShape, NamedColor};
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

use crate::keys::TerminalModes;
use crate::options::TerminalOptions;
use crate::selection::{self, GridSelection};
use crate::shell::default_shell;
use crate::snapshot::{
    CellAttributes, CursorShape, CursorSnapshot, GridCell, GridLine, GridPoint, GridSnapshot,
    SelectionSpan, TermColor,
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

/// 滚动命令。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollCommand {
    /// 相对滚动（正数向历史方向）。
    Delta(i32),
    /// 上翻一页。
    PageUp,
    /// 下翻一页。
    PageDown,
    /// 滚动到最顶部。
    Top,
    /// 返回底部（最新输出）。
    Bottom,
}

/// 查找结果：网格坐标中的匹配范围（两端均包含）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    /// 匹配起点。
    pub start: GridPoint,
    /// 匹配终点。
    pub end: GridPoint,
}

impl SearchMatch {
    /// 生成选择范围（供高亮绘制使用）。
    pub fn to_span(self) -> SelectionSpan {
        SelectionSpan {
            start: self.start,
            end: self.end,
            is_block: false,
        }
    }
}

/// 查找缓存：查询串与已编译的匹配器。
struct SearchCache {
    query: String,
    regex: RegexSearch,
}

/// 运行中的终端会话句柄。
///
/// 持有会话的写入通道与终端状态；丢弃时关闭会话。
pub struct SessionHandle {
    term: Arc<FairMutex<Term<SessionListener>>>,
    sender: EventLoopSender,
    size: Arc<Mutex<WindowSize>>,
    search: Mutex<Option<SearchCache>>,
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
                search: Mutex::new(None),
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

    /// 滚动显示区域（不改变终端内容）。
    pub fn scroll(&self, command: ScrollCommand) {
        let scroll = match command {
            ScrollCommand::Delta(delta) => Scroll::Delta(delta),
            ScrollCommand::PageUp => Scroll::PageUp,
            ScrollCommand::PageDown => Scroll::PageDown,
            ScrollCommand::Top => Scroll::Top,
            ScrollCommand::Bottom => Scroll::Bottom,
        };
        self.term.lock().scroll_display(scroll);
    }

    /// 当前显示偏移：视口顶部上方的历史行数；`0` 表示在底部。
    pub fn display_offset(&self) -> usize {
        self.term.lock().grid().display_offset()
    }

    /// 滚动历史的总行数。
    pub fn history_size(&self) -> usize {
        self.term.lock().history_size()
    }

    /// 滚动到能看见指定网格行（尽量少移动）。
    pub fn scroll_to_reveal(&self, line: i32) {
        let mut term = self.term.lock();
        let delta = reveal_delta(&term, line);
        if delta != 0 {
            term.scroll_display(Scroll::Delta(delta));
        }
    }

    /// 设置或清除选择。
    pub fn set_selection(&self, selection: Option<GridSelection>) {
        apply_selection(&mut self.term.lock(), selection);
    }

    /// 当前选择的文本；没有选择或选择为空时返回 `None`。
    pub fn selection_text(&self) -> Option<String> {
        self.term.lock().selection_to_string()
    }

    /// 从 `after` 之后（含起点方向）查找下一个匹配；找不到时从头回绕。
    ///
    /// `after` 为 `None` 时从历史开头（向后）或末尾（向前）开始；
    /// 查询按字面量处理（特殊字符自动转义），大小写不敏感（查询全小写时）。
    pub fn find(
        &self,
        query: &str,
        after: Option<GridPoint>,
        forward: bool,
    ) -> Option<SearchMatch> {
        if query.is_empty() {
            return None;
        }
        let term = self.term.lock();
        let mut cache = self.search.lock().expect("search cache lock");
        term_find(&term, &mut cache, query, after, forward)
    }

    /// 读取当前可视区域快照。
    pub fn snapshot(&self) -> GridSnapshot {
        term_snapshot(&self.term.lock())
    }
}

/// 读取某个终端网格的可视区域快照；与 [`SessionHandle::snapshot`] 等价。
fn term_snapshot<T: EventListener>(term: &Term<T>) -> GridSnapshot {
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

    let selection_span = content.selection.map(|range| SelectionSpan {
        start: selection::point_to_grid_point(range.start),
        end: selection::point_to_grid_point(range.end),
        is_block: range.is_block,
    });

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
        lines[row as usize].cells[column] =
            grid_cell(indexed.cell, indexed.point, content.selection);
    }

    GridSnapshot {
        columns,
        rows,
        cursor,
        lines,
        modes,
        display_offset: display_offset.max(0) as usize,
        selection: selection_span,
    }
}

/// 把选择意图写入终端网格。
fn apply_selection<T: EventListener>(term: &mut Term<T>, selection: Option<GridSelection>) {
    term.selection = selection.as_ref().map(selection::selection_to_terminal);
}

/// 在终端网格中查找；`after` 为已包含的起点，`None` 表示整体搜索。
fn term_find<T: EventListener>(
    term: &Term<T>,
    cache: &mut Option<SearchCache>,
    query: &str,
    after: Option<GridPoint>,
    forward: bool,
) -> Option<SearchMatch> {
    if cache.as_ref().map(|cache| cache.query.as_str()) != Some(query) {
        let regex = RegexSearch::new(&escape_literal(query)).ok()?;
        *cache = Some(SearchCache {
            query: query.to_owned(),
            regex,
        });
    }
    let cache = cache.as_mut()?;

    let columns = term.columns();
    let last_column = column_index(term.last_column());
    let top = Point::new(term.topmost_line(), Column(0));
    let bottom = Point::new(term.bottommost_line(), Column(last_column));

    let origin = match (forward, after) {
        (true, Some(point)) => advance(point, columns),
        (false, Some(point)) => retreat(point, columns),
        (true, None) => selection::point_to_grid_point(top),
        (false, None) => selection::point_to_grid_point(bottom),
    };
    let origin = Point::new(Line(origin.line), Column(origin.column.min(last_column)));

    let (direction, side) = if forward {
        (Direction::Right, Side::Right)
    } else {
        (Direction::Left, Side::Left)
    };

    let found = term
        .search_next(&mut cache.regex, origin, direction, side, None)
        .or_else(|| {
            // 回绕：从头（或尾）再试一次。
            let wrap_origin = match (after, forward) {
                (Some(_), true) => Some(top),
                (Some(_), false) => Some(bottom),
                (None, _) => None,
            };
            wrap_origin.and_then(|origin| {
                term.search_next(&mut cache.regex, origin, direction, side, None)
            })
        });

    found.map(|range| SearchMatch {
        start: selection::point_to_grid_point(*range.start()),
        end: selection::point_to_grid_point(*range.end()),
    })
}

/// 让指定网格行可见所需的滚轮增量为多少（0 表示已可见）。
fn reveal_delta<T: EventListener>(term: &Term<T>, line: i32) -> i32 {
    let history = term.history_size() as i32;
    let current = term.grid().display_offset() as i32;
    let desired = (-line).clamp(0, history);
    desired - current
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
    modes.set(
        TerminalModes::MOUSE_REPORT_CLICK,
        mode.contains(TermMode::MOUSE_REPORT_CLICK),
    );
    modes.set(
        TerminalModes::MOUSE_DRAG,
        mode.contains(TermMode::MOUSE_DRAG),
    );
    modes.set(
        TerminalModes::MOUSE_MOTION,
        mode.contains(TermMode::MOUSE_MOTION),
    );
    modes.set(TerminalModes::SGR_MOUSE, mode.contains(TermMode::SGR_MOUSE));
    modes.set(
        TerminalModes::UTF8_MOUSE,
        mode.contains(TermMode::UTF8_MOUSE),
    );
    modes.set(
        TerminalModes::ALTERNATE_SCROLL,
        mode.contains(TermMode::ALTERNATE_SCROLL),
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

fn grid_cell(
    cell: &alacritty_terminal::term::cell::Cell,
    point: Point,
    selection: Option<SelectionRange>,
) -> GridCell {
    let flags = cell.flags;
    GridCell {
        character: if flags.contains(Flags::HIDDEN) {
            ' '
        } else {
            cell.c
        },
        zerowidth: cell
            .zerowidth()
            .map(|characters| characters.to_vec())
            .unwrap_or_default(),
        fg: term_color(cell.fg),
        bg: term_color(cell.bg),
        attributes: CellAttributes {
            bold: flags.contains(Flags::BOLD),
            dim: flags.contains(Flags::DIM),
            italic: flags.contains(Flags::ITALIC),
            underline: flags.intersects(Flags::ALL_UNDERLINES),
            strikeout: flags.contains(Flags::STRIKEOUT),
            inverse: flags.contains(Flags::INVERSE),
            wide: flags.contains(Flags::WIDE_CHAR),
            wide_char_spacer: flags.contains(Flags::WIDE_CHAR_SPACER),
        },
        selected: selection.is_some_and(|range| selection_contains(range, point, flags)),
    }
}

/// 判断单元是否被选中；宽字符的前半格被选中时，其占位格也算选中。
fn selection_contains(range: SelectionRange, point: Point, flags: Flags) -> bool {
    range.contains(point)
        || (flags.contains(Flags::WIDE_CHAR)
            && range.contains(Point::new(point.line, point.column + 1)))
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

fn column_index(column: Column) -> usize {
    column.0
}

/// 列内前进一格；越过末列时到下一行行首。
fn advance(point: GridPoint, columns: usize) -> GridPoint {
    if columns == 0 {
        return point;
    }
    if point.column + 1 >= columns {
        GridPoint::new(point.line.saturating_add(1), 0)
    } else {
        GridPoint::new(point.line, point.column + 1)
    }
}

/// 列内后退一格；越过行首时到上一行末列。
fn retreat(point: GridPoint, columns: usize) -> GridPoint {
    if point.column == 0 {
        GridPoint::new(point.line.saturating_sub(1), columns.saturating_sub(1))
    } else {
        GridPoint::new(point.line, point.column - 1)
    }
}

/// 把查询串转义为正则字面量（查找按字面量处理）。
fn escape_literal(query: &str) -> String {
    const META: &[char] = &[
        '\\', '.', '+', '*', '?', '(', ')', '|', '[', ']', '{', '}', '^', '$', '#', '&', '-', '~',
    ];
    let mut escaped = String::with_capacity(query.len());
    for character in query.chars() {
        if META.contains(&character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::Processor;

    use crate::selection::{SelectionKind, SelectionSide};

    fn make_term(columns: usize, rows: usize) -> Term<VoidListener> {
        let size = TermSize::new(columns, rows);
        Term::new(
            Config {
                scrolling_history: 100,
                ..Config::default()
            },
            &size,
            VoidListener,
        )
    }

    fn feed(term: &mut Term<VoidListener>, bytes: &[u8]) {
        let mut processor = Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new();
        processor.advance(term, bytes);
    }

    #[test]
    fn escape_literal_escapes_metacharacters() {
        assert_eq!(escape_literal("a.b(c)"), "a\\.b\\(c\\)");
        assert_eq!(escape_literal("中文"), "中文");
    }

    #[test]
    fn advance_and_retreat_wrap_lines() {
        let columns = 10;
        assert_eq!(advance(GridPoint::new(0, 3), columns), GridPoint::new(0, 4));
        assert_eq!(advance(GridPoint::new(0, 9), columns), GridPoint::new(1, 0));
        assert_eq!(retreat(GridPoint::new(0, 3), columns), GridPoint::new(0, 2));
        assert_eq!(retreat(GridPoint::new(1, 0), columns), GridPoint::new(0, 9));
    }

    #[test]
    fn selection_contains_wide_char_spacer() {
        let lead_only = SelectionRange {
            start: Point::new(Line(0), Column(0)),
            end: Point::new(Line(0), Column(0)),
            is_block: false,
        };
        let spacer_only = SelectionRange {
            start: Point::new(Line(0), Column(1)),
            end: Point::new(Line(0), Column(1)),
            is_block: false,
        };
        // 选中宽字符前半格时，其右侧占位格通过前半格的标记被包含。
        assert!(selection_contains(
            lead_only,
            Point::new(Line(0), Column(0)),
            Flags::WIDE_CHAR
        ));
        // 宽字符前半格未被选中时，右侧占位格不算选中。
        assert!(!selection_contains(
            lead_only,
            Point::new(Line(0), Column(1)),
            Flags::WIDE_CHAR_SPACER
        ));
        // 选中占位格时，宽字符前半格同样视为选中。
        assert!(selection_contains(
            spacer_only,
            Point::new(Line(0), Column(0)),
            Flags::WIDE_CHAR
        ));
        assert!(selection_contains(
            spacer_only,
            Point::new(Line(0), Column(1)),
            Flags::WIDE_CHAR_SPACER
        ));
    }

    #[test]
    fn snapshot_reports_wide_and_combining_characters() {
        let mut term = make_term(20, 4);
        // 中文 + "e" 带组合重音。
        feed(&mut term, "中文e\u{301}".as_bytes());

        let snapshot = term_snapshot(&term);
        assert_eq!(snapshot.lines[0].cells[0].character, '中');
        assert!(snapshot.lines[0].cells[0].attributes.wide);
        assert!(snapshot.lines[0].cells[1].attributes.wide_char_spacer);
        assert_eq!(snapshot.lines[0].cells[2].character, '文');
        assert!(snapshot.lines[0].cells[3].attributes.wide_char_spacer);

        let base = &snapshot.lines[0].cells[4];
        assert_eq!(base.character, 'e');
        assert_eq!(base.zerowidth, vec!['\u{301}']);
    }

    #[test]
    fn snapshot_marks_selected_cells() {
        let mut term = make_term(20, 4);
        feed(&mut term, b"hello\r\nworld");

        // 选中第二行的 "wor"。
        apply_selection(
            &mut term,
            Some(
                GridSelection::new(
                    SelectionKind::Simple,
                    GridPoint::new(1, 0),
                    SelectionSide::Left,
                )
                .with_cursor(GridPoint::new(1, 2), SelectionSide::Right),
            ),
        );

        let snapshot = term_snapshot(&term);
        assert_eq!(
            snapshot.selection,
            Some(SelectionSpan {
                start: GridPoint::new(1, 0),
                end: GridPoint::new(1, 2),
                is_block: false,
            })
        );
        assert!(snapshot.lines[1].cells[0].selected);
        assert!(snapshot.lines[1].cells[2].selected);
        assert!(!snapshot.lines[1].cells[3].selected);
        assert!(!snapshot.lines[0].cells[0].selected);
    }

    #[test]
    fn selection_text_extracts_selection() {
        let mut term = make_term(20, 4);
        feed(&mut term, b"hello world");
        apply_selection(
            &mut term,
            Some(
                GridSelection::new(
                    SelectionKind::Simple,
                    GridPoint::new(0, 6),
                    SelectionSide::Left,
                )
                .with_cursor(GridPoint::new(0, 10), SelectionSide::Right),
            ),
        );
        let text = term.selection_to_string();
        assert_eq!(text.as_deref(), Some("world"));
    }

    #[test]
    fn find_navigates_and_wraps() {
        let mut term = make_term(20, 6);
        feed(&mut term, b"alpha\r\nbeta\r\nalpha\r\ngamma");
        let mut cache = None;

        let first = term_find(&term, &mut cache, "alpha", None, true).expect("first match");
        assert_eq!(first.start, GridPoint::new(0, 0));
        assert_eq!(first.end, GridPoint::new(0, 4));

        let second =
            term_find(&term, &mut cache, "alpha", Some(first.end), true).expect("second match");
        assert_eq!(second.start, GridPoint::new(2, 0));

        // 末尾之后回绕到第一个匹配。
        let wrapped =
            term_find(&term, &mut cache, "alpha", Some(second.end), true).expect("wrapped match");
        assert_eq!(wrapped.start, GridPoint::new(0, 0));

        // 查询全小写时大小写不敏感；含大写时区分大小写。
        assert!(term_find(&term, &mut cache, "BETA", None, true).is_none());
        let beta = term_find(&term, &mut cache, "beta", None, true).expect("lowercase match");
        assert_eq!(beta.start, GridPoint::new(1, 0));

        // 向前（向上）查找。
        let previous =
            term_find(&term, &mut cache, "alpha", Some(beta.start), false).expect("previous match");
        assert_eq!(previous.start, GridPoint::new(0, 0));
    }

    #[test]
    fn find_matches_wide_characters() {
        let mut term = make_term(20, 4);
        feed(&mut term, "中文测试".as_bytes());
        let mut cache = None;
        let found = term_find(&term, &mut cache, "文测", None, true).expect("wide match");
        assert_eq!(found.start, GridPoint::new(0, 2));
        assert_eq!(found.end, GridPoint::new(0, 5));
    }

    #[test]
    fn reveal_delta_scrolls_to_line() {
        let mut term = make_term(10, 4);
        for index in 0..20 {
            feed(&mut term, format!("line {index}\r\n").as_bytes());
        }
        // 最近的行仍在视口内，无需滚动。
        assert_eq!(reveal_delta(&term, 0), 0);
        // 历史中的行需要上滚。
        assert_eq!(reveal_delta(&term, -3), 3);
        term.scroll_display(Scroll::Delta(3));
        assert_eq!(term.grid().display_offset(), 3);
        assert_eq!(reveal_delta(&term, -3), 0);
    }

    #[test]
    fn terminal_modes_map_mouse_reporting() {
        let mut term = make_term(20, 4);
        feed(&mut term, b"\x1b[?1000h\x1b[?1006h");
        let snapshot = term_snapshot(&term);
        assert!(snapshot.modes.contains(TerminalModes::MOUSE_REPORT_CLICK));
        assert!(snapshot.modes.contains(TerminalModes::SGR_MOUSE));
        assert!(snapshot.modes.mouse_reporting());

        feed(&mut term, b"\x1b[?1000l\x1b[?1006l");
        let snapshot = term_snapshot(&term);
        assert!(!snapshot.modes.mouse_reporting());
    }
}
