//! 分屏树：标签页内的窗格布局模型。
//!
//! 纯逻辑，不依赖 GPUI：身份、结构操作、几何与最小尺寸约束都可以
//! 单独测试。几何用 0–1 比例表达，最终像素布局由布局元素完成。

/// 标签页身份；标签顺序变化不影响身份。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub(crate) u64);

/// 窗格身份；与终端会话视图一一对应。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(pub(crate) u64);

/// 分屏节点身份；分隔线拖动与布局操作使用稳定键。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SplitId(pub(crate) u64);

impl TabId {
    pub(crate) fn value(self) -> u64 {
        self.0
    }
}

impl PaneId {
    pub(crate) fn value(self) -> u64 {
        self.0
    }
}

impl SplitId {
    pub(crate) fn value(self) -> u64 {
        self.0
    }
}

/// 分屏方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// 左右排列（向右分屏）。
    Horizontal,
    /// 上下排列（向下分屏）。
    Vertical,
}

/// 聚焦移动方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// 向左。
    Left,
    /// 向右。
    Right,
    /// 向上。
    Up,
    /// 向下。
    Down,
}

/// 分屏树：叶节点是一个窗格，分屏节点把区域分成两份。
#[derive(Clone, Debug, PartialEq)]
pub enum PaneTree {
    /// 单个窗格。
    Leaf(PaneId),
    /// 两路分屏；`ratio` 是第一个子区域占用的比例。
    Split {
        /// 节点身份。
        id: SplitId,
        /// 排列方向。
        axis: Axis,
        /// 第一个子区域的比例（0–1）。
        ratio: f32,
        /// 靠近原点的子区域（左 / 上）。
        first: Box<PaneTree>,
        /// 远离原点的子区域（右 / 下）。
        second: Box<PaneTree>,
    },
}

/// 相对父容器的矩形区域（0–1 比例）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneRect {
    /// 左边界。
    pub x: f32,
    /// 上边界。
    pub y: f32,
    /// 宽度。
    pub width: f32,
    /// 高度。
    pub height: f32,
}

impl PaneRect {
    /// 区域中心。
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.width / 2., self.y + self.height / 2.)
    }
}

/// 分屏节点在父容器中的区域。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitRect {
    /// 节点身份。
    pub id: SplitId,
    /// 排列方向。
    pub axis: Axis,
    /// 节点占用的区域。
    pub rect: PaneRect,
}

/// 树中所有窗格与分屏节点的几何。
#[derive(Clone, Debug, Default)]
pub struct TreeGeometry {
    /// 窗格区域，按树的先序排列。
    pub panes: Vec<(PaneId, PaneRect)>,
    /// 分屏节点区域。
    pub splits: Vec<SplitRect>,
}

impl TreeGeometry {
    /// 指定窗格的区域。
    pub fn pane_rect(&self, pane: PaneId) -> Option<PaneRect> {
        self.panes
            .iter()
            .find(|(id, _)| *id == pane)
            .map(|(_, rect)| *rect)
    }
}

/// 窗格最小宽度（rem，与设计稿一致）。
pub const MIN_PANE_WIDTH_REMS: f32 = 12.;
/// 窗格最小高度（rem，与设计稿一致）。
pub const MIN_PANE_HEIGHT_REMS: f32 = 8.;
/// 分隔线拖动时子区域的比例下限。
pub const MIN_RATIO: f32 = 0.15;
/// 分隔线拖动时子区域的比例上限。
pub const MAX_RATIO: f32 = 0.85;

impl PaneTree {
    /// 创建单窗格树。
    pub fn leaf(pane: PaneId) -> Self {
        Self::Leaf(pane)
    }

    /// 创建两路分屏；`ratio` 是第一个子区域占用的比例。
    pub fn split(id: SplitId, axis: Axis, ratio: f32, first: Self, second: Self) -> Self {
        Self::Split {
            id,
            axis,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// 按顺序收集所有窗格。
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut leaves = Vec::new();
        self.collect_leaves(&mut leaves);
        leaves
    }

    fn collect_leaves(&self, leaves: &mut Vec<PaneId>) {
        match self {
            Self::Leaf(pane) => leaves.push(*pane),
            Self::Split { first, second, .. } => {
                first.collect_leaves(leaves);
                second.collect_leaves(leaves);
            }
        }
    }

    /// 树中是否包含指定窗格。
    pub fn contains(&self, pane: PaneId) -> bool {
        match self {
            Self::Leaf(leaf) => *leaf == pane,
            Self::Split { first, second, .. } => first.contains(pane) || second.contains(pane),
        }
    }

    /// 在目标窗格处插入新窗格；目标不属于本树时返回 `false`。
    pub fn split_at(&mut self, target: PaneId, id: SplitId, axis: Axis, new_pane: PaneId) -> bool {
        let (tree, inserted) = std::mem::replace(self, Self::Leaf(new_pane))
            .split_at_inner(target, id, axis, new_pane);
        *self = tree;
        inserted
    }

    fn split_at_inner(
        self,
        target: PaneId,
        id: SplitId,
        axis: Axis,
        new_pane: PaneId,
    ) -> (PaneTree, bool) {
        match self {
            Self::Leaf(pane) if pane == target => (
                Self::Split {
                    id,
                    axis,
                    ratio: 0.5,
                    first: Box::new(Self::Leaf(pane)),
                    second: Box::new(Self::Leaf(new_pane)),
                },
                true,
            ),
            Self::Leaf(pane) => (Self::Leaf(pane), false),
            Self::Split {
                id: node_id,
                axis: node_axis,
                ratio,
                first,
                second,
            } => {
                let (first, first_inserted) = first.split_at_inner(target, id, axis, new_pane);
                if first_inserted {
                    return (
                        Self::Split {
                            id: node_id,
                            axis: node_axis,
                            ratio,
                            first: Box::new(first),
                            second,
                        },
                        true,
                    );
                }
                let (second, second_inserted) = second.split_at_inner(target, id, axis, new_pane);
                (
                    Self::Split {
                        id: node_id,
                        axis: node_axis,
                        ratio,
                        first: Box::new(first),
                        second: Box::new(second),
                    },
                    second_inserted,
                )
            }
        }
    }

    /// 移除窗格并折叠只含一个子节点的分屏；树清空时返回 `None`。
    pub fn without(self, pane: PaneId) -> Option<PaneTree> {
        match self {
            Self::Leaf(leaf) => (leaf != pane).then_some(Self::Leaf(leaf)),
            Self::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => {
                let first = first.without(pane);
                let second = second.without(pane);
                match (first, second) {
                    (Some(first), Some(second)) => Some(Self::Split {
                        id,
                        axis,
                        ratio,
                        first: Box::new(first),
                        second: Box::new(second),
                    }),
                    (Some(tree), None) | (None, Some(tree)) => Some(tree),
                    (None, None) => None,
                }
            }
        }
    }

    /// 设置指定分屏节点的比例；节点不存在时返回 `false`。
    ///
    /// 比例按 [`clamp_ratio`] 的规则生效；调用方负责用当次布局的
    /// 像素尺寸计算边界，模型侧只做兜底范围的限制。
    pub fn set_ratio(&mut self, id: SplitId, ratio: f32) -> bool {
        match self {
            Self::Leaf(_) => false,
            Self::Split {
                id: node_id,
                ratio: node_ratio,
                first,
                second,
                ..
            } => {
                if *node_id == id {
                    *node_ratio = ratio.clamp(MIN_RATIO, MAX_RATIO);
                    return true;
                }
                first.set_ratio(id, ratio) || second.set_ratio(id, ratio)
            }
        }
    }

    /// 指定分屏节点的方向与当前比例；节点不存在时为 `None`。
    pub fn split_node(&self, id: SplitId) -> Option<(Axis, f32)> {
        match self {
            Self::Leaf(_) => None,
            Self::Split {
                id: node_id,
                axis,
                ratio,
                first,
                second,
            } => {
                if *node_id == id {
                    Some((*axis, *ratio))
                } else {
                    first.split_node(id).or_else(|| second.split_node(id))
                }
            }
        }
    }

    /// 交换两个窗格的位置，保持树的形状；任一窗格不属于本树时返回 `false`。
    ///
    /// 交换只改变窗格在布局中的位置：会话、输出与选择都随窗格身份保留。
    pub fn swap(&mut self, first: PaneId, second: PaneId) -> bool {
        if !self.contains(first) || !self.contains(second) {
            return false;
        }
        self.swap_inner(first, second);
        true
    }

    fn swap_inner(&mut self, first: PaneId, second: PaneId) {
        match self {
            Self::Leaf(pane) => {
                if *pane == first {
                    *pane = second;
                } else if *pane == second {
                    *pane = first;
                }
            }
            Self::Split {
                first: first_tree,
                second: second_tree,
                ..
            } => {
                first_tree.swap_inner(first, second);
                second_tree.swap_inner(first, second);
            }
        }
    }

    /// 只比较形状（方向与比例，忽略身份）；布局是否需要重排用它判断。
    pub fn same_shape(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Leaf(_), Self::Leaf(_)) => true,
            (
                Self::Split {
                    axis,
                    ratio,
                    first,
                    second,
                    ..
                },
                Self::Split {
                    axis: other_axis,
                    ratio: other_ratio,
                    first: other_first,
                    second: other_second,
                    ..
                },
            ) => {
                axis == other_axis
                    && (ratio - other_ratio).abs() < 1e-3
                    && first.same_shape(other_first)
                    && second.same_shape(other_second)
            }
            _ => false,
        }
    }

    /// 计算所有窗格与分屏节点的几何。
    pub fn geometry(&self) -> TreeGeometry {
        let mut geometry = TreeGeometry::default();
        self.layout(
            PaneRect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
            &mut geometry,
        );
        geometry
    }

    fn layout(&self, rect: PaneRect, out: &mut TreeGeometry) {
        match self {
            Self::Leaf(pane) => out.panes.push((*pane, rect)),
            Self::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => {
                out.splits.push(SplitRect {
                    id: *id,
                    axis: *axis,
                    rect,
                });
                let (first_rect, second_rect) = sub_rects(rect, *axis, *ratio);
                first.layout(first_rect, out);
                second.layout(second_rect, out);
            }
        }
    }

    /// 子树在指定方向上占据的最小尺寸（rem）。
    ///
    /// 与分屏方向一致时子区域并列，最小尺寸相加；方向不一致时
    /// 子区域重叠占用同一条带，取较大者。
    pub fn min_extent(&self, axis: Axis) -> f32 {
        match self {
            Self::Leaf(_) => match axis {
                Axis::Horizontal => MIN_PANE_WIDTH_REMS,
                Axis::Vertical => MIN_PANE_HEIGHT_REMS,
            },
            Self::Split {
                axis: node_axis,
                first,
                second,
                ..
            } => {
                if *node_axis == axis {
                    first.min_extent(axis) + second.min_extent(axis)
                } else {
                    first.min_extent(axis).max(second.min_extent(axis))
                }
            }
        }
    }
}

/// 按比例把区域分成两份。
pub fn sub_rects(rect: PaneRect, axis: Axis, ratio: f32) -> (PaneRect, PaneRect) {
    let ratio = ratio.clamp(0., 1.);
    match axis {
        Axis::Horizontal => {
            let first_width = rect.width * ratio;
            (
                PaneRect {
                    width: first_width,
                    ..rect
                },
                PaneRect {
                    x: rect.x + first_width,
                    width: rect.width - first_width,
                    ..rect
                },
            )
        }
        Axis::Vertical => {
            let first_height = rect.height * ratio;
            (
                PaneRect {
                    height: first_height,
                    ..rect
                },
                PaneRect {
                    y: rect.y + first_height,
                    height: rect.height - first_height,
                    ..rect
                },
            )
        }
    }
}

/// 按最小尺寸约束限制拖动得到的分屏比例。
///
/// 两侧子区域都不小于各自的最小尺寸；比例同时保持在
/// [`MIN_RATIO`]–[`MAX_RATIO`] 的设计范围内。
pub fn clamp_ratio(ratio: f32, extent: f32, min_first: f32, min_second: f32) -> f32 {
    if extent <= 0. {
        return ratio.clamp(MIN_RATIO, MAX_RATIO);
    }
    let lower = (min_first / extent).clamp(MIN_RATIO, 0.45);
    let upper = (1. - min_second / extent).clamp(0.55, MAX_RATIO);
    ratio.clamp(lower, upper)
}

/// 在指定方向上寻找中心最近且位于该方向的窗格。
pub fn nearest_in_direction(
    geometry: &TreeGeometry,
    from: PaneId,
    direction: Direction,
) -> Option<PaneId> {
    let source = geometry.pane_rect(from)?;
    let (source_x, source_y) = source.center();
    let mut best: Option<(PaneId, f32)> = None;

    for (pane, rect) in &geometry.panes {
        if *pane == from {
            continue;
        }
        let (x, y) = rect.center();
        let (dx, dy) = (x - source_x, y - source_y);

        let in_direction = match direction {
            Direction::Left => dx < 0.,
            Direction::Right => dx > 0.,
            Direction::Up => dy < 0.,
            Direction::Down => dy > 0.,
        };
        if !in_direction {
            continue;
        }

        // 主方向距离优先，垂直方向的偏差加两倍权重。
        let cost = match direction {
            Direction::Left | Direction::Right => dx.abs() + 2. * dy.abs(),
            Direction::Up | Direction::Down => dy.abs() + 2. * dx.abs(),
        };
        if best.is_none_or(|(_, best_cost)| cost < best_cost) {
            best = Some((*pane, cost));
        }
    }

    best.map(|(pane, _)| pane)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(value: u64) -> PaneId {
        PaneId(value)
    }

    fn split_id(value: u64) -> SplitId {
        SplitId(value)
    }

    fn sample_tree() -> PaneTree {
        // 左侧一个窗格，右侧上下两个：与设计稿的初始布局一致。
        PaneTree::Split {
            id: split_id(1),
            axis: Axis::Horizontal,
            ratio: 2. / 3.,
            first: Box::new(PaneTree::leaf(pane(1))),
            second: Box::new(PaneTree::split(
                split_id(2),
                Axis::Vertical,
                0.5,
                PaneTree::leaf(pane(2)),
                PaneTree::leaf(pane(3)),
            )),
        }
    }

    #[test]
    fn leaves_follow_tree_order() {
        let tree = sample_tree();
        assert_eq!(tree.leaves(), vec![pane(1), pane(2), pane(3)]);
        assert!(tree.contains(pane(2)));
        assert!(!tree.contains(pane(9)));
    }

    #[test]
    fn split_at_inserts_beside_target() {
        let mut tree = sample_tree();
        assert!(tree.split_at(pane(2), split_id(3), Axis::Horizontal, pane(4)));
        assert_eq!(tree.leaves(), vec![pane(1), pane(2), pane(4), pane(3)]);

        let inner = match &tree {
            PaneTree::Split { second, .. } => second,
            _ => panic!("root split expected"),
        };
        let nested = match inner.as_ref() {
            PaneTree::Split { first, .. } => first,
            _ => panic!("nested split expected"),
        };
        match nested.as_ref() {
            PaneTree::Split { first, second, .. } => {
                assert_eq!(first.as_ref(), &PaneTree::Leaf(pane(2)));
                assert_eq!(second.as_ref(), &PaneTree::Leaf(pane(4)));
            }
            _ => panic!("split at target expected"),
        }
    }

    #[test]
    fn split_at_rejects_unknown_target() {
        let mut tree = sample_tree();
        let original = tree.clone();
        assert!(!tree.split_at(pane(9), split_id(3), Axis::Horizontal, pane(4)));
        assert_eq!(tree, original);
    }

    #[test]
    fn without_collapses_single_child_split() {
        // 移除右侧下方的窗格后，上下分屏折叠，右侧只剩下方的窗格。
        let tree = sample_tree().without(pane(3)).expect("tree remains");
        assert_eq!(tree.leaves(), vec![pane(1), pane(2)]);
        match tree {
            PaneTree::Split { first, second, .. } => {
                assert_eq!(first.as_ref(), &PaneTree::Leaf(pane(1)));
                assert_eq!(second.as_ref(), &PaneTree::Leaf(pane(2)));
            }
            _ => panic!("root split expected"),
        }
    }

    #[test]
    fn without_last_pane_empties_tree() {
        let tree = PaneTree::leaf(pane(1));
        assert!(tree.without(pane(1)).is_none());
    }

    #[test]
    fn geometry_uses_ratios() {
        let geometry = sample_tree().geometry();
        let left = geometry.pane_rect(pane(1)).expect("left pane");
        assert!((left.width - 2. / 3.).abs() < 1e-6);
        assert!((left.height - 1.).abs() < 1e-6);

        let top_right = geometry.pane_rect(pane(2)).expect("top right pane");
        assert!((top_right.x - 2. / 3.).abs() < 1e-6);
        assert!((top_right.height - 0.5).abs() < 1e-6);
        let bottom_right = geometry.pane_rect(pane(3)).expect("bottom right pane");
        assert!((bottom_right.y - 0.5).abs() < 1e-6);
    }

    #[test]
    fn set_ratio_updates_split_and_clamps() {
        let mut tree = sample_tree();
        assert!(tree.set_ratio(split_id(2), 0.8));
        let geometry = tree.geometry();
        let top_right = geometry.pane_rect(pane(2)).expect("top right pane");
        assert!((top_right.height - 0.8).abs() < 1e-6);

        // 超出范围的比例被限制在 15%–85%。
        assert!(tree.set_ratio(split_id(2), 0.02));
        assert!(!tree.set_ratio(split_id(9), 0.5));
        let geometry = tree.geometry();
        let top_right = geometry.pane_rect(pane(2)).expect("top right pane");
        assert!((top_right.height - MIN_RATIO).abs() < 1e-6);
    }

    #[test]
    fn min_extent_sums_along_axis_and_maxes_across() {
        let tree = sample_tree();
        // 水平方向：左侧窗格 + 右侧条带（右侧两窗格上下排列，宽度取较大者）。
        assert!((tree.min_extent(Axis::Horizontal) - (12. + 12.)).abs() < 1e-6);
        // 垂直方向：左列与右侧上下相加取较大者。
        assert!((tree.min_extent(Axis::Vertical) - (8. + 8.)).abs() < 1e-6);
    }

    #[test]
    fn swap_exchanges_positions_and_keeps_shape() {
        let mut tree = sample_tree();
        let shape = tree.clone();

        // 跨子树交换：左窗格与右下窗格互换。
        assert!(tree.swap(pane(1), pane(3)));
        assert_eq!(tree.leaves(), vec![pane(3), pane(2), pane(1)]);
        assert!(tree.same_shape(&shape), "交换不改变布局形状");
        assert_eq!(
            tree.geometry().pane_rect(pane(3)),
            shape.geometry().pane_rect(pane(1)),
        );

        // 同一子树内交换。
        assert!(tree.swap(pane(2), pane(1)));
        assert_eq!(tree.leaves(), vec![pane(3), pane(1), pane(2)]);

        // 未知窗格不改变结构。
        let before = tree.clone();
        assert!(!tree.swap(pane(2), pane(9)));
        assert!(!tree.swap(pane(9), pane(2)));
        assert_eq!(tree, before);
    }

    #[test]
    fn split_node_reads_axis_and_ratio() {
        let tree = sample_tree();
        assert_eq!(
            tree.split_node(split_id(1)),
            Some((Axis::Horizontal, 2. / 3.))
        );
        assert_eq!(tree.split_node(split_id(2)), Some((Axis::Vertical, 0.5)));
        assert_eq!(tree.split_node(split_id(9)), None);
        assert_eq!(PaneTree::leaf(pane(1)).split_node(split_id(1)), None);
    }

    #[test]
    fn same_shape_ignores_identity_and_ratio_noise() {
        let tree = sample_tree();
        let mut moved = tree.clone();
        assert!(moved.swap(pane(1), pane(3)));
        assert!(moved.same_shape(&tree));

        let mirrored = PaneTree::Split {
            id: split_id(7),
            axis: Axis::Horizontal,
            ratio: 2. / 3.,
            first: Box::new(PaneTree::leaf(pane(9))),
            second: Box::new(PaneTree::split(
                split_id(8),
                Axis::Vertical,
                0.5,
                PaneTree::leaf(pane(8)),
                PaneTree::leaf(pane(7)),
            )),
        };
        assert!(mirrored.same_shape(&tree));

        let different = PaneTree::split(
            split_id(1),
            Axis::Vertical,
            0.5,
            PaneTree::leaf(pane(1)),
            PaneTree::leaf(pane(2)),
        );
        assert!(!different.same_shape(&tree));
    }

    #[test]
    fn clamp_ratio_respects_minimum_sizes() {
        // 1000px 宽，左侧最小 240px，右侧最小 240px：比例限制在 0.24–0.76。
        let ratio = clamp_ratio(0.1, 1000., 240., 240.);
        assert!((ratio - 0.24).abs() < 1e-6);
        let ratio = clamp_ratio(0.9, 1000., 240., 240.);
        assert!((ratio - 0.76).abs() < 1e-6);
        // 空间充足时保持设计范围：15%–85%。
        let ratio = clamp_ratio(0.05, 4000., 240., 240.);
        assert!((ratio - MIN_RATIO).abs() < 1e-6);
        // 空间不足时最低/最高限制仍然有效，不会反转。
        let ratio = clamp_ratio(0.5, 300., 240., 240.);
        assert!((ratio - 0.5).abs() < 1e-6);
    }

    #[test]
    fn nearest_in_direction_picks_full_side_neighbor() {
        let tree = sample_tree();
        let geometry = tree.geometry();
        assert_eq!(
            nearest_in_direction(&geometry, pane(1), Direction::Right),
            Some(pane(2))
        );
        assert_eq!(
            nearest_in_direction(&geometry, pane(2), Direction::Down),
            Some(pane(3))
        );
        assert_eq!(
            nearest_in_direction(&geometry, pane(3), Direction::Up),
            Some(pane(2))
        );
        assert_eq!(
            nearest_in_direction(&geometry, pane(2), Direction::Left),
            Some(pane(1))
        );
        // 边界方向没有候选窗格时不移动。
        assert_eq!(
            nearest_in_direction(&geometry, pane(1), Direction::Left),
            None
        );
        assert_eq!(
            nearest_in_direction(&geometry, pane(2), Direction::Up),
            None
        );
        assert_eq!(
            nearest_in_direction(&geometry, pane(3), Direction::Right),
            None
        );
    }
}
