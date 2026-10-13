//! 输入法：预编辑文本只属于视图，正式提交只写入 PTY 一次。
//!
//! GPUI 把平台输入法的组合过程转成 [`EntityInputHandler`] 调用：
//! `replace_and_mark_text_in_range` 更新预编辑文本（仅视图状态），
//! `replace_text_in_range` 提交正式文本（写入终端），候选窗口通过
//! `bounds_for_range` 跟随光标。

use std::ops::Range;

use gpui_kit::*;

use crate::TerminalView;

impl TerminalView {
    /// 提交输入法结果：清除预编辑并写入终端。
    pub(crate) fn commit_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.ime.preedit = None;
        if text.is_empty() {
            cx.notify();
            return;
        }
        self.send_input(text.as_bytes(), cx);
    }

    /// 更新预编辑文本；不写入终端。
    pub(crate) fn set_preedit(&mut self, text: String, cx: &mut Context<Self>) {
        self.ime.preedit = if text.is_empty() { None } else { Some(text) };
        cx.notify();
    }

    /// 清除预编辑文本。
    pub(crate) fn clear_preedit(&mut self, cx: &mut Context<Self>) {
        if self.ime.preedit.take().is_some() {
            cx.notify();
        }
    }

    /// 光标处的单元矩形（窗口坐标），用于候选窗口定位。
    pub(crate) fn cursor_bounds(&self) -> Option<Bounds<Pixels>> {
        let geometry = self.geometry.as_ref()?;
        let snapshot = self.snapshot.as_ref()?;
        let cursor = snapshot.cursor?;

        let wide = snapshot
            .lines
            .get(cursor.row)
            .and_then(|line| line.cells.get(cursor.column))
            .is_some_and(|cell| cell.attributes.wide);

        Some(Bounds {
            origin: point(
                geometry.origin.x + geometry.cell_width * cursor.column as f32,
                geometry.origin.y + geometry.line_height * cursor.row as f32,
            ),
            size: size(
                geometry.cell_width * if wide { 2. } else { 1. },
                geometry.line_height,
            ),
        })
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        // 终端没有可编辑缓冲区；文本查询不适用。
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // 始终返回有效的光标位置，即使处于备用屏幕（TUI）也用于候选窗口定位。
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.ime
            .preedit
            .as_ref()
            .map(|_| 0..self.ime.preedit_len_utf16())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.clear_preedit(cx);
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_text(text, cx);
        window.invalidate_character_coordinates();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_preedit(new_text.to_owned(), cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let cursor = self.cursor_bounds()?;
        let cell_width = self.geometry.as_ref()?.cell_width;
        let offset = cell_width * range_utf16.start as f32;
        let _ = element_bounds;
        Some(Bounds {
            origin: point(cursor.origin.x + offset, cursor.origin.y),
            size: cursor.size,
        })
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
