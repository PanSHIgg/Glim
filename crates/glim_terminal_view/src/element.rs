//! 终端网格的自定义元素：绘制背景、文本、选择、搜索高亮与光标，
//! 并注册输入法输入处理器与鼠标监听。
//!
//! 测量与几何上报也在这里：`prepaint` 得到最终区域后用单元格尺寸
//! 换算出列数与行数，并把几何信息交给视图（视图负责调整会话、
//! 换算鼠标坐标与定位输入法候选窗口）。
//!
//! 绘制几何按设备像素对齐：行高、行数与网格原点都取整到物理像素，
//! 背景、选择、光标与块元素矩形也吸到设备像素网格上；备用屏幕（TUI）
//! 把剩余高度放在网格上方，让最后一行贴住底边。
//!
//! 块元素（`▀`–`▟`、八分之一块、六分之一块）不用字体字形，而是按
//! 单元格子网格（8 列 × 24 行）画矩形，`░▒▓` 用整格不透明度代替
//! 网点图案，像素画、阴影与进度条因此没有字形缝隙。
//!
//! ANSI 0–15 颜色在对单元格背景上做 APCA 最小对比度调整；TUI 明确
//! 指定的 256 色 / 真彩色与装饰字符保持原样。

use std::sync::Arc;

use gpui_kit::*;

use glim_terminal::{
    CursorShape, CursorSnapshot, GridCell, GridLine, GridSnapshot, SelectionSpan, TermColor,
    TerminalModes,
};
use glim_ui::{TERMINAL_MINIMUM_CONTRAST, TerminalPalette, ensure_minimum_contrast, indexed_color};

use crate::TerminalView;

/// 行高相对字号的倍数。
const LINE_HEIGHT_SCALE: f32 = 1.5;
/// 块元素的子网格列数（八分之一块与象限块共用）。
const BLOCK_SUBCELL_COLUMNS: i32 = 8;
/// 块元素的子网格行数（1/8 与 1/3 切分的最小公倍数）。
const BLOCK_SUBCELL_LINES: i32 = 24;
/// 终端内容内边距（rem）：上。
const PADDING_TOP_REMS: f32 = 1.;
/// 终端内容内边距（rem）：左右。
const PADDING_SIDE_REMS: f32 = 1.;
/// 终端内容内边距（rem）：下。
const PADDING_BOTTOM_REMS: f32 = 1.5;
/// 滚动条滑块宽度（物理界限）。
const SCROLLBAR_WIDTH: f32 = 6.;
/// 滚动条滑块最小高度（物理界限）。
const SCROLLBAR_MIN_HEIGHT: f32 = 24.;
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

        // 行高取整到设备像素，行数与剩余高度也按设备像素计算，
        // 让每一行的基线落在相同的像素相位上（调整窗口时不会抖动）。
        let scale_factor = window.scale_factor().max(1.);
        let line_height_device = (f32::from(self.font_size) * LINE_HEIGHT_SCALE * scale_factor)
            .round()
            .max(1.);
        let line_height = px(line_height_device / scale_factor);

        // 内容内边距按设计稿：上 1rem、左右 1rem、下 1.5rem；网格按内缩后
        // 的区域测量，滚动条绘制在元素右缘（窗格边缘）。
        let rem = window.rem_size();
        let padding_left = rem * PADDING_SIDE_REMS;
        let padding_top = rem * PADDING_TOP_REMS;
        let inner_width = (bounds.size.width - padding_left * 2.).max(px(0.));
        let inner_height =
            (bounds.size.height - padding_top - rem * PADDING_BOTTOM_REMS).max(px(0.));

        let columns = (f32::from(inner_width) / f32::from(cell_width))
            .floor()
            .max(1.0) as u16;
        let available_device = (f32::from(inner_height) * scale_factor).floor().max(0.);
        let rows = (available_device / line_height_device).floor().max(1.) as u16;
        let leftover_device = (available_device - rows as f32 * line_height_device).max(0.);

        // 备用屏幕的 TUI 占满整屏：剩余高度放在网格上方，最后一行贴住
        // 底边；普通回显保持在顶部，多余空间留在下方。
        let anchored_to_bottom = self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.modes.contains(TerminalModes::ALT_SCREEN));
        let mut origin_y = snap_to_device_pixel(bounds.origin.y + padding_top, scale_factor);
        if anchored_to_bottom {
            origin_y += px(leftover_device / scale_factor);
        }

        let geometry = ElementGeometry {
            origin: point(
                snap_to_device_pixel(bounds.origin.x + padding_left, scale_factor),
                origin_y,
            ),
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
        let origin = geometry.geometry.origin;
        let scale_factor = window.scale_factor().max(1.);
        window.paint_quad(fill(bounds, self.palette.background));

        let search = self.search;

        // 会话尚未启动时没有快照：只画背景，测量与输入注册照常进行。
        if let Some(snapshot) = &self.snapshot {
            // 颜色组合按帧缓存，避免逐格重复做 APCA 调整。
            let mut contrast = ContrastCache::default();
            for (row, line) in snapshot.lines.iter().enumerate() {
                let painter = LinePainter {
                    palette: &self.palette,
                    origin,
                    cell_width: geometry.geometry.cell_width,
                    line_height: geometry.geometry.line_height,
                    scale_factor,
                    row_y: origin.y + geometry.geometry.line_height * row as f32,
                    row: row as i32,
                    cursor: snapshot.cursor.filter(|cursor| cursor.row == row),
                    focused: self.focused,
                    search,
                    font_family: &self.font_family,
                    font_size: self.font_size,
                };
                painter.paint_background(line, &mut contrast, window);
                painter.paint_search(line, window);
                painter.paint_text(line, &mut contrast, window, cx);
                painter.paint_block_elements(line, &mut contrast, window);
                painter.paint_cursor(window);
            }

            if let Some(preedit) = &self.preedit {
                paint_preedit(
                    preedit,
                    &self.palette,
                    origin,
                    &geometry.geometry,
                    scale_factor,
                    snapshot,
                    &self.font_family,
                    self.font_size,
                    window,
                    cx,
                );
            }

            paint_scrollbar(bounds, snapshot, &self.palette, scale_factor, window);
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
    scale_factor: f32,
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
    /// 单元矩形（对齐到设备像素）。
    fn cell_bounds(&self, column: usize, width_cells: usize) -> Bounds<Pixels> {
        snap_to_device_pixels(
            Bounds {
                origin: point(self.origin.x + self.cell_width * column as f32, self.row_y),
                size: size(self.cell_width * width_cells as f32, self.line_height),
            },
            self.scale_factor,
        )
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

    /// 单元的有效前景 / 背景色（已处理反转、最小对比度、变暗与方块光标）。
    fn cell_style(
        &self,
        line: &GridLine,
        column: usize,
        contrast: &mut ContrastCache,
    ) -> CellStyle {
        let cell = &line.cells[column];
        let (mut fg_key, mut bg_key) = (cell.fg, cell.bg);
        if cell.attributes.inverse {
            std::mem::swap(&mut fg_key, &mut bg_key);
        }
        let mut fg = resolve_color(fg_key, self.palette);
        let mut bg = resolve_color(bg_key, self.palette);

        // 光标单元由光标颜色表达；TUI 明确指定的颜色与装饰字符保持原样。
        let cursor_cell = self.is_block_cursor_at(line, column);
        if !cursor_cell
            && !is_app_chosen_exact_color(fg_key)
            && !is_decorative_character(cell.character)
        {
            fg = contrast.adjusted(fg_key, bg_key, fg, bg);
        }
        if cell.attributes.dim {
            fg.a *= 0.65;
        }

        if cell.selected {
            // 选择背景始终可见；方块光标不再反转选中单元。
            bg = self.palette.selection;
        } else if cursor_cell {
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
    fn paint_background(&self, line: &GridLine, contrast: &mut ContrastCache, window: &mut Window) {
        let mut run: Option<(usize, Hsla)> = None;

        for (column, cell) in line.cells.iter().enumerate() {
            let mut background = self.cell_style(line, column, contrast).background;
            // 宽字符前半格被选中时，其占位格跟随同一底色，避免选择出现缺口。
            if cell.attributes.wide_char_spacer
                && column > 0
                && line.cells[column - 1].attributes.wide
            {
                background = self.cell_style(line, column - 1, contrast).background;
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
        window.paint_quad(fill(
            snap_to_device_pixels(bounds, self.scale_factor),
            self.palette.search_highlight,
        ));
    }

    /// 绘制一行文本与装饰线。
    ///
    /// 连续的单宽字符按样式成批定位（`force_width` 锁定单元网格）；
    /// 宽字符与组合字符单独成形并按单元位置放置，保证列对齐。
    /// 块元素由 [`Self::paint_block_elements`] 按子格矩形绘制。
    fn paint_text(
        &self,
        line: &GridLine,
        contrast: &mut ContrastCache,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut batch: Option<(usize, String, CellStyle)> = None;
        let mut next_column = 0;

        for (column, cell) in line.cells.iter().enumerate() {
            if cell.attributes.wide_char_spacer {
                continue;
            }
            if is_block_element(cell.character) {
                self.paint_batch(batch.take(), window, cx);
                next_column = column + 1;
                continue;
            }
            let style = self.cell_style(line, column, contrast);
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

    /// 绘制行内的块元素：按单元格子网格的矩形替代字体字形。
    ///
    /// 相邻的同色矩形合并成一段，像素画、二维码与进度条因此没有接缝；
    /// `░▒▓` 用整格不透明度近似，不依赖字体的网点图案。
    fn paint_block_elements(
        &self,
        line: &GridLine,
        contrast: &mut ContrastCache,
        window: &mut Window,
    ) {
        let subcell_width = self.cell_width / BLOCK_SUBCELL_COLUMNS as f32;
        let subcell_height = self.line_height / BLOCK_SUBCELL_LINES as f32;
        let mut pending: Option<(Bounds<Pixels>, Hsla)> = None;

        for (column, cell) in line.cells.iter().enumerate() {
            let shape = if cell.attributes.wide_char_spacer {
                None
            } else {
                block_element(cell.character)
            };
            let Some(shape) = shape else {
                flush_block_element(&mut pending, self.scale_factor, window);
                continue;
            };

            let color = self.cell_style(line, column, contrast).fg;
            let mut rects = [(0, 0, 0, 0); MAX_BLOCK_RECTS];
            let (count, opacity) = shape.write_rects(&mut rects);
            let color = color.opacity(opacity);
            let cell_x = self.origin.x + self.cell_width * column as f32;

            for (sub_column, sub_line, columns, lines) in rects.into_iter().take(count) {
                let bounds = Bounds {
                    origin: point(
                        cell_x + subcell_width * sub_column as f32,
                        self.row_y + subcell_height * sub_line as f32,
                    ),
                    size: size(
                        subcell_width * columns as f32,
                        subcell_height * lines as f32,
                    ),
                };
                pending = extend_block_element(pending, bounds, color, self.scale_factor, window);
            }
        }

        flush_block_element(&mut pending, self.scale_factor, window);
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
///
/// 背景由 [`LinePainter::paint_background`] 按单元合并绘制，这里不再
/// 让文本运行重复携带背景（否则半透明底色会被叠加两次）。
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
        background_color: None,
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

/// 块元素的最大矩形数（六分之一块最多 5 个填充子格）。
const MAX_BLOCK_RECTS: usize = 6;

/// 按子格矩形绘制的块元素。
///
/// 子网格为每格 8 列 × 24 行（八分之一块与三分之一块的最小公倍数），
/// 与 Zed 的终端一致。
#[derive(Clone, Copy, Debug, PartialEq)]
enum BlockElement {
    /// 单个矩形：列、行、宽、高（子格单位）。
    Rect(i32, i32, i32, i32),
    /// 整格填充与不透明度（`░▒▓`）。
    Shade(f32),
    /// 2×2 象限块：位 `row * 2 + column` 为 1 表示填充。
    Quadrant(u8),
    /// 2×3 六分之一块：位 `row * 2 + column` 为 1 表示填充。
    Sextant(u8),
}

impl BlockElement {
    /// 写入子格矩形，返回矩形数量与填充不透明度。
    fn write_rects(self, rects: &mut [(i32, i32, i32, i32); MAX_BLOCK_RECTS]) -> (usize, f32) {
        match self {
            BlockElement::Rect(column, line, columns, lines) => {
                rects[0] = (column, line, columns, lines);
                (1, 1.)
            }
            BlockElement::Shade(opacity) => {
                rects[0] = (0, 0, BLOCK_SUBCELL_COLUMNS, BLOCK_SUBCELL_LINES);
                (1, opacity)
            }
            BlockElement::Quadrant(filled) => {
                let mut count = 0;
                for row in 0..2 {
                    for column in 0..2 {
                        if filled & (1 << (row * 2 + column)) != 0 {
                            rects[count] = (column * 4, row * 12, 4, 12);
                            count += 1;
                        }
                    }
                }
                (count, 1.)
            }
            BlockElement::Sextant(filled) => {
                let mut count = 0;
                for row in 0..3 {
                    for column in 0..2 {
                        if filled & (1 << (row * 2 + column)) != 0 {
                            rects[count] = (column * 4, row * 8, 4, 8);
                            count += 1;
                        }
                    }
                }
                (count, 1.)
            }
        }
    }
}

/// 字符是否按子格矩形绘制（[`LinePainter::paint_text`] 会跳过这些字形）。
fn is_block_element(ch: char) -> bool {
    block_element(ch).is_some()
}

/// 块元素字符 → 形状；普通字符返回 `None`，仍由字体绘制。
fn block_element(ch: char) -> Option<BlockElement> {
    let codepoint = ch as u32;
    match codepoint {
        // ▀ 上半块。
        0x2580 => Some(BlockElement::Rect(0, 0, 8, 12)),
        // ▁▂▃▄▅▆▇█ 下 1..=8 个八分之一块。
        0x2581..=0x2588 => {
            let eighths = (codepoint - 0x2580) as i32;
            Some(BlockElement::Rect(0, 24 - eighths * 3, 8, eighths * 3))
        }
        // ▉▊▋▌▍▎▏ 左 7..=1 个八分之一块。
        0x2589..=0x258F => Some(BlockElement::Rect(0, 0, (0x2590 - codepoint) as i32, 24)),
        // ▐ 右半块；▔ 上八分之一块；▕ 右八分之一块。
        0x2590 => Some(BlockElement::Rect(4, 0, 4, 24)),
        0x2594 => Some(BlockElement::Rect(0, 0, 8, 3)),
        0x2595 => Some(BlockElement::Rect(7, 0, 1, 24)),
        // ░▒▓ 用整格不透明度近似字体的网点图案。
        0x2591 => Some(BlockElement::Shade(0.25)),
        0x2592 => Some(BlockElement::Shade(0.5)),
        0x2593 => Some(BlockElement::Shade(0.75)),
        _ => quadrant_char_to_filled_bits(ch)
            .map(BlockElement::Quadrant)
            .or_else(|| sextant_char_to_filled_bits(ch).map(BlockElement::Sextant)),
    }
}

/// 象限块（U+2596–U+259F）的填充位图，位 `row * 2 + column`。
fn quadrant_char_to_filled_bits(ch: char) -> Option<u8> {
    Some(match ch {
        '▘' => 0b0001,
        '▝' => 0b0010,
        '▖' => 0b0100,
        '▗' => 0b1000,
        '▚' => 0b1001,
        '▞' => 0b0110,
        '▛' => 0b0111,
        '▜' => 0b1011,
        '▙' => 0b1101,
        '▟' => 0b1110,
        _ => return None,
    })
}

/// 六分之一块（U+1FB00–U+1FB3B）的填充位图，位 `row * 2 + column`。
///
/// 该区段枚举 2×3 子格的全部填充组合，但跳过已经是块元素的四种
/// （空、`▌`、`▐`、`█`），因此偏移需要回补。
fn sextant_char_to_filled_bits(ch: char) -> Option<u8> {
    let offset = (ch as u32).checked_sub(0x1FB00)?;
    if offset > 0x3B {
        return None;
    }
    Some((offset + 1 + u32::from(offset >= 20) + u32::from(offset >= 40)) as u8)
}

/// 追加一段块元素矩形；与上一段同色、同行且相邻时合并。
fn extend_block_element(
    pending: Option<(Bounds<Pixels>, Hsla)>,
    bounds: Bounds<Pixels>,
    color: Hsla,
    scale_factor: f32,
    window: &mut Window,
) -> Option<(Bounds<Pixels>, Hsla)> {
    let Some((previous, previous_color)) = pending else {
        return Some((bounds, color));
    };

    let contiguous = previous_color == color
        && (previous.origin.y - bounds.origin.y).abs() < px(0.01)
        && (previous.size.height - bounds.size.height).abs() < px(0.01)
        && (previous.origin.x + previous.size.width - bounds.origin.x).abs() < px(0.01);
    if contiguous {
        Some((
            Bounds {
                origin: previous.origin,
                size: size(
                    previous.size.width + bounds.size.width,
                    previous.size.height,
                ),
            },
            previous_color,
        ))
    } else {
        window.paint_quad(fill(
            snap_to_device_pixels(previous, scale_factor),
            previous_color,
        ));
        Some((bounds, color))
    }
}

/// 画出一段待合并的块元素矩形。
fn flush_block_element(
    pending: &mut Option<(Bounds<Pixels>, Hsla)>,
    scale_factor: f32,
    window: &mut Window,
) {
    if let Some((bounds, color)) = pending.take() {
        window.paint_quad(fill(snap_to_device_pixels(bounds, scale_factor), color));
    }
}

/// 把坐标向下取整到设备像素。
fn snap_to_device_pixel(value: Pixels, scale_factor: f32) -> Pixels {
    px((f32::from(value) * scale_factor).floor() / scale_factor)
}

/// 按（前景色, 背景色）缓存对比度调整结果。
///
/// 一屏里同样的颜色组合会重复成千上万次（默认前景、同色的 TUI 面板、
/// 状态行……），逐格做 APCA 二分查找代价太高；这里只保存最近的组合。
#[derive(Default)]
struct ContrastCache {
    entries: Vec<((TermColor, TermColor), Hsla)>,
}

impl ContrastCache {
    /// 缓存条数上限；超出后从头替换，够覆盖常见主题。
    const CAPACITY: usize = 64;

    /// 取得在 `bg_color` 上不小于最小对比度的 `fg_color`。
    fn adjusted(&mut self, fg: TermColor, bg: TermColor, fg_color: Hsla, bg_color: Hsla) -> Hsla {
        if let Some((_, color)) = self
            .entries
            .iter()
            .find(|((entry_fg, entry_bg), _)| *entry_fg == fg && *entry_bg == bg)
        {
            return *color;
        }

        let color = ensure_minimum_contrast(fg_color, bg_color, TERMINAL_MINIMUM_CONTRAST);
        if self.entries.len() >= Self::CAPACITY {
            self.entries.remove(0);
        }
        self.entries.push(((fg, bg), color));
        color
    }
}

/// 把长度向上取整到设备像素。
fn ceil_to_device_pixel(value: Pixels, scale_factor: f32) -> Pixels {
    px((f32::from(value) * scale_factor).ceil() / scale_factor)
}

/// 把四边形对齐到设备像素：原点向下取整、尺寸向上取整，相邻色块不会露出接缝。
fn snap_to_device_pixels(bounds: Bounds<Pixels>, scale_factor: f32) -> Bounds<Pixels> {
    Bounds {
        origin: point(
            snap_to_device_pixel(bounds.origin.x, scale_factor),
            snap_to_device_pixel(bounds.origin.y, scale_factor),
        ),
        size: size(
            ceil_to_device_pixel(bounds.size.width, scale_factor),
            ceil_to_device_pixel(bounds.size.height, scale_factor),
        ),
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
    window.paint_quad(fill(
        snap_to_device_pixels(bounds, painter.scale_factor),
        color,
    ));
}

/// 绘制滚动条：有滚动历史时在元素右缘显示滑块。
///
/// `display_offset` 为 0 时视口在底部（滑块到底），等于历史总行数时
/// 视口在顶部（滑块到顶）。
fn paint_scrollbar(
    bounds: Bounds<Pixels>,
    snapshot: &GridSnapshot,
    palette: &TerminalPalette,
    scale_factor: f32,
    window: &mut Window,
) {
    let total = snapshot.rows + snapshot.history_size;
    if snapshot.history_size == 0 || total == 0 {
        return;
    }
    let track_height = bounds.size.height;
    let thumb_height = (track_height * (snapshot.rows as f32 / total as f32))
        .max(px(SCROLLBAR_MIN_HEIGHT))
        .min(track_height);
    let progress = (snapshot
        .history_size
        .saturating_sub(snapshot.display_offset)) as f32
        / snapshot.history_size as f32;
    let thumb = Bounds {
        origin: point(
            bounds.origin.x + bounds.size.width - px(SCROLLBAR_WIDTH) - px(2.),
            bounds.origin.y + (track_height - thumb_height) * progress,
        ),
        size: size(px(SCROLLBAR_WIDTH), thumb_height),
    };
    window.paint_quad(
        fill(
            snap_to_device_pixels(thumb, scale_factor),
            palette.scrollbar_thumb,
        )
        .corner_radii(px(SCROLLBAR_WIDTH / 2.)),
    );
}

/// 绘制输入法预编辑文本：覆盖原位文本并带下划线，候选窗口跟随此处。
#[allow(clippy::too_many_arguments)]
fn paint_preedit(
    preedit: &SharedString,
    palette: &TerminalPalette,
    origin: Point<Pixels>,
    geometry: &ElementGeometry,
    scale_factor: f32,
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
    window.paint_quad(fill(
        snap_to_device_pixels(background, scale_factor),
        palette.background,
    ));

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

/// 终端颜色 → 可绘制颜色；索引色按 xterm 256 色展开。
fn resolve_color(color: TermColor, palette: &TerminalPalette) -> Hsla {
    match color {
        TermColor::Foreground => palette.foreground,
        TermColor::Background => palette.background,
        TermColor::Cursor => palette.cursor,
        TermColor::Indexed(index) => indexed_color(index, palette),
        TermColor::Rgb(r, g, b) => rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into(),
    }
}

/// TUI 明确指定的颜色（真彩色或 256 色中 ≥ 16 的索引）：不做对比度调整。
fn is_app_chosen_exact_color(color: TermColor) -> bool {
    matches!(color, TermColor::Rgb(..) | TermColor::Indexed(16..=255))
}

/// 装饰字符：颜色需要与相邻背景/字形贴合，不做对比度调整。
fn is_decorative_character(character: char) -> bool {
    matches!(
        character as u32,
        // Unicode 盒线与块元素。
        0x2500..=0x257F | 0x2580..=0x259F
        // 几何图形（■ ▶ ● 等分隔符）。
        | 0x25A0..=0x25FF
        // 终端二维码常用的六分之一块。
        | 0x1FB00..=0x1FB3B
        // Powerline 分隔符（私有区）。
        | 0xE0B0..=0xE0D7
    )
}

#[cfg(test)]
mod tests {
    use super::{
        BLOCK_SUBCELL_COLUMNS, BLOCK_SUBCELL_LINES, BlockElement, ContrastCache, MAX_BLOCK_RECTS,
        block_element, ceil_to_device_pixel, is_app_chosen_exact_color, is_block_element,
        is_decorative_character, snap_to_device_pixel, snap_to_device_pixels,
    };
    use glim_terminal::TermColor;
    use glim_ui::{TERMINAL_MINIMUM_CONTRAST, ensure_minimum_contrast};
    use gpui_kit::{Bounds, Hsla, point, px, rgb, size};

    /// 展开形状的子格矩形（按数量截取）。
    fn rects(ch: char) -> Vec<(i32, i32, i32, i32)> {
        let shape = block_element(ch).expect("a block element");
        let mut buffer = [(0, 0, 0, 0); MAX_BLOCK_RECTS];
        let (count, _) = shape.write_rects(&mut buffer);
        buffer.into_iter().take(count).collect()
    }

    /// 八分之一块与半块的矩形在 8×24 子网格上的位置。
    #[test]
    fn block_chars_map_to_subcell_rects() {
        assert_eq!(rects('█'), vec![(0, 0, 8, 24)]);
        assert_eq!(rects('▀'), vec![(0, 0, 8, 12)]);
        assert_eq!(rects('▄'), vec![(0, 12, 8, 12)]);
        assert_eq!(rects('▌'), vec![(0, 0, 4, 24)]);
        assert_eq!(rects('▐'), vec![(4, 0, 4, 24)]);
        assert_eq!(rects('▏'), vec![(0, 0, 1, 24)]);
        assert_eq!(rects('▔'), vec![(0, 0, 8, 3)]);
        assert_eq!(rects('▕'), vec![(7, 0, 1, 24)]);

        // 下 3/8 块：从第 15 行到第 24 行。
        assert_eq!(rects('▃'), vec![(0, 15, 8, 9)]);
    }

    /// 象限块与六分之一块按位图展开，`░▒▓` 用整格不透明度。
    #[test]
    fn quadrant_sextant_and_shade_shapes() {
        assert_eq!(rects('▚'), vec![(0, 0, 4, 12), (4, 12, 4, 12)]);

        // U+1FB00 只填充左上子格；U+1FB3B 除左上外全部填充。
        assert_eq!(rects('\u{1FB00}'), vec![(0, 0, 4, 8)]);
        assert_eq!(
            rects('\u{1FB3B}'),
            vec![
                (4, 0, 4, 8),
                (0, 8, 4, 8),
                (4, 8, 4, 8),
                (0, 16, 4, 8),
                (4, 16, 4, 8)
            ]
        );

        let mut buffer = [(0, 0, 0, 0); MAX_BLOCK_RECTS];
        let (count, opacity) = block_element('░')
            .expect("shade element")
            .write_rects(&mut buffer);
        assert_eq!(count, 1);
        assert_eq!(opacity, 0.25);
        assert_eq!(
            buffer[0],
            (0, 0, BLOCK_SUBCELL_COLUMNS, BLOCK_SUBCELL_LINES)
        );
        assert_eq!(block_element('▒'), Some(BlockElement::Shade(0.5)));
        assert_eq!(block_element('▓'), Some(BlockElement::Shade(0.75)));

        // 普通字符不参与块元素绘制。
        assert_eq!(block_element('a'), None);
        assert!(!is_block_element('中'));
        assert!(is_block_element('█'));
    }

    /// 坐标向下取整、长度向上取整到设备像素。
    #[test]
    fn device_pixel_snapping() {
        assert_eq!(snap_to_device_pixel(px(10.4), 1.), px(10.));
        assert_eq!(snap_to_device_pixel(px(10.5), 1.25), px(10.4));
        assert_eq!(ceil_to_device_pixel(px(8.4), 1.), px(9.));
        assert_eq!(ceil_to_device_pixel(px(8.5), 1.25), px(8.8));

        let snapped = snap_to_device_pixels(
            Bounds {
                origin: point(px(10.4), px(20.2)),
                size: size(px(8.4), px(16.1)),
            },
            2.,
        );
        assert_eq!(snapped.origin, point(px(10.), px(20.)));
        assert_eq!(snapped.size, size(px(8.5), px(16.5)));
    }

    /// 装饰字符与 TUI 指定颜色跳过对比度调整。
    #[test]
    fn contrast_skips_decorations_and_exact_colors() {
        assert!(is_decorative_character('─'));
        assert!(is_decorative_character('█'));
        assert!(is_decorative_character('\u{E0B0}'));
        assert!(!is_decorative_character('a'));
        assert!(!is_decorative_character('中'));

        assert!(is_app_chosen_exact_color(TermColor::Rgb(1, 2, 3)));
        assert!(is_app_chosen_exact_color(TermColor::Indexed(200)));
        assert!(!is_app_chosen_exact_color(TermColor::Indexed(15)));
        assert!(!is_app_chosen_exact_color(TermColor::Foreground));
    }

    /// 颜色组合缓存命中同一结果，且与直接计算一致。
    #[test]
    fn contrast_cache_matches_direct_call() {
        let mut cache = ContrastCache::default();
        let fg: Hsla = rgb(0xf4f5ef).into();
        let bg: Hsla = rgb(0xf4f5ef).into();

        let adjusted = cache.adjusted(TermColor::Indexed(15), TermColor::Background, fg, bg);
        assert_eq!(
            adjusted,
            ensure_minimum_contrast(fg, bg, TERMINAL_MINIMUM_CONTRAST)
        );
        assert_eq!(
            cache.adjusted(TermColor::Indexed(15), TermColor::Background, fg, bg),
            adjusted,
            "同一组合应命中缓存"
        );
        assert_eq!(cache.entries.len(), 1, "缓存不重复记录同一组合");
    }
}
