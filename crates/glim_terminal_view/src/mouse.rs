//! 鼠标坐标到网格坐标的映射。
//!
//! 元素在 prepaint 阶段上报几何信息（原点、单元尺寸），视图据此把
//! 窗口坐标换算为网格坐标与单元内的左右半区。

use gpui_kit::{Pixels, Point};

use glim_terminal::{GridPoint, SelectionSide};

use crate::element::ElementGeometry;

/// 一次指针位置对应的网格坐标与所在的半区。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GridPointer {
    /// 网格坐标。
    pub point: GridPoint,
    /// 单元内位置：左半或右半。
    pub side: SelectionSide,
}

/// 把窗口坐标换算为网格坐标。
///
/// 视口之外的坐标会被钳制到最近的单元；视口上方返回负行号。
pub(crate) fn grid_pointer(
    geometry: &ElementGeometry,
    columns: usize,
    rows: usize,
    display_offset: usize,
    position: Point<Pixels>,
) -> GridPointer {
    let x = f32::from(position.x - geometry.origin.x);
    let y = f32::from(position.y - geometry.origin.y);
    let cell_width = f32::from(geometry.cell_width).max(1.0);
    let line_height = f32::from(geometry.line_height).max(1.0);

    let mut side = if x.rem_euclid(cell_width) > cell_width / 2.0 {
        SelectionSide::Right
    } else {
        SelectionSide::Left
    };

    let mut column = (x / cell_width).floor().max(0.0) as usize;
    let last_column = columns.saturating_sub(1);
    if column > last_column {
        column = last_column;
        side = SelectionSide::Right;
    }

    let mut row = (y / line_height).floor().max(0.0) as i32;
    let last_row = rows.saturating_sub(1) as i32;
    if row > last_row {
        row = last_row;
    }
    if y < 0.0 {
        side = SelectionSide::Left;
    }

    GridPointer {
        point: GridPoint::new(row - display_offset as i32, column),
        side,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glim_terminal::GridPoint;
    use gpui_kit::{point, px};

    fn geometry() -> ElementGeometry {
        ElementGeometry {
            origin: point(px(10.), px(20.)),
            cell_width: px(8.),
            line_height: px(16.),
        }
    }

    #[test]
    fn maps_inside_cells() {
        // 第 3 列、第 2 行的左半区。
        let pointer = grid_pointer(&geometry(), 80, 24, 0, point(px(34.), px(52.)));
        assert_eq!(pointer.point, GridPoint::new(2, 3));
        assert_eq!(pointer.side, SelectionSide::Left);

        // 右半区。
        let pointer = grid_pointer(&geometry(), 80, 24, 0, point(px(39.), px(52.)));
        assert_eq!(pointer.point, GridPoint::new(2, 3));
        assert_eq!(pointer.side, SelectionSide::Right);
    }

    #[test]
    fn scroll_offset_moves_grid_lines() {
        let pointer = grid_pointer(&geometry(), 80, 24, 5, point(px(10.), px(20.)));
        assert_eq!(pointer.point, GridPoint::new(-5, 0));
    }

    #[test]
    fn clamps_outside_viewport() {
        let pointer = grid_pointer(&geometry(), 80, 24, 0, point(px(-100.), px(-100.)));
        assert_eq!(pointer.point, GridPoint::new(0, 0));
        assert_eq!(pointer.side, SelectionSide::Left);

        let pointer = grid_pointer(&geometry(), 80, 24, 0, point(px(10_000.), px(10_000.)));
        assert_eq!(pointer.point, GridPoint::new(23, 79));
        assert_eq!(pointer.side, SelectionSide::Right);
    }
}
