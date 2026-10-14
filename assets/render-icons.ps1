# 由 assets/glim-app-icon.svg 生成 crates/glim/resources/windows/glim.ico
# （仅在改动 SVG 或尺寸列表后重跑；生成的 .ico 已随仓库提交，日常构建不需要运行本脚本）。
#
# 渲染使用 Microsoft Edge 无头模式（Windows 10/11 自带），不引入额外依赖：
# 每个尺寸直接渲染矢量图形一内联 SVG 按目标像素写死宽高（百分比宽高在无头模式下不绘制），
# 窗口开得比目标尺寸大（截图 = 窗口，客户区比窗口小），再裁剪出左上角的精确像素，
# 校验圆角透明、瓦片边缘完整后组装 ICO（PNG 条目）。
#
# 本文件保存为 UTF-8 带 BOM（Windows PowerShell 5.1 解析中文所需）。

[CmdletBinding()]
param(
    [string] $EdgePath
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$repositoryRoot = Split-Path $PSScriptRoot -Parent
$svgPath = Join-Path $PSScriptRoot 'glim-app-icon.svg'
$icoPath = Join-Path $repositoryRoot 'crates\glim\resources\windows\glim.ico'

# ICO 内包含的尺寸；256 在目录项中记作 0。
$iconSizes = @(16, 20, 24, 32, 40, 48, 64, 128, 256)

if (-not $EdgePath) {
    $EdgePath = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'),
        (Join-Path $env:ProgramFiles 'Microsoft\Edge\Application\msedge.exe')
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $EdgePath) { throw '找不到 Microsoft Edge；请用 -EdgePath 指定 msedge.exe。' }

$svg = [System.IO.File]::ReadAllText($svgPath, [System.Text.Encoding]::UTF8)
# 只改写根 <svg> 标签的宽高：文件里其他 width="256" 属于用户坐标，不能动。
$openTagEnd = $svg.IndexOf('>')
if ($openTagEnd -lt 0 -or -not $svg.StartsWith('<svg ')) {
    throw 'glim-app-icon.svg 的根 <svg> 标签缺失；脚本依赖它把图标缩放到目标尺寸。'
}
$openTag = $svg.Substring(0, $openTagEnd + 1)
if (-not ($openTag.Contains('width="256"') -and $openTag.Contains('height="256"'))) {
    throw 'glim-app-icon.svg 的根 <svg> 应为 width="256" height="256" 画布。'
}
$svgBody = $svg.Substring($openTagEnd + 1)

$workDirectory = Join-Path $env:TEMP ('glim-icon-' + [guid]::NewGuid().ToString('n'))
New-Item -ItemType Directory -Path $workDirectory | Out-Null

function Invoke-EdgeScreenshot {
    param([int] $Size, [string] $OutputPath)

    $resizedOpenTag = $openTag -replace 'width="256"', "width=`"$Size`"" -replace 'height="256"', "height=`"$Size`""
    $inlineSvg = $resizedOpenTag + $svgBody
    $pagePath = Join-Path $workDirectory "icon-$Size.html"
    $page = @"
<!doctype html>
<html><head><meta charset="utf-8"><style>
html, body { margin: 0; padding: 0; background: transparent; overflow: hidden; }
svg { display: block; }
</style></head><body>
$inlineSvg
</body></html>
"@
    [System.IO.File]::WriteAllText($pagePath, $page, [System.Text.UTF8Encoding]::new($false))
    $pageUrl = 'file:///' + $pagePath.Replace('\', '/')

    foreach ($mode in @('--headless=new', '--headless')) {
        if (Test-Path -LiteralPath $OutputPath) { break }
        # 截图尺寸等于窗口尺寸，客户区（视口）比窗口小（本机约 24×93px）。
        # 窗口开大 256px，确保视口不小于目标尺寸，图标不会被裁切。
        $viewport = $Size + 256
        # 每种模式用独立的 profile 目录：超时被终止的进程不会锁住下一次渲染。
        $modeName = $mode -replace '[^a-z0-9]', ''
        $arguments = @($mode) + @(
            '--disable-gpu', '--hide-scrollbars', '--force-device-scale-factor=1',
            '--default-background-color=00000000', '--no-first-run', '--no-default-browser-check',
            "--user-data-dir=$workDirectory\profile-$Size-$modeName", "--window-size=$viewport,$viewport",
            '--virtual-time-budget=1500', "--screenshot=$OutputPath", $pageUrl
        )
        # 用 Start-Process 而不是 `&`：Edge 的子进程会继承管道句柄，
        # 直接调用时 PowerShell 可能等不到管道关闭而挂起。
        $process = Start-Process -FilePath $EdgePath -ArgumentList $arguments -PassThru -NoNewWindow `
            -RedirectStandardOutput (Join-Path $workDirectory "edge-$Size.out.log") `
            -RedirectStandardError (Join-Path $workDirectory "edge-$Size.err.log")
        if (-not $process.WaitForExit(60000)) {
            $process.Kill()
            throw "Edge 在 $Size×$Size 截图时超过 60 秒。"
        }
    }
    if (-not (Test-Path -LiteralPath $OutputPath)) {
        throw "Edge 未能在 $Size×$Size 下生成截图。"
    }

    # 截图比目标尺寸大，裁剪左上角的 $Size×$Size（目标矩形与源矩形相同，按像素 1:1 拷贝）。
    $bitmap = [System.Drawing.Bitmap]::FromFile($OutputPath)
    try {
        if ($bitmap.Width -lt $Size -or $bitmap.Height -lt $Size) {
            throw "$Size×$Size：截图只有 $($bitmap.Width)×$($bitmap.Height)。"
        }
        if ($bitmap.Width -ne $Size -or $bitmap.Height -ne $Size) {
            $crop = New-Object System.Drawing.Bitmap($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
            $graphics = [System.Drawing.Graphics]::FromImage($crop)
            try {
                $rectangle = New-Object System.Drawing.Rectangle(0, 0, $Size, $Size)
                $graphics.DrawImage($bitmap, $rectangle, $rectangle, [System.Drawing.GraphicsUnit]::Pixel)
            } finally {
                $graphics.Dispose()
            }
            $bitmap.Dispose()
            $crop.Save($OutputPath, [System.Drawing.Imaging.ImageFormat]::Png)
            $crop.Dispose()
            $bitmap = [System.Drawing.Bitmap]::FromFile($OutputPath)
        }

        # 四个角应透明（圆角）；若图标被裁切，被裁掉的角会是不透明像素。
        # 小尺寸下圆角的抗锯齿会在角点留下少量覆盖率，允许 32/255 的容差。
        $last = $Size - 1
        foreach ($corner in @(@(0, 0), @($last, 0), @(0, $last), @($last, $last))) {
            $alpha = $bitmap.GetPixel($corner[0], $corner[1]).A
            if ($alpha -gt 32) {
                throw "$Size×$Size：圆角外应透明，($($corner[0]),$($corner[1])) alpha=$alpha。"
            }
        }
        # 四条边的中点应落在瓦片内（不透明），用于确认内容完整且尺寸正确。
        $middle = [int]($Size / 2)
        foreach ($edge in @(@($middle, 0), @($middle, $last), @(0, $middle), @($last, $middle))) {
            $alpha = $bitmap.GetPixel($edge[0], $edge[1]).A
            if ($alpha -lt 250) {
                throw "$Size×$Size：瓦片边缘应不透明，($($edge[0]),$($edge[1])) alpha=$alpha。"
            }
        }
    } finally {
        $bitmap.Dispose()
    }
}

$succeeded = $false
try {
    $rendered = @{}
    foreach ($size in $iconSizes) {
        $output = Join-Path $workDirectory "icon-$size.png"
        Invoke-EdgeScreenshot -Size $size -OutputPath $output
        $rendered[$size] = $output
    }
    $renderedSizes = $rendered.Keys | Sort-Object

    # ICO：ICONDIR + ICONDIRENTRY[] + PNG 负载。
    $iconDirectory = Split-Path $icoPath -Parent
    New-Item -ItemType Directory -Path $iconDirectory -Force | Out-Null
    $stream = [System.IO.File]::Create($icoPath)
    $writer = New-Object System.IO.BinaryWriter($stream)
    try {
        $writer.Write([uint16]0)                      # reserved
        $writer.Write([uint16]1)                      # type: icon
        $writer.Write([uint16]$rendered.Count)
        $offset = 6 + 16 * $rendered.Count
        foreach ($size in $renderedSizes) {
            $png = [System.IO.File]::ReadAllBytes($rendered[$size])
            $writer.Write([byte]($(if ($size -ge 256) { 0 } else { $size })))  # width
            $writer.Write([byte]($(if ($size -ge 256) { 0 } else { $size })))  # height
            $writer.Write([byte]0)                    # palette colors
            $writer.Write([byte]0)                    # reserved
            $writer.Write([uint16]1)                  # planes
            $writer.Write([uint16]32)                 # bits per pixel
            $writer.Write([uint32]$png.Length)
            $writer.Write([uint32]$offset)
            $offset += $png.Length
        }
        foreach ($size in $renderedSizes) {
            $writer.Write([System.IO.File]::ReadAllBytes($rendered[$size]))
        }
    } finally {
        $writer.Dispose()
        $stream.Dispose()
    }

    $succeeded = $true
    Write-Output "已生成 $icoPath（$($renderedSizes -join '、') px）"
} finally {
    if ($succeeded) {
        Remove-Item -LiteralPath $workDirectory -Recurse -Force -ErrorAction SilentlyContinue
    } else {
        Write-Warning "渲染失败，临时目录保留在 $workDirectory"
    }
}
