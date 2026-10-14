//! Glim 构建脚本：在 Windows 上把图标与版本信息嵌入 exe。
//!
//! 同一个图标用于资源管理器的文件图标、任务栏图标与窗口标题栏图标
//! （GPUI 在 Windows 上按资源 ID 1 读取窗口图标，见 `resources/windows/glim.rc`），
//! 版本信息显示在文件「属性 → 详细信息」中。

fn main() {
    #[cfg(windows)]
    embed_windows_resources();
}

/// 编译 `resources/windows/glim.rc`（需要 Windows SDK 中的 `rc.exe`）。
#[cfg(windows)]
fn embed_windows_resources() {
    println!("cargo:rerun-if-changed=resources/windows/glim.rc");
    println!("cargo:rerun-if-changed=resources/windows/glim.ico");

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| String::from("0.0.0"));
    // FILEVERSION 需要 4 段数字；预发布后缀等非数字段回退为 0。
    let mut numbers = version
        .split(['.', '-', '+'])
        .map(|part| part.parse::<u32>().unwrap_or(0));
    let mut file_version = [0_u32; 4];
    for number in &mut file_version[..3] {
        *number = numbers.next().unwrap_or(0);
    }

    embed_resource::compile(
        "resources/windows/glim.rc",
        [
            format!(
                "GLIM_VERSION={},{},{},{}",
                file_version[0], file_version[1], file_version[2], file_version[3]
            ),
            format!("GLIM_VERSION_STR=\"{version}\""),
        ],
    )
    .manifest_required()
    .expect("嵌入 Windows 图标与版本资源失败：需要 Windows SDK 中的 rc.exe");
}
