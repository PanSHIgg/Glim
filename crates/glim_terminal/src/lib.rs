//! 终端引擎：Alacritty 适配、PTY、终端网格、输入编码、进程信息与资源回收。
//!
//! 不依赖 GPUI。对外提供 [`TerminalOptions`]、[`SessionHandle`]、[`SessionEvents`]、
//! [`GridSnapshot`]、[`KeyInput`] 与输入 / 鼠标编码；Alacritty 与平台实现类型
//! 留在 crate 内部。

mod input;
mod keys;
mod mouse;
mod options;
mod selection;
mod session;
mod shell;
mod snapshot;

pub use input::{alternate_scroll_bytes, focus_report, paste_bytes};
pub use keys::{KeyInput, TerminalModes};
pub use mouse::{MouseButton, MouseEventKind, MouseModifiers, encode_mouse_event};
pub use options::TerminalOptions;
pub use selection::{GridSelection, SelectionKind, SelectionSide};
pub use session::{ScrollCommand, SearchMatch, SessionEvent, SessionEvents, SessionHandle};
pub use shell::{ShellSpec, default_shell};
pub use snapshot::{
    CellAttributes, CursorShape, CursorSnapshot, GridCell, GridLine, GridPoint, GridSnapshot,
    SelectionSpan, TermColor,
};
