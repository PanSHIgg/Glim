//! 平台数据目录与文件名。
//!
//! 配置与工作区数据放在系统约定的用户配置目录下：Windows
//! `%APPDATA%\Glim`、macOS `~/Library/Application Support/Glim`、
//! Linux `$XDG_CONFIG_HOME/glim`（缺省 `~/.config/glim`）。
//! `GLIM_CONFIG_DIR` 可以覆盖目录位置（便携或测试场景）。

use std::path::{Path, PathBuf};

/// 配置文件名。
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// 应用数据目录；平台不提供配置目录时为 `None`（此时不持久化，
/// 应用仍可正常运行）。
pub fn data_directory() -> Option<PathBuf> {
    let override_dir = std::env::var_os("GLIM_CONFIG_DIR").filter(|value| !value.is_empty());
    directory_from(override_dir, dirs::config_dir())
}

/// 由环境覆盖与平台配置目录解析数据目录（纯函数，便于测试）。
///
/// 覆盖值本身就是数据目录；平台目录再加产品子目录。
fn directory_from(
    override_dir: Option<std::ffi::OsString>,
    config_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    match override_dir {
        Some(directory) => Some(PathBuf::from(directory)),
        None => config_dir.map(|directory| directory.join("Glim")),
    }
}

/// 用户主目录；恢复目录缺失时回退到这里。
pub fn home_directory() -> Option<PathBuf> {
    dirs::home_dir()
}

/// 数据目录中的配置文件路径。
pub fn config_path(directory: &Path) -> PathBuf {
    directory.join(CONFIG_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_directory_wins() {
        let directory = directory_from(
            Some(std::ffi::OsString::from("D:\\portable")),
            Some(PathBuf::from("C:\\Users\\test\\AppData\\Roaming")),
        );
        assert_eq!(directory, Some(PathBuf::from("D:\\portable")));
    }

    #[test]
    fn falls_back_to_platform_config_directory() {
        let directory = directory_from(None, Some(PathBuf::from("/home/user/.config")));
        assert_eq!(directory, Some(PathBuf::from("/home/user/.config/Glim")));
        assert_eq!(directory_from(None, None), None);
    }

    #[test]
    fn config_path_uses_well_known_name() {
        assert_eq!(
            config_path(Path::new("C:\\data")),
            PathBuf::from("C:\\data\\config.toml")
        );
    }
}
