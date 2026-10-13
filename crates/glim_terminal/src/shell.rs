//! shell 查找：默认 shell 的解析规则。

use std::path::PathBuf;

/// 要启动的程序与参数。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellSpec {
    program: String,
    args: Vec<String>,
}

impl ShellSpec {
    /// 以程序路径创建。
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    /// 追加参数。
    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// 程序路径。
    pub fn program(&self) -> &str {
        &self.program
    }

    /// 参数。
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// 用于界面展示的名称（程序文件名，去掉 `.exe` 后缀）。
    pub fn display_name(&self) -> String {
        let name = PathBuf::from(&self.program)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.program.clone());
        match name.len().checked_sub(4) {
            Some(cut) if name[cut..].eq_ignore_ascii_case(".exe") => name[..cut].to_owned(),
            _ => name,
        }
    }
}

/// 解析默认 shell。
///
/// 查找顺序：`GLIM_SHELL` 环境变量（临时调试入口，阶段 5 由配置替代）、
/// `pwsh`、Windows PowerShell、`COMSPEC`（Windows 上另有 `cmd.exe` 兜底）。
pub fn default_shell() -> Option<ShellSpec> {
    if let Some(program) = std::env::var_os("GLIM_SHELL")
        && !program.is_empty()
    {
        return Some(ShellSpec::new(program.to_string_lossy().into_owned()));
    }

    #[cfg(windows)]
    {
        if let Some(path) = find_program("pwsh.exe") {
            return Some(ShellSpec::new(path.to_string_lossy().into_owned()));
        }
        if let Some(path) = system_windows_powershell().or_else(|| find_program("powershell.exe")) {
            return Some(ShellSpec::new(path.to_string_lossy().into_owned()));
        }
        if let Some(comspec) = std::env::var_os("COMSPEC")
            && !comspec.is_empty()
        {
            return Some(ShellSpec::new(comspec.to_string_lossy().into_owned()));
        }
        Some(ShellSpec::new("cmd.exe"))
    }

    #[cfg(not(windows))]
    {
        None
    }
}

/// 在 `PATH` 中查找可执行文件。
#[cfg(windows)]
fn find_program(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

/// Windows PowerShell 的固定安装位置。
#[cfg(windows)]
fn system_windows_powershell() -> Option<PathBuf> {
    let system_root = std::env::var_os("SystemRoot")?;
    let candidate = PathBuf::from(system_root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    candidate.is_file().then_some(candidate)
}
