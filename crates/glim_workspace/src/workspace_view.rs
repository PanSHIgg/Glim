//! 工作区视图：标签页、分屏窗格、状态栏与关闭确认。
//!
//! 工作区拥有标签页与分屏树，每个窗格是一个独立的终端视图实体。
//! 切换标签、分屏、缩放与拖动分隔线只改变布局，不重建会话；
//! 关闭运行中的会话需要确认，应用退出时统一汇总一次。

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use glim_terminal_view::{TerminalStatus, TerminalView, TerminalViewEvent};
use glim_ui::{ThemeCapsule, ThemePreference};

use crate::pane_tree::{Axis, Direction, PaneId, PaneTree, SplitId, TabId, nearest_in_direction};

actions!(
    workspace,
    [
        /// 新建标签页。
        NewTab,
        /// 切换到下一个标签页。
        NextTab,
        /// 切换到上一个标签页。
        PreviousTab,
        /// 在当前窗格向右分屏。
        SplitRight,
        /// 在当前窗格向下分屏。
        SplitDown,
        /// 关闭当前窗格。
        ClosePane,
        /// 放大当前窗格 / 还原分屏。
        ToggleZoom,
        /// 聚焦左侧窗格。
        FocusPaneLeft,
        /// 聚焦右侧窗格。
        FocusPaneRight,
        /// 聚焦上方窗格。
        FocusPaneUp,
        /// 聚焦下方窗格。
        FocusPaneDown,
    ]
);

/// 工作区快捷键：标签页、分屏、窗格焦点与缩放。
///
/// 终端持有键盘焦点时上下文为 "Terminal"；工作区动作绑定在更外层的
/// "Workspace" 上。按键没有工作区绑定时照常写入终端（如 Alt+方向键）。
fn bind_workspace_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-t", NewTab, Some("Workspace")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Workspace")),
        KeyBinding::new("ctrl-shift-d", SplitRight, Some("Workspace")),
        KeyBinding::new("ctrl-shift-e", SplitDown, Some("Workspace")),
        KeyBinding::new("ctrl-shift-w", ClosePane, Some("Workspace")),
        KeyBinding::new("ctrl-shift-enter", ToggleZoom, Some("Workspace")),
        KeyBinding::new("alt-left", FocusPaneLeft, Some("Workspace")),
        KeyBinding::new("alt-right", FocusPaneRight, Some("Workspace")),
        KeyBinding::new("alt-up", FocusPaneUp, Some("Workspace")),
        KeyBinding::new("alt-down", FocusPaneDown, Some("Workspace")),
    ]);
}

/// 一个标签页：分屏树、活动窗格与放大的窗格。
struct TabEntry {
    id: TabId,
    /// 分屏树；`None` 表示空标签页（全部窗格已关闭）。
    tree: Option<PaneTree>,
    /// 当前活动窗格。
    active: Option<PaneId>,
    /// 被放大的窗格；`None` 表示正常分屏。
    zoomed: Option<PaneId>,
}

/// 一个窗格：终端视图与它的订阅。
struct PaneEntry {
    view: Entity<TerminalView>,
    _events: Subscription,
}

/// 工作区外壳。
pub struct WorkspaceView {
    theme_preference: ThemePreference,
    /// 标签页顺序。
    tabs: Vec<TabEntry>,
    /// 当前标签页。
    active_tab: TabId,
    /// 所有标签页的窗格；切换标签不重建会话。
    panes: HashMap<PaneId, PaneEntry>,
    /// 标签栏滚动句柄；新建或切换标签后把目标标签滚入可见区。
    tab_scroll: ScrollHandle,
    /// 分隔线拖动中的分屏节点。
    dragging_split: Option<SplitId>,
    /// 用户已确认退出；窗口关闭回调据此放行。
    exit_confirmed: bool,
    /// 空工作区的焦点入口。
    empty_focus: FocusHandle,
    next_tab_id: u64,
    next_pane_id: u64,
    next_split_id: u64,
    _appearance_observer: Subscription,
}

impl WorkspaceView {
    /// 创建工作区：启动一个真实终端，并登记窗口关闭确认。
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_workspace_keys(cx);

        let theme_preference = ThemePreference::default();
        let appearance_observer = cx.observe_window_appearance(window, |this, window, cx| {
            if this.theme_preference.is_system() {
                glim_ui::apply_theme_preference(this.theme_preference, Some(window), cx);
            }
        });

        let mut workspace = Self {
            theme_preference,
            tabs: Vec::new(),
            active_tab: TabId(0),
            panes: HashMap::new(),
            tab_scroll: ScrollHandle::new(),
            dragging_split: None,
            exit_confirmed: false,
            empty_focus: cx.focus_handle(),
            next_tab_id: 1,
            next_pane_id: 1,
            next_split_id: 1,
            _appearance_observer: appearance_observer,
        };

        // 首次启动打开一个真实终端。
        let tab = workspace.new_tab();
        let pane = workspace.new_pane(None, window, cx);
        workspace.active_tab = tab;
        if let Some(entry) = workspace.tab_mut(tab) {
            entry.tree = Some(PaneTree::leaf(pane));
            entry.active = Some(pane);
        }
        workspace.focus_pane(pane, window, cx);

        // 窗口关闭：运行中的会话汇总为一次确认。
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let Some(workspace) = weak.upgrade() else {
                return true;
            };
            if workspace.read(cx).exit_confirmed {
                return true;
            }
            let busy = workspace
                .read(cx)
                .panes
                .values()
                .filter(|entry| entry.view.read(cx).has_running_task())
                .count();
            if busy == 0 {
                return true;
            }
            workspace.update(cx, |workspace, cx| {
                workspace.open_exit_confirm(busy, window, cx);
            });
            false
        });

        workspace
    }

    // ---- 标签页 ----

    /// 新建一个空标签页并返回身份。
    fn new_tab(&mut self) -> TabId {
        let id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        self.tabs.push(TabEntry {
            id,
            tree: None,
            active: None,
            zoomed: None,
        });
        id
    }

    /// 新建窗格；给定目录时以该目录启动 shell。
    fn new_pane(
        &mut self,
        directory: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PaneId {
        let id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        let view = cx.new(|cx| TerminalView::with_working_directory(directory, window, cx));
        let events = cx.subscribe(&view, |_this, _view, _event: &TerminalViewEvent, cx| {
            // 标题与状态变化需要在标签与状态栏上反映。
            cx.notify();
        });
        self.panes.insert(
            id,
            PaneEntry {
                view,
                _events: events,
            },
        );
        id
    }

    /// 当前活动窗格的已知工作目录；新标签与新分屏继承它。
    fn active_directory(&self, cx: &App) -> Option<PathBuf> {
        let pane = self.active_tab_entry()?.active?;
        self.panes
            .get(&pane)
            .and_then(|entry| entry.view.read(cx).working_directory())
    }

    /// 打开新标签页，继承当前活动窗格的工作目录。
    fn open_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let directory = self.active_directory(cx);
        let tab = self.new_tab();
        let pane = self.new_pane(directory, window, cx);
        self.active_tab = tab;
        if let Some(entry) = self.tab_mut(tab) {
            entry.tree = Some(PaneTree::leaf(pane));
            entry.active = Some(pane);
        }
        self.reveal_active_tab();
        self.focus_pane(pane, window, cx);
        cx.notify();
    }

    /// 选中标签页并聚焦它的活动窗格。
    fn select_tab_index(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let id = tab.id;
        let active = tab.active;
        self.active_tab = id;
        self.reveal_active_tab();
        match active {
            Some(pane) => self.focus_pane(pane, window, cx),
            None => window.focus(&self.empty_focus, cx),
        }
        cx.notify();
    }

    /// 按偏移切换标签页（`Ctrl+Tab` / `Ctrl+Shift+Tab`）。
    fn switch_tab(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let current = self.active_tab_index().unwrap_or(0) as isize;
        let len = self.tabs.len() as isize;
        let next = (current + step).rem_euclid(len) as usize;
        self.select_tab_index(next, window, cx);
    }

    /// 关闭标签页；存在运行中的任务时先确认。
    fn close_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let busy = self.tab_busy_count(id, cx);
        if busy > 0 {
            self.confirm_close_tab(id, busy, window, cx);
            return;
        }
        if let Some(handle) = self.close_tab_now(id, cx) {
            window.focus(&handle, cx);
        }
    }

    /// 关闭标签页并释放它的全部会话；返回之后应聚焦的句柄。
    fn close_tab_now(&mut self, id: TabId, cx: &mut Context<Self>) -> Option<FocusHandle> {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return None;
        };
        let tab = self.tabs.remove(index);
        if let Some(tree) = &tab.tree {
            for pane in tree.leaves() {
                self.panes.remove(&pane);
            }
        }

        // 始终保留至少一个（空）标签页，作为空工作区恢复入口。
        if self.tabs.is_empty() {
            let id = self.new_tab();
            self.active_tab = id;
            cx.notify();
            return Some(self.empty_focus.clone());
        }

        if self.active_tab != tab.id {
            cx.notify();
            return None;
        }

        let next_index = index.min(self.tabs.len() - 1);
        self.active_tab = self.tabs[next_index].id;
        self.reveal_active_tab();
        let active = self.tabs[next_index].active;
        let handle = active.and_then(|pane| self.pane_focus_handle(pane, cx));
        cx.notify();
        Some(handle.unwrap_or_else(|| self.empty_focus.clone()))
    }

    /// 标签页内仍在运行任务的窗格数。
    fn tab_busy_count(&self, id: TabId, cx: &App) -> usize {
        self.tabs
            .iter()
            .find(|tab| tab.id == id)
            .and_then(|tab| tab.tree.as_ref())
            .map(|tree| {
                tree.leaves()
                    .iter()
                    .filter(|pane| self.pane_busy(**pane, cx))
                    .count()
            })
            .unwrap_or(0)
    }

    // ---- 窗格 ----

    /// 在活动窗格处分屏，新窗格继承当前工作目录并成为活动窗格。
    fn split_active(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.active_tab_index() else {
            return;
        };
        let active = self.tabs[index].active;
        let directory = active.and_then(|pane| {
            self.panes
                .get(&pane)
                .and_then(|entry| entry.view.read(cx).working_directory())
        });
        let new_pane = self.new_pane(directory, window, cx);
        let split = SplitId(self.next_split_id);
        self.next_split_id += 1;

        let tab = &mut self.tabs[index];
        let inserted = match (tab.tree.as_mut(), active) {
            (Some(tree), Some(active)) => tree.split_at(active, split, axis, new_pane),
            _ => false,
        };
        if !inserted {
            // 活动窗格不在树中（不应发生）：把整棵树与新窗格并排，避免丢窗格。
            let existing = tab.tree.take();
            tab.tree = Some(match existing {
                Some(tree) => PaneTree::split(split, axis, tree, PaneTree::leaf(new_pane)),
                None => PaneTree::leaf(new_pane),
            });
        }
        tab.active = Some(new_pane);
        tab.zoomed = None;
        self.focus_pane(new_pane, window, cx);
        cx.notify();
    }

    /// 点击窗格任何位置：设为活动窗格并聚焦它的终端。
    fn activate_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.active_tab_entry_mut() {
            if tab.tree.as_ref().is_some_and(|tree| tree.contains(pane)) {
                tab.active = Some(pane);
            }
        }
        self.focus_pane(pane, window, cx);
        cx.notify();
    }

    /// 放大 / 还原窗格（`Ctrl+Shift+Enter`）。
    fn toggle_zoom_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.active_tab_index() else {
            return;
        };
        let tab = &mut self.tabs[index];
        if !tab.tree.as_ref().is_some_and(|tree| tree.contains(pane)) {
            return;
        }
        tab.active = Some(pane);
        tab.zoomed = if tab.zoomed == Some(pane) {
            None
        } else {
            Some(pane)
        };
        self.focus_pane(pane, window, cx);
        cx.notify();
    }

    /// 在指定方向寻找并聚焦最近的窗格（`Alt+方向键`）。
    fn focus_direction(
        &mut self,
        direction: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.active_tab_index() else {
            return;
        };
        let tab = &mut self.tabs[index];
        let (Some(tree), Some(active)) = (tab.tree.as_ref(), tab.active) else {
            return;
        };
        let geometry = tree.geometry();
        let Some(target) = nearest_in_direction(&geometry, active, direction) else {
            return;
        };
        tab.active = Some(target);
        if tab.zoomed.is_some() {
            // 放大状态下移动焦点：放大跟随焦点窗格。
            tab.zoomed = Some(target);
        }
        self.focus_pane(target, window, cx);
        cx.notify();
    }

    /// 关闭窗格；存在运行中的任务时先确认。
    fn close_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        if self.pane_busy(pane, cx) {
            self.confirm_close_pane(pane, window, cx);
            return;
        }
        if let Some(handle) = self.close_pane_now(pane, cx) {
            window.focus(&handle, cx);
        }
    }

    /// 关闭窗格并释放会话；返回之后应聚焦的句柄。
    fn close_pane_now(&mut self, pane: PaneId, cx: &mut Context<Self>) -> Option<FocusHandle> {
        let index = self.active_tab_index()?;
        let tab = &mut self.tabs[index];
        if !tab.tree.as_ref().is_some_and(|tree| tree.contains(pane)) {
            return None;
        }
        if let Some(tree) = tab.tree.take() {
            tab.tree = tree.without(pane);
        }
        self.panes.remove(&pane);
        if tab.active == Some(pane) {
            tab.active = tab
                .tree
                .as_ref()
                .and_then(|tree| tree.leaves().first().copied());
        }
        if tab.zoomed == Some(pane) {
            tab.zoomed = None;
        }
        let handle = tab.active.and_then(|next| self.pane_focus_handle(next, cx));
        cx.notify();
        Some(handle.unwrap_or_else(|| self.empty_focus.clone()))
    }

    /// 重新启动已退出或启动失败的会话。
    fn restart_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.panes.get(&pane) {
            entry.view.update(cx, |view, cx| view.restart(cx));
        }
        self.focus_pane(pane, window, cx);
    }

    /// 空标签页的恢复入口：新建一个终端。
    fn open_pane_in_empty_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.active_tab_index() else {
            return;
        };
        let pane = self.new_pane(None, window, cx);
        let tab = &mut self.tabs[index];
        tab.tree = Some(PaneTree::leaf(pane));
        tab.active = Some(pane);
        tab.zoomed = None;
        self.focus_pane(pane, window, cx);
        cx.notify();
    }

    /// 窗格会话是否有运行中的任务。
    fn pane_busy(&self, pane: PaneId, cx: &App) -> bool {
        self.panes
            .get(&pane)
            .is_some_and(|entry| entry.view.read(cx).has_running_task())
    }

    /// 聚焦窗格的终端视图。
    fn focus_pane(&self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.pane_focus_handle(pane, cx) {
            window.focus(&handle, cx);
        }
    }

    fn pane_focus_handle(&self, pane: PaneId, cx: &App) -> Option<FocusHandle> {
        self.panes
            .get(&pane)
            .map(|entry| entry.view.read(cx).focus_handle())
    }

    // ---- 分隔线拖动 ----

    /// 开始拖动分隔线。
    pub(crate) fn begin_divider_drag(&mut self, split: SplitId, cx: &mut Context<Self>) {
        self.dragging_split = Some(split);
        cx.notify();
    }

    /// 分隔线是否正在拖动。
    pub(crate) fn is_dragging_split(&self, split: SplitId) -> bool {
        self.dragging_split == Some(split)
    }

    /// 拖动中写回比例；布局元素已按最小尺寸限制。
    pub(crate) fn set_split_ratio(&mut self, split: SplitId, ratio: f32, cx: &mut Context<Self>) {
        if let Some(tree) = self
            .active_tab_entry_mut()
            .and_then(|tab| tab.tree.as_mut())
            && tree.set_ratio(split, ratio)
        {
            cx.notify();
        }
    }

    /// 结束拖动。
    pub(crate) fn end_divider_drag(&mut self, cx: &mut Context<Self>) {
        if self.dragging_split.take().is_some() {
            cx.notify();
        }
    }

    // ---- 关闭确认 ----

    /// 关闭窗格前的确认：空闲 shell 直接关闭，运行中任务需要确认。
    fn confirm_close_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let title = self
            .panes
            .get(&pane)
            .map(|entry| pane_heading(entry.view.read(cx)));
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let entity = entity.clone();
            alert
                .title(match &title {
                    Some(title) => format!("关闭“{title}”？"),
                    None => "关闭窗格？".to_owned(),
                })
                .description("该窗格仍在运行任务。关闭后进程结束，输出不会保留。")
                .confirm()
                .ok_text("关闭")
                .cancel_text("取消")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let handle =
                        entity.update(cx, |workspace, cx| workspace.close_pane_now(pane, cx));
                    if let Some(handle) = handle {
                        window.focus(&handle, cx);
                    }
                    true
                })
        });
    }

    /// 关闭标签页前的确认。
    fn confirm_close_tab(
        &mut self,
        id: TabId,
        busy: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = self.tab_title(id, cx);
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let entity = entity.clone();
            alert
                .title(format!("关闭“{title}”？"))
                .description(format!(
                    "该标签页有 {busy} 个窗格仍在运行任务。关闭后进程结束，输出不会保留。"
                ))
                .confirm()
                .ok_text("关闭")
                .cancel_text("取消")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let handle = entity.update(cx, |workspace, cx| workspace.close_tab_now(id, cx));
                    if let Some(handle) = handle {
                        window.focus(&handle, cx);
                    }
                    true
                })
        });
    }

    /// 应用退出的汇总确认：一次确认后统一关闭全部会话。
    fn open_exit_confirm(&mut self, busy: usize, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let weak = weak.clone();
            alert
                .title("退出 Glim？")
                .description(format!(
                    "{busy} 个窗格仍在运行任务。退出后这些进程会结束，输出不会保留。"
                ))
                .confirm()
                .ok_text("退出")
                .cancel_text("取消")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, _window, cx| {
                    if let Some(workspace) = weak.upgrade() {
                        workspace.update(cx, |workspace, _cx| workspace.exit_confirmed = true);
                    }
                    cx.quit();
                    true
                })
        });
    }

    // ---- 渲染 ----

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let active_index = self.active_tab_index().unwrap_or(0);
        let tabs: Vec<(TabId, SharedString)> = self
            .tabs
            .iter()
            .map(|tab| (tab.id, self.tab_title(tab.id, cx)))
            .collect();

        h_flex()
            .h_10()
            .px_2()
            .gap_1()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().title_bar)
            .child(
                TabBar::new("terminal-tabs")
                    .track_scroll(&self.tab_scroll)
                    .max_width(px(200.))
                    .selected_index(active_index)
                    .on_click(cx.listener(|this, index: &usize, window, cx| {
                        this.select_tab_index(*index, window, cx);
                    }))
                    .children(tabs.into_iter().map(|(id, title)| {
                        Tab::new().label(title).suffix(
                            Button::new(("tab-close", id.value()))
                                .icon(IconName::X)
                                .ghost()
                                .xsmall()
                                .tooltip("关闭标签页")
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    this.close_tab(id, window, cx);
                                })),
                        )
                    })),
            )
            .child(
                Button::new("new-tab")
                    .icon(IconName::Plus)
                    .ghost()
                    .tooltip("新建标签页 · Ctrl+Shift+T")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_tab(window, cx);
                    })),
            )
            .child(div().flex_1())
            .child(self.render_split_button())
            .into_any_element()
    }

    /// 顶部的分屏入口；与窗格上的分屏按钮共用同一组动作。
    fn render_split_button(&self) -> AnyElement {
        Button::new("split-menu")
            .icon(IconName::Columns2)
            .label("分屏…")
            .ghost()
            .tooltip("分屏 · Ctrl+Shift+D / E")
            .dropdown_menu(|menu, _window, _cx| {
                menu.item(
                    PopupMenuItem::new("向右分屏")
                        .icon(IconName::Columns2)
                        .action(Box::new(SplitRight)),
                )
                .item(
                    PopupMenuItem::new("向下分屏")
                        .icon(IconName::Rows2)
                        .action(Box::new(SplitDown)),
                )
            })
            .into_any_element()
    }

    /// 当前标签页的窗格区域：正常分屏、放大单窗格或空状态。
    fn render_panes(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(tab) = self.active_tab_entry() else {
            return div().into_any_element();
        };
        let Some(tree) = tab.tree.as_ref() else {
            return self.render_empty_tab(window, cx).into_any_element();
        };

        let active = tab.active;
        let mut panes = HashMap::new();
        match tab.zoomed {
            Some(zoomed) => {
                if let Some(element) = self.render_pane(zoomed, true, true, cx) {
                    panes.insert(zoomed, element);
                }
            }
            None => {
                for pane in tree.leaves() {
                    if let Some(element) = self.render_pane(pane, active == Some(pane), false, cx) {
                        panes.insert(pane, element);
                    }
                }
            }
        }

        if let Some(zoomed) = tab.zoomed {
            return panes
                .remove(&zoomed)
                .unwrap_or_else(|| div().into_any_element());
        }
        crate::split_element::build(tree, &mut panes, cx.entity())
    }

    /// 单个窗格：标题栏（身份与操作）与终端内容。
    fn render_pane(
        &self,
        pane: PaneId,
        active: bool,
        zoomed: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let entry = self.panes.get(&pane)?;
        let view = entry.view.clone();
        let (title, exited, failed) = {
            let terminal = view.read(cx);
            (
                pane_heading(terminal),
                terminal.has_exited(),
                terminal.has_failed(),
            )
        };

        let header = h_flex()
            .h_8()
            .px_2()
            .gap_1()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(if active {
                cx.theme().title_bar
            } else {
                cx.theme().background
            })
            .text_color(if active {
                cx.theme().foreground
            } else {
                cx.theme().muted_foreground
            })
            .child(Icon::new(IconName::SquareTerminal).size_3p5())
            .child(div().flex_1().min_w_0().truncate().text_sm().child(title))
            .when(exited || failed, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(if failed {
                            cx.theme().danger
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(if failed { "启动失败" } else { "已退出" }),
                )
            })
            .child(
                Button::new(("pane-split", pane.value()))
                    .icon(IconName::Columns2)
                    .ghost()
                    .xsmall()
                    .tooltip("分屏…")
                    .dropdown_menu(|menu, _window, _cx| {
                        menu.item(
                            PopupMenuItem::new("向右分屏")
                                .icon(IconName::Columns2)
                                .action(Box::new(SplitRight)),
                        )
                        .item(
                            PopupMenuItem::new("向下分屏")
                                .icon(IconName::Rows2)
                                .action(Box::new(SplitDown)),
                        )
                    }),
            )
            .child(
                Button::new(("pane-zoom", pane.value()))
                    .icon(if zoomed {
                        IconName::Minimize2
                    } else {
                        IconName::Maximize2
                    })
                    .ghost()
                    .xsmall()
                    .tooltip(if zoomed {
                        "恢复分屏 · Ctrl+Shift+Enter"
                    } else {
                        "聚焦窗格 · Ctrl+Shift+Enter"
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.toggle_zoom_pane(pane, window, cx);
                    })),
            )
            .when(exited || failed, |this| {
                this.child(
                    Button::new(("pane-restart", pane.value()))
                        .icon(IconName::RotateCcw)
                        .ghost()
                        .xsmall()
                        .tooltip("重新启动")
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.restart_pane(pane, window, cx);
                        })),
                )
            })
            .child(
                Button::new(("pane-close", pane.value()))
                    .icon(IconName::X)
                    .ghost()
                    .xsmall()
                    .tooltip("关闭窗格 · Ctrl+Shift+W")
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.close_pane(pane, window, cx);
                    })),
            );

        Some(
            v_flex()
                .id(("pane", pane.value()))
                .test_support()
                .size_full()
                .overflow_hidden()
                .bg(cx.theme().background)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.activate_pane(pane, window, cx);
                    }),
                )
                .child(header)
                .child(div().flex_1().min_h_0().child(view))
                .into_any_element(),
        )
    }

    /// 空标签页：说明与恢复入口。
    fn render_empty_tab(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let shortcut = Kbd::binding_for_action(&NewTab, Some("Workspace"), window);
        v_flex()
            .size_full()
            .track_focus(&self.empty_focus)
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                Icon::new(IconName::SquareTerminal)
                    .size_8()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                div()
                    .text_lg()
                    .text_color(cx.theme().foreground)
                    .child("从一个终端开始"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("打开终端，继续手头的工作。"),
            )
            .child(
                Button::new("empty-new-pane")
                    .icon(IconName::Plus)
                    .label("新建终端")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_pane_in_empty_tab(window, cx);
                    })),
            )
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("新建标签页")
                    .when_some(shortcut, |this, kbd| this.child(kbd)),
            )
            .into_any_element()
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.active_tab_entry().and_then(|tab| tab.active);
        let (dot, shell, status_text) = match active.and_then(|pane| self.panes.get(&pane)) {
            Some(entry) => {
                let terminal = entry.view.read(cx);
                let (dot, text) = match terminal.status() {
                    TerminalStatus::Starting => (cx.theme().muted_foreground, "启动中".to_owned()),
                    TerminalStatus::Running => (cx.theme().success, "运行中".to_owned()),
                    TerminalStatus::Exited { code } => (
                        cx.theme().muted_foreground,
                        match code {
                            Some(code) => format!("已退出（代码 {code}）"),
                            None => "已退出".to_owned(),
                        },
                    ),
                    TerminalStatus::Failed { .. } => (cx.theme().danger, "启动失败".to_owned()),
                };
                (dot, terminal.shell_label().to_owned(), text)
            }
            None => (
                cx.theme().muted_foreground,
                "—".to_owned(),
                "没有活动终端".to_owned(),
            ),
        };

        let pane_count = self
            .active_tab_entry()
            .and_then(|tab| tab.tree.as_ref())
            .map(|tree| tree.leaves().len())
            .unwrap_or(0);
        let count_text = if self.zoomed_pane().is_some() {
            format!("{pane_count} 个窗格 · 已聚焦")
        } else {
            format!("{pane_count} 个窗格")
        };

        h_flex()
            .h_9()
            .px_4()
            .gap_2()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .border_t_1()
            .border_color(cx.theme().status_bar_border)
            .bg(cx.theme().status_bar)
            .child(div().size_2().rounded_full().bg(dot))
            .child(shell)
            .child(div().w(px(1.)).h_2p5().bg(cx.theme().border))
            .child(status_text)
            .child(div().flex_1())
            .child(count_text)
            .child(div().w(px(1.)).h_2p5().bg(cx.theme().border))
            .child(
                ThemeCapsule::new(self.theme_preference).on_select(cx.listener(Self::select_theme)),
            )
            .into_any_element()
    }

    // ---- 查询与辅助 ----

    fn active_tab_index(&self) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == self.active_tab)
    }

    fn active_tab_entry(&self) -> Option<&TabEntry> {
        self.tabs.iter().find(|tab| tab.id == self.active_tab)
    }

    fn active_tab_entry_mut(&mut self) -> Option<&mut TabEntry> {
        self.tabs.iter_mut().find(|tab| tab.id == self.active_tab)
    }

    fn tab_mut(&mut self, id: TabId) -> Option<&mut TabEntry> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    fn zoomed_pane(&self) -> Option<PaneId> {
        self.active_tab_entry().and_then(|tab| tab.zoomed)
    }

    /// 标签名称：活动窗格的会话标题，缺省用 shell 名称。
    fn tab_title(&self, id: TabId, cx: &App) -> SharedString {
        self.tabs
            .iter()
            .find(|tab| tab.id == id)
            .and_then(|tab| tab.active)
            .and_then(|pane| self.panes.get(&pane))
            .map(|entry| SharedString::from(pane_title(entry.view.read(cx))))
            .unwrap_or_else(|| SharedString::from("终端"))
    }

    /// 把活动标签页滚入标签栏可见区。
    fn reveal_active_tab(&self) {
        if let Some(index) = self.active_tab_index() {
            self.tab_scroll.scroll_to_item(index);
        }
    }

    fn select_theme(
        &mut self,
        preference: &ThemePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.theme_preference == *preference {
            return;
        }
        self.theme_preference = *preference;
        glim_ui::apply_theme_preference(*preference, Some(window), cx);
        cx.notify();
    }

    // ---- Actions ----

    fn new_tab_action(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tab(window, cx);
    }

    fn next_tab_action(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_tab(1, window, cx);
    }

    fn previous_tab_action(
        &mut self,
        _: &PreviousTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.switch_tab(-1, window, cx);
    }

    fn split_right_action(&mut self, _: &SplitRight, window: &mut Window, cx: &mut Context<Self>) {
        self.split_active(Axis::Horizontal, window, cx);
    }

    fn split_down_action(&mut self, _: &SplitDown, window: &mut Window, cx: &mut Context<Self>) {
        self.split_active(Axis::Vertical, window, cx);
    }

    fn close_pane_action(&mut self, _: &ClosePane, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane) = self.active_tab_entry().and_then(|tab| tab.active) {
            self.close_pane(pane, window, cx);
        }
    }

    fn toggle_zoom_action(&mut self, _: &ToggleZoom, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane) = self.active_tab_entry().and_then(|tab| tab.active) {
            self.toggle_zoom_pane(pane, window, cx);
        }
    }

    fn focus_left_action(
        &mut self,
        _: &FocusPaneLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_direction(Direction::Left, window, cx);
    }

    fn focus_right_action(
        &mut self,
        _: &FocusPaneRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_direction(Direction::Right, window, cx);
    }

    fn focus_up_action(&mut self, _: &FocusPaneUp, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_direction(Direction::Up, window, cx);
    }

    fn focus_down_action(
        &mut self,
        _: &FocusPaneDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_direction(Direction::Down, window, cx);
    }
}

/// 窗格的展示名称：会话标题优先，否则使用 shell 名称。
fn pane_title(terminal: &TerminalView) -> String {
    terminal
        .title()
        .map(str::to_owned)
        .unwrap_or_else(|| terminal.shell_label().to_owned())
}

/// 窗格标题：优先显示跟踪到的工作目录，其次会话标题，最后 shell 名称。
fn pane_heading(terminal: &TerminalView) -> String {
    terminal
        .tracked_directory()
        .map(|directory| directory.display().to_string())
        .unwrap_or_else(|| pane_title(terminal))
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .key_context("Workspace")
            .bg(cx.theme().background)
            .on_action(cx.listener(Self::new_tab_action))
            .on_action(cx.listener(Self::next_tab_action))
            .on_action(cx.listener(Self::previous_tab_action))
            .on_action(cx.listener(Self::split_right_action))
            .on_action(cx.listener(Self::split_down_action))
            .on_action(cx.listener(Self::close_pane_action))
            .on_action(cx.listener(Self::toggle_zoom_action))
            .on_action(cx.listener(Self::focus_left_action))
            .on_action(cx.listener(Self::focus_right_action))
            .on_action(cx.listener(Self::focus_up_action))
            .on_action(cx.listener(Self::focus_down_action))
            .child(self.render_tab_bar(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .relative()
                    .child(self.render_panes(window, cx)),
            )
            .child(self.render_status_bar(cx))
    }
}

#[cfg(test)]
mod tests;
