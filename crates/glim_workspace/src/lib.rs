//! 工作区：标签页、分屏窗格、状态栏与关闭确认。
//!
//! 工作区拥有标签页与分屏树，每个窗格是一个独立的终端视图实体。
//! 布局操作与关闭流程见 [`WorkspaceView`]；分屏结构是可单独测试的
//! 纯逻辑（`pane_tree`），像素布局由 `split_element` 完成。

mod pane_tree;
mod split_element;
mod workspace_view;

pub use workspace_view::WorkspaceView;
