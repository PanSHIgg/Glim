//! 工作区恢复：`workspace.toml` 的结构、收集与恢复。
//!
//! 只保存布局、工作目录、shell 引用与窗口状态；恢复时为每个窗格启动
//! 新会话，不保存后台会话，也不重放历史输出。读取失败或数据无效时
//! 整份忽略并把问题留给启动提示，绝不阻塞应用启动。

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use gpui_kit::{App, Bounds, Global, WindowBounds, px, size};
use serde::{Deserialize, Serialize};

use crate::pane_tree::{Axis, MAX_RATIO, MIN_RATIO, PaneId, PaneTree, SplitId};

/// 工作区数据文件名。
pub(crate) const WORKSPACE_FILE_NAME: &str = "workspace.toml";
/// 恢复数据版本；不匹配时整份忽略。
const STATE_VERSION: u32 = 1;
/// 恢复上限：超出视为损坏，避免一次启动大量进程。
const MAX_TABS: usize = 16;
/// 单个标签页的窗格上限。
const MAX_PANES_PER_TAB: usize = 16;
/// 恢复窗口的最小尺寸（产品最小窗口）。
const MIN_WINDOW_WIDTH: f32 = 800.;
/// 恢复窗口的最小高度。
const MIN_WINDOW_HEIGHT: f32 = 600.;
/// 恢复窗口的尺寸上限（防止损坏数据创建超大窗口）。
const MAX_WINDOW_DIMENSION: f32 = 20_000.;

/// 工作区恢复数据。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WorkspaceState {
    /// 数据版本。
    pub(crate) version: u32,
    /// 窗口状态。
    pub(crate) window: Option<WindowState>,
    /// 活动标签页下标。
    pub(crate) active_tab: usize,
    /// 标签页。
    pub(crate) tabs: Vec<TabState>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            window: None,
            active_tab: 0,
            tabs: Vec::new(),
        }
    }
}

impl WorkspaceState {
    /// 校验并收敛数据；返回错误说明时调用方应整份忽略。
    pub(crate) fn validate(&mut self) -> Result<(), String> {
        if self.version != STATE_VERSION {
            return Err(format!("版本 {} 不受支持", self.version));
        }
        if self.tabs.is_empty() {
            return Err("没有标签页".to_owned());
        }
        if self.tabs.len() > MAX_TABS {
            return Err(format!("标签页数量 {} 超过上限", self.tabs.len()));
        }
        for tab in &mut self.tabs {
            let panes = match &mut tab.tree {
                Some(node) => count_and_clamp(node)?,
                None => 0,
            };
            if panes > MAX_PANES_PER_TAB {
                return Err(format!("窗格数量 {panes} 超过上限"));
            }
            let active = tab.active_pane.filter(|index| *index < panes).unwrap_or(0);
            tab.active_pane = if panes == 0 { None } else { Some(active) };
        }
        if self.active_tab >= self.tabs.len() {
            self.active_tab = 0;
        }
        Ok(())
    }
}

/// 窗口状态：尺寸与是否最大化；位置由系统在启动时决定。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WindowState {
    /// 窗口宽度（逻辑像素）。
    pub(crate) width: f32,
    /// 窗口高度（逻辑像素）。
    pub(crate) height: f32,
    /// 是否最大化。
    pub(crate) maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 1200.,
            height: 750.,
            maximized: false,
        }
    }
}

impl WindowState {
    /// 读取当前窗口状态。
    pub(crate) fn from_window(window: &gpui_kit::Window) -> Self {
        let bounds = match window.window_bounds() {
            WindowBounds::Maximized(bounds) => {
                return Self {
                    width: f32::from(bounds.size.width),
                    height: f32::from(bounds.size.height),
                    maximized: true,
                };
            }
            WindowBounds::Windowed(bounds) => bounds,
            WindowBounds::Fullscreen(bounds) => bounds,
        };
        Self {
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
            maximized: false,
        }
    }

    /// 恢复时的窗口边界；尺寸收敛到产品最小窗口与合理上限。
    pub(crate) fn to_window_bounds(&self, cx: &App) -> WindowBounds {
        let width = self.width.clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_DIMENSION);
        let height = self.height.clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_DIMENSION);
        let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
        if self.maximized {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        }
    }
}

/// 单个标签页的恢复数据。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct TabState {
    /// 分屏树；`None` 表示空标签页。
    pub(crate) tree: Option<LayoutNode>,
    /// 活动窗格的叶序下标。
    pub(crate) active_pane: Option<usize>,
}

/// 恢复数据中的布局节点：叶节点（目录与 shell 引用）或分屏。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LayoutNode {
    /// 叶节点：工作目录。
    pub(crate) directory: Option<String>,
    /// 叶节点：启动该窗格使用的 shell 程序引用。
    pub(crate) shell: Option<String>,
    /// 分屏节点；存在时忽略叶字段。
    pub(crate) split: Option<SplitNode>,
}

/// 恢复数据中的分屏节点。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SplitNode {
    /// 排列方向。
    pub(crate) axis: Axis,
    /// 第一个子区域的比例。
    pub(crate) ratio: f32,
    /// 靠近原点的子区域。
    pub(crate) first: Box<LayoutNode>,
    /// 远离原点的子区域。
    pub(crate) second: Box<LayoutNode>,
}

/// 叶节点的启动信息。
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LeafState {
    /// 工作目录。
    pub(crate) directory: Option<PathBuf>,
    /// shell 程序引用。
    pub(crate) shell: Option<String>,
}

/// 校验节点并收敛比例；返回叶节点数量。
fn count_and_clamp(node: &mut LayoutNode) -> Result<usize, String> {
    match &mut node.split {
        Some(split) => {
            if !split.ratio.is_finite() {
                return Err("分屏比例不是有效数字".to_owned());
            }
            split.ratio = split.ratio.clamp(MIN_RATIO, MAX_RATIO);
            let first = count_and_clamp(&mut split.first)?;
            let second = count_and_clamp(&mut split.second)?;
            Ok(first + second)
        }
        None => Ok(1),
    }
}

/// 收集恢复数据中的叶节点，按树序排列。
pub(crate) fn leaves(node: &LayoutNode) -> Vec<LeafState> {
    let mut leaves = Vec::new();
    collect_leaves(node, &mut leaves);
    leaves
}

fn collect_leaves(node: &LayoutNode, out: &mut Vec<LeafState>) {
    match &node.split {
        Some(split) => {
            collect_leaves(&split.first, out);
            collect_leaves(&split.second, out);
        }
        None => out.push(LeafState {
            directory: node
                .directory
                .as_deref()
                .map(str::trim)
                .filter(|directory| !directory.is_empty())
                .map(PathBuf::from),
            shell: node
                .shell
                .as_deref()
                .map(str::trim)
                .filter(|shell| !shell.is_empty())
                .map(str::to_owned),
        }),
    }
}

/// 由恢复数据构建分屏树；窗格身份按树序取自 `panes`。
///
/// 调用方先用 [`leaves`] 保证数量一致；数量不足时视为数据损坏并跳过
/// 缺失的叶节点（不会 panic）。
pub(crate) fn build_tree(
    node: &LayoutNode,
    panes: &mut std::vec::IntoIter<PaneId>,
    next_split: &mut impl FnMut() -> SplitId,
) -> Option<PaneTree> {
    match &node.split {
        Some(split) => {
            let first = build_tree(&split.first, panes, next_split)?;
            let second = build_tree(&split.second, panes, next_split)?;
            Some(PaneTree::split(
                next_split(),
                split.axis,
                split.ratio,
                first,
                second,
            ))
        }
        None => panes.next().map(PaneTree::leaf),
    }
}

/// 收集分屏树为恢复数据；叶节点信息由 `leaf` 提供。
pub(crate) fn collect_tree(
    tree: &PaneTree,
    leaf: &mut impl FnMut(PaneId) -> LeafState,
) -> LayoutNode {
    match tree {
        PaneTree::Leaf(pane) => {
            let state = leaf(*pane);
            LayoutNode {
                directory: state
                    .directory
                    .map(|directory| directory.to_string_lossy().into_owned()),
                shell: state.shell,
                split: None,
            }
        }
        PaneTree::Split {
            axis,
            ratio,
            first,
            second,
            ..
        } => LayoutNode {
            directory: None,
            shell: None,
            split: Some(SplitNode {
                axis: *axis,
                ratio: *ratio,
                first: Box::new(collect_tree(first, leaf)),
                second: Box::new(collect_tree(second, leaf)),
            }),
        },
    }
}

/// 组装一份待保存的工作区数据。
pub(crate) fn new_state(
    window: &gpui_kit::Window,
    active_tab: usize,
    tabs: Vec<TabState>,
) -> WorkspaceState {
    WorkspaceState {
        version: STATE_VERSION,
        window: Some(WindowState::from_window(window)),
        active_tab,
        tabs,
    }
}

/// 解析恢复目录：目录不存在时回退到用户目录并报告。
pub(crate) fn resolve_directory(directory: Option<&Path>) -> (Option<PathBuf>, bool) {
    match directory {
        Some(directory) if directory.is_dir() => (Some(directory.to_path_buf()), false),
        Some(_) => (glim_settings::home_directory(), true),
        None => (None, false),
    }
}

/// 工作区数据文件路径。
pub(crate) fn file_in(directory: &Path) -> PathBuf {
    directory.join(WORKSPACE_FILE_NAME)
}

/// 工作区持久化全局：文件位置、窗口状态与启动时读取的恢复数据。
pub(crate) struct WorkspaceStorage {
    path: Option<PathBuf>,
    window: Option<WindowState>,
    restore: RefCell<Option<WorkspaceState>>,
    /// 启动提示（数据无效、目录回退等）；主入口在窗口打开后通知。
    warnings: RefCell<Vec<String>>,
}

impl Global for WorkspaceStorage {}

impl WorkspaceStorage {
    fn new(
        path: Option<PathBuf>,
        window: Option<WindowState>,
        restore: Option<WorkspaceState>,
    ) -> Self {
        Self {
            path,
            window,
            restore: RefCell::new(restore),
            warnings: RefCell::new(Vec::new()),
        }
    }

    /// 记录一条启动提示。
    pub(crate) fn warn(&self, message: impl Into<String>) {
        self.warnings.borrow_mut().push(message.into());
    }
}

/// 读取工作区数据并登记全局；`data_directory` 为 `None` 时不持久化。
pub fn init(cx: &mut App, data_directory: Option<PathBuf>) {
    let mut storage = WorkspaceStorage::new(None, None, None);
    if let Some(directory) = data_directory {
        let path = file_in(&directory);
        storage.path = Some(path.clone());
        match glim_settings::load_toml::<WorkspaceState>(&path) {
            Ok(Some(mut state)) => match state.validate() {
                Ok(()) => {
                    storage.window = state.window.clone();
                    storage.restore = RefCell::new(Some(state));
                }
                Err(message) => storage.warn(format!(
                    "工作区恢复数据无效（{message}），已按默认工作区启动"
                )),
            },
            Ok(None) => {}
            Err(error) => storage.warn(format!("{error}，已按默认工作区启动")),
        }
    }
    cx.set_global(storage);
}

/// 取出启动时读取的恢复数据；只生效一次。
pub(crate) fn take_restore(cx: &App) -> Option<WorkspaceState> {
    let storage = cx.try_global::<WorkspaceStorage>()?;
    storage.restore.borrow_mut().take()
}

/// 记录启动提示；未登记持久化时忽略。
pub(crate) fn push_warning(cx: &App, message: impl Into<String>) {
    if let Some(storage) = cx.try_global::<WorkspaceStorage>() {
        storage.warn(message);
    }
}

/// 取出启动提示。
pub fn take_startup_warnings(cx: &App) -> Vec<String> {
    cx.try_global::<WorkspaceStorage>()
        .map(|storage| std::mem::take(&mut *storage.warnings.borrow_mut()))
        .unwrap_or_default()
}

/// 上次退出时的窗口边界；没有可用数据时为 `None`。
pub fn window_bounds(cx: &App) -> Option<WindowBounds> {
    let window = cx.try_global::<WorkspaceStorage>()?.window.as_ref()?;
    Some(window.to_window_bounds(cx))
}

/// 工作区数据文件路径；未启用持久化时为 `None`。
pub(crate) fn workspace_file(cx: &App) -> Option<PathBuf> {
    cx.try_global::<WorkspaceStorage>()?.path.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane_tree::PaneId;

    fn pane(value: u64) -> PaneId {
        PaneId(value)
    }

    /// 左侧一个窗格，右侧上下两个：与设计稿的初始布局一致。
    fn sample_tree() -> PaneTree {
        PaneTree::split(
            SplitId(1),
            Axis::Horizontal,
            2. / 3.,
            PaneTree::leaf(pane(1)),
            PaneTree::split(
                SplitId(2),
                Axis::Vertical,
                0.5,
                PaneTree::leaf(pane(2)),
                PaneTree::leaf(pane(3)),
            ),
        )
    }

    #[test]
    fn collect_then_build_keeps_shape_and_order() {
        let tree = sample_tree();
        let node = collect_tree(&tree, &mut |pane| LeafState {
            directory: Some(PathBuf::from(format!("C:\\dir\\{}", pane.value()))),
            shell: (pane.value() == 2).then(|| "pwsh.exe".to_owned()),
        });

        let text = toml::to_string_pretty(&WorkspaceState {
            version: STATE_VERSION,
            window: Some(WindowState {
                width: 1000.,
                height: 700.,
                maximized: true,
            }),
            active_tab: 0,
            tabs: vec![TabState {
                tree: Some(node),
                active_pane: Some(2),
            }],
        })
        .expect("serialize");
        let mut state: WorkspaceState = toml::from_str(&text).expect("parse");
        state.validate().expect("valid");

        let node = &state.tabs[0].tree.clone().expect("tree");
        let leaves = leaves(node);
        assert_eq!(
            leaves
                .iter()
                .map(|leaf| leaf.shell.clone())
                .collect::<Vec<_>>(),
            vec![None, Some("pwsh.exe".to_owned()), None]
        );
        assert_eq!(leaves[2].directory, Some(PathBuf::from("C:\\dir\\3")));

        let mut panes = vec![pane(10), pane(11), pane(12)].into_iter();
        let rebuilt = build_tree(node, &mut panes, &mut || SplitId(9)).expect("tree");
        assert!(rebuilt.same_shape(&tree), "形状保持");
        assert_eq!(rebuilt.leaves(), vec![pane(10), pane(11), pane(12)]);
        assert!(panes.next().is_none(), "窗格按叶序消费");
    }

    #[test]
    fn validate_rejects_bad_versions_and_oversized_data() {
        let mut wrong_version = WorkspaceState {
            version: 99,
            ..WorkspaceState::default()
        };
        wrong_version.tabs.push(TabState::default());
        assert!(wrong_version.validate().is_err());

        let mut no_tabs = WorkspaceState::default();
        assert!(no_tabs.validate().is_err(), "没有标签页的数据无效");

        let mut too_many = WorkspaceState::default();
        for _ in 0..=MAX_TABS {
            too_many.tabs.push(TabState::default());
        }
        assert!(too_many.validate().is_err());

        let mut wide_tree = WorkspaceState::default();
        wide_tree.tabs.push(TabState {
            tree: Some(wide_node(MAX_PANES_PER_TAB + 1)),
            active_pane: None,
        });
        assert!(wide_tree.validate().is_err(), "窗格数量超过上限");

        let mut nan_ratio = WorkspaceState::default();
        nan_ratio.tabs.push(TabState {
            tree: Some(LayoutNode {
                split: Some(SplitNode {
                    axis: Axis::Horizontal,
                    ratio: f32::NAN,
                    first: Box::default(),
                    second: Box::default(),
                }),
                ..LayoutNode::default()
            }),
            active_pane: None,
        });
        assert!(nan_ratio.validate().is_err());
    }

    /// 构造 `count` 个叶节点的单行分屏。
    fn wide_node(count: usize) -> LayoutNode {
        let mut node = LayoutNode::default();
        for _ in 1..count {
            node = LayoutNode {
                split: Some(SplitNode {
                    axis: Axis::Horizontal,
                    ratio: 0.5,
                    first: Box::new(LayoutNode::default()),
                    second: Box::new(node),
                }),
                ..LayoutNode::default()
            };
        }
        node
    }

    #[test]
    fn validate_clamps_ratio_and_active_pane() {
        let mut state = WorkspaceState::default();
        state.tabs.push(TabState {
            tree: Some(LayoutNode {
                split: Some(SplitNode {
                    axis: Axis::Vertical,
                    ratio: 0.99,
                    first: Box::default(),
                    second: Box::default(),
                }),
                ..LayoutNode::default()
            }),
            active_pane: Some(7),
        });
        state.active_tab = 5;
        state.validate().expect("valid");

        let split = state.tabs[0].tree.as_ref().expect("tree").split.as_ref();
        assert_eq!(split.expect("split").ratio, MAX_RATIO);
        assert_eq!(state.tabs[0].active_pane, Some(0), "越界下标回到首个窗格");
        assert_eq!(state.active_tab, 0, "越界标签回到首个标签");
    }

    #[test]
    fn resolve_directory_keeps_existing_and_falls_back_for_missing() {
        let directory = std::env::temp_dir()
            .join("glim-workspace-state-tests")
            .join(format!("dir-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("create directory");
        let (resolved, fell_back) = resolve_directory(Some(&directory));
        assert_eq!(resolved, Some(directory.clone()));
        assert!(!fell_back);

        let missing = directory.join("not-there");
        let (resolved, fell_back) = resolve_directory(Some(&missing));
        assert!(fell_back, "缺失目录应报告回退");
        assert_eq!(resolved, glim_settings::home_directory());

        let (resolved, fell_back) = resolve_directory(None);
        assert_eq!(resolved, None);
        assert!(!fell_back);
    }
}
