//! 分屏布局元素：按分屏树把窗格与分隔线放到精确像素位置。
//!
//! 每个分屏节点是一个自定义元素：`request_layout` 占满父区域，
//! `prepaint` 用实际像素区域计算两个子区域并布局、放置，`paint` 绘制
//! 子元素与分隔线，并注册分隔线的拖动监听。元素不持有业务状态：
//! 拖动只把新比例写回工作区，取消或提交都不重建会话。
//!
//! 分隔线可以持有键盘焦点：点击后方向键调整比例，`Home` 回到均分。
//! 布局预览用同一元素摆放占位窗格，但不绘制分隔线，也不注册鼠标与
//! 焦点行为。

use std::collections::HashMap;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

use crate::pane_tree::{Axis, PaneId, PaneTree, SplitId, clamp_ratio};
use crate::workspace_view::{SplitMetrics, WorkspaceView};

/// 分隔线的命中宽度（像素；拖动操作的物理界限）。
const DIVIDER_HIT_WIDTH: f32 = 7.;
/// 分隔线的可见宽度（像素；单条边界线）。
const DIVIDER_LINE_WIDTH: f32 = 1.;

/// 分屏布局选项。
pub(crate) struct SplitLayoutOptions {
    /// 分隔线是否可交互；预览与拖动中只摆放窗格。
    pub dividers: bool,
    /// 当前聚焦的分隔线。
    pub focused_split: Option<SplitId>,
}

/// 构建分屏布局元素；`panes` 中列出的窗格元素会被消耗。
pub(crate) fn build(
    tree: &PaneTree,
    panes: &mut HashMap<PaneId, AnyElement>,
    workspace: Entity<WorkspaceView>,
    options: &SplitLayoutOptions,
) -> AnyElement {
    match tree {
        PaneTree::Leaf(pane) => panes
            .remove(pane)
            .unwrap_or_else(|| div().into_any_element()),
        PaneTree::Split {
            id,
            axis,
            ratio,
            first,
            second,
        } => {
            let min_first = first.min_extent(*axis);
            let min_second = second.min_extent(*axis);
            let first = build(first, panes, workspace.clone(), options);
            let second = build(second, panes, workspace.clone(), options);
            SplitElement {
                id: *id,
                axis: *axis,
                ratio: *ratio,
                min_first,
                min_second,
                first,
                second,
                workspace,
                dividers: options.dividers,
                focused: options.focused_split == Some(*id),
            }
            .into_any_element()
        }
    }
}

/// 一个分屏节点。
struct SplitElement {
    id: SplitId,
    axis: Axis,
    ratio: f32,
    /// 第一个子区域的最小尺寸（rem）。
    min_first: f32,
    /// 第二个子区域的最小尺寸（rem）。
    min_second: f32,
    first: AnyElement,
    second: AnyElement,
    workspace: Entity<WorkspaceView>,
    /// 是否绘制并响应分隔线。
    dividers: bool,
    /// 分隔线是否持有键盘焦点。
    focused: bool,
}

/// 布局结果：分隔线的命中区域与节点区域（拖动换算用）。
struct SplitElementLayout {
    bounds: Bounds<Pixels>,
    divider_bounds: Bounds<Pixels>,
    hitbox: Hitbox,
}

/// 两个子区域与分隔线的像素位置。
struct Placement {
    first_origin: Point<Pixels>,
    first_size: Size<Pixels>,
    second_origin: Point<Pixels>,
    second_size: Size<Pixels>,
    divider_bounds: Bounds<Pixels>,
}

impl SplitElement {
    /// 计算两个子区域与分隔线的像素位置。
    fn place(&self, bounds: Bounds<Pixels>) -> Placement {
        match self.axis {
            Axis::Horizontal => {
                let first_width = (bounds.size.width * self.ratio).round();
                let second_width = bounds.size.width - first_width;
                Placement {
                    first_origin: bounds.origin,
                    first_size: size(first_width, bounds.size.height),
                    second_origin: point(bounds.origin.x + first_width, bounds.origin.y),
                    second_size: size(second_width, bounds.size.height),
                    divider_bounds: Bounds {
                        origin: point(
                            bounds.origin.x + first_width - px(DIVIDER_HIT_WIDTH / 2.),
                            bounds.origin.y,
                        ),
                        size: size(px(DIVIDER_HIT_WIDTH), bounds.size.height),
                    },
                }
            }
            Axis::Vertical => {
                let first_height = (bounds.size.height * self.ratio).round();
                let second_height = bounds.size.height - first_height;
                Placement {
                    first_origin: bounds.origin,
                    first_size: size(bounds.size.width, first_height),
                    second_origin: point(bounds.origin.x, bounds.origin.y + first_height),
                    second_size: size(bounds.size.width, second_height),
                    divider_bounds: Bounds {
                        origin: point(
                            bounds.origin.x,
                            bounds.origin.y + first_height - px(DIVIDER_HIT_WIDTH / 2.),
                        ),
                        size: size(bounds.size.width, px(DIVIDER_HIT_WIDTH)),
                    },
                }
            }
        }
    }
}

impl Element for SplitElement {
    type RequestLayoutState = ();
    type PrepaintState = SplitElementLayout;

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::from(("split", self.id.value())))
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
        (window.request_layout(style, [], cx), ())
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
        let placement = self.place(bounds);
        self.first
            .layout_as_root(placement.first_size.into(), window, cx);
        self.first.prepaint_at(placement.first_origin, window, cx);
        self.second
            .layout_as_root(placement.second_size.into(), window, cx);
        self.second.prepaint_at(placement.second_origin, window, cx);

        let hitbox = window.insert_hitbox(placement.divider_bounds, HitboxBehavior::Normal);
        SplitElementLayout {
            bounds,
            divider_bounds: placement.divider_bounds,
            hitbox,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        layout: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.first.paint(window, cx);
        self.second.paint(window, cx);

        if !self.dividers {
            return;
        }

        let dragging = self.workspace.read(cx).is_dragging_split(self.id);
        window.set_cursor_style(
            match self.axis {
                Axis::Horizontal => CursorStyle::ResizeColumn,
                Axis::Vertical => CursorStyle::ResizeRow,
            },
            &layout.hitbox,
        );

        // 可见线居中于命中区域；拖动中或聚焦时显示强调色。
        let line = match self.axis {
            Axis::Horizontal => Bounds {
                origin: point(
                    layout.divider_bounds.origin.x
                        + px((DIVIDER_HIT_WIDTH - DIVIDER_LINE_WIDTH) / 2.),
                    layout.divider_bounds.origin.y,
                ),
                size: size(px(DIVIDER_LINE_WIDTH), layout.divider_bounds.size.height),
            },
            Axis::Vertical => Bounds {
                origin: point(
                    layout.divider_bounds.origin.x,
                    layout.divider_bounds.origin.y
                        + px((DIVIDER_HIT_WIDTH - DIVIDER_LINE_WIDTH) / 2.),
                ),
                size: size(layout.divider_bounds.size.width, px(DIVIDER_LINE_WIDTH)),
            },
        };
        window.paint_quad(fill(
            line,
            if dragging || self.focused {
                cx.theme().primary
            } else {
                cx.theme().border
            },
        ));

        let divider_bounds = layout.divider_bounds;
        let node_bounds = layout.bounds;
        let hitbox = layout.hitbox.clone();
        let workspace = self.workspace.clone();
        let id = self.id;
        let axis = self.axis;
        let min_first_rem = self.min_first;
        let min_second_rem = self.min_second;
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !divider_bounds.contains(&event.position)
            {
                return;
            }
            window.capture_pointer(hitbox.id);
            // 键盘调整使用与拖动一致的界限：节点区域与两侧最小尺寸。
            let rem = f32::from(window.rem_size());
            let extent = match axis {
                Axis::Horizontal => f32::from(node_bounds.size.width),
                Axis::Vertical => f32::from(node_bounds.size.height),
            }
            .max(1.);
            let min_first = min_first_rem * rem;
            let min_second = min_second_rem * rem;
            workspace.update(cx, |workspace, cx| {
                workspace.begin_split_interaction(
                    id,
                    SplitMetrics {
                        axis,
                        extent,
                        min_first,
                        min_second,
                    },
                    window,
                    cx,
                );
            });
            cx.stop_propagation();
        });

        let min_first = self.min_first * f32::from(window.rem_size());
        let min_second = self.min_second * f32::from(window.rem_size());
        let workspace = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, _phase, _window, cx| {
            if !workspace.read(cx).is_dragging_split(id) {
                return;
            }
            let (extent, position) = match axis {
                Axis::Horizontal => (
                    node_bounds.size.width,
                    event.position.x - node_bounds.origin.x,
                ),
                Axis::Vertical => (
                    node_bounds.size.height,
                    event.position.y - node_bounds.origin.y,
                ),
            };
            let extent = f32::from(extent).max(1.);
            let ratio = clamp_ratio(f32::from(position) / extent, extent, min_first, min_second);
            workspace.update(cx, |workspace, cx| workspace.set_split_ratio(id, ratio, cx));
        });

        let workspace = self.workspace.clone();
        window.on_mouse_event(move |_event: &MouseUpEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || !workspace.read(cx).is_dragging_split(id) {
                return;
            }
            workspace.update(cx, |workspace, cx| workspace.end_divider_drag(cx));
        });
    }
}

impl IntoElement for SplitElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
