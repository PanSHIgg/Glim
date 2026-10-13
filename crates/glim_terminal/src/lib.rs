//! 终端引擎：Alacritty 适配、PTY、终端网格、输入编码、进程信息与资源回收。
//!
//! 不依赖 GPUI。对外提供 [`TerminalOptions`]、[`SessionHandle`]、[`SessionEvents`]、
//! [`GridSnapshot`] 与 [`KeyInput`]；Alacritty 与平台实现类型留在 crate 内部。

mod keys;
mod options;
mod session;
mod shell;
mod snapshot;

pub use keys::{KeyInput, TerminalModes};
pub use options::TerminalOptions;
pub use session::{SessionEvent, SessionEvents, SessionHandle};
pub use shell::{ShellSpec, default_shell};
pub use snapshot::{
    CellAttributes, CursorShape, CursorSnapshot, GridCell, GridLine, GridSnapshot, TermColor,
};
