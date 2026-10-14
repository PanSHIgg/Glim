//! TOML 文档读写：安全替换写入与可恢复的读取错误。
//!
//! 写入先落到同目录的 `*.tmp`，同步完成后再整体替换目标文件；进程在
//! 任意时刻中断，目标文件要么保持原内容，要么是新内容，不会出现半截
//! 文件。读取只认目标文件本身，遗留的临时文件不影响启动。

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// 读取或写入文档时的错误；包含出错路径，便于界面直接展示。
#[derive(Debug)]
pub enum StoreError {
    /// 文件系统错误。
    Io {
        /// 出错路径。
        path: PathBuf,
        /// 底层错误。
        source: io::Error,
    },
    /// TOML 解析或序列化失败。
    Format {
        /// 出错路径。
        path: PathBuf,
        /// 失败原因。
        message: String,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "无法访问 {}：{source}", path.display())
            }
            Self::Format { path, message } => {
                write!(f, "{} 格式无效：{message}", path.display())
            }
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Format { .. } => None,
        }
    }
}

/// 读取 TOML 文档；文件不存在时返回 `Ok(None)`。
pub fn load_toml<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, StoreError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(StoreError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    toml::from_str(&text)
        .map(Some)
        .map_err(|error| StoreError::Format {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
}

/// 安全替换写入 TOML 文档；父目录不存在时自动创建。
pub fn save_toml<T: Serialize>(path: &Path, value: &T) -> Result<(), StoreError> {
    let text = toml::to_string_pretty(value).map_err(|error| StoreError::Format {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| StoreError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    // 临时文件与目标同目录：重命名在同一卷内是原子替换。
    let temporary = temporary_path(path);
    write_all(&temporary, text.as_bytes()).map_err(|source| StoreError::Io {
        path: temporary.clone(),
        source,
    })?;
    fs::rename(&temporary, path).map_err(|source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

fn write_all(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Settings, ShellConfig, ThemeSetting};

    /// 测试用目录：`${TEMP}/glim-store-tests/<name>/`。
    fn test_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir()
            .join("glim-store-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("create test directory");
        directory
    }

    #[test]
    fn save_then_load_round_trips_settings() {
        let directory = test_directory("round-trip");
        let path = directory.join("config.toml");
        let settings = Settings::default()
            .with_theme(ThemeSetting::Light)
            .with_font_family(Some("JetBrains Mono"))
            .with_font_size(Some(14.5))
            .with_scrollback(20_000)
            .with_shell(Some(
                ShellConfig::new("pwsh.exe")
                    .with_args(["-NoLogo"])
                    .with_env("TERM", "xterm-256color"),
            ));

        save_toml(&path, &settings).expect("save");
        let loaded: Settings = load_toml(&path).expect("load").expect("file exists");
        assert_eq!(loaded, settings);
    }

    #[test]
    fn missing_file_loads_as_none() {
        let directory = test_directory("missing");
        let loaded: Option<Settings> = load_toml(&directory.join("config.toml")).expect("load");
        assert!(loaded.is_none());
    }

    #[test]
    fn invalid_document_reports_error_without_touching_file() {
        let directory = test_directory("invalid");
        let path = directory.join("config.toml");
        let original = "theme = \"这个值不合法\"\n";
        fs::write(&path, original).expect("write invalid file");

        let error = load_toml::<Settings>(&path).expect_err("invalid document");
        assert!(matches!(error, StoreError::Format { .. }));
        assert!(error.to_string().contains("格式无效"));
        assert_eq!(
            fs::read_to_string(&path).expect("read back"),
            original,
            "读取失败不应修改原文件"
        );
    }

    #[test]
    fn partial_document_uses_defaults() {
        let directory = test_directory("partial");
        let path = directory.join("config.toml");
        fs::write(&path, "theme = \"system\"\nscrollback = 5000\n").expect("write partial");

        let loaded: Settings = load_toml(&path).expect("load").expect("file exists");
        assert_eq!(loaded.theme(), ThemeSetting::System);
        assert_eq!(loaded.scrollback(), 5_000);
        assert_eq!(loaded.font_family(), None, "未写出的字段使用默认值");
        assert!(loaded.shell().is_none());
    }

    #[test]
    fn save_replaces_content_and_leaves_no_temporary_file() {
        let directory = test_directory("replace");
        let path = directory.join("config.toml");
        save_toml(&path, &Settings::default().with_scrollback(1_000)).expect("first save");
        save_toml(&path, &Settings::default().with_scrollback(2_000)).expect("second save");

        let loaded: Settings = load_toml(&path).expect("load").expect("file exists");
        assert_eq!(loaded.scrollback(), 2_000);

        let leftovers: Vec<_> = fs::read_dir(&directory)
            .expect("read directory")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "不应遗留临时文件: {leftovers:?}");
    }

    #[test]
    fn interrupted_write_keeps_previous_document() {
        let directory = test_directory("interrupted");
        let path = directory.join("config.toml");
        save_toml(&path, &Settings::default().with_scrollback(3_000)).expect("save");
        // 模拟上次写入在替换前中断：遗留临时文件。
        fs::write(temporary_path(&path), "scrollback = \"垃圾内容\"").expect("leftover tmp");

        let loaded: Settings = load_toml(&path).expect("load").expect("file exists");
        assert_eq!(loaded.scrollback(), 3_000, "目标文件保持上次成功写入的内容");

        save_toml(&path, &Settings::default().with_scrollback(4_000)).expect("save again");
        let loaded: Settings = load_toml(&path).expect("load").expect("file exists");
        assert_eq!(loaded.scrollback(), 4_000);
        assert!(!temporary_path(&path).exists(), "成功写入清理临时文件");
    }

    #[test]
    fn save_creates_missing_parent_directory() {
        let directory = test_directory("nested");
        let path = directory.join("a").join("b").join("config.toml");
        save_toml(&path, &Settings::default()).expect("save into nested directory");
        assert!(load_toml::<Settings>(&path).expect("load").is_some());
    }

    #[test]
    fn unknown_fields_are_preserved_on_save() {
        let directory = test_directory("unknown");
        let path = directory.join("config.toml");
        fs::write(
            &path,
            "theme = \"light\"\nfuture_option = true\n[future_section]\nkey = 1\n",
        )
        .expect("write");

        let loaded: Settings = load_toml(&path).expect("load").expect("file exists");
        assert_eq!(loaded.theme(), ThemeSetting::Light);

        // 高级/未知设置不因界面保存而丢失。
        save_toml(&path, &loaded.with_scrollback(500)).expect("save");
        let text = fs::read_to_string(&path).expect("read back");
        assert!(
            text.contains("future_option = true"),
            "未知字段保留: {text}"
        );
        assert!(text.contains("[future_section]"), "未知表保留: {text}");
        assert!(text.contains("scrollback = 500"), "已知字段更新: {text}");
    }
}
