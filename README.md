# Glim

<img src="assets/glim-app-icon.svg" alt="Glim 图标" width="96" align="right">

基于 GPUI 的本地终端，优先支持 Windows。品牌标记就是终端提示符「&gt;_」。

会话是真实 PTY：默认 shell 按 `pwsh`、Windows PowerShell、`COMSPEC` 顺序查找；
分屏与新标签继承当前窗格的工作目录；界面、快捷键与终端绘制共用一套主题令牌。

## 功能

- **多标签与嵌套分屏**：标签新建 / 切换 / 关闭，向右 / 向下分屏，分隔线拖动或键盘调整比例，
  窗格放大与还原；布局操作不重建会话，进程、输出与滚动位置保持不变。
- **拖拽与推荐布局**：拖动窗格标题交换位置，拖到顶部热区打开布局面板（`Alt+Z`），
  缩略图预览（悬停或方向键选择）后 `Enter` 应用，`Esc` 取消并恢复焦点。
- **日常输入与 TUI**：中文输入法（候选窗口跟随光标，正式提交只写一次 PTY）、宽字符、
  选择复制（`Ctrl+Shift+C/V`）、查找、滚动历史、备用屏幕、鼠标报告、bracketed paste。
- **终端绘制**：网格几何按设备像素对齐；块元素按单元格子网格绘制；
  ANSI 0–15 颜色按 APCA 最小对比度（Lc 45）相对单元格背景调整；256 色完整展开；
  盒线、块元素与 Powerline 等装饰字符保持原样。
- **配置与工作区恢复**：`config.toml`（主题、终端字体、默认 shell、滚动历史）与
  `workspace.toml`（标签、分屏树与比例、各窗格工作目录、窗口尺寸与最大化状态）；
  写入采用临时文件 + 原子替换，无效数据回退默认并提示；设置对话框（`Ctrl+,`）里主题与字体立即生效。
- **运行中任务保护**：关闭运行任务的窗格前确认，退出时汇总确认一次；空闲 shell 直接关闭。

## 快捷键

| 操作 | 快捷键 |
| --- | --- |
| 新建标签 / 关闭窗格 | `Ctrl+Shift+T` / `Ctrl+Shift+W` |
| 切换标签 | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| 向右 / 向下分屏 | `Ctrl+Shift+D` / `Ctrl+Shift+E` |
| 切换窗格焦点 | `Alt+方向键` |
| 交换窗格 | `Ctrl+Shift+方向键` |
| 调整分隔线比例 | 聚焦分隔线后方向键，`Home` 回到均分 |
| 放大或还原窗格 | `Ctrl+Shift+Enter` |
| 布局面板 | `Alt+Z` |
| 设置 | `Ctrl+,` |
| 复制 / 粘贴 | `Ctrl+Shift+C` / `Ctrl+Shift+V` |

## 运行要求

Windows 10 / 11，显卡支持 DirectX 11（功能级别 10.1 以上）。发行版静态链接 C 运行库，
不需要安装 VC++ 运行库。

## 构建与测试

```powershell
cargo run -p glim            # 启动应用
cargo test --workspace       # 纯逻辑 + 真实 PTY + GPUI 界面集成测试
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
cargo build --release        # 分发给其他机器（调试版不能跨机器使用，见故障排查）
```

## 打包安装程序（Windows）

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-msi.ps1
```

- 产物为 `target\wix\Glim-<版本>-x64.msi`：每机器安装、中文向导；
- 向导里可以修改**安装位置**、勾选是否**创建桌面快捷方式**（默认创建），并写入开始菜单快捷方式；
- 同版本覆盖安装，高版本自动提示，降级安装被阻止；卸载时文件与快捷方式一并清理；
- 未安装 WiX Toolset 3 时脚本会从 nuget.org 下载官方 `wix` 包到 `target\wix` 并校验 SHA256；
  已安装 WiX 的环境用 `-WixDirectory` 指定，`-SkipBuild` 跳过构建；
- 静默安装：`msiexec /i <msi> /qn INSTALLFOLDER="路径" DESKTOP_SHORTCUT=0`；
- 安装包默认不签名（SmartScreen 可能提示）；正式分发前用 `signtool` 对 `.msi` 与 `glim.exe` 签名。

## 数据目录与日志

`%APPDATA%\Glim`（可用 `GLIM_CONFIG_DIR` 覆盖）：`config.toml`、`workspace.toml` 与
`glim.log` —— 每次启动写一行，启动阶段的 panic 追加位置与消息，超过 256 KiB 后清空重来。

## 仓库结构

| 路径 | 内容 |
| --- | --- |
| `crates/` | Cargo workspace：`glim`（应用入口）、`glim_workspace`（标签、分屏树、布局与恢复）、`glim_terminal`（Alacritty 适配与 PTY）、`glim_terminal_view`（绘制、输入法、选择）、`glim_settings`（配置）、`glim_ui`（主题与公共组件） |
| `assets/` | 品牌资源：应用图标与标记的 SVG 源文件，以及由 SVG 生成 exe 图标的脚本 |
| `packaging/` | Windows MSI 安装包定义（WiX 3）与构建脚本 |

## 图标

品牌标记是终端提示符「&gt;_」：米金强调色（`#c9ba91`，浅色背景用 `#786442`）的尖括号与下划线，
落在深色圆角瓦片上（`assets/glim-app-icon.svg`；无底色的纯标记见 `assets/glim-mark.svg`）。

exe 通过 Windows 资源内置该图标（资源 ID 1，GPUI 按此读取窗口 / 任务栏图标）与版本信息
（`crates/glim/resources/windows`），资源管理器、任务栏、标题栏与安装包都用它。
改动 SVG 后重新生成 `.ico`：

```powershell
powershell -ExecutionPolicy Bypass -File assets\render-icons.ps1   # 需要 Microsoft Edge
```

## 故障排查

- 启动失败（窗口没有出现，或一闪而过）先看 `%APPDATA%\Glim\glim.log`：没有「启动」行说明
  进程没有进入主程序，常见原因是安全软件 / SmartScreen 拦截（右键属性里「解除锁定」）；
  发行版已静态链接 C 运行库，不需要单独安装 VC++ 运行库。
- `failed to initialize DirectX ...`：目标机器没有可用的 DirectX 11 设备；虚拟机、远程桌面或
  缺少显卡驱动时可能不满足。
- 换机器后报 `failed to initialize DirectX: ... 系统找不到指定的路径 (os error 3)`：拷过去的是
  **调试版**；分发请用 `cargo build --release`，着色器在编译期内嵌进 exe。
- 其他 panic：日志里记录了位置与消息；调试版从终端运行并设置 `$env:RUST_BACKTRACE=1`
  还能看到完整调用栈。
