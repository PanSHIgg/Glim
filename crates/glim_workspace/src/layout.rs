//! 推荐布局：按窗格数量给出可预览、可应用的排列方案。
//!
//! 纯逻辑，不依赖 GPUI：方案只描述形状与窗格顺序，构建时由工作区
//! 分配分屏身份。方案列表的第一个槽位是主槽位（活动或正在拖动的
//! 窗格），其余窗格按原有顺序填入，对应设计稿 `templatesFor` 的行为。

use crate::pane_tree::{Axis, PaneId, PaneTree, SplitId};

/// 主次方案中主窗格的比例（设计稿：约三分之二）。
const MAIN_RATIO: f32 = 2. / 3.;

/// 一个推荐布局方案。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayoutTemplate {
    name: &'static str,
    shape: LayoutShape,
}

impl LayoutTemplate {
    /// 方案名称；用作缩略图下方的标签。
    pub(crate) fn name(&self) -> &'static str {
        self.name
    }

    /// 覆盖的窗格数量；与方案列表的输入数量一致。
    pub(crate) fn pane_count(&self) -> usize {
        self.shape.pane_count()
    }

    /// 按顺序把窗格填入形状；`panes` 的第一个元素占据主槽位。
    ///
    /// `next_split` 提供新的分屏身份，由工作区保证唯一。
    pub(crate) fn build(
        &self,
        panes: &[PaneId],
        next_split: &mut dyn FnMut() -> SplitId,
    ) -> PaneTree {
        assert_eq!(panes.len(), self.pane_count(), "方案与窗格数量必须一致",);
        self.shape.build(panes, next_split)
    }
}

/// 布局形状：窗格按顺序填入的树描述。
#[derive(Clone, Debug, PartialEq)]
enum LayoutShape {
    /// 单个窗格。
    Leaf,
    /// 沿轴均分为 `count` 份。
    Even { axis: Axis, count: usize },
    /// 两路分屏；`ratio` 是第一部分占用的比例。
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<LayoutShape>,
        second: Box<LayoutShape>,
    },
}

impl LayoutShape {
    fn pane_count(&self) -> usize {
        match self {
            Self::Leaf => 1,
            Self::Even { count, .. } => *count,
            Self::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }

    fn build(&self, panes: &[PaneId], next_split: &mut dyn FnMut() -> SplitId) -> PaneTree {
        match self {
            Self::Leaf => PaneTree::leaf(panes[0]),
            Self::Even { axis, count } => even(*axis, *count, panes, next_split),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let first_count = first.pane_count();
                let first = first.build(&panes[..first_count], next_split);
                let second = second.build(&panes[first_count..], next_split);
                PaneTree::split(next_split(), *axis, *ratio, first, second)
            }
        }
    }
}

/// 沿轴均分：第一份占 `1 / count`，其余递归排列。
fn even(
    axis: Axis,
    count: usize,
    panes: &[PaneId],
    next_split: &mut dyn FnMut() -> SplitId,
) -> PaneTree {
    if count <= 1 {
        return PaneTree::leaf(panes[0]);
    }
    let rest = even(axis, count - 1, &panes[1..], next_split);
    PaneTree::split(
        next_split(),
        axis,
        1. / count as f32,
        PaneTree::leaf(panes[0]),
        rest,
    )
}

fn split(axis: Axis, ratio: f32, first: LayoutShape, second: LayoutShape) -> LayoutShape {
    LayoutShape::Split {
        axis,
        ratio,
        first: Box::new(first),
        second: Box::new(second),
    }
}

fn leaf() -> LayoutShape {
    LayoutShape::Leaf
}

fn even_shape(axis: Axis, count: usize) -> LayoutShape {
    LayoutShape::Even { axis, count }
}

/// 按窗格数量给出的推荐方案；顺序与设计稿一致。
pub(crate) fn templates_for(count: usize) -> Vec<LayoutTemplate> {
    let template = |name: &'static str, shape: LayoutShape| LayoutTemplate { name, shape };

    match count {
        0 => Vec::new(),
        1 => vec![template("整窗", leaf())],
        2 => vec![
            template("左右", split(Axis::Horizontal, 0.5, leaf(), leaf())),
            template("上下", split(Axis::Vertical, 0.5, leaf(), leaf())),
            template("主次", split(Axis::Horizontal, MAIN_RATIO, leaf(), leaf())),
        ],
        3 => vec![
            template(
                "主次",
                split(
                    Axis::Horizontal,
                    MAIN_RATIO,
                    leaf(),
                    split(Axis::Vertical, 0.5, leaf(), leaf()),
                ),
            ),
            template(
                "左二右一",
                split(
                    Axis::Horizontal,
                    0.5,
                    split(Axis::Vertical, 0.5, leaf(), leaf()),
                    leaf(),
                ),
            ),
            template("三列", even_shape(Axis::Horizontal, 3)),
            template("三行", even_shape(Axis::Vertical, 3)),
            template(
                "上一下二",
                split(
                    Axis::Vertical,
                    0.5,
                    leaf(),
                    split(Axis::Horizontal, 0.5, leaf(), leaf()),
                ),
            ),
        ],
        4 => vec![
            template(
                "四宫格",
                split(
                    Axis::Horizontal,
                    0.5,
                    split(Axis::Vertical, 0.5, leaf(), leaf()),
                    split(Axis::Vertical, 0.5, leaf(), leaf()),
                ),
            ),
            template(
                "主次",
                split(
                    Axis::Horizontal,
                    MAIN_RATIO,
                    leaf(),
                    even_shape(Axis::Vertical, 3),
                ),
            ),
            template("四列", even_shape(Axis::Horizontal, 4)),
            template("四行", even_shape(Axis::Vertical, 4)),
        ],
        count => {
            let first_column = count.div_ceil(2);
            vec![
                template(
                    "网格",
                    split(
                        Axis::Horizontal,
                        0.5,
                        even_shape(Axis::Vertical, first_column),
                        even_shape(Axis::Vertical, count - first_column),
                    ),
                ),
                template("横排", even_shape(Axis::Horizontal, count)),
                template("竖排", even_shape(Axis::Vertical, count)),
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(value: u64) -> PaneId {
        PaneId(value)
    }

    fn panes(count: u64) -> Vec<PaneId> {
        (1..=count).map(pane).collect()
    }

    /// 构建时按顺序分配身份，与工作区的分配方式一致。
    fn build(template: &LayoutTemplate, panes: &[PaneId]) -> PaneTree {
        let mut next = 1_u64;
        template.build(panes, &mut || {
            let id = SplitId(next);
            next += 1;
            id
        })
    }

    #[test]
    fn template_list_matches_design_by_count() {
        let names = |count| {
            templates_for(count)
                .into_iter()
                .map(|template| template.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(1), vec!["整窗"]);
        assert_eq!(names(2), vec!["左右", "上下", "主次"]);
        assert_eq!(
            names(3),
            vec!["主次", "左二右一", "三列", "三行", "上一下二"]
        );
        assert_eq!(names(4), vec!["四宫格", "主次", "四列", "四行"]);
        assert_eq!(names(5), vec!["网格", "横排", "竖排"]);
        assert!(templates_for(0).is_empty());
    }

    #[test]
    fn templates_cover_every_pane_exactly_once() {
        let panes = panes(3);
        for template in templates_for(3) {
            assert_eq!(template.pane_count(), panes.len());
            // 第一个槽位是主窗格（活动窗格所在位置）。
            let tree = build(&template, &panes);
            let leaves = tree.leaves();
            assert_eq!(leaves.len(), panes.len(), "{}", template.name());
            let mut sorted = leaves.clone();
            sorted.sort();
            let mut expected = panes.clone();
            expected.sort();
            assert_eq!(sorted, expected, "{}", template.name());
        }
    }

    #[test]
    fn main_secondary_keeps_active_pane_in_main_slot() {
        let panes = panes(3);
        let template = templates_for(3).remove(0);
        let tree = build(&template, &panes);
        let geometry = tree.geometry();
        let main = geometry.pane_rect(pane(1)).expect("主窗格");
        assert!((main.width - MAIN_RATIO).abs() < 1e-6);
        assert!((main.height - 1.).abs() < 1e-6);

        let secondary = geometry.pane_rect(pane(2)).expect("次窗格");
        assert!((secondary.x - MAIN_RATIO).abs() < 1e-6);
        assert!((secondary.height - 0.5).abs() < 1e-6);
    }

    #[test]
    fn even_templates_split_equally() {
        let panes = panes(4);
        let template = templates_for(4)
            .into_iter()
            .find(|template| template.name == "四列")
            .expect("四列方案");
        let tree = build(&template, &panes);
        let geometry = tree.geometry();
        for (index, pane) in panes.iter().enumerate() {
            let rect = geometry.pane_rect(*pane).expect("窗格");
            assert!((rect.width - 0.25).abs() < 1e-6, "第 {index} 列");
            assert!((rect.x - index as f32 * 0.25).abs() < 1e-6, "第 {index} 列");
        }
    }

    #[test]
    fn grid_template_covers_every_pane() {
        let panes = panes(5);
        let template = templates_for(5)
            .into_iter()
            .find(|template| template.name == "网格")
            .expect("网格方案");
        let tree = build(&template, &panes);
        let geometry = tree.geometry();
        assert_eq!(geometry.panes.len(), panes.len());
        // 三加二：左列三个窗格，右列两个。
        assert!(geometry.pane_rect(pane(3)).expect("左列第三个").y > 0.6);
        assert!(geometry.pane_rect(pane(4)).expect("右列第一个").x > 0.4);
    }

    #[test]
    fn templates_reuse_workspace_split_identity() {
        // 构建使用调用方提供的身份分配；预览与真实应用共用这一入口。
        let template = templates_for(2).remove(0);
        let mut allocated = Vec::new();
        let tree = template.build(&panes(2), &mut || {
            let id = SplitId(41 + allocated.len() as u64);
            allocated.push(id);
            id
        });
        assert_eq!(allocated, vec![SplitId(41)]);
        assert!(tree.split_node(SplitId(41)).is_some());
    }
}
