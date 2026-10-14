# 构建 Glim 的 Windows 安装包（.msi）。
#
# 流程：cargo build --release -p glim → WiX Toolset 3（candle + light）→ target\wix\Glim-<版本>-x64.msi。
# WiX 取自官方 NuGet 包（wix 3.14.1）；未检测到现成安装时自动下载到 target\wix 并校验 SHA256。
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File packaging\windows\build-msi.ps1
#   powershell -ExecutionPolicy Bypass -File packaging\windows\build-msi.ps1 -SkipBuild
#   powershell -ExecutionPolicy Bypass -File packaging\windows\build-msi.ps1 -WixDirectory C:\wix314\bin
#
# 产物未签名：正式分发前用 signtool 对 .msi 与 glim.exe 签名。
# 本文件保存为 UTF-8 带 BOM（Windows PowerShell 5.1 解析中文所需）。

[CmdletBinding()]
param(
    [switch] $SkipBuild,
    [string] $WixDirectory
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem

$repositoryRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$binaryPath = Join-Path $repositoryRoot 'target\release\glim.exe'
$iconPath = Join-Path $repositoryRoot 'crates\glim\resources\windows\glim.ico'
$wxsPath = Join-Path $PSScriptRoot 'glim.wxs'
$welcomeRtf = Join-Path $PSScriptRoot 'welcome.rtf'
$outputDirectory = Join-Path $repositoryRoot 'target\wix'
$wixPackageDirectory = Join-Path $outputDirectory 'wix-3.14.1'
$wixToolsDirectory = Join-Path $wixPackageDirectory 'tools'

# WiX Toolset 3.14.1 官方 NuGet 包（nuget.org，发布者 WiX Toolset Team）与其 SHA256。
$wixPackageUrl = 'https://api.nuget.org/v3-flatcontainer/wix/3.14.1/wix.3.14.1.nupkg'
$wixPackageSha256 = '15D50463C73DCE31FBEA5440AC33AF47E92D54D4188166D207E9E39577B8FE0F'

# 读取 workspace 版本号（Cargo.toml 的 [workspace.package] version）。
function Get-WorkspaceVersion {
    $inPackageSection = $false
    foreach ($line in Get-Content (Join-Path $repositoryRoot 'Cargo.toml') -Encoding UTF8) {
        if ($line -match '^\s*\[') {
            $inPackageSection = ($line -match '^\s*\[workspace\.package\]')
            continue
        }
        if ($inPackageSection -and $line -match '^\s*version\s*=\s*"([^"]+)"') {
            return $Matches[1]
        }
    }
    throw 'Cargo.toml 中没有找到 [workspace.package] 的 version。'
}

# 执行原生命令：把 stderr 合并进输出（PowerShell 5.1 会把原生命令的 stderr 当错误），
# 并用退出码判断成败。
function Invoke-Tool {
    param([string] $Executable, [string[]] $Arguments)

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Executable @Arguments 2>&1 | ForEach-Object { Write-Host $_ }
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($exitCode -ne 0) {
        throw "「$Executable」执行失败（exit=$exitCode）。"
    }
}

# 查找 candle.exe / light.exe：-WixDirectory → %WIX% → 已下载的 NuGet 包 tools 目录。
function Find-WixExecutable {
    param([string] $Name)

    $candidates = @()
    if ($WixDirectory) { $candidates += (Join-Path $WixDirectory $Name) }
    if ($env:WIX) { $candidates += (Join-Path $env:WIX "bin\$Name") }
    $candidates += (Join-Path $wixToolsDirectory $Name)
    $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}

# 下载并校验 WiX Toolset 3.14.1 的官方 NuGet 包，解出 tools 目录。
function Install-Wix {
    Write-Output "未找到 WiX；下载官方 NuGet 包 wix 3.14.1 到 $wixPackageDirectory …"
    New-Item -ItemType Directory -Path $wixPackageDirectory -Force | Out-Null
    $package = Join-Path $wixPackageDirectory 'wix.3.14.1.nupkg'
    $webClient = New-Object System.Net.WebClient
    try {
        $webClient.DownloadFile($wixPackageUrl, $package)
    } finally {
        $webClient.Dispose()
    }
    $hash = (Get-FileHash $package -Algorithm SHA256).Hash
    if ($hash -ne $wixPackageSha256) {
        Remove-Item $package -Force -ErrorAction SilentlyContinue
        throw "WiX 包校验失败：期望 $wixPackageSha256，实际 $hash。"
    }
    # nupkg 是 zip：解包后保留 tools 目录（candle.exe、light.exe 与各扩展 DLL）。
    [System.IO.Compression.ZipFile]::ExtractToDirectory($package, $wixPackageDirectory)
    Remove-Item $package -Force
}

# --- 1. release 构建 ----------------------------------------------------------

if ($SkipBuild) {
    if (-not (Test-Path $binaryPath)) {
        throw "没有找到 $binaryPath；去掉 -SkipBuild 先构建。"
    }
} else {
    Write-Output 'cargo build --release -p glim'
    Invoke-Tool -Executable 'cargo' -Arguments @('build', '--release', '-p', 'glim')
}

# --- 2. WiX 工具 --------------------------------------------------------------

$candle = Find-WixExecutable 'candle.exe'
$light = Find-WixExecutable 'light.exe'
if (-not $candle -or -not $light) {
    Install-Wix
    $candle = Find-WixExecutable 'candle.exe'
    $light = Find-WixExecutable 'light.exe'
}
if (-not $candle -or -not $light) {
    throw 'WiX 的 candle.exe / light.exe 不可用；用 -WixDirectory 指定工具目录。'
}

# --- 3. 生成 MSI --------------------------------------------------------------

$version = Get-WorkspaceVersion
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
$msiPath = Join-Path $outputDirectory "Glim-$version-x64.msi"

# 安装包源文件在 packaging\windows：candle 以 -d 传入版本、exe、图标与欢迎文本路径。
Invoke-Tool -Executable $candle -Arguments @(
    '-nologo',
    '-arch', 'x64',
    "-dGlimVersion=$version",
    "-dBinaryPath=$binaryPath",
    "-dIconPath=$iconPath",
    "-dWelcomeRtf=$welcomeRtf",
    '-out', "$outputDirectory\",
    $wxsPath
)

# zh-CN 界面来自 WixUIExtension 内置本地化（-cultures:zh-CN）。
# ICE61：AllowSameVersionUpgrades 是刻意选择（同版本可覆盖安装），抑制该提示。
Invoke-Tool -Executable $light -Arguments @(
    '-nologo',
    '-ext', 'WixUIExtension',
    '-cultures:zh-CN',
    '-sice:ICE61',
    '-out', $msiPath,
    (Join-Path $outputDirectory 'glim.wixobj')
)

$size = [Math]::Round((Get-Item $msiPath).Length / 1MB, 1)
Write-Output "已生成 $msiPath（$size MB，版本 $version）"
Write-Output '提示：安装包未签名；正式分发前用 signtool 签名。'
