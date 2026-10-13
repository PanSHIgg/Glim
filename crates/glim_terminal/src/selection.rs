//! 选择状态：视图侧的选择意图与网格坐标表示。
//!
//! 选择本身存储在终端内核中（Alacritty 的 `Selection`），因此滚动、
//! 调整尺寸与复制都由同一份数据推导。本模块只负责把视图的表达
//! 转换为内核类型，并保持与 UI 框架无关。

use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};

use crate::snapshot::GridPoint;

/// 选择类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    /// 普通选择：精确覆盖起点与终点之间的单元。
    Simple,
    /// 块选择：矩形区域。
    Block,
    /// 语义选择：按词边界扩展（双击）。
    Semantic,
    /// 整行选择（三击）。
    Lines,
}

/// 位于某个单元的左半或右半。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionSide {
    /// 单元左半。
    Left,
    /// 单元右半。
    Right,
}

/// 一次选择的意图：锚点（按下位置）与当前端点。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSelection {
    /// 锚点：按下时的网格坐标。
    pub anchor: GridPoint,
    /// 锚点位于单元哪一侧。
    pub anchor_side: SelectionSide,
    /// 当前端点：拖动时的最新坐标。
    pub cursor: GridPoint,
    /// 端点位于单元哪一侧。
    pub cursor_side: SelectionSide,
    /// 选择类型。
    pub kind: SelectionKind,
}

impl GridSelection {
    /// 从锚点开始的一次选择（端点在拖动时更新）。
    pub fn new(kind: SelectionKind, anchor: GridPoint, anchor_side: SelectionSide) -> Self {
        Self {
            anchor,
            anchor_side,
            cursor: anchor,
            cursor_side: anchor_side,
            kind,
        }
    }

    /// 更新端点。
    pub fn with_cursor(mut self, cursor: GridPoint, cursor_side: SelectionSide) -> Self {
        self.cursor = cursor;
        self.cursor_side = cursor_side;
        self
    }
}

pub(crate) fn grid_point_to_point(point: GridPoint) -> Point {
    Point::new(Line(point.line), Column(point.column))
}

pub(crate) fn point_to_grid_point(point: Point) -> GridPoint {
    GridPoint::new(point.line.0, point.column.0)
}

pub(crate) fn selection_to_terminal(selection: &GridSelection) -> Selection {
    let kind = match selection.kind {
        SelectionKind::Simple => SelectionType::Simple,
        SelectionKind::Block => SelectionType::Block,
        SelectionKind::Semantic => SelectionType::Semantic,
        SelectionKind::Lines => SelectionType::Lines,
    };
    let anchor_side = match selection.anchor_side {
        SelectionSide::Left => Side::Left,
        SelectionSide::Right => Side::Right,
    };
    let cursor_side = match selection.cursor_side {
        SelectionSide::Left => Side::Left,
        SelectionSide::Right => Side::Right,
    };
    let mut converted = Selection::new(kind, grid_point_to_point(selection.anchor), anchor_side);
    converted.update(grid_point_to_point(selection.cursor), cursor_side);
    converted
}
