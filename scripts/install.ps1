# Workroam 安装脚本（Windows）。
#
#   irm https://github.com/kverona-ai/workroam/releases/latest/download/install-cli.ps1 | iex
#
# 环境变量：
#   WORKROAM_VERSION       默认最新版本
#   WORKROAM_INSTALL_DIR   默认 %LOCALAPPDATA%\Programs\Workroam\bin，不需要管理员权限
#   WORKROAM_COMPONENT     cli（默认，workroam + workroamd）或 server（workroam-server）
#   WORKROAM_DOWNLOAD_URL  下载地址前缀（镜像或内网），其下按 v<版本>/<文件名> 组织
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Repo = 'kverona-ai/workroam'
$Component = if ($env:WORKROAM_COMPONENT) { $env:WORKROAM_COMPONENT } else { 'cli' }
$Version = if ($env:WORKROAM_VERSION) { $env:WORKROAM_VERSION } else { 'latest' }
$InstallDir = if ($env:WORKROAM_INSTALL_DIR) { $env:WORKROAM_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\Workroam\bin' }
$DownloadUrl = if ($env:WORKROAM_DOWNLOAD_URL) { $env:WORKROAM_DOWNLOAD_URL } else { "https://github.com/$Repo/releases/download" }

switch ($Component) {
  'cli' { $Package = 'workroam'; $Bins = @('workroam', 'workroamd') }
  'server' { $Package = 'workroam-server'; $Bins = @('workroam-server') }
  default { throw "WORKROAM_COMPONENT 只能是 cli 或 server" }
}

$Arch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()) {
  'X64' { 'x86_64' }
  'Arm64' { 'aarch64' }
  default { throw "不支持的处理器架构：$_（支持 x64 与 arm64）" }
}
$Target = "$Arch-pc-windows-msvc"

if ($Version -eq 'latest') {
  $resp = Invoke-WebRequest -Uri "https://github.com/$Repo/releases/latest" -MaximumRedirection 0 -ErrorAction SilentlyContinue -UseBasicParsing
  $location = $resp.Headers.Location
  if (-not $location) {
    $location = (Invoke-WebRequest -Uri "https://github.com/$Repo/releases/latest" -UseBasicParsing).BaseResponse.ResponseUri.AbsoluteUri
  }
  if ("$location" -notmatch '/tag/v([^/]+)$') { throw '无法获取最新版本号，请设置 WORKROAM_VERSION' }
  $Version = $Matches[1]
}
$Version = $Version.TrimStart('v')

$Archive = "$Package-$Version-$Target.zip"
$Base = "$DownloadUrl/v$Version"
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("workroam-" + [System.Guid]::NewGuid())
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
  Write-Host "workroam-install: 下载 $Archive"
  Invoke-WebRequest -Uri "$Base/$Archive" -OutFile (Join-Path $Tmp $Archive) -UseBasicParsing
  Invoke-WebRequest -Uri "$Base/SHA256SUMS" -OutFile (Join-Path $Tmp 'SHA256SUMS') -UseBasicParsing

  $line = Get-Content (Join-Path $Tmp 'SHA256SUMS') | Where-Object { ($_ -split '\s+')[1].TrimStart('*') -eq $Archive } | Select-Object -First 1
  if (-not $line) { throw "SHA256SUMS 中没有 $Archive" }
  $expected = ($line -split '\s+')[0].ToLower()
  $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $Tmp $Archive)).Hash.ToLower()
  if ($expected -ne $actual) { throw "校验和不一致：期望 $expected，实际 $actual" }

  Expand-Archive -Path (Join-Path $Tmp $Archive) -DestinationPath $Tmp -Force
  New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
  foreach ($bin in $Bins) {
    $src = Join-Path $Tmp "$Package-$Version-$Target\$bin.exe"
    if (-not (Test-Path $src)) { throw "压缩包中缺少 $bin.exe" }
    $dst = Join-Path $InstallDir "$bin.exe"
    # 正在运行的 exe 不能覆盖，但可以改名
    if (Test-Path $dst) { Move-Item -Force $dst "$dst.old" }
    Copy-Item $src $dst
    Remove-Item -Force "$dst.old" -ErrorAction SilentlyContinue
    Write-Host "workroam-install: 已安装 $dst"
  }
} finally {
  Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not (($userPath -split ';') -contains $InstallDir)) {
  [Environment]::SetEnvironmentVariable('Path', (@($userPath, $InstallDir) | Where-Object { $_ }) -join ';', 'User')
  $env:Path = "$env:Path;$InstallDir"
  Write-Host "workroam-install: 已把 $InstallDir 加入用户 PATH（新开的终端生效）"
}

if ($Component -eq 'server') {
  Write-Host 'workroam-install: 完成。启动中继：workroam-server --bind <绑定码> --tunnel'
} else {
  Write-Host 'workroam-install: 完成。运行 workroam setup 开始初始化，或安装桌面应用使用图形向导。'
}
