//! 终端网格快照：一次读取得到的可视区域只读视图。
//!
//! 快照描述“这一帧要画什么”：单元格内容与样式、选择状态、光标、
//! 当前显示偏移与终端模式。坐标使用网格行号（可为负，表示滚动历史）
//! 与列号（0 起）。

use crate::TerminalModes;

/// 网格坐标：行可为负（视口上方的历史），列 0 起。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GridPoint {
    /// 行；`0` 是屏幕第一行，负值表示滚动历史。
    pub line: i32,
    /// 列；`0` 起。
    pub column: usize,
}

impl GridPoint {
    /// 创建网格坐标。
    pub fn new(line: i32, column: usize) -> Self {
        Self { line, column }
    }
}

/// 选择范围（网格坐标；已按选择类型展开）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionSpan {
    /// 起点（左上）。
    pub start: GridPoint,
    /// 终点（右下）。
    pub end: GridPoint,
    /// 是否为块选择（矩形区域）。
    pub is_block: bool,
}

/// 终端颜色的语义表示，与调色板无关。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermColor {
    /// 终端默认前景色。
    Foreground,
    /// 终端默认背景色。
    Background,
    /// 光标颜色。
    Cursor,
    /// ANSI 调色板索引（0–7 基础色，8–15 亮色）。
    Indexed(u8),
    /// 24 位真彩色。
    Rgb(u8, u8, u8),
}

/// 单元格样式标记。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellAttributes {
    /// 加粗。
    pub bold: bool,
    /// 变暗。
    pub dim: bool,
    /// 斜体。
    pub italic: bool,
    /// 下划线（含双线等变体）。
    pub underline: bool,
    /// 删除线。
    pub strikeout: bool,
    /// 前景 / 背景反转。
    pub inverse: bool,
    /// 宽字符的前半格（占两列）。
    pub wide: bool,
    /// 宽字符占位格；渲染时应跳过。
    pub wide_char_spacer: bool,
}

/// 一个网格单元。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridCell {
    /// 字符；隐藏内容与宽字符占位格为空格。
    pub character: char,
    /// 组合字符（零宽），绘制时跟随 `character`。
    pub zerowidth: Vec<char>,
    /// 前景色。
    pub fg: TermColor,
    /// 背景色。
    pub bg: TermColor,
    /// 样式标记。
    pub attributes: CellAttributes,
    /// 是否落在当前选择范围内。
    pub selected: bool,
}

impl GridCell {
    pub(crate) fn blank() -> Self {
        Self {
            character: ' ',
            zerowidth: Vec::new(),
            fg: TermColor::Foreground,
            bg: TermColor::Background,
            attributes: CellAttributes::default(),
            selected: false,
        }
    }
}

/// 一行网格单元；长度等于 [`GridSnapshot::columns`]。
#[derive(Clone, Debug)]
pub struct GridLine {
    /// 行内单元。
    pub cells: Vec<GridCell>,
}

/// 光标形状。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    /// 方块。
    Block,
    /// 下划线。
    Underline,
    /// 竖线。
    Beam,
    /// 空心方块（窗口失焦时使用）。
    HollowBlock,
}

/// 光标位置与形状。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorSnapshot {
    /// 列。
    pub column: usize,
    /// 行（视口坐标，0 为第一可见行）。
    pub row: usize,
    /// 形状。
    pub shape: CursorShape,
}

/// 可视区域的网格快照；由 [`crate::SessionHandle::snapshot`] 生成。
#[derive(Clone, Debug)]
pub struct GridSnapshot {
    /// 列数。
    pub columns: usize,
    /// 行数。
    pub rows: usize,
    /// 光标；`None` 表示隐藏或不在视口内。
    pub cursor: Option<CursorSnapshot>,
    /// 行；长度等于 `rows`，每行长度等于 `columns`。
    pub lines: Vec<GridLine>,
    /// 当前终端模式。
    pub modes: TerminalModes,
    /// 显示偏移：视口顶部上方的历史行数。
    pub display_offset: usize,
    /// 当前选择范围；`None` 表示没有选择。
    pub selection: Option<SelectionSpan>,
}
