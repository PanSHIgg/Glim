# Glim

基于 GPUI 的本地终端，优先支持 Windows；分阶段构建计划见 [plan.md](plan.md)。

## 当前进度

- 阶段 0：工程骨架与依赖验证（`cargo run -p glim` 可启动原生窗口）。
- 阶段 1：单个真实终端（ConPTY、自定义网格绘制、窗口调整尺寸）。
- 阶段 2：日常输入与 TUI（中文输入法、宽字符、选择复制、查找、备用屏幕与鼠标报告）。
- 阶段 3：多标签与基础分屏（进行中）：
  - 标签页新建 / 切换 / 关闭，标签保留独立会话；
  - 嵌套分屏、分隔线拖动、方向键切换焦点、窗格放大与还原；
  - 空工作区恢复入口；shell 退出后可重新启动；
  - 运行中任务的关闭确认，应用退出时汇总一次确认。

会话是真实 PTY：默认 shell 按 `pwsh`、Windows PowerShell、`COMSPEC` 顺序查找。
分屏与新标签继承当前窗格的工作目录。

## 开发与验证

```powershell
cargo run -p glim            # 启动应用
cargo test --workspace       # 纯逻辑 + 真实 PTY + GPUI 界面集成测试
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
```

界面集成测试使用无头窗口与真实 PowerShell 会话，覆盖分屏、拖动、
缩放、标签切换、关闭确认与退出确认；会话相关的用例带有明确超时。

## 设计原型

双击打开 [design/prototype.html](design/prototype.html) 预览交互设计，
详细说明见 [设计文档](design/README.md)。原型中的终端输出为示例内容，
最终应用使用原生 GPUI 界面。
