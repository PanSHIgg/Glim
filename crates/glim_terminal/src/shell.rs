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

/// Windows PowerShell（含 pwsh）的提示符包装脚本。
///
/// PowerShell 的 `cd` 只更新 `$PWD`，不会同步进程工作目录：外部进程
/// 查询（分屏继承、窗格标题、工作区保存）永远只看到启动目录。这里在
/// 每次显示提示符前把当前目录同步给进程，其余行为保持不变：
/// 先保存 `$LASTEXITCODE`、调用原有的提示符函数，避免影响用户自定义提示符；
/// 非文件系统位置（注册表等）没有 `ProviderPath`，跳过同步。
const POWERSHELL_PROMPT_HOOK: &str = concat!(
    "$global:__glim_previous_prompt=$function:prompt;",
    "function global:prompt{",
    "$__glim_code=$LASTEXITCODE;",
    "$__glim_path=$PWD.ProviderPath;",
    "if($__glim_path){try{[Environment]::CurrentDirectory=$__glim_path}catch{}};",
    "$global:LASTEXITCODE=$__glim_code;",
    "& $global:__glim_previous_prompt",
    "}"
);

/// shell 集成的附加启动参数；不需要时返回空。
///
/// 只对 PowerShell 系列注入：`cd` 之后进程目录能跟随更新，会话被外部
/// 查询到的当前位置才是真实位置。参数里已经出现 `-Command` / `-File`
/// 等命令开关时跳过（此时命令行语义由用户接管）。注入的参数必须放在
/// 末尾：PowerShell 把 `-Command` 之后的内容按原样执行。
pub(crate) fn integration_args(shell: &ShellSpec) -> Vec<String> {
    if !is_powershell(shell.program()) || has_command_switch(shell.args()) {
        return Vec::new();
    }
    vec![
        "-NoExit".to_owned(),
        "-Command".to_owned(),
        POWERSHELL_PROMPT_HOOK.to_owned(),
    ]
}

/// 程序名是否为 PowerShell（`powershell` / `pwsh`，忽略扩展名与大小写）。
fn is_powershell(program: &str) -> bool {
    let name = PathBuf::from(program)
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.eq_ignore_ascii_case("powershell") || name.eq_ignore_ascii_case("pwsh")
}

/// 参数里是否已有命令开关（此时不注入集成参数）。
fn has_command_switch(args: &[String]) -> bool {
    args.iter().any(|arg| {
        let switch = arg.trim_start_matches('-');
        ["c", "command", "f", "file", "e", "encodedcommand"]
            .iter()
            .any(|candidate| switch.eq_ignore_ascii_case(candidate))
    })
}

/// 解析默认 shell。
///
/// 查找顺序：`GLIM_SHELL` 环境变量（调试入口；配置了默认 shell 时由
/// 配置优先）、`pwsh`、Windows PowerShell、`COMSPEC`（Windows 上另有
/// `cmd.exe` 兜底）。
pub fn default_shell() -> Option<ShellSpec> {
    if let Some(program) = std::env::var_os("GLIM_SHELL")
        && !program.is_empty()
    {
        return Some(ShellSpec::new(program.to_string_lossy().into_owned()));
    }

    if let Some(shell) = available_shells().into_iter().next() {
        return Some(shell);
    }

    #[cfg(windows)]
    {
        Some(ShellSpec::new("cmd.exe"))
    }

    #[cfg(not(windows))]
    {
        None
    }
}

/// 探测本机可用的 shell，按默认查找顺序排列。
///
/// 只返回实际存在的程序；设置界面的候选列表用它，避免让用户手写路径。
pub fn available_shells() -> Vec<ShellSpec> {
    #[cfg(windows)]
    {
        let mut shells = Vec::new();
        if let Some(path) = find_pwsh() {
            shells.push(ShellSpec::new(path.to_string_lossy().into_owned()));
        }
        if let Some(path) = system_windows_powershell().or_else(|| find_program("powershell.exe")) {
            shells.push(ShellSpec::new(path.to_string_lossy().into_owned()));
        }
        if let Some(program) = std::env::var_os("COMSPEC")
            .filter(|program| !program.is_empty())
            .map(PathBuf::from)
            .filter(|program| program.is_file())
        {
            shells.push(ShellSpec::new(program.to_string_lossy().into_owned()));
        }
        shells
    }

    #[cfg(not(windows))]
    {
        // 平台适配待后续阶段：macOS / Linux 默认 shell 的探测在这里补齐。
        Vec::new()
    }
}

/// PowerShell 7：先看 `PATH`，再看标准安装位置。
#[cfg(windows)]
fn find_pwsh() -> Option<PathBuf> {
    find_program("pwsh.exe").or_else(|| {
        std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .map(|root| root.join("PowerShell").join("7").join("pwsh.exe"))
            .filter(|candidate| candidate.is_file())
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// PowerShell 系列注入提示符包装；其他 shell 不动。
    #[test]
    fn integration_targets_powershell_only() {
        for program in [
            "powershell.exe",
            "PowerShell",
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            "pwsh",
            r"C:\Program Files\PowerShell\7\pwsh.exe",
        ] {
            let spec = ShellSpec::new(program);
            let args = integration_args(&spec);
            assert_eq!(args.len(), 3, "{program} 应注入集成参数");
            assert_eq!(args[0], "-NoExit");
            assert_eq!(args[1], "-Command");
            assert!(args[2].contains("prompt"), "钩子应包装提示符");
            assert!(
                args[2].contains("CurrentDirectory"),
                "钩子应同步进程工作目录"
            );
        }

        for program in [
            "cmd.exe",
            "bash.exe",
            "wsl.exe",
            r"C:\msys64\usr\bin\zsh.exe",
        ] {
            assert!(
                integration_args(&ShellSpec::new(program)).is_empty(),
                "{program} 不需要集成参数"
            );
        }
    }

    /// 用户已经指定命令开关时不再注入。
    #[test]
    fn integration_respects_configured_command_arguments() {
        for args in [
            vec!["-NoLogo", "-Command", "Write-Host hi"],
            vec!["-File", "script.ps1"],
            vec!["-c", "Get-Date"],
            vec!["-EncodedCommand", "AA=="],
        ] {
            let spec = ShellSpec::new("powershell.exe").with_args(args);
            assert!(integration_args(&spec).is_empty(), "已有命令开关时不应注入");
        }

        // 普通开关不阻止注入，注入参数排在用户参数之后。
        let spec = ShellSpec::new("powershell.exe").with_args(["-NoProfile", "-NoLogo"]);
        assert_eq!(integration_args(&spec)[0], "-NoExit");
    }

    /// 候选列表没有重复项（按程序名忽略大小写去重）。
    #[test]
    fn available_shells_have_no_duplicates() {
        let shells = available_shells();
        let mut programs = shells
            .iter()
            .map(|shell| shell.program().to_ascii_lowercase())
            .collect::<Vec<_>>();
        let total = programs.len();
        programs.sort_unstable();
        programs.dedup();
        assert_eq!(programs.len(), total, "候选项不应重复：{shells:?}");
    }

    /// 默认 shell 取候选列表的第一项（`GLIM_SHELL` 优先）。
    #[test]
    fn default_shell_follows_the_candidate_order() {
        let Some(default) = default_shell() else {
            // 非 Windows 或没有可探测的 shell：没有默认值也是合法结果。
            return;
        };
        if let Some(program) = std::env::var_os("GLIM_SHELL").filter(|value| !value.is_empty()) {
            assert_eq!(default.program(), program.to_string_lossy());
            return;
        }
        if let Some(first) = available_shells().first() {
            assert_eq!(default.program(), first.program());
        }
    }
}
