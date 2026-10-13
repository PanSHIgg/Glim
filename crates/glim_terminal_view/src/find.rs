//! 查找栏：滚动历史查找的输入与导航。
//!
//! 栏本身是终端视图的兄弟节点（不是子节点），因此输入焦点落入查找框时
//! 键盘事件不会触达终端的按键处理与 Actions。

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::*;

use glim_terminal::SearchMatch;

use crate::TerminalView;

/// 查找状态；由终端视图持有。
pub(crate) struct FindState {
    /// 是否显示查找栏。
    pub visible: bool,
    /// 查询输入框状态。
    pub input: Entity<InputState>,
    /// 当前查询串。
    pub query: String,
    /// 当前匹配；`None` 表示没有匹配或未搜索。
    pub current: Option<SearchMatch>,
    /// 查询没有匹配结果。
    pub not_found: bool,
}

impl FindState {
    /// 创建查找状态（输入框在首次打开前也可以存在，避免重建）。
    pub fn new(window: &mut Window, cx: &mut Context<TerminalView>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("查找"));
        Self {
            visible: false,
            input,
            query: String::new(),
            current: None,
            not_found: false,
        }
    }
}

impl TerminalView {
    /// 渲染查找栏；仅在 `find.visible` 时调用。
    pub(crate) fn render_find_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let mut bar = h_flex()
            .absolute()
            .top_2()
            .right_2()
            .occlude()
            .gap_1()
            .p_1()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .shadow_md()
            .child(div().w_48().child(Input::new(&self.find.input).small()));

        if self.find.not_found {
            bar = bar.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("无匹配"),
            );
        }

        bar.child(
            Button::new("find-previous")
                .ghost()
                .xsmall()
                .icon(gpui_kit::assets::IconName::ChevronUp)
                .tooltip("上一个")
                .on_click(cx.listener(|this, _, _, cx| this.find_previous(cx))),
        )
        .child(
            Button::new("find-next")
                .ghost()
                .xsmall()
                .icon(gpui_kit::assets::IconName::ChevronDown)
                .tooltip("下一个")
                .on_click(cx.listener(|this, _, _, cx| this.find_next(cx))),
        )
        .child(
            Button::new("find-close")
                .ghost()
                .xsmall()
                .icon(gpui_kit::assets::IconName::X)
                .tooltip("关闭查找")
                .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
        )
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                this.close_find(window, cx);
                cx.stop_propagation();
            }
        }))
    }
}
