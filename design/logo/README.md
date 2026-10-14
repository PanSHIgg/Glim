# Glim 图标

品牌标记是终端提示符 `>_`：米金色的尖括号与下划线，落在深色圆角瓦片上。几何与配色
取自设计稿的主题令牌（`design/styles.css`），同时用于 exe 图标、任务栏 / 窗口图标与
安装包。

## 文件

| 文件 | 说明 |
| --- | --- |
| `glim-app-icon.svg` | 应用图标源文件（256×256 画布：圆角瓦片 + `>_`），几何的唯一来源 |
| `glim-mark.svg` | 纯标记（24×24，无底色；与应用图标同一套 `>_` 几何） |
| `glim-app-icon.png` | 512×512 预览（由渲染脚本生成） |
| `render-icons.ps1` | 由 SVG 生成预览与 `crates/glim/resources/windows/glim.ico` |

## 几何与配色

- 瓦片：256×256，圆角 56（约 22%），纵向渐变 `#252625 → #151616`，内描边 `#ffffff` 7%；
- 标记：圆头描边宽 26（约 10% 边长），尖括号 `M63 76 121 128 63 180`、下划线
  `M142 180 193 180`，整体居中于画布；
- 颜色：深色主题强调色 `--primary` `#c9ba91`；浅色背景上使用浅色主题的 `--primary`
  `#786442`（`glim-mark.svg` 内置的是深色主题值，浅色场景按需替换）。

## 重新生成图标

修改 SVG 或尺寸列表后运行（需要 Microsoft Edge，Windows 10/11 自带）：

```powershell
powershell -ExecutionPolicy Bypass -File design\logo\render-icons.ps1
```

脚本按 16、20、24、32、40、48、64、128、256 逐个尺寸直接渲染矢量图形（无头 Edge 截图，
窗口开大于目标尺寸再裁剪出精确像素），校验圆角透明与瓦片边缘不透明，最后组装
`crates/glim/resources/windows/glim.ico`（PNG 条目）。生成的 `.ico` 与预览都已提交，
日常构建不需要运行这个脚本。

## 使用位置

- `crates/glim/resources/windows/glim.rc`：资源 ID 1 的图标，GPUI 在 Windows 上按该 ID
  读取窗口图标；资源管理器文件图标、任务栏与标题栏图标都来自它；
- `packaging/windows/glim.wxs`：安装包的应用图标与快捷方式图标（`ARPPRODUCTICON`）。
