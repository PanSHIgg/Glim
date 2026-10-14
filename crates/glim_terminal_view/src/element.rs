//! 终端网格的自定义元素：绘制背景、文本、选择、搜索高亮与光标，
//! 并注册输入法输入处理器与鼠标监听。
//!
//! 测量与几何上报也在这里：`prepaint` 得到最终区域后用单元格尺寸
//! 换算出列数与行数，并把几何信息交给视图（视图负责调整会话、
//! 换算鼠标坐标与定位输入法候选窗口）。

use std::sync::Arc;

use gpui_kit::*;

use glim_terminal::{
    CursorShape, CursorSnapshot, GridCell, GridLine, GridSnapshot, SelectionSpan, TermColor,
};
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

/// 终端字体的回退链：主字族缺少字形时按序尝试。
///
/// 覆盖等宽备用字体与常见中文字族；未安装的字族会被文本系统跳过。
fn fallback_families() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &[
            "Consolas",
            "Microsoft YaHei",
            "Microsoft YaHei UI",
            "SimSun",
        ]
    }
    #[cfg(target_os = "macos")]
    {
        &["Menlo", "PingFang SC", "Hiragino Sans GB"]
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        &[
            "DejaVu Sans Mono",
            "Noto Sans Mono",
            "Noto Sans Mono CJK SC",
            "Noto Sans CJK SC",
        ]
    }
}

/// 回退链只构建一次并共享。
fn terminal_fallbacks() -> FontFallbacks {
    static FALLBACKS: std::sync::OnceLock<FontFallbacks> = std::sync::OnceLock::new();
    FALLBACKS
        .get_or_init(|| {
            FontFallbacks::from_fonts(
                fallback_families()
                    .iter()
                    .map(|family| (*family).to_owned())
                    .collect(),
            )
        })
        .clone()
}

/// 视口几何：元素原点与单元尺寸。
#[derive(Clone, Copy, Debug)]
pub(crate) struct ElementGeometry {
    /// 元素左上角（窗口坐标）。
    pub origin: Point<Pixels>,
    /// 单元宽度。
    pub cell_width: Pixels,
    /// 行高。
    pub line_height: Pixels,
}

/// 视口坐标中的一段高亮范围（两端包含）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SelectionHighlight {
    /// 起始行（视口行，0 起）。
    pub start_line: i32,
    /// 起始列。
    pub start_column: usize,
    /// 结束行。
    pub end_line: i32,
    /// 结束列。
    pub end_column: usize,
}

impl SelectionHighlight {
    /// 由网格坐标的范围与当前显示偏移换算。
    pub fn from_span(span: SelectionSpan, display_offset: usize) -> Self {
        Self {
            start_line: span.start.line + display_offset as i32,
            start_column: span.start.column,
            end_line: span.end.line + display_offset as i32,
            end_column: span.end.column,
        }
    }
}

/// 终端网格元素；每次渲染按当前快照重建，不保留状态。
///
/// 会话尚未启动时快照为 `None`：元素仍参与测量并上报几何信息，
/// 视图据此按实际网格尺寸启动会话。
pub(crate) struct TerminalGridElement {
    pub view: Entity<TerminalView>,
    pub snapshot: Option<Arc<GridSnapshot>>,
    pub palette: TerminalPalette,
    pub font_family: SharedString,
    pub font_size: Pixels,
    pub focused: bool,
    /// 输入法预编辑文本；绘制在光标处。
    pub preedit: Option<SharedString>,
    /// 当前查找匹配（视口坐标）；高亮显示。
    pub search: Option<SelectionHighlight>,
}

/// 测量结果：单元格几何与命中区域。
pub(crate) struct GridGeometry {
    geometry: ElementGeometry,
    hitbox: Hitbox,
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
        let font = terminal_font(&self.font_family, FontWeight::NORMAL, FontStyle::Normal);
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

        let geometry = ElementGeometry {
            origin: bounds.origin,
            cell_width,
            line_height,
        };

        self.view.update(cx, |view, cx| {
            view.apply_geometry(columns, rows, geometry, cx);
        });

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        GridGeometry { geometry, hitbox }
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

        let search = self.search;

        // 会话尚未启动时没有快照：只画背景，测量与输入注册照常进行。
        if let Some(snapshot) = &self.snapshot {
            for (row, line) in snapshot.lines.iter().enumerate() {
                let painter = LinePainter {
                    palette: &self.palette,
                    origin,
                    cell_width: geometry.geometry.cell_width,
                    line_height: geometry.geometry.line_height,
                    row_y: origin.y + geometry.geometry.line_height * row as f32,
                    row: row as i32,
                    cursor: snapshot.cursor.filter(|cursor| cursor.row == row),
                    focused: self.focused,
                    search,
                    font_family: &self.font_family,
                    font_size: self.font_size,
                };
                painter.paint_background(line, window);
                painter.paint_search(line, window);
                painter.paint_text(line, window, cx);
                painter.paint_cursor(window);
            }

            if let Some(preedit) = &self.preedit {
                paint_preedit(
                    preedit,
                    &self.palette,
                    origin,
                    &geometry.geometry,
                    snapshot,
                    &self.font_family,
                    self.font_size,
                    window,
                    cx,
                );
            }
        }

        let focus_handle = self.view.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );

        register_mouse_listeners(self.view.clone(), geometry.hitbox.clone(), window);
        window.set_cursor_style(CursorStyle::IBeam, &geometry.hitbox);
    }
}

/// 注册窗口级鼠标监听（按下、移动、抬起、滚轮）。
///
/// 左键按下时捕获指针，拖动选择在指针移出元素后仍然有效。
fn register_mouse_listeners(view: Entity<TerminalView>, hitbox: Hitbox, window: &mut Window) {
    let down_hitbox = hitbox.clone();
    let down_view = view.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !down_hitbox.is_hovered(window) {
            return;
        }
        if event.button == MouseButton::Left {
            window.capture_pointer(down_hitbox.id);
        }
        let focus_handle = down_view.read(cx).focus_handle.clone();
        window.focus(&focus_handle, cx);
        down_view.update(cx, |view, cx| view.pointer_down(event, cx));
    });

    let move_hitbox = hitbox.clone();
    let move_view = view.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !move_hitbox.is_hovered(window) {
            return;
        }
        move_view.update(cx, |view, cx| view.pointer_move(event, cx));
    });

    let up_hitbox = hitbox.clone();
    let up_view = view.clone();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !up_hitbox.is_hovered(window) {
            return;
        }
        up_view.update(cx, |view, cx| view.pointer_up(event, cx));
    });

    let wheel_view = view;
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
            return;
        }
        wheel_view.update(cx, |view, cx| view.wheel(event, cx));
    });
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
    row: i32,
    cursor: Option<CursorSnapshot>,
    focused: bool,
    search: Option<SelectionHighlight>,
    font_family: &'a SharedString,
    font_size: Pixels,
}

/// 一个单元的有效绘制样式。
#[derive(Clone, Copy, PartialEq)]
struct CellStyle {
    fg: Hsla,
    background: Option<Hsla>,
    bold: bool,
    italic: bool,
    underline: bool,
    strikeout: bool,
}

impl LinePainter<'_> {
    fn cell_bounds(&self, column: usize, width_cells: usize) -> Bounds<Pixels> {
        Bounds {
            origin: point(self.origin.x + self.cell_width * column as f32, self.row_y),
            size: size(self.cell_width * width_cells as f32, self.line_height),
        }
    }

    /// 方块光标是否覆盖该单元（宽字符时包含其占位格）。
    fn is_block_cursor_at(&self, line: &GridLine, column: usize) -> bool {
        let Some(cursor) = self.cursor else {
            return false;
        };
        if !self.focused || cursor.shape != CursorShape::Block {
            return false;
        }
        if cursor.column == column {
            return true;
        }
        cursor.column + 1 == column
            && line
                .cells
                .get(column)
                .is_some_and(|cell| cell.attributes.wide_char_spacer)
    }

    /// 单元的有效前景 / 背景色（已处理反转、变暗与方块光标）。
    fn cell_style(&self, line: &GridLine, column: usize) -> CellStyle {
        let cell = &line.cells[column];
        let mut fg = resolve_color(cell.fg, self.palette);
        let mut bg = resolve_color(cell.bg, self.palette);
        if cell.attributes.inverse {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.attributes.dim {
            fg.a *= 0.65;
        }

        if cell.selected {
            // 选择背景始终可见；方块光标不再反转选中单元。
            bg = self.palette.selection;
        } else if self.is_block_cursor_at(line, column) {
            fg = self.palette.background;
            bg = self.palette.cursor;
        }

        let background = (bg != self.palette.background).then_some(bg);
        CellStyle {
            fg,
            background,
            bold: cell.attributes.bold,
            italic: cell.attributes.italic,
            underline: cell.attributes.underline,
            strikeout: cell.attributes.strikeout,
        }
    }

    /// 绘制行内非默认背景：合并相邻同色单元，每段画一个矩形。
    fn paint_background(&self, line: &GridLine, window: &mut Window) {
        let mut run: Option<(usize, Hsla)> = None;

        for (column, cell) in line.cells.iter().enumerate() {
            let mut background = self.cell_style(line, column).background;
            // 宽字符前半格被选中时，其占位格跟随同一底色，避免选择出现缺口。
            if cell.attributes.wide_char_spacer
                && column > 0
                && line.cells[column - 1].attributes.wide
            {
                background = self.cell_style(line, column - 1).background;
            }
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

    /// 绘制查找匹配的高亮底色。
    fn paint_search(&self, line: &GridLine, window: &mut Window) {
        let Some(search) = self.search else {
            return;
        };
        if self.row < search.start_line || self.row > search.end_line {
            return;
        }
        let last_column = line.cells.len().saturating_sub(1);
        let start_column = if self.row == search.start_line {
            search.start_column
        } else {
            0
        };
        let end_column = if self.row == search.end_line {
            search.end_column.min(last_column)
        } else {
            last_column
        };
        if end_column < start_column {
            return;
        }
        let bounds = Bounds {
            origin: point(
                self.origin.x + self.cell_width * start_column as f32,
                self.row_y,
            ),
            size: size(
                self.cell_width * (end_column - start_column + 1) as f32,
                self.line_height,
            ),
        };
        window.paint_quad(fill(bounds, self.palette.search_highlight));
    }

    /// 绘制一行文本与装饰线。
    ///
    /// 连续的单宽字符按样式成批定位（`force_width` 锁定单元网格）；
    /// 宽字符与组合字符单独成形并按单元位置放置，保证列对齐。
    fn paint_text(&self, line: &GridLine, window: &mut Window, cx: &mut App) {
        let mut batch: Option<(usize, String, CellStyle)> = None;
        let mut next_column = 0;

        for (column, cell) in line.cells.iter().enumerate() {
            if cell.attributes.wide_char_spacer {
                continue;
            }
            let style = self.cell_style(line, column);
            let single_width = !cell.attributes.wide && cell.zerowidth.is_empty();

            if single_width {
                let extend = batch
                    .as_ref()
                    .is_some_and(|(_, _, existing)| *existing == style && next_column == column);
                if extend {
                    if let Some((_, text, _)) = batch.as_mut() {
                        text.push(cell.character);
                    }
                } else {
                    self.paint_batch(batch.take(), window, cx);
                    let mut text = String::with_capacity(2);
                    text.push(cell.character);
                    batch = Some((column, text, style));
                }
                next_column = column + 1;
            } else {
                self.paint_batch(batch.take(), window, cx);
                self.paint_cluster(cell, column, style, window, cx);
                next_column = column + if cell.attributes.wide { 2 } else { 1 };
            }
        }

        self.paint_batch(batch.take(), window, cx);
    }

    /// 绘制一批连续的单宽字符；`force_width` 保证字符落在单元网格上。
    fn paint_batch(
        &self,
        batch: Option<(usize, String, CellStyle)>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some((start_column, text, style)) = batch else {
            return;
        };
        let run = text_run(self.font_family, &style, text.len());
        let shaped = window.text_system().shape_line(
            text.into(),
            self.font_size,
            &[run],
            Some(self.cell_width),
        );
        let _ = shaped.paint(
            point(
                self.origin.x + self.cell_width * start_column as f32,
                self.row_y,
            ),
            self.line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }

    /// 绘制一个宽字符或带组合字符的簇，位置由单元列决定。
    fn paint_cluster(
        &self,
        cell: &GridCell,
        column: usize,
        style: CellStyle,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut text = String::new();
        text.push(cell.character);
        for character in &cell.zerowidth {
            text.push(*character);
        }

        let run = text_run(self.font_family, &style, text.len());
        let shaped = window
            .text_system()
            .shape_line(text.into(), self.font_size, &[run], None);

        let column_x = self.origin.x + self.cell_width * column as f32;
        let x = if cell.attributes.wide {
            // 宽字符居中于两格；字形自然宽度通常接近两格。
            column_x + (self.cell_width * 2. - shaped.width).max(px(0.)) / 2.
        } else {
            column_x
        };

        let _ = shaped.paint(
            point(x, self.row_y),
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
                let cell = self.cell_bounds(cursor.column, 1);
                window.paint_quad(outline(cell, self.palette.cursor, BorderStyle::default()));
            }
            CursorShape::Beam => {
                let cell = self.cell_bounds(cursor.column, 1);
                let beam = Bounds {
                    origin: cell.origin,
                    size: size(px(BEAM_CURSOR_WIDTH), cell.size.height),
                };
                window.paint_quad(fill(beam, self.palette.cursor));
            }
            CursorShape::Underline => {
                let cell = self.cell_bounds(cursor.column, 1);
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

/// 由有效样式与字节长度构造文本运行。
fn text_run(font_family: &SharedString, style: &CellStyle, len: usize) -> TextRun {
    TextRun {
        len,
        font: terminal_font(
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
        ),
        color: style.fg,
        background_color: style.background,
        underline: style.underline.then(|| UnderlineStyle {
            thickness: px(DECORATION_THICKNESS),
            color: Some(style.fg),
            wavy: false,
        }),
        strikethrough: style.strikeout.then(|| StrikethroughStyle {
            thickness: px(DECORATION_THICKNESS),
            color: Some(style.fg),
        }),
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

/// 绘制输入法预编辑文本：覆盖原位文本并带下划线，候选窗口跟随此处。
#[allow(clippy::too_many_arguments)]
fn paint_preedit(
    preedit: &SharedString,
    palette: &TerminalPalette,
    origin: Point<Pixels>,
    geometry: &ElementGeometry,
    snapshot: &GridSnapshot,
    font_family: &SharedString,
    font_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(cursor) = snapshot.cursor else {
        return;
    };
    let position = point(
        origin.x + geometry.cell_width * cursor.column as f32,
        origin.y + geometry.line_height * cursor.row as f32,
    );

    let font = terminal_font(font_family, FontWeight::NORMAL, FontStyle::Normal);
    let runs = [TextRun {
        len: preedit.len(),
        font,
        color: palette.foreground,
        underline: Some(UnderlineStyle {
            thickness: px(DECORATION_THICKNESS),
            color: Some(palette.foreground),
            wavy: false,
        }),
        ..Default::default()
    }];
    let shaped = window
        .text_system()
        .shape_line(preedit.clone(), font_size, &runs, None);

    let background = Bounds {
        origin: position,
        size: size(shaped.width.max(geometry.cell_width), geometry.line_height),
    };
    window.paint_quad(fill(background, palette.background));

    let _ = shaped.paint(
        position,
        geometry.line_height,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn terminal_font(family: &SharedString, weight: FontWeight, style: FontStyle) -> Font {
    Font {
        family: family.clone(),
        features: FontFeatures::disable_ligatures(),
        fallbacks: Some(terminal_fallbacks()),
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
