//! 工作区界面集成测试：真实会话下的标签页、分屏、焦点与关闭流程。
//!
//! 通过键盘与指针事件驱动真实界面，再读取工作区与终端的内部状态。
//! 会话是真实 PTY（Windows PowerShell），子进程相关的用例使用真实时间
//! 与明确超时，其余用例只依赖同步的布局与动作。

use std::time::{Duration, Instant};

use gpui_base::test_support::{self as observation, ElementSnapshot};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Root;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Entity, InputEvent as _, KeyDownEvent, KeyUpEvent,
    Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    TestAppContext, VisualTestContext, Window, point, px, size,
};

use super::WorkspaceView;
use crate::pane_tree::{Axis, PaneId, PaneTree, TabId};

/// 当前分屏的根节点方向与比例。
fn root_split(workspace: &Entity<WorkspaceView>, cx: &App) -> (Axis, f32) {
    match active_tree(workspace, cx) {
        PaneTree::Split { axis, ratio, .. } => (axis, ratio),
        PaneTree::Leaf(_) => panic!("根节点应为分屏"),
    }
}

/// 活动会话的实体身份；用于断言布局操作不重建会话。
fn session_id(workspace: &Entity<WorkspaceView>, pane: PaneId, cx: &App) -> gpui_kit::EntityId {
    pane_session(workspace, pane, cx).entity_id()
}

/// 打开测试窗口：安装组件与产品主题，启动一个真实终端。
///
/// 会话是真实 PTY：输出在后台线程产生，允许调度器进行非确定性等待。
fn open_workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<WorkspaceView>) {
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    cx.update(glim_ui::init);
    let mut workspace = None;
    let handle = cx.open_window(size(px(1000.), px(700.)), |window, cx| {
        let view = cx.new(|cx| WorkspaceView::new(window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    (handle.into(), workspace.expect("workspace constructed"))
}

// ---- 界面操作（等价于 gpui-kit 测试工具的底层实现） ----

fn render_frame(window: &mut Window, cx: &mut App) {
    window.refresh();
    window.draw(cx).clear(cx);
}

fn find(window: &Window, id: impl Into<gpui_kit::ElementId>) -> ElementSnapshot {
    let id = id.into();
    observation::find(window, &[], &id).unwrap_or_else(|| {
        panic!(
            "missing ElementId {id:?}. Registered paths: {}",
            observation::registered_paths(window)
        )
    })
}

/// 元素是否存在于当前帧且可见；用于断言面板与预览的开关。
fn is_visible(window: &Window, id: impl Into<gpui_kit::ElementId>) -> bool {
    let id = id.into();
    observation::find(window, &[], &id).is_some_and(|snapshot| snapshot.visible())
}

fn move_pointer(
    window: &mut Window,
    position: Point<Pixels>,
    pressed_button: Option<MouseButton>,
    cx: &mut App,
) {
    window.dispatch_event(
        MouseMoveEvent {
            position,
            pressed_button,
            modifiers: Default::default(),
        }
        .to_platform_input(),
        cx,
    );
    render_frame(window, cx);
}

fn mouse_button(
    window: &mut Window,
    position: Point<Pixels>,
    button: MouseButton,
    down: bool,
    cx: &mut App,
) {
    if down {
        window.dispatch_event(
            MouseDownEvent {
                button,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
    } else {
        window.dispatch_event(
            MouseUpEvent {
                button,
                position,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
    }
    render_frame(window, cx);
}

fn click(window: &mut Window, id: impl Into<gpui_kit::ElementId>, cx: &mut App) {
    render_frame(window, cx);
    let position = find(window, id).bounds().center();
    move_pointer(window, position, None, cx);
    mouse_button(window, position, MouseButton::Left, true, cx);
    mouse_button(window, position, MouseButton::Left, false, cx);
}

fn drag(window: &mut Window, from: Point<Pixels>, to: Point<Pixels>, cx: &mut App) {
    render_frame(window, cx);
    move_pointer(window, from, None, cx);
    mouse_button(window, from, MouseButton::Left, true, cx);
    for step in 1..=8 {
        let fraction = step as f32 / 8.;
        move_pointer(
            window,
            point(
                from.x + (to.x - from.x) * fraction,
                from.y + (to.y - from.y) * fraction,
            ),
            Some(MouseButton::Left),
            cx,
        );
    }
    mouse_button(window, to, MouseButton::Left, false, cx);
}

fn press(window: &mut Window, key: &str, cx: &mut App) {
    let key = Keystroke::parse(key).expect("valid test keystroke");
    render_frame(window, cx);
    // Enter 与 Tab 只派发绑定，不额外插入文本。
    let key = if matches!(key.key.as_str(), "enter" | "tab") {
        window.dispatch_event(
            KeyDownEvent {
                keystroke: key.clone(),
                is_held: false,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
        key
    } else {
        let key = key.with_simulated_ime();
        window.dispatch_keystroke(key.clone(), cx);
        key
    };
    window.dispatch_event(KeyUpEvent { keystroke: key }.to_platform_input(), cx);
    render_frame(window, cx);
}

fn input_text(window: &mut Window, text: &str, cx: &mut App) {
    render_frame(window, cx);
    for character in text.chars() {
        let text = character.to_string();
        let mut key = Keystroke::parse(&text).expect("a Unicode character is a valid keystroke");
        key.key_char = Some(text);
        window.dispatch_keystroke(key, cx);
        render_frame(window, cx);
    }
}

// ---- 状态读取 ----

/// 当前标签页的活动窗格。
fn active_pane(workspace: &Entity<WorkspaceView>, cx: &App) -> PaneId {
    let state = workspace.read(cx);
    state
        .tabs
        .iter()
        .find(|tab| tab.id == state.active_tab)
        .and_then(|tab| tab.active)
        .expect("an active pane")
}

/// 当前标签页的分屏树。
fn active_tree(workspace: &Entity<WorkspaceView>, cx: &App) -> PaneTree {
    let state = workspace.read(cx);
    state
        .tabs
        .iter()
        .find(|tab| tab.id == state.active_tab)
        .and_then(|tab| tab.tree.clone())
        .expect("a pane tree")
}

fn pane_session(
    workspace: &Entity<WorkspaceView>,
    pane: PaneId,
    cx: &App,
) -> Entity<glim_terminal_view::TerminalView> {
    workspace.read(cx).panes[&pane].view.clone()
}

fn active_tab_id(workspace: &Entity<WorkspaceView>, cx: &App) -> TabId {
    workspace.read(cx).active_tab
}

/// 等待所有会话进入稳定的空闲状态。
///
/// shell 启动或绘制提示符时可能短暂产生子进程；“空闲可直接关闭”
/// 的用例需要一个确定空闲的时刻。
fn wait_until_idle(cx: &TestAppContext, workspace: &Entity<WorkspaceView>) {
    let busy = |cx: &TestAppContext| {
        cx.update(|cx| {
            workspace
                .read(cx)
                .panes
                .values()
                .any(|entry| entry.view.read(cx).has_running_task())
        })
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if !busy(cx) {
            // 再观察一小段时间，确认不是启动过程中的短暂空闲。
            std::thread::sleep(Duration::from_millis(600));
            if !busy(cx) {
                return;
            }
        }
        assert!(Instant::now() < deadline, "会话未在超时内空闲");
        std::thread::sleep(Duration::from_millis(100));
    }
}

// ---- 用例 ----

/// 分屏：新增一个独立会话，已有会话身份保持。
#[gpui_kit::test]
fn splitting_adds_a_pane_and_keeps_the_existing_session(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        let (first, first_session) = {
            let state = workspace.read(cx);
            assert_eq!(state.tabs.len(), 1);
            assert_eq!(state.panes.len(), 1);
            let first = active_pane(&workspace, cx);
            (first, pane_session(&workspace, first, cx).entity_id())
        };

        press(window, "ctrl-shift-d", cx);

        let state = workspace.read(cx);
        assert_eq!(state.panes.len(), 2, "分屏后应有两个窗格");
        let tab = &state.tabs[0];
        let tree = tab.tree.as_ref().expect("分屏树");
        assert_eq!(tree.leaves().len(), 2);
        match tree {
            PaneTree::Split { axis, ratio, .. } => {
                assert_eq!(*axis, Axis::Horizontal);
                assert!((*ratio - 0.5).abs() < 0.01, "新分屏应均分");
            }
            _ => panic!("根节点应为分屏"),
        }
        let second = tab.active.expect("新窗格成为活动窗格");
        assert_ne!(second, first);
        assert_eq!(
            pane_session(&workspace, first, cx).entity_id(),
            first_session,
            "已有会话不应被重建"
        );
        assert!(
            !pane_session(&workspace, second, cx).read(cx).has_failed(),
            "新会话应正常启动"
        );
    })
    .expect("window stays open");
}

/// 放大与还原：只改变渲染范围，会话身份不变。
#[gpui_kit::test]
fn zooming_keeps_both_sessions(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        press(window, "ctrl-shift-d", cx);
        let (first, second, first_session, second_session) = {
            let mut leaves = active_tree(&workspace, cx).leaves();
            let first = leaves.remove(0);
            let second = leaves.remove(0);
            (
                first,
                second,
                pane_session(&workspace, first, cx).entity_id(),
                pane_session(&workspace, second, cx).entity_id(),
            )
        };

        press(window, "ctrl-shift-enter", cx);
        {
            let state = workspace.read(cx);
            let tab = &state.tabs[0];
            assert_eq!(tab.zoomed, Some(second), "活动窗格被放大");
            assert_eq!(state.panes.len(), 2, "放大不释放其他会话");
        }

        press(window, "ctrl-shift-enter", cx);
        let state = workspace.read(cx);
        assert_eq!(state.tabs[0].zoomed, None, "再次按下还原分屏");
        assert_eq!(
            pane_session(&workspace, first, cx).entity_id(),
            first_session
        );
        assert_eq!(
            pane_session(&workspace, second, cx).entity_id(),
            second_session
        );
    })
    .expect("window stays open");
}

/// 方向键焦点移动：活动窗格按几何方向切换。
#[gpui_kit::test]
fn keyboard_focus_moves_between_panes(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        press(window, "ctrl-shift-d", cx);
        let (first, second) = {
            let mut leaves = active_tree(&workspace, cx).leaves();
            (leaves.remove(0), leaves.remove(0))
        };
        assert_eq!(active_pane(&workspace, cx), second);

        press(window, "alt-left", cx);
        assert_eq!(active_pane(&workspace, cx), first);

        press(window, "alt-right", cx);
        assert_eq!(active_pane(&workspace, cx), second);
    })
    .expect("window stays open");
}

/// 分隔线拖动：比例变化、会话保持。
#[gpui_kit::test]
fn dragging_the_divider_changes_the_ratio(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        press(window, "ctrl-shift-d", cx);
        let first = active_tree(&workspace, cx).leaves()[0];
        let first_session = pane_session(&workspace, first, cx).entity_id();

        let bounds = find(window, ("pane", first.value())).bounds();
        let divider = bounds.right();
        let y = bounds.center().y;
        drag(window, point(divider, y), point(divider - px(150.), y), cx);

        let state = workspace.read(cx);
        match state.tabs[0].tree.as_ref().expect("tree") {
            PaneTree::Split { ratio, .. } => {
                assert!(*ratio < 0.45, "向左拖动后比例应明显变小: {ratio}");
                assert!(*ratio >= 0.15, "比例不应低于下限");
            }
            _ => panic!("根节点应为分屏"),
        }
        assert_eq!(state.panes.len(), 2, "拖动不释放会话");
        assert_eq!(
            pane_session(&workspace, first, cx).entity_id(),
            first_session
        );
    })
    .expect("window stays open");
}

/// 标签页：新建、切换，各自保留独立会话。
#[gpui_kit::test]
fn tabs_keep_independent_sessions(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        let first_tab = active_tab_id(&workspace, cx);
        let first_session = pane_session(&workspace, active_pane(&workspace, cx), cx).entity_id();

        press(window, "ctrl-shift-t", cx);
        let (second_tab, second_session) = {
            let state = workspace.read(cx);
            assert_eq!(state.tabs.len(), 2, "新建标签页");
            assert_eq!(state.panes.len(), 2, "每个标签页一个会话");
            let second_tab = state.active_tab;
            assert_ne!(second_tab, first_tab);
            let pane = active_pane(&workspace, cx);
            (second_tab, pane_session(&workspace, pane, cx).entity_id())
        };

        press(window, "ctrl-tab", cx);
        assert_eq!(
            active_tab_id(&workspace, cx),
            first_tab,
            "Ctrl+Tab 切回第一个标签"
        );
        assert_eq!(
            pane_session(&workspace, active_pane(&workspace, cx), cx).entity_id(),
            first_session,
            "切换标签不重建会话"
        );

        press(window, "ctrl-shift-tab", cx);
        assert_eq!(active_tab_id(&workspace, cx), second_tab);
        assert_eq!(
            pane_session(&workspace, active_pane(&workspace, cx), cx).entity_id(),
            second_session
        );
    })
    .expect("window stays open");
}

/// 关闭空闲窗格：直接关闭并释放会话；空工作区可以恢复。
#[gpui_kit::test]
fn closing_an_idle_pane_releases_the_session_and_can_be_restored(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| render_frame(window, cx))
        .expect("window stays open");
    wait_until_idle(cx, &workspace);

    let weak = cx
        .update_window(handle, |_, window, cx| {
            let pane = active_pane(&workspace, cx);
            let session = pane_session(&workspace, pane, cx);
            let weak = session.downgrade();

            press(window, "ctrl-shift-w", cx);

            {
                let state = workspace.read(cx);
                assert!(state.panes.is_empty(), "空闲窗格直接关闭");
                assert!(state.tabs[0].tree.is_none(), "空标签页");
                assert_eq!(state.tabs[0].active, None);

                // 空工作区恢复入口。
                let empty = find(window, "empty-new-pane");
                assert!(empty.visible(), "空状态应可见");
            }
            click(window, "empty-new-pane", cx);
            assert_eq!(workspace.read(cx).panes.len(), 1, "恢复一个终端");
            assert_eq!(active_tree(&workspace, cx).leaves().len(), 1);
            weak
        })
        .expect("window stays open");

    // 会话视图随窗格关闭释放（不保留后台视图）。
    cx.update_window(handle, |_, window, cx| render_frame(window, cx))
        .expect("window stays open");
    cx.run_until_parked();
    assert!(weak.upgrade().is_none(), "关闭窗格后终端视图应被释放");
}

/// 运行中任务：关闭窗格需要确认，取消保留窗格，确认后关闭。
#[gpui_kit::test]
fn closing_a_busy_pane_asks_for_confirmation(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        // 启动一个长时间运行的子进程（Start-Sleep 在 shell 进程内，不会被检测到）。
        input_text(window, "cmd /c \"ping -n 60 127.0.0.1 > nul\"", cx);
        press(window, "enter", cx);
    })
    .expect("window stays open");

    // 子进程出现需要真实时间；GPUI 的测试时钟是虚拟的，这里显式等待。
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let busy = cx.update(|cx| {
            workspace
                .read(cx)
                .panes
                .values()
                .any(|entry| entry.view.read(cx).has_running_task())
        });
        if busy {
            break;
        }
        assert!(Instant::now() < deadline, "子进程未在超时内出现");
        std::thread::sleep(Duration::from_millis(50));
    }

    cx.update_window(handle, |_, window, cx| {
        let pane_count = workspace.read(cx).panes.len();
        press(window, "ctrl-shift-w", cx);
        assert_eq!(
            workspace.read(cx).panes.len(),
            pane_count,
            "取消前窗格应保留"
        );

        press(window, "escape", cx);
        assert_eq!(workspace.read(cx).panes.len(), pane_count, "取消后窗格保留");

        press(window, "ctrl-shift-w", cx);
        press(window, "enter", cx);
        assert_eq!(
            workspace.read(cx).panes.len(),
            pane_count - 1,
            "确认后关闭窗格"
        );
    })
    .expect("window stays open");
}

/// 应用退出：运行中的会话汇总为一次确认；空闲时直接关闭。
#[gpui_kit::test]
fn exiting_with_a_running_task_asks_for_confirmation(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let visual = VisualTestContext::from_window(handle, cx).into_mut();

    visual.update(render_frame);
    wait_until_idle(cx, &workspace);
    assert!(visual.simulate_close(), "空闲时窗口可以直接关闭");

    // 启动一个长时间运行的子进程。
    visual.update(|window, cx| {
        input_text(window, "cmd /c \"ping -n 60 127.0.0.1 > nul\"", cx);
        press(window, "enter", cx);
    });

    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let busy = cx.update(|cx| {
            workspace
                .read(cx)
                .panes
                .values()
                .any(|entry| entry.view.read(cx).has_running_task())
        });
        if busy {
            break;
        }
        assert!(Instant::now() < deadline, "子进程未在超时内出现");
        std::thread::sleep(Duration::from_millis(50));
    }

    // 运行中：窗口不关闭，先汇总确认。
    assert!(!visual.simulate_close(), "运行中任务需要确认");
    visual.update(|window, _| {
        assert!(find(window, "cancel").visible(), "退出确认对话框应出现");
    });

    // 取消后再次请求关闭仍然需要确认；确认后窗口放行（触发应用退出）。
    visual.update(|window, cx| press(window, "escape", cx));
    assert!(!visual.simulate_close(), "取消后仍然需要确认");
    visual.update(|window, cx| press(window, "enter", cx));
}

// ---- 阶段 4：布局面板、拖拽交换与分隔线键盘调整 ----

/// 布局面板：`Alt+Z` 打开、方向键预览、`Enter` 应用；预览不修改分屏树。
#[gpui_kit::test]
fn layout_panel_previews_and_applies_a_preset(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        press(window, "ctrl-shift-d", cx);
        let (first, second) = {
            let leaves = active_tree(&workspace, cx).leaves();
            (leaves[0], leaves[1])
        };
        let first_session = session_id(&workspace, first, cx);
        let second_session = session_id(&workspace, second, cx);
        let tree_before = active_tree(&workspace, cx);

        // 打开面板：第一个方案与当前布局形状一致，仍显示真实窗格。
        press(window, "alt-z", cx);
        assert!(is_visible(window, "layout-panel"), "布局面板应打开");
        assert!(
            is_visible(window, ("pane", first.value())),
            "同形状时不预览"
        );
        assert_eq!(active_tree(&workspace, cx), tree_before, "预览不修改分屏树");

        // 方向键切到“上下”：工作区显示占位窗格，真实窗格不再渲染。
        press(window, "right", cx);
        assert!(
            is_visible(window, ("pane-preview", first.value())),
            "预览应显示占位窗格"
        );
        assert!(
            !is_visible(window, ("pane", first.value())),
            "预览不渲染真实窗格"
        );
        assert_eq!(active_tree(&workspace, cx), tree_before, "预览不修改分屏树");
        assert_eq!(session_id(&workspace, first, cx), first_session);

        // Enter 应用：布局变为上下，会话身份保持，面板关闭。
        press(window, "enter", cx);
        assert!(!is_visible(window, "layout-panel"), "应用后关闭面板");
        assert_eq!(root_split(&workspace, cx).0, Axis::Vertical, "应用上下布局");
        // 主槽位是活动窗格：它在上方，另一个窗格在下方。
        assert_eq!(active_tree(&workspace, cx).leaves(), vec![second, first]);
        assert_eq!(session_id(&workspace, first, cx), first_session);
        assert_eq!(session_id(&workspace, second, cx), second_session);
    })
    .expect("window stays open");
}

/// 悬停方案预览不重建会话；`Esc` 取消并恢复焦点。
#[gpui_kit::test]
fn hovering_a_layout_option_previews_until_cancelled(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        press(window, "ctrl-shift-d", cx);
        let leaves = active_tree(&workspace, cx).leaves();
        let (first, second) = (leaves[0], leaves[1]);
        let tree_before = active_tree(&workspace, cx);
        let second_session = session_id(&workspace, second, cx);

        press(window, "alt-z", cx);
        let option = find(window, ("layout-option", 1_usize)).bounds().center();
        move_pointer(window, option, None, cx);

        assert!(
            is_visible(window, ("pane-preview", first.value())),
            "悬停方案应显示预览"
        );
        assert_eq!(active_tree(&workspace, cx), tree_before, "预览不修改分屏树");
        assert_eq!(session_id(&workspace, second, cx), second_session);

        // Esc 取消：面板关闭、真实窗格恢复、焦点回到活动终端。
        press(window, "escape", cx);
        assert!(!is_visible(window, "layout-panel"), "Esc 关闭面板");
        assert!(
            is_visible(window, ("pane", first.value())),
            "取消后恢复真实布局"
        );
        assert_eq!(active_tree(&workspace, cx), tree_before, "取消不修改分屏树");
        let active = active_pane(&workspace, cx);
        assert!(
            pane_session(&workspace, active, cx)
                .read(cx)
                .focus_handle()
                .is_focused(window),
            "取消后焦点回到活动终端"
        );
    })
    .expect("window stays open");
}

/// 拖拽标题到另一个窗格：交换位置，会话与输出保持。
#[gpui_kit::test]
fn dragging_a_pane_title_onto_another_swaps_positions(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| render_frame(window, cx))
        .expect("window stays open");
    wait_until_idle(cx, &workspace);

    cx.update_window(handle, |_, window, cx| {
        press(window, "ctrl-shift-d", cx);
        let (first, second) = {
            let leaves = active_tree(&workspace, cx).leaves();
            (leaves[0], leaves[1])
        };
        let first_session = session_id(&workspace, first, cx);
        let second_session = session_id(&workspace, second, cx);

        let from = find(window, ("pane-drag", first.value())).bounds().center();
        let to = find(window, ("pane", second.value())).bounds().center();
        drag(window, from, to, cx);

        let tree = active_tree(&workspace, cx);
        assert_eq!(tree.leaves(), vec![second, first], "拖动后交换位置");
        assert_eq!(active_pane(&workspace, cx), first, "拖动窗格成为活动窗格");
        assert_eq!(session_id(&workspace, first, cx), first_session);
        assert_eq!(session_id(&workspace, second, cx), second_session);
        assert!(!is_visible(window, "layout-panel"), "交换不打开布局面板");
    })
    .expect("window stays open");
}

/// 拖拽标题到顶部热区：打开布局面板，释放在方案上应用它。
#[gpui_kit::test]
fn dragging_a_pane_title_to_the_top_applies_a_preset(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| render_frame(window, cx))
        .expect("window stays open");
    wait_until_idle(cx, &workspace);

    cx.update_window(handle, |_, window, cx| {
        press(window, "ctrl-shift-d", cx);
        let (first, second) = {
            let leaves = active_tree(&workspace, cx).leaves();
            (leaves[0], leaves[1])
        };
        let first_session = session_id(&workspace, first, cx);
        let second_session = session_id(&workspace, second, cx);

        // 从拖动窗格（second）的标题拖到窗格区域顶部。
        let header = find(window, ("pane-drag", second.value()))
            .bounds()
            .center();
        let area_top = find(window, ("pane", second.value())).bounds().top();
        move_pointer(window, header, None, cx);
        mouse_button(window, header, MouseButton::Left, true, cx);
        move_pointer(
            window,
            point(header.x, area_top + px(10.)),
            Some(MouseButton::Left),
            cx,
        );
        assert!(is_visible(window, "layout-panel"), "拖到热区应打开布局面板");

        // 移到“上下”方案（下标 1）并松手。
        let option = find(window, ("layout-option", 1_usize)).bounds().center();
        move_pointer(window, option, Some(MouseButton::Left), cx);
        mouse_button(window, option, MouseButton::Left, false, cx);

        assert!(!is_visible(window, "layout-panel"), "应用后关闭面板");
        assert_eq!(root_split(&workspace, cx).0, Axis::Vertical, "应用上下布局");
        // 主槽位是拖动窗格：它在上方，另一个窗格在下方。
        assert_eq!(active_tree(&workspace, cx).leaves(), vec![second, first]);
        assert_eq!(session_id(&workspace, first, cx), first_session);
        assert_eq!(session_id(&workspace, second, cx), second_session);
    })
    .expect("window stays open");
}

/// 聚焦分隔线后用方向键调整比例，`Home` 回到均分；会话保持。
#[gpui_kit::test]
fn keyboard_adjusts_the_focused_divider(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        press(window, "ctrl-shift-d", cx);
        let first = active_tree(&workspace, cx).leaves()[0];
        let first_session = session_id(&workspace, first, cx);

        // 点击分隔线聚焦（点击位置即两个窗格的边界）。
        let bounds = find(window, ("pane", first.value())).bounds();
        let divider = point(bounds.right(), bounds.center().y);
        move_pointer(window, divider, None, cx);
        mouse_button(window, divider, MouseButton::Left, true, cx);
        mouse_button(window, divider, MouseButton::Left, false, cx);
        assert!(
            workspace.read(cx).split_interaction.is_some(),
            "点击分隔线应记录交互"
        );
        assert!(
            workspace.read(cx).split_focus.is_focused(window),
            "点击分隔线应聚焦分隔线"
        );
        assert!(
            (root_split(&workspace, cx).1 - 0.5).abs() < 1e-3,
            "均分起点"
        );

        // 方向键：左右调整 5%，垂直方向忽略。
        press(window, "left", cx);
        let ratio = root_split(&workspace, cx).1;
        assert!((ratio - 0.45).abs() < 1e-3, "向左调整 5%: {ratio}");
        press(window, "down", cx);
        assert!(
            (root_split(&workspace, cx).1 - ratio).abs() < 1e-3,
            "垂直方向不调整水平分隔线"
        );
        press(window, "right", cx);
        assert!(
            (root_split(&workspace, cx).1 - 0.5).abs() < 1e-3,
            "向右调整回均分"
        );

        // Home 回到均分。
        press(window, "left", cx);
        press(window, "home", cx);
        assert!(
            (root_split(&workspace, cx).1 - 0.5).abs() < 1e-3,
            "Home 均分"
        );
        assert_eq!(session_id(&workspace, first, cx), first_session);
    })
    .expect("window stays open");
}

/// `Ctrl+Shift+方向键` 交换窗格位置；焦点跟随窗格身份。
#[gpui_kit::test]
fn keyboard_swaps_panes_in_direction(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        press(window, "ctrl-shift-d", cx);
        let (first, second) = {
            let leaves = active_tree(&workspace, cx).leaves();
            (leaves[0], leaves[1])
        };
        let first_session = session_id(&workspace, first, cx);
        let second_session = session_id(&workspace, second, cx);
        assert_eq!(active_pane(&workspace, cx), second);

        // 与左侧窗格交换：second 移到左侧。
        press(window, "ctrl-shift-left", cx);
        assert_eq!(active_tree(&workspace, cx).leaves(), vec![second, first]);
        assert_eq!(active_pane(&workspace, cx), second, "焦点跟随窗格");
        let geometry = active_tree(&workspace, cx).geometry();
        assert!(
            (geometry.pane_rect(second).expect("second pane").x).abs() < 1e-3,
            "second 移动到左侧"
        );

        // 与右侧窗格交换：回到原位。
        press(window, "ctrl-shift-right", cx);
        assert_eq!(active_tree(&workspace, cx).leaves(), vec![first, second]);
        assert_eq!(session_id(&workspace, first, cx), first_session);
        assert_eq!(session_id(&workspace, second, cx), second_session);
    })
    .expect("window stays open");
}

// ---- 阶段 5：配置与工作区恢复 ----

use crate::workspace_state::{self, LayoutNode, SplitNode, TabState, WindowState, WorkspaceState};
/// 测试用数据目录：`${TEMP}/glim-workspace-tests/<name>/`。
fn data_directory(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir()
        .join("glim-workspace-tests")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create data directory");
    directory
}

/// 初始化设置与工作区持久化，再打开窗口。
fn open_workspace_with_storage(
    cx: &mut TestAppContext,
    config_path: std::path::PathBuf,
    data_directory: std::path::PathBuf,
) -> (AnyWindowHandle, Entity<WorkspaceView>) {
    cx.update(|cx| {
        glim_settings::init_with_path(cx, Some(config_path));
        crate::init(cx, Some(data_directory));
    });
    open_workspace(cx)
}

/// 叶节点：目录与 shell 引用。
fn leaf(directory: Option<&str>, shell: Option<&str>) -> LayoutNode {
    LayoutNode {
        directory: directory.map(str::to_owned),
        shell: shell.map(str::to_owned),
        split: None,
    }
}

/// 分屏节点。
fn split(axis: Axis, ratio: f32, first: LayoutNode, second: LayoutNode) -> LayoutNode {
    LayoutNode {
        directory: None,
        shell: None,
        split: Some(SplitNode {
            axis,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }),
    }
}

/// 恢复：标签页、分屏形状、目录与活动标签都按保存数据重建。
#[gpui_kit::test]
fn restores_tabs_layout_and_directories(cx: &mut TestAppContext) {
    let data = data_directory("restore");
    let config_path = data.join("config.toml");
    let left = data.join("left");
    let right = data.join("right");
    std::fs::create_dir_all(&left).expect("left dir");
    std::fs::create_dir_all(&right).expect("right dir");
    let missing = data.join("missing");

    let state = WorkspaceState {
        version: 1,
        window: Some(WindowState {
            width: 1100.,
            height: 700.,
            maximized: false,
        }),
        active_tab: 1,
        tabs: vec![
            TabState {
                tree: Some(split(
                    Axis::Horizontal,
                    2. / 3.,
                    leaf(Some(&left.to_string_lossy()), Some("pwsh.exe")),
                    leaf(Some(&right.to_string_lossy()), None),
                )),
                active_pane: Some(1),
            },
            TabState {
                tree: Some(leaf(Some(&missing.to_string_lossy()), None)),
                active_pane: Some(0),
            },
        ],
    };
    glim_settings::save_toml(&workspace_state::file_in(&data), &state).expect("write state");

    let (handle, workspace) = open_workspace_with_storage(cx, config_path, data.clone());
    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        let state = workspace.read(cx);
        assert_eq!(state.tabs.len(), 2, "恢复两个标签页");
        assert_eq!(state.active_tab, state.tabs[1].id, "活动标签按保存数据恢复");
        assert_eq!(state.panes.len(), 3, "每个叶节点一个会话");

        let tree = state.tabs[0].tree.as_ref().expect("first tree");
        assert_eq!(tree.leaves().len(), 2);
        match tree {
            PaneTree::Split { axis, ratio, .. } => {
                assert_eq!(*axis, Axis::Horizontal);
                assert!((*ratio - 2. / 3.).abs() < 1e-3, "比例恢复");
            }
            _ => panic!("第一个标签页应为分屏"),
        }

        // 目录：存在时沿用；缺失时回退到用户目录。
        let leaves = tree.leaves();
        assert_eq!(
            pane_session(&workspace, leaves[0], cx)
                .read(cx)
                .configured_directory(),
            Some(left.as_path()),
            "目录按保存数据恢复"
        );
        assert_eq!(
            pane_session(&workspace, leaves[1], cx)
                .read(cx)
                .configured_directory(),
            Some(right.as_path())
        );
        let restored_missing = state.tabs[1].tree.as_ref().expect("second tree").leaves()[0];
        assert_eq!(
            pane_session(&workspace, restored_missing, cx)
                .read(cx)
                .configured_directory(),
            glim_settings::home_directory().as_deref(),
            "缺失目录回退到用户目录"
        );
    })
    .expect("window stays open");

    let warnings = cx.update(|cx| crate::take_startup_warnings(cx));
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("回退到用户目录")),
        "缺失目录应有启动提示: {warnings:?}"
    );
}

/// 恢复后 shell 真的在保存的目录里启动（真实 PTY）；目录由进程信息核对。
#[gpui_kit::test]
fn restored_sessions_start_in_the_saved_directory(cx: &mut TestAppContext) {
    let data = data_directory("restore-cwd");
    let workdir = data.join("workdir");
    std::fs::create_dir_all(&workdir).expect("create workdir");

    let state = WorkspaceState {
        version: 1,
        window: None,
        active_tab: 0,
        tabs: vec![TabState {
            tree: Some(leaf(Some(&workdir.to_string_lossy()), None)),
            active_pane: Some(0),
        }],
    };
    glim_settings::save_toml(&workspace_state::file_in(&data), &state).expect("write state");

    let (handle, workspace) =
        open_workspace_with_storage(cx, data.join("config.toml"), data.clone());
    let pane = cx.update(|cx| active_pane(&workspace, cx));
    let view = cx.update(|cx| pane_session(&workspace, pane, cx));
    let expected = workdir.canonicalize().expect("canonicalize workdir");

    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let directory = cx
            .update_window(handle, |_, window, cx| {
                render_frame(window, cx);
                view.read(cx).working_directory()
            })
            .expect("window stays open");
        if let Some(directory) = directory {
            assert_eq!(
                directory.canonicalize().ok(),
                Some(expected.clone()),
                "shell 应在保存的目录中启动"
            );
            return;
        }
        assert!(Instant::now() < deadline, "会话未在超时内上报工作目录");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// 关闭时保存：布局、目录与窗口状态写入 workspace.toml。
#[gpui_kit::test]
fn closing_saves_layout_and_window_state(cx: &mut TestAppContext) {
    let data = data_directory("save");
    let config_path = data.join("config.toml");
    let (handle, workspace) = open_workspace_with_storage(cx, config_path, data.clone());

    let visual = VisualTestContext::from_window(handle, cx).into_mut();
    visual.update(render_frame);
    wait_until_idle(cx, &workspace);
    visual.update(|window, cx| press(window, "ctrl-shift-d", cx));

    let (first, second) = visual.update(|_, cx| {
        let leaves = active_tree(&workspace, cx).leaves();
        (leaves[0], leaves[1])
    });
    let (first_session, second_session) = visual.update(|_, cx| {
        (
            session_id(&workspace, first, cx),
            session_id(&workspace, second, cx),
        )
    });
    assert_ne!(first_session, second_session, "两个窗格是独立会话");
    visual.update(|_, cx| {
        assert_eq!(active_tree(&workspace, cx).leaves(), vec![first, second]);
    });
    // 新窗格的 shell 也要进入空闲，才能验证「空闲工作区可以直接关闭」。
    wait_until_idle(cx, &workspace);
    assert!(visual.simulate_close(), "空闲工作区可以直接关闭");

    let path = workspace_state::file_in(&data);
    let mut saved: WorkspaceState = glim_settings::load_toml(&path)
        .expect("load")
        .expect("file exists");
    saved.validate().expect("valid state");
    assert_eq!(saved.tabs.len(), 1);
    assert_eq!(saved.active_tab, 0);
    let tree = saved.tabs[0].tree.as_ref().expect("tree");
    match tree {
        LayoutNode {
            split: Some(split), ..
        } => {
            assert_eq!(split.axis, Axis::Horizontal);
            assert!((split.ratio - 0.5).abs() < 1e-3);
        }
        _ => panic!("保存的分屏树应包含分屏节点"),
    }
    let saved_leaves = workspace_state::leaves(tree);
    assert_eq!(saved_leaves.len(), 2);
    for leaf in &saved_leaves {
        assert!(leaf.directory.is_some(), "保存已知工作目录");
    }
    let window = saved.window.expect("window state");
    assert!(window.width >= 800. && window.height >= 600.);
}

/// 无效恢复数据：整份忽略、按默认工作区启动并提示。
#[gpui_kit::test]
fn invalid_workspace_file_falls_back_to_default(cx: &mut TestAppContext) {
    let data = data_directory("invalid");
    std::fs::write(workspace_state::file_in(&data), "version = \"坏数据\"\n")
        .expect("write invalid state");

    let (handle, workspace) =
        open_workspace_with_storage(cx, data.join("config.toml"), data.clone());
    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        let state = workspace.read(cx);
        assert_eq!(state.tabs.len(), 1, "无效数据回退到默认工作区");
        assert_eq!(state.panes.len(), 1);
    })
    .expect("window stays open");

    let warnings = cx.update(|cx| crate::take_startup_warnings(cx));
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("已按默认工作区启动")),
        "无效数据应有启动提示: {warnings:?}"
    );
}

/// 设置界面：`Ctrl+,` 打开对话框，保存写入 config.toml。
#[gpui_kit::test]
fn settings_dialog_saves_config(cx: &mut TestAppContext) {
    let data = data_directory("settings-dialog");
    let config_path = data.join("config.toml");
    let (handle, _workspace) = open_workspace_with_storage(cx, config_path.clone(), data.clone());

    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        press(window, "ctrl-,", cx);
        assert!(is_visible(window, "settings-save"), "设置对话框应打开");

        // 在对话框内改选主题后保存。
        click(window, "settings-theme-light", cx);
        click(window, "settings-save", cx);
        assert!(!is_visible(window, "settings-save"), "保存后对话框关闭");
    })
    .expect("window stays open");

    let settings: glim_settings::Settings = glim_settings::load_toml(&config_path)
        .expect("load")
        .expect("file exists");
    assert_eq!(settings.theme(), glim_settings::ThemeSetting::Light);
    assert_eq!(settings.scrollback(), glim_settings::DEFAULT_SCROLLBACK);
    assert!(
        settings.shell().is_none(),
        "未选择具体 shell 时不写入 shell 配置"
    );
    assert!(settings.font_family().is_none(), "默认字族不写入配置");
}

/// 设置界面：shell 与字体下拉都从候选中选择，保存后写入配置。
#[gpui_kit::test]
fn settings_dialog_picks_shell_and_font_from_candidates(cx: &mut TestAppContext) {
    let available = glim_terminal::available_shells();
    if available.is_empty() {
        // 没有可探测的 shell 时只验证对话框能打开（非 Windows 平台）。
        return;
    }
    let installed = cx.update(|cx| cx.text_system().all_font_names());
    if installed.is_empty() {
        return;
    }

    let data = data_directory("settings-candidates");
    let config_path = data.join("config.toml");
    let (handle, _workspace) = open_workspace_with_storage(cx, config_path.clone(), data.clone());

    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        press(window, "ctrl-,", cx);
        assert!(is_visible(window, "settings-save"), "设置对话框应打开");
        assert!(is_visible(window, "settings-font-family"), "字体下拉应存在");
        assert!(
            is_visible(window, "settings-shell-program"),
            "shell 下拉应存在"
        );

        // shell：打开下拉，选中默认项之后的第一个候选，并在参数框输入参数。
        click(window, "settings-shell-program", cx);
        press(window, "down", cx);
        press(window, "enter", cx);
        click(window, "settings-shell-args", cx);
        input_text(window, "-NoLogo", cx);

        // 字体：搜索已安装字族并确认。
        click(window, "settings-font-family", cx);
        input_text(window, &installed[0], cx);
        press(window, "enter", cx);

        click(window, "settings-save", cx);
        assert!(!is_visible(window, "settings-save"), "保存后对话框关闭");
    })
    .expect("window stays open");

    let settings: glim_settings::Settings = glim_settings::load_toml(&config_path)
        .expect("load")
        .expect("file exists");
    let shell = settings.shell().expect("选择的 shell 写入配置");
    assert_eq!(shell.program(), available[0].program());
    assert_eq!(shell.args(), ["-NoLogo"]);
    assert_eq!(settings.font_family(), Some(installed[0].as_str()));
}

/// 设置变化：主题胶囊写入配置并立即生效；字体变化应用到主题。
#[gpui_kit::test]
fn settings_changes_persist_and_apply(cx: &mut TestAppContext) {
    let data = data_directory("settings-apply");
    let config_path = data.join("config.toml");
    let (handle, workspace) = open_workspace_with_storage(cx, config_path.clone(), data.clone());

    // 点击与断言分开在两次更新中：事件在更新结束时派发。
    cx.update_window(handle, |_, window, cx| {
        render_frame(window, cx);
        click(window, "theme-capsule-light", cx);
    })
    .expect("window stays open");

    cx.update_window(handle, |_, _window, cx| {
        assert!(!cx.theme().is_dark(), "主题立即生效");
        assert_eq!(
            glim_settings::handle(cx)
                .expect("settings service")
                .read(cx)
                .current()
                .theme(),
            glim_settings::ThemeSetting::Light,
            "设置已写入"
        );
        assert_eq!(
            workspace.read(cx).theme_preference,
            glim_ui::ThemePreference::Light,
            "视图跟随设置"
        );
    })
    .expect("window stays open");

    let settings: glim_settings::Settings = glim_settings::load_toml(&config_path)
        .expect("load")
        .expect("file exists");
    assert_eq!(settings.theme(), glim_settings::ThemeSetting::Light);

    // 字体变化经订阅应用：字号进入主题，终端在下一帧使用新值。
    let settings_state = cx.update(|cx| glim_settings::handle(cx).expect("settings service"));
    cx.update(|cx| {
        settings_state.update(cx, |state, cx| {
            let next = state
                .current()
                .clone()
                .with_font_size(Some(16.))
                .with_font_family(None::<String>);
            state.apply(next, cx);
        });
    });
    assert_eq!(
        cx.update(|cx| cx.theme().mono_font_size),
        px(16.),
        "字号已应用"
    );

    // 默认 shell 变更作用于新会话：分屏创建的新窗格使用新配置。
    cx.update(|cx| {
        settings_state.update(cx, |state, cx| {
            let next = state
                .current()
                .clone()
                .with_shell(Some(glim_settings::ShellConfig::new("cmd.exe")));
            state.apply(next, cx);
        });
    });
    cx.update_window(handle, |_, window, cx| press(window, "ctrl-shift-d", cx))
        .expect("window stays open");
    cx.update_window(handle, |_, _window, cx| {
        let active = active_pane(&workspace, cx);
        assert_eq!(
            pane_session(&workspace, active, cx)
                .read(cx)
                .shell_program(),
            Some("cmd.exe"),
            "新会话使用配置的默认 shell"
        );
    })
    .expect("window stays open");
}
