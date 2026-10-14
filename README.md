# Glim

<img src="design/logo/glim-app-icon.png" alt="Glim 图标" width="96" align="right">

基于 GPUI 的本地终端，优先支持 Windows；分阶段构建计划见 [plan.md](plan.md)。

## 当前进度

- 阶段 0：工程骨架与依赖验证（`cargo run -p glim` 可启动原生窗口）。
- 阶段 1：单个真实终端（ConPTY、自定义网格绘制、窗口调整尺寸）。
- 阶段 2：日常输入与 TUI（中文输入法、宽字符、选择复制、查找、备用屏幕与鼠标报告）。
- 阶段 3：多标签与基础分屏（标签页新建 / 切换 / 关闭，嵌套分屏、分隔线拖动、方向键切换焦点、窗格放大与还原，空工作区恢复入口，运行中任务的关闭确认与退出汇总确认）。
- 阶段 4：拖拽与推荐布局：
  - 布局面板（`Alt+Z` 或把窗格标题拖到顶部热区）：按窗格数量给出推荐方案，缩略图展示形状，已应用的方案有选中状态；
  - 悬停缩略图或方向键选择即预览，`Enter` 点击应用；预览只渲染占位窗格，不修改分屏树、不调整 PTY 尺寸；
  - `Esc` 取消预览并恢复焦点；面板外的点击关闭面板；
  - 拖动窗格标题交换位置（松手交换、目标窗格高亮）；拖到顶部热区打开布局面板，释放在方案上直接应用；
  - `Ctrl+Shift+方向键` 交换窗格；聚焦分隔线后方向键调整比例、`Home` 回到均分。
- 阶段 5：配置与工作区恢复：
  - `config.toml`（`%APPDATA%\Glim\config.toml`，可用 `GLIM_CONFIG_DIR` 覆盖）：主题、终端字体（族与字号）、默认 shell（程序 / 参数 / 环境变量）、滚动历史；
  - 设置对话框（工具栏「设置」或 `Ctrl+,`）：主题分段选择、字体（已安装字族下拉，可搜索）、默认 shell（探测到的 pwsh、Windows PowerShell、cmd 下拉）、滚动历史；保存写入配置，shell 环境变量等高级设置与未知字段保留在配置文件中；
  - 主题与字体立即生效；默认 shell 与滚动历史作用于新会话；
  - `workspace.toml` 保存标签页、分屏树（方向与比例）、每个窗格的工作目录与 shell 引用、窗口尺寸与最大化状态；重启后恢复布局与目录并启动新 shell；
  - 目录缺失回退到用户目录并提示；无效配置与恢复数据按默认启动并提示；写入采用临时文件 + 原子替换，中断不会破坏之前可用的数据。

界面与设计稿对齐：自定义标题栏（品牌、标签页、会话状态（窗格数、UTF-8、主题胶囊）与设置入口、
窗口控件同一行）、窗格标题栏（活动与非活动底色、目录、前台任务标签与分屏菜单）
与终端内边距和行距都取自 `design/styles.css` 的主题令牌；窗格标题在运行任务时显示
`进程名 · 运行中`，终端右缘有滚动条。

终端绘制细节（对照 Zed 的实现补齐）：

- 网格几何按设备像素对齐：行高、行数与网格原点都取整到物理像素，备用屏幕（TUI）时把
  剩余高度放在网格上方、最后一行贴住底边，高 DPI（125%、150%）下文字与色块不会落在
  半个物理像素上，调整窗口时基线不抖动；
- 块元素（`▀█`、八分之一块、象限块、U+1FB00–U+1FB3B 六分之一块）按单元格子网格
  （8×24）的矩形绘制，`░▒▓` 用整格不透明度替代字体的网点图案；像素画、阴影、二维码与
  进度条没有字形缝隙，相邻同色矩形还会合并成一段；
- ANSI 0–15 颜色对照所在单元格背景做 APCA 最小对比度调整（Lc 45）：浅色主题下的亮色
  不再和背景糊在一起，深色主题下的暗色也不会看不见；TUI 明确指定的 256 色（索引 ≥ 16）
  与真彩色、以及盒线、块元素、Powerline 等装饰字符保持原样；
- 256 色索引完整展开（6×6×6 色立方与 24 级灰阶），不再回退到亮白；
- 文本运行不再重复携带背景色，选择与底色只绘制一次。

窗口不设底部状态栏：目录与 shell 已由窗格标题栏显示，标题栏右侧保留窗格数、UTF-8 与主题胶囊；
分屏入口放在窗格标题栏，布局面板用 `Alt+Z` 或把窗格标题拖到顶部热区打开，标题栏不重复这两个入口。

会话是真实 PTY：默认 shell 按 `pwsh`、Windows PowerShell、`COMSPEC` 顺序查找。
分屏与新标签继承当前窗格的工作目录。布局操作只改变分屏树，会话、输出与滚动位置保持。

工作目录跟踪：窗格标题、分屏 / 新标签继承与 `workspace.toml` 保存都读取 shell 的
当前目录，取自进程信息。Windows PowerShell 的 `cd` 只更新 `$PWD`、不同步进程目录，
所以启动 `powershell` / `pwsh` 时会注入一段提示符包装（`-NoExit -Command`，用户参数里
已有 `-Command` / `-File` 等命令开关时跳过）：每次显示提示符前把当前目录同步给进程，
`cd` 之后外部查询到的是真实位置。`cmd`、`bash` 等其他 shell 自身会同步进程目录，不做处理。

快捷键：`Ctrl+Shift+T` 新标签、`Ctrl+Tab` / `Ctrl+Shift+Tab` 切换标签、`Ctrl+Shift+D/E` 向右 / 向下分屏、
`Ctrl+Shift+W` 关闭窗格、`Ctrl+Shift+Enter` 缩放或还原、`Alt+方向键` 切换焦点、`Alt+Z` 布局面板、
`Ctrl+Shift+方向键` 交换窗格、`Ctrl+,` 设置；终端内 `Ctrl+Shift+C/V` 复制粘贴，分隔线聚焦后方向键调整、`Home` 等分。

## 开发与验证

```powershell
cargo run -p glim            # 启动应用
cargo test --workspace       # 纯逻辑 + 真实 PTY + GPUI 界面集成测试
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
cargo build --release        # 分发给其他机器（调试版不能跨机器使用，见故障排查）
```

界面集成测试使用无头窗口与真实 PowerShell 会话，覆盖分屏、拖动、
缩放、标签切换、布局面板预览与应用、拖拽交换、分隔线键盘调整、
设置对话框与配置写入、布局与目录恢复、无效数据回退、
关闭确认与退出确认；会话相关的用例带有明确超时。

纯逻辑测试覆盖配置读写与安全替换、恢复数据的校验与重建、
分屏树与最小尺寸约束；`config.toml` 与 `workspace.toml` 的读取失败
都回退到默认值，不会阻塞启动。

### 打包安装程序（Windows）

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-msi.ps1
```

脚本先做 `cargo build --release -p glim`，再用 WiX Toolset 3 生成
`target\wix\Glim-<版本>-x64.msi`：每机器安装，安装向导可修改**安装位置**、勾选是否
**创建桌面快捷方式**（默认创建），写入开始菜单快捷方式，同版本号覆盖、高版本自动提示，
卸载时一并清理。静默安装用 `DESKTOP_SHORTCUT=0` 关闭桌面快捷方式，`INSTALLFOLDER="路径"`
指定安装位置。exe 内置图标与版本信息
（`crates/glim/resources/windows`，图标是设计稿的「&gt;_」标记，见 [design/logo](design/logo/README.md)）。

未检测到 WiX 时脚本会从 nuget.org 下载官方 `wix` 3.14.1 包到 `target\wix` 并校验 SHA256；
已安装 WiX 的环境用 `-WixDirectory` 指定，`-SkipBuild` 可以跳过构建。
安装包默认不签名（SmartScreen 可能提示）；正式分发前用 `signtool sign` 对生成的 `.msi`
与 `glim.exe` 签名。

## 故障排查

启动失败（窗口没有出现，或一闪而过）时先看数据目录的运行日志
`%APPDATA%\Glim\glim.log`（目录可用 `GLIM_CONFIG_DIR` 覆盖）：每次启动写一行，
启动阶段的 panic 追加一行（位置与消息），超过 256 KiB 后从空文件重新开始。

- 没有任何「启动」行：进程没有进入主程序。发行版已静态链接 C 运行库
  （`.cargo/config.toml` 的 `+crt-static`），不需要安装 VC++ 运行库；
  常见原因是被安全软件 / SmartScreen 拦截（右键属性里「解除锁定」）。
- `panic ... failed to initialize DirectX`：目标机器没有可用的 DirectX 11 设备。
  界面由 GPUI 用 Direct3D 渲染，需要支持功能级别 10.1 以上的适配器；
  虚拟机、远程桌面或缺少显卡驱动时可能不满足。
- `failed to initialize DirectX: ... 系统找不到指定的路径 (os error 3)`：拷过去的是
  **调试版**。调试版的 GPUI 会在运行时从编译机的 cargo registry 路径
  （`CARGO_MANIFEST_DIR`）现场编译 HLSL，换机器或换用户后该路径不存在，
  DirectX 初始化随之失败；分发请用 `cargo build --release`，着色器在编译期
  编译并内嵌进 exe。
- 其他 panic：按日志中的文件与行号定位。

发行版没有控制台窗口（`windows_subsystem = "windows"`），只能看日志；
调试版从终端运行并设置 `$env:RUST_BACKTRACE=1` 还可以在控制台看到同样的
panic 与调用栈。

## 设计原型

双击打开 [design/prototype.html](design/prototype.html) 预览交互设计，
详细说明见 [设计文档](design/README.md)。原型中的终端输出为示例内容，
最终应用使用原生 GPUI 界面。
