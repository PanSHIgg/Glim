//! 会话配置。

use std::collections::HashMap;
use std::path::PathBuf;

use crate::ShellSpec;

/// 启动终端会话所需的配置。
///
/// 通过 `with_` 构建器组装；默认值对应产品默认：默认滚动历史
/// 10,000 行、初始网格 80×24。
#[derive(Clone, Debug)]
pub struct TerminalOptions {
    pub(crate) shell: Option<ShellSpec>,
    pub(crate) working_directory: Option<PathBuf>,
    pub(crate) env: HashMap<String, String>,
    pub(crate) scrollback: usize,
    pub(crate) columns: u16,
    pub(crate) rows: u16,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        Self {
            shell: None,
            working_directory: None,
            env: HashMap::new(),
            scrollback: 10_000,
            columns: 80,
            rows: 24,
        }
    }
}

impl TerminalOptions {
    /// 创建默认配置。
    pub fn new() -> Self {
        Self::default()
    }

    /// 指定要运行的程序；缺省时按平台默认 shell 顺序查找。
    pub fn with_shell(mut self, shell: ShellSpec) -> Self {
        self.shell = Some(shell);
        self
    }

    /// 指定启动目录。
    pub fn with_working_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.working_directory = Some(directory.into());
        self
    }

    /// 追加子进程环境变量。
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// 滚动历史行数。
    pub fn with_scrollback(mut self, lines: usize) -> Self {
        self.scrollback = lines;
        self
    }

    /// 初始网格尺寸（列 × 行）。
    pub fn with_dimensions(mut self, columns: u16, rows: u16) -> Self {
        self.columns = columns.max(1);
        self.rows = rows.max(1);
        self
    }

    /// 已指定的 shell；`None` 表示使用平台默认。
    pub fn shell(&self) -> Option<&ShellSpec> {
        self.shell.as_ref()
    }

    /// 滚动历史行数。
    pub fn scrollback(&self) -> usize {
        self.scrollback
    }
}
