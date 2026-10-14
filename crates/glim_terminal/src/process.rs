//! 进程信息：子进程检测与工作目录查询。
//!
//! 关闭会话前需要知道 shell 是否仍在运行任务（存在子进程），分屏时
//! 需要知道 shell 的当前工作目录；两者都通过系统进程表按需查询。
//! 查询发生在用户操作时，不做轮询；失败一律按“未知”处理（认为空闲、
//! 无目录），避免阻塞或误报。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// 进程树中 shell 是否仍有子进程在运行。
///
/// 空闲 shell 没有子进程；`cargo build`、`git` 等命令会产生子进程。
pub(crate) fn has_running_child(shell_pid: u32) -> bool {
    let parents = process_parents();
    any_descendant(shell_pid, &parents)
}

/// 查询指定进程的当前工作目录；无法读取时返回 `None`。
pub(crate) fn working_directory(pid: u32) -> Option<PathBuf> {
    let mut system = System::new();
    let pids = [Pid::from_u32(pid)];
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        ProcessRefreshKind::new().with_cwd(UpdateKind::Always),
    );
    system
        .process(Pid::from_u32(pid))
        .and_then(|process| process.cwd())
        .map(Path::to_path_buf)
}

/// 读取 `pid -> 父 pid` 映射；只取进程树本身需要的信息。
fn process_parents() -> HashMap<u32, u32> {
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, ProcessRefreshKind::new());
    system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            process
                .parent()
                .map(|parent| (pid.as_u32(), parent.as_u32()))
        })
        .collect()
}

/// 前台任务：shell 后代中层级最深的进程名（去掉 `.exe` 后缀）。
///
/// 用于窗格标题的进程标签；没有子进程时返回 `None`。
pub(crate) fn foreground_process(shell_pid: u32) -> Option<String> {
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, ProcessRefreshKind::new());

    let mut best: Option<(usize, String)> = None;
    for (pid, process) in system.processes() {
        let Some(depth) = descendant_depth(pid.as_u32(), shell_pid, system.processes()) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|(best_depth, _)| depth > *best_depth)
        {
            let name = process.name().to_string_lossy().into_owned();
            best = Some((depth, strip_exe(&name)));
        }
    }
    best.map(|(_, name)| name)
}

/// `pid` 相对 `root` 的深度；不是后代时返回 `None`。
fn descendant_depth(
    pid: u32,
    root: u32,
    processes: &HashMap<Pid, sysinfo::Process>,
) -> Option<usize> {
    let mut current = pid;
    for depth in 1..=64 {
        let parent = processes.get(&Pid::from_u32(current))?.parent()?.as_u32();
        if parent == root {
            return Some(depth);
        }
        if parent == 0 || parent == current {
            return None;
        }
        current = parent;
    }
    None
}

/// 去掉 Windows 可执行文件后缀。
fn strip_exe(name: &str) -> String {
    match name.len().checked_sub(4) {
        Some(cut) if name[cut..].eq_ignore_ascii_case(".exe") => name[..cut].to_owned(),
        _ => name.to_owned(),
    }
}

/// 是否存在任一进程的祖先为 `root`。
fn any_descendant(root: u32, parents: &HashMap<u32, u32>) -> bool {
    parents.keys().any(|&pid| is_descendant(pid, root, parents))
}

/// 沿着父链向上查找 `root`；用步数上限兜底，防止异常数据成环。
fn is_descendant(pid: u32, root: u32, parents: &HashMap<u32, u32>) -> bool {
    let mut current = pid;
    for _ in 0..64 {
        let Some(&parent) = parents.get(&current) else {
            return false;
        };
        if parent == root {
            return true;
        }
        if parent == 0 || parent == current {
            return false;
        }
        current = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parents(entries: &[(u32, u32)]) -> HashMap<u32, u32> {
        entries.iter().copied().collect()
    }

    #[test]
    fn detects_direct_and_nested_descendants() {
        // shell 100 → cargo 200 → rustc 300；另有无关进程 400。
        let tree = parents(&[(200, 100), (300, 200), (400, 1), (1, 0)]);
        assert!(any_descendant(100, &tree));
        assert!(!any_descendant(400, &tree));
        assert!(!any_descendant(300, &parents(&[(200, 100)])));
    }

    #[test]
    fn stops_on_cycles_and_missing_parents() {
        // 成环的异常数据不应无限循环。
        let tree = parents(&[(2, 3), (3, 2)]);
        assert!(!any_descendant(99, &tree));
        // 父进程不存在时视为不是后代。
        assert!(!any_descendant(1, &parents(&[(2, 99)])));
        // pid 与父 pid 相同是异常值，直接停止。
        assert!(!any_descendant(5, &parents(&[(6, 6)])));
    }
}
