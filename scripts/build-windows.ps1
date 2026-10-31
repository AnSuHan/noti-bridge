# Windows 앱을 빌드해 MSIX 로 묶는다.
# -Sign 을 주면 CertPath 의 pfx 로 서명한다. 서명 없는 MSIX 는 설치되지 않는다.
param(
    [switch]$Sign,
    [string]$CertPath = "windows\msix\noti-bridge-dev.pfx",
    [string]$CertPassword = ""
)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

npm --prefix shared/desktop-ui ci
if ($LASTEXITCODE -ne 0) { throw "npm ci 실패" }
npm --prefix shared/desktop-ui run build
if ($LASTEXITCODE -ne 0) { throw "UI 빌드 실패" }
cargo build --release -p noti-bridge-windows
if ($LASTEXITCODE -ne 0) { throw "cargo build 실패" }

$kit = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.*\x64\makeappx.exe" |
    Sort-Object FullName | Select-Object -Last 1
if (-not $kit) { throw "Windows SDK 의 makeappx.exe 를 찾지 못했습니다" }

$layout = "target\msix-layout"
Remove-Item $layout -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory $layout | Out-Null
Copy-Item "target\release\noti-bridge.exe" $layout
Copy-Item "windows\msix\*" $layout -Include *.xml, *.png

$output = "target\noti-bridge.msix"
& $kit.FullName pack /o /d $layout /p $output
if ($LASTEXITCODE -ne 0) { throw "makeappx 실패" }

if ($Sign) {
    $signtool = Join-Path $kit.DirectoryName "signtool.exe"
    & $signtool sign /fd SHA256 /f $CertPath /p $CertPassword $output
    if ($LASTEXITCODE -ne 0) { throw "서명 실패" }
}
Write-Host "완료: $output"
