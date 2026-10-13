//! 工作区界面集成测试：真实会话下的标签页、分屏、焦点与关闭流程。
//!
//! 通过键盘与指针事件驱动真实界面，再读取工作区与终端的内部状态。
//! 会话是真实 PTY（Windows PowerShell），子进程相关的用例使用真实时间
//! 与明确超时，其余用例只依赖同步的布局与动作。

use std::time::{Duration, Instant};

use gpui_base::test_support::{self as observation, ElementSnapshot};
use gpui_kit::component::Root;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Entity, InputEvent as _, KeyDownEvent, KeyUpEvent,
    Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    TestAppContext, VisualTestContext, Window, point, px, size,
};

use super::WorkspaceView;
use crate::pane_tree::{Axis, PaneId, PaneTree, TabId};

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
    let mut visual = VisualTestContext::from_window(handle, cx).into_mut();

    visual.update(|window, cx| render_frame(window, cx));
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
