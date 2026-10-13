//! 终端网格的自定义元素：把快照绘制为背景、文本与光标。
//!
//! 测量与尺寸上报也在这里：`prepaint` 得到最终区域后用单元格尺寸
//! 换算出列数与行数，并把结果交给视图（视图负责调整会话）。

use std::sync::Arc;

use gpui_kit::*;

use glim_terminal::{CursorShape, CursorSnapshot, GridLine, GridSnapshot, TermColor};
use glim_ui::TerminalPalette;

use crate::TerminalView;

/// 行高相对字号的倍数。
const LINE_HEIGHT_SCALE: f32 = 1.35;
/// 竖线光标宽度（物理界限，不随缩放变化）。
const BEAM_CURSOR_WIDTH: f32 = 2.;
/// 下划线光标高度（物理界限）。
const UNDERLINE_CURSOR_HEIGHT: f32 = 2.;
/// 文本装饰线粗细（物理界限）。
const DECORATION_THICKNESS: f32 = 1.;

/// 终端网格元素；每次渲染按当前快照重建，不保留状态。
pub(crate) struct TerminalGridElement {
    pub view: WeakEntity<TerminalView>,
    pub snapshot: Arc<GridSnapshot>,
    pub palette: TerminalPalette,
    pub font_family: SharedString,
    pub font_size: Pixels,
    pub focused: bool,
}

/// 测量结果：单元格几何。
pub(crate) struct GridGeometry {
    cell_width: Pixels,
    line_height: Pixels,
}

impl Element for TerminalGridElement {
    type RequestLayoutState = ();
    type PrepaintState = GridGeometry;

    fn id(&self) -> Option<ElementId> {
        Some("terminal-grid".into())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: size(
                Length::Definite(relative(1.0)),
                Length::Definite(relative(1.0)),
            ),
            ..Style::default()
        };
        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let font = mono_font(&self.font_family, FontWeight::NORMAL, FontStyle::Normal);
        let font_id = window.text_system().resolve_font(&font);
        let cell_width = window
            .text_system()
            .advance(font_id, self.font_size, 'm')
            .map(|advance| advance.width)
            .unwrap_or(self.font_size * 0.6);
        let line_height = self.font_size * LINE_HEIGHT_SCALE;

        let columns = (f32::from(bounds.size.width) / f32::from(cell_width))
            .floor()
            .max(1.0) as u16;
        let rows = (f32::from(bounds.size.height) / f32::from(line_height))
            .floor()
            .max(1.0) as u16;

        let _ = self.view.update(cx, |view, cx| {
            view.apply_grid_size(
                columns,
                rows,
                f32::from(cell_width) as u16,
                f32::from(line_height) as u16,
                cx,
            );
        });

        GridGeometry {
            cell_width,
            line_height,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        geometry: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let origin = bounds.origin;
        window.paint_quad(fill(bounds, self.palette.background));

        for (row, line) in self.snapshot.lines.iter().enumerate() {
            let painter = LinePainter {
                palette: &self.palette,
                origin,
                cell_width: geometry.cell_width,
                line_height: geometry.line_height,
                row_y: origin.y + geometry.line_height * row as f32,
                cursor: self.snapshot.cursor.filter(|cursor| cursor.row == row),
                focused: self.focused,
            };
            painter.paint_background(line, window);
            painter.paint_text(line, &self.font_family, self.font_size, window, cx);
            painter.paint_cursor(window);
        }
    }
}

impl IntoElement for TerminalGridElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// 一行的绘制上下文。
struct LinePainter<'a> {
    palette: &'a TerminalPalette,
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    row_y: Pixels,
    cursor: Option<CursorSnapshot>,
    focused: bool,
}

impl LinePainter<'_> {
    fn cell_bounds(&self, column: usize) -> Bounds<Pixels> {
        Bounds {
            origin: point(self.origin.x + self.cell_width * column as f32, self.row_y),
            size: size(self.cell_width, self.line_height),
        }
    }

    fn is_block_cursor_at(&self, column: usize) -> bool {
        self.focused
            && self
                .cursor
                .is_some_and(|cursor| cursor.shape == CursorShape::Block && cursor.column == column)
    }

    /// 单元格的有效前 / 背景色（已处理反转、变暗与方块光标）。
    fn cell_colors(&self, line: &GridLine, column: usize) -> (Hsla, Option<Hsla>) {
        let cell = &line.cells[column];
        let mut fg = resolve_color(cell.fg, self.palette);
        let mut bg = resolve_color(cell.bg, self.palette);
        if cell.attributes.inverse {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.attributes.dim {
            fg.a *= 0.65;
        }
        if self.is_block_cursor_at(column) {
            fg = self.palette.background;
            bg = self.palette.cursor;
        }
        let background = (bg != self.palette.background).then_some(bg);
        (fg, background)
    }

    /// 绘制非默认背景：合并相邻同色单元，每段画一个矩形。
    fn paint_background(&self, line: &GridLine, window: &mut Window) {
        let mut run: Option<(usize, Hsla)> = None;

        for (column, _) in line.cells.iter().enumerate() {
            let background = self.cell_colors(line, column).1;
            match (run, background) {
                (Some((_, existing)), Some(next)) if existing == next => {}
                (Some((start, color)), next) => {
                    paint_background_run(window, self, start, column, color);
                    run = next.map(|color| (column, color));
                }
                (None, Some(next)) => run = Some((column, next)),
                (None, None) => {}
            }
        }

        if let Some((start, color)) = run {
            paint_background_run(window, self, start, line.cells.len(), color);
        }
    }

    /// 绘制一行文本与装饰线。
    fn paint_text(
        &self,
        line: &GridLine,
        font_family: &SharedString,
        font_size: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        #[derive(Clone, Copy, PartialEq)]
        struct RunStyle {
            fg: Hsla,
            bg: Option<Hsla>,
            bold: bool,
            italic: bool,
            underline: bool,
            strikeout: bool,
        }

        let mut text = String::new();
        let mut runs: Vec<TextRun> = Vec::new();
        let mut current: Option<RunStyle> = None;

        for (column, cell) in line.cells.iter().enumerate() {
            if cell.attributes.wide_char_spacer {
                continue;
            }

            let (fg, bg) = self.cell_colors(line, column);
            let style = RunStyle {
                fg,
                bg,
                bold: cell.attributes.bold,
                italic: cell.attributes.italic,
                underline: cell.attributes.underline,
                strikeout: cell.attributes.strikeout,
            };

            let before = text.len();
            text.push(cell.character);
            let len = text.len() - before;

            if current == Some(style) {
                if let Some(run) = runs.last_mut() {
                    run.len += len;
                }
            } else {
                let font = mono_font(
                    font_family,
                    if style.bold {
                        FontWeight::BOLD
                    } else {
                        FontWeight::NORMAL
                    },
                    if style.italic {
                        FontStyle::Italic
                    } else {
                        FontStyle::Normal
                    },
                );
                runs.push(TextRun {
                    len,
                    font,
                    color: style.fg,
                    background_color: style.bg,
                    underline: style.underline.then(|| UnderlineStyle {
                        thickness: px(DECORATION_THICKNESS),
                        color: Some(style.fg),
                        wavy: false,
                    }),
                    strikethrough: style.strikeout.then(|| StrikethroughStyle {
                        thickness: px(DECORATION_THICKNESS),
                        color: Some(style.fg),
                    }),
                });
                current = Some(style);
            }
        }

        if text.is_empty() {
            return;
        }

        let shaped = window
            .text_system()
            .shape_line(text.into(), font_size, &runs, None);
        let _ = shaped.paint(
            point(self.origin.x, self.row_y),
            self.line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }

    /// 绘制光标中不参与文本颜色的部分（竖线、下划线、空心方块）。
    fn paint_cursor(&self, window: &mut Window) {
        let Some(cursor) = self.cursor else {
            return;
        };

        match cursor.shape {
            CursorShape::Block if self.focused => {
                // 方块光标通过文字颜色反转呈现。
            }
            CursorShape::Block | CursorShape::HollowBlock => {
                let cell = self.cell_bounds(cursor.column);
                window.paint_quad(outline(cell, self.palette.cursor, BorderStyle::default()));
            }
            CursorShape::Beam => {
                let cell = self.cell_bounds(cursor.column);
                let beam = Bounds {
                    origin: cell.origin,
                    size: size(px(BEAM_CURSOR_WIDTH), cell.size.height),
                };
                window.paint_quad(fill(beam, self.palette.cursor));
            }
            CursorShape::Underline => {
                let cell = self.cell_bounds(cursor.column);
                let underline = Bounds {
                    origin: point(
                        cell.origin.x,
                        cell.origin.y + cell.size.height - px(UNDERLINE_CURSOR_HEIGHT),
                    ),
                    size: size(cell.size.width, px(UNDERLINE_CURSOR_HEIGHT)),
                };
                window.paint_quad(fill(underline, self.palette.cursor));
            }
        }
    }
}

fn paint_background_run(
    window: &mut Window,
    painter: &LinePainter<'_>,
    start: usize,
    end: usize,
    color: Hsla,
) {
    if end <= start {
        return;
    }
    let bounds = Bounds {
        origin: point(
            painter.origin.x + painter.cell_width * start as f32,
            painter.row_y,
        ),
        size: size(
            painter.cell_width * (end - start) as f32,
            painter.line_height,
        ),
    };
    window.paint_quad(fill(bounds, color));
}

fn mono_font(family: &SharedString, weight: FontWeight, style: FontStyle) -> Font {
    Font {
        family: family.clone(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight,
        style,
    }
}

fn resolve_color(color: TermColor, palette: &TerminalPalette) -> Hsla {
    match color {
        TermColor::Foreground => palette.foreground,
        TermColor::Background => palette.background,
        TermColor::Cursor => palette.cursor,
        TermColor::Indexed(index) => palette.ansi[(index as usize).min(palette.ansi.len() - 1)],
        TermColor::Rgb(r, g, b) => rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into(),
    }
}
