//! 终端视图：终端绘制、焦点、输入、输入法、鼠标、选择与滚动。
//!
//! 视图持有一个终端会话：会话在后台运行，视图消费会话事件、
//! 把键盘与鼠标输入编码后写入 PTY，并用自定义元素绘制当前快照。
//! 滚动历史、选择、复制粘贴与查找也由本视图管理。

mod element;
mod find;
mod ime;
mod ime_state;
mod input;
mod mouse;

use std::sync::Arc;

use gpui_kit::component::input::InputEvent;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use glim_terminal::{
    GridPoint, GridSelection, GridSnapshot, MouseButton as TerminalMouseButton, MouseEventKind,
    MouseModifiers, ScrollCommand, SelectionKind, SelectionSide, SessionEvent, SessionEvents,
    SessionHandle, TerminalOptions,
};
use glim_ui::TerminalPalette;

use element::{ElementGeometry, SelectionHighlight, TerminalGridElement};
use find::FindState;
use ime_state::ImeState;
use input::keystroke_bytes;
use mouse::grid_pointer;

actions!(
    terminal,
    [
        /// 复制当前选择。
        CopySelection,
        /// 粘贴剪贴板内容。
        PasteClipboard,
        /// 打开查找栏。
        OpenFind,
        /// 关闭查找栏。
        CloseFind,
        /// 上翻一页。
        ScrollPageUp,
        /// 下翻一页。
        ScrollPageDown,
        /// 滚动到最顶部。
        ScrollToTop,
        /// 回到最新输出。
        ScrollToBottom,
    ]
);

/// 会话状态；工作区状态栏据此显示。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalStatus {
    /// 尚未启动；窗口首次布局得到实际网格尺寸后再创建会话。
    Starting,
    /// 运行中。
    Running,
    /// 已退出；`code` 为退出码，无法取得时为 `None`。
    Exited {
        /// 子进程退出码。
        code: Option<i32>,
    },
    /// 启动失败。
    Failed {
        /// 失败原因。
        message: String,
    },
}

/// 终端视图事件。
#[derive(Clone, Copy, Debug)]
pub enum TerminalViewEvent {
    /// 会话状态变化（启动失败、退出等）。
    StatusChanged,
}

impl EventEmitter<TerminalViewEvent> for TerminalView {}

/// 当前选择意图；拖动时持续更新端点。
struct SelectionState {
    anchor: GridPoint,
    anchor_side: SelectionSide,
    cursor: GridPoint,
    cursor_side: SelectionSide,
    kind: SelectionKind,
    dragging: bool,
}

impl SelectionState {
    fn to_grid_selection(&self) -> GridSelection {
        GridSelection {
            anchor: self.anchor,
            anchor_side: self.anchor_side,
            cursor: self.cursor,
            cursor_side: self.cursor_side,
            kind: self.kind,
        }
    }
}

/// 终端视图实体。
pub struct TerminalView {
    session: Option<SessionHandle>,
    status: TerminalStatus,
    snapshot: Option<Arc<GridSnapshot>>,
    focus_handle: FocusHandle,
    shell_label: String,
    applied_size: Option<(u16, u16)>,
    /// 是否已经收到过客户端输出；用于判断客户端控制台是否已经就绪。
    client_ready: bool,
    /// 客户端就绪前发送过尺寸调整；首个输出后需要重发一次。
    resize_pending: bool,
    geometry: Option<ElementGeometry>,
    selection: Option<SelectionState>,
    ime: ImeState,
    find: FindState,
    /// 滚轮像素累积；避免触控板的小步进被截断丢弃。
    scroll_accumulator: Pixels,
    /// 最近一次上报给终端的窗口激活状态。
    reported_focus: Option<bool>,
    _events_task: Option<Task<()>>,
    _activation_observer: Subscription,
    _find_subscription: Subscription,
}

impl TerminalView {
    /// 创建视图：记住默认 shell 的展示名，等首次布局拿到网格尺寸后启动会话。
    ///
    /// 会话必须按实际网格尺寸创建：ConPTY 在客户端控制台就绪前会丢弃尺寸调整，
    /// 先按默认 80×24 启动再调整会让客户端停留在错误尺寸（内容错位、清屏不完整）。
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let options = TerminalOptions::default();
        let shell_label = options
            .shell()
            .cloned()
            .or_else(glim_terminal::default_shell)
            .map(|shell| shell.display_name())
            .unwrap_or_else(|| "shell".to_owned());

        let focus_handle = cx.focus_handle();
        let find = FindState::new(window, cx);
        let find_subscription = cx.subscribe_in(
            &find.input,
            window,
            |this, _input, event: &InputEvent, _window, cx| {
                this.on_find_event(event, cx);
            },
        );
        let activation_observer = cx.observe_window_activation(window, |this, window, _cx| {
            this.report_focus(window.is_window_active());
        });

        let view = Self {
            session: None,
            status: TerminalStatus::Starting,
            snapshot: None,
            focus_handle,
            shell_label,
            applied_size: None,
            client_ready: false,
            resize_pending: false,
            geometry: None,
            selection: None,
            ime: ImeState::default(),
            find,
            scroll_accumulator: px(0.),
            reported_focus: None,
            _events_task: None,
            _activation_observer: activation_observer,
            _find_subscription: find_subscription,
        };
        window.focus(&view.focus_handle, cx);
        view
    }

    /// 按实际网格尺寸启动会话；仅在尚未启动时生效。
    fn start_session(&mut self, columns: u16, rows: u16, cx: &mut Context<Self>) {
        if self.session.is_some() || !matches!(self.status, TerminalStatus::Starting) {
            return;
        }
        let options = TerminalOptions::default().with_dimensions(columns, rows);
        match SessionHandle::start(&options) {
            Ok((session, events)) => {
                let events_task = cx.spawn(async move |this, cx| {
                    Self::consume_events(this, events, cx).await;
                });
                self.session = Some(session);
                self.status = TerminalStatus::Running;
                self.applied_size = Some((columns, rows));
                self._events_task = Some(events_task);
                self.refresh(cx);
                cx.emit(TerminalViewEvent::StatusChanged);
            }
            Err(error) => {
                self.status = TerminalStatus::Failed {
                    message: error.to_string(),
                };
                cx.emit(TerminalViewEvent::StatusChanged);
                cx.notify();
            }
        }
    }

    async fn consume_events(this: WeakEntity<Self>, mut events: SessionEvents, cx: &mut AsyncApp) {
        while let Some(event) = events.next().await {
            let updated = this.update(cx, |view, cx| view.handle_session_event(event, cx));
            if updated.is_err() {
                break;
            }
        }
    }

    fn handle_session_event(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        if matches!(event, SessionEvent::Output) {
            self.mark_client_ready();
        }
        match event {
            SessionEvent::Output => self.refresh(cx),
            SessionEvent::Title(_) => {}
            SessionEvent::Bell => {}
            SessionEvent::Exited { code } => {
                self.status = TerminalStatus::Exited { code };
                self.refresh(cx);
                cx.emit(TerminalViewEvent::StatusChanged);
            }
        }
    }

    /// 客户端开始输出说明它的控制台已经就绪；此前发送的尺寸调整可能被丢弃。
    fn mark_client_ready(&mut self) {
        if self.client_ready {
            return;
        }
        self.client_ready = true;
        if std::mem::take(&mut self.resize_pending)
            && let Some(session) = &self.session
        {
            session.resend_size();
        }
    }

    /// 工作区状态栏读取的会话状态。
    pub fn status(&self) -> &TerminalStatus {
        &self.status
    }

    /// 当前 shell 的展示名。
    pub fn shell_label(&self) -> &str {
        &self.shell_label
    }

    /// 元素测量后上报几何信息：首次按实际尺寸启动会话，之后网格尺寸变化时调整会话。
    pub(crate) fn apply_geometry(
        &mut self,
        columns: u16,
        rows: u16,
        geometry: ElementGeometry,
        cx: &mut Context<Self>,
    ) {
        self.geometry = Some(geometry);
        if self.session.is_none() {
            // 会话按首次测量到的实际网格尺寸创建，避免启动后再调整尺寸
            // （ConPTY 在客户端就绪前会丢弃调整）。
            self.start_session(columns, rows, cx);
            return;
        }
        if self.applied_size == Some((columns, rows)) {
            return;
        }
        let columns_changed = self
            .applied_size
            .is_some_and(|(previous_columns, _)| previous_columns != columns);
        self.applied_size = Some((columns, rows));
        if columns_changed {
            // 列数变化会使终端内核中的选择失效；视图侧同步清理。
            self.selection = None;
        }

        if let Some(session) = &self.session {
            session.resize(
                columns,
                rows,
                f32::from(geometry.cell_width) as u16,
                f32::from(geometry.line_height) as u16,
            );
            if !self.client_ready {
                // 客户端就绪前的调整可能被丢弃，首个输出后重发一次。
                self.resize_pending = true;
            }
        }
        self.refresh(cx);
    }

    /// 更新快照并请求重绘。
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            self.snapshot = Some(Arc::new(session.snapshot()));
        }
        cx.notify();
    }

    /// 当前终端模式；没有快照时使用默认值。
    fn modes(&self) -> glim_terminal::TerminalModes {
        self.snapshot
            .as_ref()
            .map(|snapshot| snapshot.modes)
            .unwrap_or_default()
    }

    /// 写入 PTY；输入前回到最新输出（标准终端行为）。
    fn send_input(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let scrolled = self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.display_offset > 0);
        if scrolled {
            if let Some(session) = &self.session {
                session.scroll(ScrollCommand::Bottom);
            }
            self.refresh(cx);
        }
        if let Some(session) = &self.session {
            session.write(bytes);
        }
    }

    /// 清除选择（视图与终端内核保持一致）。
    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            session.set_selection(None);
        }
        if self.selection.take().is_some() {
            cx.notify();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() || !self.focus_handle.is_focused(window) {
            return;
        }
        let modes = self.modes();
        if let Some(bytes) = keystroke_bytes(&event.keystroke, event.prefer_character_input, modes)
        {
            self.clear_selection(cx);
            self.send_input(&bytes, cx);
            cx.stop_propagation();
        }
    }

    /// 上报窗口激活状态；终端的焦点进入 / 离开报告跟随窗口激活。
    fn report_focus(&mut self, focused: bool) {
        if self.reported_focus == Some(focused) {
            return;
        }
        self.reported_focus = Some(focused);
        let modes = self.modes();
        if let Some(bytes) = glim_terminal::focus_report(focused, modes)
            && let Some(session) = &self.session
        {
            session.write(bytes);
        }
    }

    // ---- 鼠标 ----

    fn pointer_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(pointer) = self.pointer_to_grid(event.position) else {
            return;
        };
        let modes = self.modes();
        let mouse_button = match event.button {
            MouseButton::Left => TerminalMouseButton::Left,
            MouseButton::Middle => TerminalMouseButton::Middle,
            MouseButton::Right => TerminalMouseButton::Right,
            MouseButton::Navigate(_) => return,
        };

        if modes.mouse_reporting() && !event.modifiers.shift {
            let bytes = glim_terminal::encode_mouse_event(
                MouseEventKind::Press,
                Some(mouse_button),
                pointer.point,
                mouse_modifiers(event.modifiers),
                modes,
            );
            if let Some(bytes) = bytes {
                self.send_input(&bytes, cx);
            }
            return;
        }

        if event.button != MouseButton::Left {
            return;
        }

        let kind = match event.click_count {
            0 | 1 => SelectionKind::Simple,
            2 => SelectionKind::Semantic,
            _ => SelectionKind::Lines,
        };
        let selection = SelectionState {
            anchor: pointer.point,
            anchor_side: pointer.side,
            cursor: pointer.point,
            cursor_side: pointer.side,
            kind,
            dragging: true,
        };
        if let Some(session) = &self.session {
            session.set_selection(Some(selection.to_grid_selection()));
        }
        self.selection = Some(selection);
        cx.notify();
    }

    fn pointer_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(pointer) = self.pointer_to_grid(event.position) else {
            return;
        };
        let modes = self.modes();
        let pressed = event.pressed_button.and_then(to_terminal_button);

        if modes.mouse_reporting() && !event.modifiers.shift {
            let kind = MouseEventKind::Move { button: pressed };
            if let Some(bytes) = glim_terminal::encode_mouse_event(
                kind,
                pressed,
                pointer.point,
                mouse_modifiers(event.modifiers),
                modes,
            ) && let Some(session) = &self.session
            {
                session.write(&bytes);
            }
            return;
        }

        let Some(selection) = &mut self.selection else {
            return;
        };
        if !selection.dragging || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        selection.cursor = pointer.point;
        selection.cursor_side = pointer.side;
        let grid_selection = selection.to_grid_selection();
        if let Some(session) = &self.session {
            session.set_selection(Some(grid_selection));
        }
        self.refresh(cx);
    }

    fn pointer_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let Some(pointer) = self.pointer_to_grid(event.position) else {
            return;
        };
        let modes = self.modes();

        if let Some(selection) = &mut self.selection {
            selection.dragging = false;
        }

        if modes.mouse_reporting() && !event.modifiers.shift {
            if let Some(button) = to_terminal_button(event.button)
                && let Some(bytes) = glim_terminal::encode_mouse_event(
                    MouseEventKind::Release,
                    Some(button),
                    pointer.point,
                    mouse_modifiers(event.modifiers),
                    modes,
                )
            {
                self.send_input(&bytes, cx);
            }
        } else if let Some(session) = &self.session {
            // 空选择（单击 / 抖动）在抬起时清理，避免留下不可见的选区。
            if session.selection_text().is_none() {
                session.set_selection(None);
                self.selection = None;
                self.refresh(cx);
            }
        }
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        if matches!(event.touch_phase, TouchPhase::Started) {
            self.scroll_accumulator = px(0.);
        }

        let Some(geometry) = self.geometry else {
            return;
        };
        let lines = match event.delta {
            ScrollDelta::Pixels(delta) => {
                self.scroll_accumulator += delta.y;
                let line_height = geometry.line_height.max(px(1.));
                let lines = (self.scroll_accumulator / line_height).trunc() as i32;
                self.scroll_accumulator -= line_height * lines as f32;
                lines
            }
            ScrollDelta::Lines(delta) => delta.y.round() as i32,
        };
        if lines == 0 || self.session.is_none() {
            return;
        }

        let modes = self.modes();

        if modes.mouse_reporting() && !event.modifiers.shift {
            if let Some(pointer) = self.pointer_to_grid(event.position) {
                let kind = if lines > 0 {
                    MouseEventKind::WheelUp
                } else {
                    MouseEventKind::WheelDown
                };
                for _ in 0..lines.unsigned_abs() {
                    if let Some(bytes) = glim_terminal::encode_mouse_event(
                        kind,
                        None,
                        pointer.point,
                        mouse_modifiers(event.modifiers),
                        modes,
                    ) && let Some(session) = &self.session
                    {
                        session.write(&bytes);
                    }
                }
            }
            return;
        }

        if modes.contains(glim_terminal::TerminalModes::ALT_SCREEN)
            && modes.contains(glim_terminal::TerminalModes::ALTERNATE_SCROLL)
        {
            let bytes = glim_terminal::alternate_scroll_bytes(lines, modes);
            if let Some(session) = &self.session {
                session.write(&bytes);
            }
            return;
        }

        if let Some(session) = &self.session {
            session.scroll(ScrollCommand::Delta(lines));
        }
        self.refresh(cx);
    }

    /// 指针位置对应的网格坐标；几何或快照缺失时返回 `None`。
    fn pointer_to_grid(&self, position: Point<Pixels>) -> Option<mouse::GridPointer> {
        let geometry = self.geometry.as_ref()?;
        let snapshot = self.snapshot.as_ref()?;
        Some(grid_pointer(
            geometry,
            snapshot.columns,
            snapshot.rows,
            snapshot.display_offset,
            position,
        ))
    }

    // ---- Actions ----

    fn copy_selection(&mut self, _: &CopySelection, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            return;
        };
        if let Some(text) = session.selection_text()
            && !text.is_empty()
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste_clipboard(
        &mut self,
        _: &PasteClipboard,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let bytes = glim_terminal::paste_bytes(&text, self.modes());
        self.send_input(&bytes, cx);
    }

    fn open_find(&mut self, _: &OpenFind, window: &mut Window, cx: &mut Context<Self>) {
        self.find.visible = true;
        let focus = self.find.input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.run_find(None, true, cx);
    }

    fn close_find_action(&mut self, _: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.visible {
            self.close_find(window, cx);
        } else {
            // 查找栏未打开时 Escape 属于终端输入（如 vim 的普通模式）。
            cx.propagate();
        }
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find.visible = false;
        self.find.current = None;
        self.find.not_found = false;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn scroll_page_up(&mut self, _: &ScrollPageUp, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            session.scroll(ScrollCommand::PageUp);
        }
        self.refresh(cx);
    }

    fn scroll_page_down(
        &mut self,
        _: &ScrollPageDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = &self.session {
            session.scroll(ScrollCommand::PageDown);
        }
        self.refresh(cx);
    }

    fn scroll_to_top(&mut self, _: &ScrollToTop, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            session.scroll(ScrollCommand::Top);
        }
        self.refresh(cx);
    }

    fn scroll_to_bottom(
        &mut self,
        _: &ScrollToBottom,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = &self.session {
            session.scroll(ScrollCommand::Bottom);
        }
        self.refresh(cx);
    }

    // ---- 查找 ----

    fn on_find_event(&mut self, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Change => {
                let query = self.find.input.read(cx).value().to_string();
                self.find.query = query;
                self.find.current = None;
                self.find.not_found = false;
                self.run_find(None, true, cx);
            }
            InputEvent::PressEnter { shift, .. } => {
                let forward = !shift;
                let after = self
                    .find
                    .current
                    .map(|current| if forward { current.end } else { current.start });
                self.run_find(after, forward, cx);
            }
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    fn find_next(&mut self, cx: &mut Context<Self>) {
        let after = self.find.current.map(|current| current.end);
        self.run_find(after, true, cx);
    }

    fn find_previous(&mut self, cx: &mut Context<Self>) {
        let after = self.find.current.map(|current| current.start);
        self.run_find(after, false, cx);
    }

    /// 执行查找：更新当前匹配并滚动到可见位置。
    fn run_find(&mut self, after: Option<GridPoint>, forward: bool, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            return;
        };
        if self.find.query.is_empty() {
            self.find.current = None;
            self.find.not_found = false;
            cx.notify();
            return;
        }
        match session.find(&self.find.query, after, forward) {
            Some(found) => {
                self.find.current = Some(found);
                self.find.not_found = false;
                session.scroll_to_reveal(found.start.line);
            }
            None => {
                self.find.current = None;
                if after.is_none() {
                    self.find.not_found = true;
                }
            }
        }
        self.refresh(cx);
    }

    fn render_failed(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let message = match &self.status {
            TerminalStatus::Failed { message } => message.clone(),
            _ => String::new(),
        };

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(cx.theme().background)
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child("无法启动 shell"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(message),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.shell_label.clone()),
            )
    }

    fn render_terminal(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette: TerminalPalette = glim_ui::terminal_palette(cx);
        let focused = self.focus_handle.is_focused(window);

        // 启动失败时显示失败信息；其余情况（含尚未启动）都渲染网格元素，
        // 由元素测量并上报实际几何，视图据此按正确尺寸启动会话。
        let content = if matches!(self.status, TerminalStatus::Failed { .. }) {
            self.render_failed(cx).into_any_element()
        } else {
            let search = self.snapshot.as_ref().and_then(|snapshot| {
                self.find.current.map(|current| {
                    SelectionHighlight::from_span(current.to_span(), snapshot.display_offset)
                })
            });
            TerminalGridElement {
                view: cx.entity(),
                snapshot: self.snapshot.clone(),
                palette,
                font_family: cx.theme().mono_font_family.clone(),
                font_size: cx.theme().mono_font_size,
                focused,
                preedit: self.ime.preedit.clone().map(SharedString::from),
                search,
            }
            .into_any_element()
        };

        h_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context("Terminal")
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(Self::copy_selection))
            .on_action(cx.listener(Self::paste_clipboard))
            .on_action(cx.listener(Self::open_find))
            .on_action(cx.listener(Self::close_find_action))
            .on_action(cx.listener(Self::scroll_page_up))
            .on_action(cx.listener(Self::scroll_page_down))
            .on_action(cx.listener(Self::scroll_to_top))
            .on_action(cx.listener(Self::scroll_to_bottom))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event, window, cx| {
                    window.focus(&this.focus_handle, cx);
                }),
            )
            .bg(palette.background)
            .text_color(palette.foreground)
            .child(content)
            .into_any_element()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .child(self.render_terminal(window, cx))
            .when(self.find.visible, |this| {
                this.child(self.render_find_bar(cx))
            })
    }
}

fn to_terminal_button(button: MouseButton) -> Option<TerminalMouseButton> {
    match button {
        MouseButton::Left => Some(TerminalMouseButton::Left),
        MouseButton::Middle => Some(TerminalMouseButton::Middle),
        MouseButton::Right => Some(TerminalMouseButton::Right),
        MouseButton::Navigate(_) => None,
    }
}

fn mouse_modifiers(modifiers: Modifiers) -> MouseModifiers {
    MouseModifiers {
        shift: modifiers.shift,
        alt: modifiers.alt,
        ctrl: modifiers.control,
    }
}
