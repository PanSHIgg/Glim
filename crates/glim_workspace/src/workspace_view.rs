//! 工作区视图：标签页、分屏窗格、状态栏与关闭确认。
//!
//! 工作区拥有标签页与分屏树，每个窗格是一个独立的终端视图实体。
//! 切换标签、分屏、缩放与拖动分隔线只改变布局，不重建会话；
//! 关闭运行中的会话需要确认，应用退出时统一汇总一次。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{
    Button, ButtonVariant, ButtonVariants as _, Toggle, ToggleVariants as _,
};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, ThemeStyled as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use glim_settings::{Settings, SettingsEvent, SettingsState};

use glim_terminal::{ShellSpec, TerminalOptions};
use glim_terminal_view::{TerminalStatus, TerminalView, TerminalViewEvent};
use glim_ui::{ThemeCapsule, ThemePreference};

use crate::layout::{LayoutTemplate, templates_for};
use crate::pane_tree::{
    Axis, Direction, PaneId, PaneTree, SplitId, TabId, clamp_ratio, nearest_in_direction,
};
use crate::split_element::{SplitLayoutOptions, build as build_split_layout};
use crate::workspace_state::{self, LeafState, TabState, WorkspaceState};

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
        /// 打开 / 关闭布局面板。
        OpenLayout,
        /// 应用当前预览的布局方案。
        ApplyLayout,
        /// 取消预览、拖动或关闭布局面板。
        CancelLayout,
        /// 布局面板：移到上一个方案。
        PreviousLayout,
        /// 布局面板：移到下一个方案。
        NextLayout,
        /// 布局面板：移到第一个方案。
        FirstLayout,
        /// 布局面板：移到最后一个方案。
        LastLayout,
        /// 把当前窗格与左侧窗格交换位置。
        SwapPaneLeft,
        /// 把当前窗格与右侧窗格交换位置。
        SwapPaneRight,
        /// 把当前窗格与上方窗格交换位置。
        SwapPaneUp,
        /// 把当前窗格与下方窗格交换位置。
        SwapPaneDown,
        /// 分隔线向左调整。
        NudgeSplitLeft,
        /// 分隔线向右调整。
        NudgeSplitRight,
        /// 分隔线向上调整。
        NudgeSplitUp,
        /// 分隔线向下调整。
        NudgeSplitDown,
        /// 分隔线回到均分。
        ResetSplitRatio,
    ]
);

/// 拖动窗格标题的起始阈值（像素；指针精度）。
const PANE_DRAG_THRESHOLD: f32 = 5.;
/// 顶部布局热区的最大高度（rem）。
const LAYOUT_HOT_ZONE_REMS: f32 = 9.;
/// 热区占窗格区域高度的比例。
const LAYOUT_HOT_ZONE_FRACTION: f32 = 0.25;
/// 分隔线键盘调整的步长（设计稿：每次 5%）。
const SPLIT_STEP: f32 = 0.05;

/// 工作区快捷键：标签页、分屏、窗格焦点、缩放与布局。
///
/// 终端持有键盘焦点时上下文为 "Terminal"；工作区动作绑定在更外层的
/// "Workspace" 上。分隔线与布局面板各自持有焦点，方向键等按键只在其
/// 焦点上下文中生效，不会抢走终端的输入。
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
        KeyBinding::new("ctrl-shift-left", SwapPaneLeft, Some("Workspace")),
        KeyBinding::new("ctrl-shift-right", SwapPaneRight, Some("Workspace")),
        KeyBinding::new("ctrl-shift-up", SwapPaneUp, Some("Workspace")),
        KeyBinding::new("ctrl-shift-down", SwapPaneDown, Some("Workspace")),
        KeyBinding::new("alt-z", OpenLayout, Some("Workspace")),
        KeyBinding::new("ctrl-,", glim_settings::OpenSettings, Some("Workspace")),
        KeyBinding::new("escape", CancelLayout, Some("Workspace")),
        KeyBinding::new("left", PreviousLayout, Some("LayoutPanel")),
        KeyBinding::new("up", PreviousLayout, Some("LayoutPanel")),
        KeyBinding::new("right", NextLayout, Some("LayoutPanel")),
        KeyBinding::new("down", NextLayout, Some("LayoutPanel")),
        KeyBinding::new("home", FirstLayout, Some("LayoutPanel")),
        KeyBinding::new("end", LastLayout, Some("LayoutPanel")),
        KeyBinding::new("enter", ApplyLayout, Some("LayoutPanel")),
        KeyBinding::new("escape", CancelLayout, Some("LayoutPanel")),
        KeyBinding::new("left", NudgeSplitLeft, Some("SplitDivider")),
        KeyBinding::new("right", NudgeSplitRight, Some("SplitDivider")),
        KeyBinding::new("up", NudgeSplitUp, Some("SplitDivider")),
        KeyBinding::new("down", NudgeSplitDown, Some("SplitDivider")),
        KeyBinding::new("home", ResetSplitRatio, Some("SplitDivider")),
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

/// 布局面板状态；打开期间预览只渲染占位，不写入分屏树。
struct LayoutPanel {
    /// 键盘选中的方案。
    selected: Option<usize>,
    /// 指针悬停的方案；优先于键盘选择展示预览。
    hovered: Option<usize>,
    /// 由拖动标题打开时记录的来源窗格；方案主槽位放它。
    drag_source: Option<PaneId>,
}

impl LayoutPanel {
    /// 当前展示的方案：悬停优先，其次是键盘选择。
    fn preview(&self) -> Option<usize> {
        self.hovered.or(self.selected)
    }
}

/// 布局面板的键盘移动到某个方案。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayoutSelection {
    /// 上一个。
    Previous,
    /// 下一个。
    Next,
    /// 第一个。
    First,
    /// 最后一个。
    Last,
}

/// 窗格标题拖动状态。
struct PaneDrag {
    /// 被拖动的窗格。
    pane: PaneId,
    /// 按下时的指针位置；用于拖动阈值判断。
    origin: Point<Pixels>,
    /// 最近一次指针位置；用于拖动浮标。
    position: Point<Pixels>,
    /// 是否已越过拖动阈值。
    moved: bool,
    /// 指针悬停的交换目标。
    swap_target: Option<PaneId>,
    /// 指针是否位于顶部布局热区。
    hot: bool,
}

/// 分隔线调整界限：由布局元素在点击时提供，拖动与键盘调整共用。
#[derive(Clone, Copy)]
pub(crate) struct SplitMetrics {
    /// 分隔线方向。
    pub axis: Axis,
    /// 节点区域在分屏方向上的像素尺寸。
    pub extent: f32,
    /// 第一个子区域的最小尺寸（像素）。
    pub min_first: f32,
    /// 第二个子区域的最小尺寸（像素）。
    pub min_second: f32,
}

/// 工作区外壳。
pub struct WorkspaceView {
    theme_preference: ThemePreference,
    /// 已应用的外观设置快照：设置变化时只有主题或字体变化才重新应用。
    applied_settings: Settings,
    /// 设置变化订阅；未安装设置服务时为 `None`。
    _settings_subscription: Option<Subscription>,
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
    /// 分隔线焦点与键盘调整界限。
    split_interaction: Option<(SplitId, SplitMetrics)>,
    /// 分隔线焦点句柄；点击分隔线后方向键调整比例。
    split_focus: FocusHandle,
    /// 布局面板；`None` 表示关闭。
    layout: Option<LayoutPanel>,
    /// 指针是否位于布局面板上；拖动经过面板时不关闭它。
    pointer_over_layout: bool,
    /// 布局面板焦点句柄。
    layout_focus: FocusHandle,
    /// 窗格标题拖动状态。
    pane_drag: Option<PaneDrag>,
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
    /// 创建工作区：恢复上次布局，或启动一个真实终端，并登记窗口关闭确认。
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_workspace_keys(cx);

        let settings = glim_settings::current(cx);
        let theme_preference = settings.theme().to_preference();
        let appearance_observer = cx.observe_window_appearance(window, |this, window, cx| {
            if this.theme_preference.is_system() {
                glim_ui::apply_theme_preference(this.theme_preference, Some(window), cx);
            }
        });
        let settings_subscription = glim_settings::handle(cx).map(|state| {
            cx.subscribe_in(
                &state,
                window,
                |this, state, event: &SettingsEvent, window, cx| {
                    this.on_settings_changed(state, *event, window, cx);
                },
            )
        });

        let mut workspace = Self {
            theme_preference,
            applied_settings: settings,
            _settings_subscription: settings_subscription,
            tabs: Vec::new(),
            active_tab: TabId(0),
            panes: HashMap::new(),
            tab_scroll: ScrollHandle::new(),
            dragging_split: None,
            split_interaction: None,
            split_focus: cx.focus_handle(),
            layout: None,
            layout_focus: cx.focus_handle(),
            pointer_over_layout: false,
            pane_drag: None,
            exit_confirmed: false,
            empty_focus: cx.focus_handle(),
            next_tab_id: 1,
            next_pane_id: 1,
            next_split_id: 1,
            _appearance_observer: appearance_observer,
        };

        match workspace_state::take_restore(cx) {
            Some(state) => workspace.restore_state(state, window, cx),
            None => {
                // 首次启动打开一个真实终端。
                let tab = workspace.new_tab();
                let pane = workspace.new_pane(None, None, window, cx);
                workspace.active_tab = tab;
                if let Some(entry) = workspace.tab_mut(tab) {
                    entry.tree = Some(PaneTree::leaf(pane));
                    entry.active = Some(pane);
                }
                workspace.focus_pane(pane, window, cx);
            }
        }

        // 窗口关闭：运行中的会话汇总为一次确认；放行前保存工作区。
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let Some(workspace) = weak.upgrade() else {
                return true;
            };
            if workspace.read(cx).exit_confirmed {
                workspace.update(cx, |workspace, cx| workspace.save_state(window, cx));
                return true;
            }
            let busy = workspace
                .read(cx)
                .panes
                .values()
                .filter(|entry| entry.view.read(cx).has_running_task())
                .count();
            if busy == 0 {
                workspace.update(cx, |workspace, cx| workspace.save_state(window, cx));
                return true;
            }
            workspace.update(cx, |workspace, cx| {
                workspace.open_exit_confirm(busy, window, cx);
            });
            false
        });

        workspace
    }

    /// 设置变化：应用主题与字体，保存失败时通知。
    fn on_settings_changed(
        &mut self,
        state: &Entity<SettingsState>,
        event: SettingsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = state.read(cx).current().clone();
        let appearance_changed = next.theme() != self.applied_settings.theme()
            || next.font_family() != self.applied_settings.font_family()
            || next.font_size() != self.applied_settings.font_size();
        if appearance_changed {
            let warning = glim_ui::apply_appearance(
                next.theme().to_preference(),
                next.font_family(),
                next.font_size(),
                Some(window),
                cx,
            );
            if let Some(warning) = warning {
                window.push_notification(Notification::warning(warning).title("设置"), cx);
            }
        }
        self.theme_preference = next.theme().to_preference();
        self.applied_settings = next;
        if let SettingsEvent::Changed { saved: false } = event {
            let message = state
                .read(cx)
                .save_error()
                .unwrap_or("设置未保存")
                .to_owned();
            window.push_notification(Notification::error(message).title("设置"), cx);
        }
        cx.notify();
    }

    /// 恢复上次会话：为每个窗格启动新 shell，目录缺失时回退到用户目录。
    fn restore_state(
        &mut self,
        state: WorkspaceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut missing_directories = 0usize;
        for tab_state in &state.tabs {
            let tab = self.new_tab();
            if let Some(node) = &tab_state.tree {
                let leaves = workspace_state::leaves(node);
                let mut panes = Vec::with_capacity(leaves.len());
                for leaf in &leaves {
                    let (directory, fell_back) =
                        workspace_state::resolve_directory(leaf.directory.as_deref());
                    if fell_back {
                        missing_directories += 1;
                    }
                    panes.push(self.new_pane(directory, leaf.shell.clone(), window, cx));
                }
                let mut pending = panes.into_iter();
                let tree = workspace_state::build_tree(node, &mut pending, &mut || {
                    let id = SplitId(self.next_split_id);
                    self.next_split_id += 1;
                    id
                });
                if let Some(tree) = tree {
                    let active_pane = tab_state
                        .active_pane
                        .and_then(|index| tree.leaves().get(index).copied())
                        .or_else(|| tree.leaves().first().copied());
                    if let Some(entry) = self.tab_mut(tab) {
                        entry.tree = Some(tree);
                        entry.active = active_pane;
                    }
                }
            }
        }

        if let Some(tab) = self.tabs.get(state.active_tab) {
            self.active_tab = tab.id;
        }
        self.reveal_active_tab();
        if let Some(pane) = self.active_tab_entry().and_then(|tab| tab.active) {
            self.focus_pane(pane, window, cx);
        }
        if missing_directories > 0 {
            workspace_state::push_warning(
                cx,
                format!("{missing_directories} 个窗格的目录不存在，已回退到用户目录"),
            );
        }
        cx.notify();
    }

    /// 保存工作区：关闭窗口或确认退出时调用；失败不阻塞退出。
    pub(crate) fn save_state(&self, window: &Window, cx: &App) {
        let Some(path) = workspace_state::workspace_file(cx) else {
            return;
        };
        let state = self.collect_state(window, cx);
        if let Err(error) = glim_settings::save_toml(&path, &state) {
            eprintln!("保存工作区失败：{error}");
        }
    }

    /// 收集当前布局、目录、shell 引用与窗口状态。
    fn collect_state(&self, window: &Window, cx: &App) -> WorkspaceState {
        let tabs = self
            .tabs
            .iter()
            .map(|tab| {
                let tree = tab.tree.as_ref().map(|tree| {
                    workspace_state::collect_tree(tree, &mut |pane| {
                        let Some(entry) = self.panes.get(&pane) else {
                            return LeafState::default();
                        };
                        let view = entry.view.read(cx);
                        LeafState {
                            directory: view
                                .working_directory()
                                .or_else(|| view.tracked_directory().map(Path::to_path_buf)),
                            shell: view.shell_program().map(str::to_owned),
                        }
                    })
                });
                let active_pane = tab.active.and_then(|pane| {
                    tab.tree.as_ref().and_then(|tree| {
                        tree.leaves()
                            .iter()
                            .position(|candidate| *candidate == pane)
                    })
                });
                TabState { tree, active_pane }
            })
            .collect();
        workspace_state::new_state(window, self.active_tab_index().unwrap_or(0), tabs)
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
    ///
    /// `shell` 是恢复数据里的 shell 引用；`None` 时使用设置里的默认
    /// shell（再缺省则按平台顺序查找）。
    fn new_pane(
        &mut self,
        directory: Option<PathBuf>,
        shell: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PaneId {
        let id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        let options = terminal_options(directory, shell, cx);
        let view = cx.new(|cx| TerminalView::with_options(options, window, cx));
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
        let pane = self.new_pane(directory, None, window, cx);
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
        let index = self.tabs.iter().position(|tab| tab.id == id)?;
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
        let new_pane = self.new_pane(directory, None, window, cx);
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
                Some(tree) => PaneTree::split(split, axis, 0.5, tree, PaneTree::leaf(new_pane)),
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
        // 焦点回到窗格：清除分隔线的键盘调整目标。
        self.split_interaction = None;
        if let Some(tab) = self.active_tab_entry_mut()
            && tab.tree.as_ref().is_some_and(|tree| tree.contains(pane))
        {
            tab.active = Some(pane);
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
        let pane = self.new_pane(None, None, window, cx);
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

    // ---- 分隔线 ----

    /// 开始与分隔线交互：记录拖动状态与键盘调整界限。
    ///
    /// 界限取点击时布局元素的像素区域；键盘调整与拖动使用同一套
    /// 最小尺寸约束。焦点移到分隔线区域，方向键调整只在此上下文中生效。
    pub(crate) fn begin_split_interaction(
        &mut self,
        split: SplitId,
        metrics: SplitMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dragging_split = Some(split);
        self.split_interaction = Some((split, metrics));
        window.focus(&self.split_focus, cx);
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

    /// 结束拖动；焦点与调整界限保留，键盘仍可继续调整。
    pub(crate) fn end_divider_drag(&mut self, cx: &mut Context<Self>) {
        if self.dragging_split.take().is_some() {
            cx.notify();
        }
    }

    /// 清除分隔线调整目标（点击窗格或区域空白处）。
    fn clear_split_interaction(&mut self, cx: &mut Context<Self>) {
        if self.split_interaction.take().is_some() {
            cx.notify();
        }
    }

    /// 键盘调整分隔线：方向键与分隔线方向一致时移动 5%，垂直方向忽略。
    fn nudge_split(&mut self, direction: Direction, cx: &mut Context<Self>) {
        let Some((id, metrics)) = self.split_interaction else {
            return;
        };
        let step = match (metrics.axis, direction) {
            (Axis::Horizontal, Direction::Left) | (Axis::Vertical, Direction::Up) => -SPLIT_STEP,
            (Axis::Horizontal, Direction::Right) | (Axis::Vertical, Direction::Down) => SPLIT_STEP,
            _ => return,
        };
        let Some(current) = self.split_ratio(id) else {
            return;
        };
        self.apply_split_target(id, metrics, current + step, cx);
    }

    /// 分隔线回到均分（`Home`）。
    fn reset_split(&mut self, cx: &mut Context<Self>) {
        let Some((id, metrics)) = self.split_interaction else {
            return;
        };
        self.apply_split_target(id, metrics, 0.5, cx);
    }

    /// 读取分屏节点的当前比例。
    fn split_ratio(&self, split: SplitId) -> Option<f32> {
        self.active_tab_entry()
            .and_then(|tab| tab.tree.as_ref())
            .and_then(|tree| tree.split_node(split))
            .map(|(_, ratio)| ratio)
    }

    /// 在交互记录的界限内把比例调整到目标值。
    fn apply_split_target(
        &mut self,
        split: SplitId,
        metrics: SplitMetrics,
        target: f32,
        cx: &mut Context<Self>,
    ) {
        let ratio = clamp_ratio(
            target,
            metrics.extent,
            metrics.min_first,
            metrics.min_second,
        );
        self.set_split_ratio(split, ratio, cx);
    }

    // ---- 布局面板 ----

    /// 布局面板是否打开。
    fn is_layout_open(&self) -> bool {
        self.layout.is_some()
    }

    /// 打开 / 关闭布局面板（`Alt+Z`、工具栏按钮）。
    fn toggle_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_layout_open() {
            self.close_layout(window, cx);
        } else {
            self.open_layout(None, cx);
            window.focus(&self.layout_focus, cx);
        }
    }

    /// 打开布局面板；`drag_source` 记录拖动来源窗格，`select_first` 选中首个方案。
    ///
    /// 焦点由调用方设置：键盘打开时聚焦面板，指针拖动时保持原焦点。
    fn open_layout(&mut self, drag_source: Option<PaneId>, cx: &mut Context<Self>) {
        let has_tree = self
            .active_tab_entry()
            .is_some_and(|tab| tab.tree.is_some());
        if !has_tree {
            return;
        }
        match &mut self.layout {
            Some(panel) => {
                panel.drag_source = drag_source;
            }
            None => {
                self.layout = Some(LayoutPanel {
                    // 打开时选中第一个方案并立即预览；指针悬停会覆盖它。
                    selected: Some(0),
                    hovered: None,
                    drag_source,
                });
                cx.notify();
            }
        }
    }

    /// 关闭布局面板；面板持有焦点时把焦点还给活动窗格。
    fn close_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layout.take().is_none() {
            return;
        }
        if self.layout_focus.is_focused(window)
            && let Some(pane) = self.active_tab_entry().and_then(|tab| tab.active)
        {
            self.focus_pane(pane, window, cx);
        }
        cx.notify();
    }

    /// 结束拖动打开的面板；键盘或按钮打开的面板保持打开。
    fn close_layout_from_drag(&mut self, cx: &mut Context<Self>) {
        if self
            .layout
            .as_ref()
            .is_some_and(|panel| panel.drag_source.is_some())
        {
            self.layout = None;
            cx.notify();
        }
    }

    /// 在方案列表中移动键盘选择，同时更新预览。
    fn move_layout_selection(&mut self, target: LayoutSelection, cx: &mut Context<Self>) {
        let count = self.layout_pane_count();
        let Some(panel) = &mut self.layout else {
            return;
        };
        if count == 0 {
            return;
        }
        let last = templates_for(count).len().saturating_sub(1);
        let next = match target {
            LayoutSelection::Previous => panel.selected.map(|index| index.saturating_sub(1)),
            LayoutSelection::Next => panel.selected.map(|index| (index + 1).min(last)),
            LayoutSelection::First => Some(0),
            LayoutSelection::Last => Some(last),
        };
        if panel.selected != next {
            panel.selected = next;
            cx.notify();
        }
    }

    /// 指针悬停某个方案；`None` 清除悬停预览。
    fn hover_layout_option(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let Some(panel) = &mut self.layout else {
            return;
        };
        if panel.hovered != index {
            panel.hovered = index;
            cx.notify();
        }
    }

    /// 记录指针是否位于布局面板上；拖动经过面板时不关闭面板。
    fn set_layout_pointer(&mut self, over: bool, cx: &mut Context<Self>) {
        if self.pointer_over_layout != over {
            self.pointer_over_layout = over;
            cx.notify();
        }
    }

    /// 应用方案：主槽位是拖动来源或活动窗格，其余按原有顺序填入。
    fn apply_layout(
        &mut self,
        index: usize,
        source: Option<PaneId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((panes, active)) = self.ordered_panes(source) else {
            return;
        };
        let templates = templates_for(panes.len());
        let Some(template) = templates.get(index) else {
            return;
        };
        let mut next = self.next_split_id;
        let tree = template.build(&panes, &mut || {
            let id = SplitId(next);
            next += 1;
            id
        });
        self.next_split_id = next;

        let Some(tab_index) = self.active_tab_index() else {
            return;
        };
        let tab = &mut self.tabs[tab_index];
        tab.tree = Some(tree);
        tab.active = Some(active);
        tab.zoomed = None;
        self.layout = None;
        self.focus_pane(active, window, cx);
        cx.notify();
    }

    /// 当前标签页的窗格顺序：主槽位在前（拖动来源或活动窗格），其余按树序。
    fn ordered_panes(&self, source: Option<PaneId>) -> Option<(Vec<PaneId>, PaneId)> {
        let tab = self.active_tab_entry()?;
        let tree = tab.tree.as_ref()?;
        let main = source.or(tab.active).filter(|pane| tree.contains(*pane))?;
        let mut panes = vec![main];
        panes.extend(tree.leaves().into_iter().filter(|pane| *pane != main));
        Some((panes, main))
    }

    /// 布局方案的窗格数量；没有会话时为 0。
    fn layout_pane_count(&self) -> usize {
        self.active_tab_entry()
            .and_then(|tab| tab.tree.as_ref())
            .map(|tree| tree.leaves().len())
            .unwrap_or(0)
    }

    /// 当前应该展示的预览布局；形状与现有布局一致时为 `None`。
    fn preview_tree(&self) -> Option<(PaneTree, PaneId)> {
        let panel = self.layout.as_ref()?;
        let index = panel.preview()?;
        let (panes, main) = self.ordered_panes(panel.drag_source)?;
        let current = self.active_tab_entry()?.tree.as_ref()?;
        let template = templates_for(panes.len()).get(index)?.clone();
        let mut next = 0_u64;
        let tree = template.build(&panes, &mut || {
            let id = SplitId(next);
            next += 1;
            id
        });
        if tree.same_shape(current) {
            return None;
        }
        Some((tree, main))
    }

    // ---- 窗格标题拖动 ----

    /// 记录标题上的按下；越过阈值后才成为拖动。
    fn begin_pane_press(&mut self, pane: PaneId, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.pane_drag = Some(PaneDrag {
            pane,
            origin: position,
            position,
            moved: false,
            swap_target: None,
            hot: false,
        });
        cx.notify();
    }

    /// 是否有待决或进行中的窗格拖动。
    pub(crate) fn has_pane_drag(&self) -> bool {
        self.pane_drag.is_some()
    }

    /// 拖动中更新指针位置：判定热区与交换目标。
    ///
    /// `area` 是窗格区域的像素边界，`hot_zone_max` 是热区的高度上限；
    /// 分屏几何按比例换算到该区域。
    pub(crate) fn update_pane_drag(
        &mut self,
        position: Point<Pixels>,
        area: Bounds<Pixels>,
        hot_zone_max: Pixels,
        cx: &mut Context<Self>,
    ) {
        let Some((pane, moved)) = self.pane_drag.as_ref().map(|drag| {
            (
                drag.pane,
                drag.moved
                    || (position - drag.origin).magnitude() >= f64::from(PANE_DRAG_THRESHOLD),
            )
        }) else {
            return;
        };
        if let Some(drag) = &mut self.pane_drag {
            drag.position = position;
            if !moved {
                // 尚未越过阈值：只记录位置，不进入拖动状态。
                return;
            }
            drag.moved = true;
        }

        let hot_zone =
            (f32::from(area.size.height) * LAYOUT_HOT_ZONE_FRACTION).min(f32::from(hot_zone_max));
        let inside = position.x >= area.origin.x
            && position.x <= area.origin.x + area.size.width
            && position.y >= area.origin.y
            && position.y <= area.origin.y + area.size.height;
        // 指针位于面板上时保持热区：面板底部可能超出热区高度。
        let hot = self.pointer_over_layout
            || (inside && f32::from(position.y - area.origin.y) < hot_zone);

        let mut swap_target = None;
        if inside
            && !hot
            && let Some(tree) = self.active_tab_entry().and_then(|tab| tab.tree.as_ref())
        {
            let geometry = tree.geometry();
            for (candidate, rect) in &geometry.panes {
                if *candidate == pane {
                    continue;
                }
                let left = area.origin.x + area.size.width * rect.x;
                let top = area.origin.y + area.size.height * rect.y;
                let right = left + area.size.width * rect.width;
                let bottom = top + area.size.height * rect.height;
                if position.x >= left
                    && position.x < right
                    && position.y >= top
                    && position.y < bottom
                {
                    swap_target = Some(*candidate);
                    break;
                }
            }
        }

        if let Some(drag) = &mut self.pane_drag {
            drag.hot = hot;
            drag.swap_target = swap_target;
        }
        if hot {
            self.open_layout(Some(pane), cx);
        } else {
            self.close_layout_from_drag(cx);
        }
        cx.notify();
    }

    /// 结束拖动：交换窗格、应用悬停的方案，或保持原样。
    fn end_pane_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.pane_drag.take() else {
            return;
        };
        if !drag.moved {
            // 只是点击标题：仅更新活动窗格。
            self.activate_pane(drag.pane, window, cx);
            return;
        }

        if drag.hot {
            // 释放在热区：悬停在方案卡片上时应用它；否则只关闭拖动打开的面板。
            if let Some(index) = self.layout.as_ref().and_then(|panel| panel.hovered) {
                self.apply_layout(index, Some(drag.pane), window, cx);
                return;
            }
            self.close_layout_from_drag(cx);
        } else if let Some(target) = drag.swap_target {
            if let Some(tab) = self.active_tab_entry_mut()
                && let Some(tree) = tab.tree.as_mut()
                && tree.swap(drag.pane, target)
            {
                tab.active = Some(drag.pane);
            }
            self.close_layout_from_drag(cx);
            self.focus_pane(drag.pane, window, cx);
        }
        cx.notify();
    }

    /// 取消拖动：不改变布局，关闭由拖动打开的面板。
    fn cancel_pane_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(drag) = self.pane_drag.take()
            && drag.moved
        {
            self.close_layout_from_drag(cx);
            cx.notify();
        }
    }

    /// 键盘交换：与指定方向上最近的窗格交换位置（`Ctrl+Shift+方向键`）。
    fn swap_direction(
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
        if let Some(tree) = tab.tree.as_mut() {
            tree.swap(active, target);
        }
        // 焦点跟随窗格身份：交换后仍停留在原来的窗格会话上。
        self.focus_pane(active, window, cx);
        cx.notify();
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
                .on_ok(move |_, window, cx| {
                    if let Some(workspace) = weak.upgrade() {
                        workspace.update(cx, |workspace, cx| {
                            workspace.exit_confirmed = true;
                            workspace.save_state(window, cx);
                        });
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
                                .accessibility_label("关闭标签页")
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
                    .accessibility_label("新建标签页")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_tab(window, cx);
                    })),
            )
            .child(div().flex_1())
            .child(self.render_split_button())
            .child(self.render_layout_button(cx))
            .child(
                Button::new("open-settings")
                    .icon(IconName::Settings)
                    .ghost()
                    .tooltip("设置 · Ctrl+,")
                    .accessibility_label("设置")
                    .on_click(|_, window, cx| {
                        glim_settings::open_settings_dialog(window, cx);
                    }),
            )
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

    /// 顶部的布局入口：打开排列窗格面板，打开期间保持按下态。
    fn render_layout_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Toggle::new("layout-toggle")
            .ghost()
            .checked(self.is_layout_open())
            .icon(Icon::new(IconName::LayoutTemplate))
            .label("布局…")
            .tooltip("排列窗格 · Alt+Z")
            .on_click(cx.listener(|this, checked: &bool, window, cx| {
                if *checked {
                    this.open_layout(None, cx);
                    window.focus(&this.layout_focus, cx);
                } else {
                    this.close_layout(window, cx);
                }
            }))
            .into_any_element()
    }

    /// 当前标签页的窗格区域：正常分屏、放大单窗格、布局预览或空状态。
    fn render_panes(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(tab) = self.active_tab_entry() else {
            return div().into_any_element();
        };
        let Some(tree) = tab.tree.as_ref() else {
            return self.render_empty_tab(window, cx).into_any_element();
        };

        // 布局预览：只渲染占位，不触碰终端视图与 PTY 尺寸。
        if let Some((preview, main)) = self.preview_tree() {
            return self.render_preview(&preview, main, cx);
        }

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
        let dividers = !self.has_pane_drag();
        let options = SplitLayoutOptions {
            dividers,
            focused_split: self.split_interaction.map(|(split, _)| split),
        };
        build_split_layout(tree, &mut panes, cx.entity(), &options)
    }

    /// 布局预览：维护窗格身份但只展示占位内容。
    fn render_preview(&self, tree: &PaneTree, main: PaneId, cx: &mut Context<Self>) -> AnyElement {
        let mut panes = HashMap::new();
        for pane in tree.leaves() {
            panes.insert(pane, self.render_preview_pane(pane, pane == main, cx));
        }
        let options = SplitLayoutOptions {
            dividers: false,
            focused_split: None,
        };
        build_split_layout(tree, &mut panes, cx.entity(), &options)
    }

    /// 预览中的窗格占位：保留标题，终端内容以占位块表示。
    fn render_preview_pane(&self, pane: PaneId, main: bool, cx: &mut Context<Self>) -> AnyElement {
        let title = self
            .panes
            .get(&pane)
            .map(|entry| pane_heading(entry.view.read(cx)))
            .unwrap_or_default();

        v_flex()
            .id(("pane-preview", pane.value()))
            .test_support()
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().background)
            .when(main, |this| {
                this.border_1().border_color(cx.theme().primary)
            })
            .child(
                h_flex()
                    .h_8()
                    .px_2()
                    .gap_1()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(if main {
                        cx.theme().selection
                    } else {
                        cx.theme().title_bar
                    })
                    .child(Icon::new(IconName::SquareTerminal).size_3p5())
                    .child(div().flex_1().min_w_0().truncate().text_sm().child(title)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .bg(cx.theme().muted)
                    .child(
                        Icon::new(IconName::LayoutTemplate)
                            .size_5()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("布局预览"),
                    ),
            )
            .into_any_element()
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

        let dragging = self.pane_drag.as_ref().filter(|drag| drag.moved);
        let swapped = dragging.is_some_and(|drag| drag.swap_target == Some(pane));
        let lifted = dragging.is_some_and(|drag| drag.pane == pane);

        // 标题拖动区：排除操作按钮，按下并越过阈值后才成为拖动。
        let title_region = h_flex()
            .id(("pane-drag", pane.value()))
            .test_support()
            .flex_1()
            .min_w_0()
            .gap_1()
            .items_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    this.begin_pane_press(pane, event.position, cx);
                }),
            )
            .child(Icon::new(IconName::SquareTerminal).size_3p5())
            .child(div().flex_1().min_w_0().truncate().text_sm().child(title));

        let header = h_flex()
            .h_8()
            .px_2()
            .gap_1()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(if swapped {
                cx.theme().primary.alpha(0.12)
            } else if active {
                cx.theme().title_bar
            } else {
                cx.theme().background
            })
            .text_color(if active || swapped {
                cx.theme().foreground
            } else {
                cx.theme().muted_foreground
            })
            .child(title_region)
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
                    .accessibility_label("分屏")
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
                    .accessibility_label(if zoomed {
                        "恢复分屏"
                    } else {
                        "聚焦窗格"
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
                        .accessibility_label("重新启动")
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
                    .accessibility_label("关闭窗格")
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
                .relative()
                .when(lifted, |this| this.opacity(0.45))
                .when(swapped, |this| {
                    this.border_1().border_color(cx.theme().primary)
                })
                .bg(cx.theme().background)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.activate_pane(pane, window, cx);
                    }),
                )
                .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _window, cx| {
                    // 指针回到窗格上：清除方案卡片上的悬停预览。
                    this.hover_layout_option(None, cx);
                    this.set_layout_pointer(false, cx);
                }))
                .child(header)
                .child(div().flex_1().min_h_0().child(view))
                .when(swapped, |this| {
                    this.child(
                        v_flex()
                            .absolute()
                            .inset_0()
                            .items_center()
                            .justify_center()
                            .child(
                                h_flex()
                                    .gap_1()
                                    .px_3()
                                    .py_2()
                                    .rounded(cx.theme().radius)
                                    .border_1()
                                    .border_color(cx.theme().primary)
                                    .bg(cx.theme().popover)
                                    .text_xs()
                                    .text_color(cx.theme().primary)
                                    .child(Icon::new(IconName::ArrowLeftRight).size_3p5())
                                    .child("松手交换位置"),
                            ),
                    )
                })
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
        let count_text = if self.preview_tree().is_some() {
            format!("{pane_count} 个窗格 · 预览中")
        } else if self.zoomed_pane().is_some() {
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

    // ---- 布局面板与拖动反馈 ----

    /// 布局面板：顶部推荐方案的浮层；悬停或键盘选择时预览。
    fn render_layout_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(panel) = &self.layout else {
            return div().into_any_element();
        };
        let Some((panes, main)) = self.ordered_panes(panel.drag_source) else {
            return div().into_any_element();
        };
        let templates = templates_for(panes.len());

        let cards = templates
            .iter()
            .enumerate()
            .map(|(index, template)| self.render_layout_card(index, template, &panes, main, cx))
            .collect::<Vec<_>>();

        let escape = Kbd::binding_for_action(&CancelLayout, Some("LayoutPanel"), window);
        let enter = Kbd::binding_for_action(&ApplyLayout, Some("LayoutPanel"), window);

        v_flex()
            .id("layout-panel")
            .test_support()
            .track_focus(&self.layout_focus)
            .key_context("LayoutPanel")
            .on_action(cx.listener(Self::cancel_layout_action))
            .on_action(cx.listener(Self::apply_layout_action))
            .on_action(cx.listener(Self::previous_layout_action))
            .on_action(cx.listener(Self::next_layout_action))
            .on_action(cx.listener(Self::first_layout_action))
            .on_action(cx.listener(Self::last_layout_action))
            .absolute()
            .top_3()
            .right_3()
            .occlude()
            .popover_style(cx)
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _window, cx| {
                this.set_layout_pointer(true, cx);
            }))
            .child(
                h_flex()
                    .px_3()
                    .pt_3()
                    .pb_2()
                    .gap_2()
                    .items_center()
                    .child(div().text_sm().child("排列窗格"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{} 个窗格", panes.len())),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("layout-close")
                            .icon(IconName::X)
                            .ghost()
                            .xsmall()
                            .tooltip("关闭 · Esc")
                            .accessibility_label("关闭布局面板")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.close_layout(window, cx);
                            })),
                    ),
            )
            .child(h_flex().gap_2().px_3().pb_3().children(cards))
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child("悬停预览，点击应用")
                    .child(div().flex_1())
                    .when_some(enter, |this, kbd| this.child(kbd).child("应用"))
                    .when_some(escape, |this, kbd| this.child(kbd).child("取消")),
            )
            .into_any_element()
    }

    /// 一个布局方案：缩略图、名称与预览 / 已应用状态。
    fn render_layout_card(
        &self,
        index: usize,
        template: &LayoutTemplate,
        panes: &[PaneId],
        main: PaneId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // 缩略图需要方案的几何；构建时使用局部身份，不占用真实身份。
        let mut next = 0_u64;
        let tree = template.build(panes, &mut || {
            let id = SplitId(next);
            next += 1;
            id
        });
        let applied = self
            .active_tab_entry()
            .and_then(|tab| tab.tree.as_ref())
            .is_some_and(|current| tree.same_shape(current));
        let (previewed, selected) = self
            .layout
            .as_ref()
            .map(|panel| {
                (
                    panel.preview() == Some(index),
                    panel.selected == Some(index),
                )
            })
            .unwrap_or((false, false));

        let theme = cx.theme();
        let cells = tree
            .geometry()
            .panes
            .into_iter()
            .map(|(pane, rect)| {
                let primary = pane == main;
                div()
                    .absolute()
                    .left(relative(rect.x))
                    .top(relative(rect.y))
                    .w(relative(rect.width))
                    .h(relative(rect.height))
                    .child(
                        // 1px 缩略图内缝：保证相邻窗格之间有可见边界。
                        div()
                            .m(rems(0.0625))
                            .w_full()
                            .h_full()
                            .rounded_sm()
                            .border_1()
                            .border_color(if primary { theme.primary } else { theme.border })
                            .bg(if primary {
                                theme.selection
                            } else {
                                theme.muted
                            }),
                    )
            })
            .collect::<Vec<_>>();

        let border = if previewed || selected {
            theme.primary
        } else {
            theme.border
        };
        let background = if applied && !previewed {
            theme.selection
        } else {
            theme.muted
        };

        Button::new(("layout-option", index))
            .ghost()
            .xsmall()
            .w_20()
            // 卡片高度取内容需要的最小固定值（缩略图 + 名称 + 内边距）。
            .h_20()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .p_2()
            .rounded(theme.radius)
            .border_1()
            .border_color(border)
            .bg(background)
            .accessibility_label(format!("应用{}布局", template.name()))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.apply_layout(index, None, window, cx);
            }))
            .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _window, cx| {
                this.hover_layout_option(Some(index), cx);
            }))
            .on_hover(cx.listener(move |this, hovered: &bool, _window, cx| {
                if !*hovered {
                    this.hover_layout_option(None, cx);
                }
            }))
            .child(div().relative().w_full().h_10().children(cells))
            .child(div().text_xs().child(template.name().to_owned()))
            .into_any_element()
    }

    /// 顶部布局热区提示：拖动标题且不在热区时显示。
    fn render_drop_hint(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .absolute()
            .top_2()
            .left_0()
            .right_0()
            .justify_center()
            .child(
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_dashed()
                    .border_color(cx.theme().primary)
                    .bg(cx.theme().popover)
                    .text_xs()
                    .text_color(cx.theme().primary)
                    .child(Icon::new(IconName::LayoutTemplate).size_3p5())
                    .child("拖到这里，重新排列窗格"),
            )
            .into_any_element()
    }

    /// 拖动浮标：跟随指针的窗格标题。
    fn render_drag_ghost(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(drag) = self.pane_drag.as_ref().filter(|drag| drag.moved) else {
            return div().into_any_element();
        };
        let title = self
            .panes
            .get(&drag.pane)
            .map(|entry| pane_heading(entry.view.read(cx)))
            .unwrap_or_default();

        h_flex()
            .absolute()
            .left(drag.position.x)
            .top(drag.position.y)
            .ml_3()
            .mt_3()
            .gap_1()
            .px_3()
            .py_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().primary)
            .bg(cx.theme().popover)
            .text_xs()
            .text_color(cx.theme().foreground)
            .child(Icon::new(IconName::GripVertical).size_3p5())
            .child(title)
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
        // 有设置服务时经设置写入并持久化；订阅统一应用主题与字体。
        let persisted = glim_settings::handle(cx)
            .map(|state| {
                let theme = glim_settings::ThemeSetting::from(*preference);
                state.update(cx, |state, cx| state.set_theme(theme, cx));
            })
            .is_some();
        if !persisted {
            self.theme_preference = *preference;
            glim_ui::apply_theme_preference(*preference, Some(window), cx);
        }
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

    fn swap_left_action(&mut self, _: &SwapPaneLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.swap_direction(Direction::Left, window, cx);
    }

    fn swap_right_action(
        &mut self,
        _: &SwapPaneRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.swap_direction(Direction::Right, window, cx);
    }

    fn swap_up_action(&mut self, _: &SwapPaneUp, window: &mut Window, cx: &mut Context<Self>) {
        self.swap_direction(Direction::Up, window, cx);
    }

    fn swap_down_action(&mut self, _: &SwapPaneDown, window: &mut Window, cx: &mut Context<Self>) {
        self.swap_direction(Direction::Down, window, cx);
    }

    fn open_layout_action(&mut self, _: &OpenLayout, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_layout(window, cx);
    }

    fn open_settings_action(
        &mut self,
        _: &glim_settings::OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        glim_settings::open_settings_dialog(window, cx);
    }

    fn apply_layout_action(
        &mut self,
        _: &ApplyLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.layout.as_ref().and_then(|panel| panel.preview()) else {
            cx.propagate();
            return;
        };
        self.apply_layout(index, None, window, cx);
    }

    fn cancel_layout_action(
        &mut self,
        _: &CancelLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pane_drag.is_some() {
            // 拖动中的指针取消：保持原布局。
            self.cancel_pane_drag(cx);
            return;
        }
        if self.layout.is_some() {
            self.close_layout(window, cx);
            return;
        }
        // 没有可取消的操作：让按键继续传递（如终端里的 Escape）。
        cx.propagate();
    }

    fn previous_layout_action(
        &mut self,
        _: &PreviousLayout,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_layout_selection(LayoutSelection::Previous, cx);
    }

    fn next_layout_action(&mut self, _: &NextLayout, _window: &mut Window, cx: &mut Context<Self>) {
        self.move_layout_selection(LayoutSelection::Next, cx);
    }

    fn first_layout_action(
        &mut self,
        _: &FirstLayout,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_layout_selection(LayoutSelection::First, cx);
    }

    fn last_layout_action(&mut self, _: &LastLayout, _window: &mut Window, cx: &mut Context<Self>) {
        self.move_layout_selection(LayoutSelection::Last, cx);
    }

    fn nudge_left_action(
        &mut self,
        _: &NudgeSplitLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_split(Direction::Left, cx);
    }

    fn nudge_right_action(
        &mut self,
        _: &NudgeSplitRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_split(Direction::Right, cx);
    }

    fn nudge_up_action(&mut self, _: &NudgeSplitUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.nudge_split(Direction::Up, cx);
    }

    fn nudge_down_action(
        &mut self,
        _: &NudgeSplitDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_split(Direction::Down, cx);
    }

    fn reset_split_action(
        &mut self,
        _: &ResetSplitRatio,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reset_split(cx);
    }
}

/// 窗格的展示名称：会话标题优先，否则使用 shell 名称。
/// 组装会话选项：设置提供滚动历史、默认 shell 与环境变量；
/// 目录与 shell 引用来自调用方（分屏继承或恢复数据）。
fn terminal_options(
    directory: Option<PathBuf>,
    shell: Option<String>,
    cx: &App,
) -> TerminalOptions {
    let settings = glim_settings::current(cx);
    let mut options = TerminalOptions::default().with_scrollback(settings.scrollback());

    // shell 配置引用按语义解析：当前配置优先（默认 shell 变更作用于
    // 新会话），配置缺省时沿用恢复数据里的引用，再退回平台默认。
    let spec = settings
        .shell()
        .map(settings_shell_spec)
        .or_else(|| shell.map(ShellSpec::new));
    if let Some(spec) = spec {
        options = options.with_shell(spec);
    }
    if let Some(shell) = settings.shell() {
        for (key, value) in shell.env() {
            options = options.with_env(key.clone(), value.clone());
        }
    }
    if let Some(directory) = directory {
        options = options.with_working_directory(directory);
    }
    options
}

/// 设置里的 shell 配置转为会话参数。
fn settings_shell_spec(shell: &glim_settings::ShellConfig) -> ShellSpec {
    ShellSpec::new(shell.program()).with_args(shell.args().iter().cloned())
}

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
        let dragging = self.pane_drag.as_ref().is_some_and(|drag| drag.moved);
        let hint = self
            .pane_drag
            .as_ref()
            .is_some_and(|drag| drag.moved && !drag.hot && drag.swap_target.is_none());

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
            .on_action(cx.listener(Self::swap_left_action))
            .on_action(cx.listener(Self::swap_right_action))
            .on_action(cx.listener(Self::swap_up_action))
            .on_action(cx.listener(Self::swap_down_action))
            .on_action(cx.listener(Self::open_layout_action))
            .on_action(cx.listener(Self::open_settings_action))
            .on_action(cx.listener(Self::apply_layout_action))
            .on_action(cx.listener(Self::cancel_layout_action))
            .on_action(cx.listener(Self::previous_layout_action))
            .on_action(cx.listener(Self::next_layout_action))
            .on_action(cx.listener(Self::first_layout_action))
            .on_action(cx.listener(Self::last_layout_action))
            .on_action(cx.listener(Self::nudge_left_action))
            .on_action(cx.listener(Self::nudge_right_action))
            .on_action(cx.listener(Self::nudge_up_action))
            .on_action(cx.listener(Self::nudge_down_action))
            .on_action(cx.listener(Self::reset_split_action))
            .child(self.render_tab_bar(cx))
            .child(
                // 分隔线焦点区域：只聚焦分隔线时（`SplitDivider` 上下文）
                // 方向键调整生效；终端持有焦点时不受影响。
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .relative()
                    .track_focus(&self.split_focus)
                    .key_context(if self.split_focus.is_focused(window) {
                        "SplitDivider"
                    } else {
                        "PaneArea"
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                            // 区域空白处的点击：清除分隔线调整目标。
                            this.clear_split_interaction(cx);
                        }),
                    )
                    .child(PaneDragLayer {
                        workspace: cx.entity(),
                    })
                    .child(self.render_panes(window, cx))
                    .when(hint, |this| this.child(self.render_drop_hint(cx)))
                    .when(self.is_layout_open(), |this| {
                        this.child(
                            // 面板外的点击关闭面板：与浮层的关闭约定一致。
                            div()
                                .id("layout-backdrop")
                                .absolute()
                                .inset_0()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                        this.close_layout(window, cx);
                                    }),
                                )
                                .on_mouse_move(cx.listener(
                                    |this, _: &MouseMoveEvent, _window, cx| {
                                        this.set_layout_pointer(false, cx);
                                    },
                                )),
                        )
                    })
                    .child(self.render_layout_panel(window, cx)),
            )
            .child(self.render_status_bar(cx))
            .when(dragging, |this| this.child(self.render_drag_ghost(cx)))
    }
}

/// 窗格拖动层：覆盖窗格区域，跟踪拖动中的指针。
///
/// 指针位置要结合窗格区域的实际像素边界与分屏几何才能确定交换目标，
/// 因此在 prepaint 拿到边界后注册窗口级监听；监听不改变布局，
/// 只把指针状态上报工作区。
struct PaneDragLayer {
    workspace: Entity<WorkspaceView>,
}

impl Element for PaneDragLayer {
    type RequestLayoutState = ();
    type PrepaintState = Bounds<Pixels>;

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::from("pane-drag-layer"))
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
        // 监听层不参与布局：绝对定位铺满窗格区域，不挤压分屏元素。
        let style = Style {
            position: Position::Absolute,
            inset: Edges {
                top: Length::Definite(relative(0.0)),
                right: Length::Definite(relative(0.0)),
                bottom: Length::Definite(relative(0.0)),
                left: Length::Definite(relative(0.0)),
            },
            ..Style::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        bounds
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        area: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let area = *area;
        let hot_zone_max = rems(LAYOUT_HOT_ZONE_REMS).to_pixels(window.rem_size());
        let workspace = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, _phase, _window, cx| {
            if workspace.read(cx).has_pane_drag() {
                workspace.update(cx, |workspace, cx| {
                    workspace.update_pane_drag(event.position, area, hot_zone_max, cx);
                });
            }
        });

        let workspace = self.workspace.clone();
        window.on_mouse_event(move |_event: &MouseUpEvent, _phase, window, cx| {
            if workspace.read(cx).has_pane_drag() {
                workspace.update(cx, |workspace, cx| {
                    workspace.end_pane_drag(window, cx);
                });
            }
        });
    }
}

impl IntoElement for PaneDragLayer {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[cfg(test)]
mod tests;
