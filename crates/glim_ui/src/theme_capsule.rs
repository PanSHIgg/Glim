//! 主题胶囊：状态栏右下角的浅色 / 深色 / 跟随系统切换控件。

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Toggle;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex};
use gpui_kit::*;

use crate::ThemePreference;

/// 主题选择回调：接收被激活分段对应的偏好。
type SelectHandler = Rc<dyn Fn(&ThemePreference, &mut Window, &mut App) + 'static>;

/// 三段式主题选择胶囊。
///
/// 受控组件：当前偏好由 `new` 输入，点击意图通过 `on_select` 上报，
/// 自身不保存状态。三个分段都是图标按钮，可经键盘或指针激活，
/// 并带有可访问名称（悬停提示）。
#[derive(IntoElement)]
pub struct ThemeCapsule {
    selected: ThemePreference,
    on_select: Option<SelectHandler>,
}

impl ThemeCapsule {
    /// 创建胶囊；`selected` 为当前主题偏好。
    pub fn new(selected: ThemePreference) -> Self {
        Self {
            selected,
            on_select: None,
        }
    }

    /// 设置选择回调；参数为被激活分段对应的偏好。
    pub fn on_select(
        mut self,
        handler: impl Fn(&ThemePreference, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for ThemeCapsule {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let selected = self.selected;
        let on_select = self.on_select;

        let segment = move |id: &'static str,
                            preference: ThemePreference,
                            icon: IconName,
                            label: &'static str| {
            let on_select = on_select.clone();
            let checked = selected == preference;
            Toggle::new(id)
                .small()
                .rounded_full()
                .checked(checked)
                .icon(Icon::new(icon))
                .tooltip(label)
                .on_click(move |_, window, cx| {
                    if let Some(handler) = &on_select {
                        handler(&preference, window, cx);
                    }
                })
        };

        h_flex()
            .items_center()
            .gap_0p5()
            .p_0p5()
            .rounded_full()
            .border_1()
            .border_color(theme.border)
            .bg(theme.muted)
            .text_color(theme.muted_foreground)
            .child(segment(
                "theme-capsule-light",
                ThemePreference::Light,
                IconName::Sun,
                "浅色",
            ))
            .child(segment(
                "theme-capsule-dark",
                ThemePreference::Dark,
                IconName::Moon,
                "深色",
            ))
            .child(segment(
                "theme-capsule-system",
                ThemePreference::System,
                IconName::Monitor,
                "跟随系统",
            ))
    }
}
