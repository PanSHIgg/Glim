//! 设置：类型化配置、文件读写、配置变化通知与基础设置界面。
//!
//! `config.toml` 由 [`Settings`] 表示；[`init`] 在启动时读取并登记
//! [`SettingsState`]，界面通过 [`open_settings_dialog`] 编辑常用设置，
//! 高级设置直接写配置文件。写入采用安全替换，读取失败回退到默认值
//! 并把问题留给界面展示。
mod dialog;
mod paths;
mod settings;
mod state;
mod store;

pub use dialog::{OpenSettings, open_settings_dialog};
pub use paths::{CONFIG_FILE_NAME, config_path, data_directory, home_directory};
pub use settings::{
    DEFAULT_SCROLLBACK, MAX_FONT_SIZE, MAX_SCROLLBACK, MIN_FONT_SIZE, MIN_SCROLLBACK, Settings,
    ShellConfig, ThemeSetting,
};
pub use state::{
    SettingsEvent, SettingsState, current, handle, init, init_with_path, load_warning,
};
pub use store::{StoreError, load_toml, save_toml};
