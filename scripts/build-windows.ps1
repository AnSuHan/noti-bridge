# Windows 앱을 빌드해 MSIX 로 묶는다. 매니페스트 버전은 Cargo.toml 의 버전으로 채운다.
# 배포는 Microsoft Store 로 한다. 스토어에는 서명 없는 MSIX 를 올리면 스토어가 서명한다.
# -Sign 은 스토어 밖에서 시험 설치할 때만 쓴다. 서명 없는 MSIX 는 직접 설치되지 않는다.
param(
    [switch]$Sign,
    [string]$CertPath = "windows\msix\noti-bridge-dev.pfx",
    [string]$CertPassword = ""
)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

# 버전의 기준은 Cargo.toml 의 [workspace.package] 다. MSIX 는 숫자 네 자리만 받으므로 끝에 .0 을 붙인다.
$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "MSIX 에 쓸 수 없는 버전입니다: $version" }
$tauriVersion = (Get-Content windows\src-tauri\tauri.conf.json -Raw -Encoding UTF8 | ConvertFrom-Json).version
if ($tauriVersion -ne $version) { throw "tauri.conf.json 버전($tauriVersion)이 Cargo.toml 버전($version)과 다릅니다" }

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

$manifestPath = (Resolve-Path (Join-Path $layout "AppxManifest.xml")).Path
$manifest = New-Object System.Xml.XmlDocument
$manifest.PreserveWhitespace = $true
$manifest.Load($manifestPath)
$manifest.Package.Identity.Version = "$version.0"
$manifest.Save($manifestPath)
Write-Host "MSIX 버전: $version.0"

$output = "target\noti-bridge.msix"
& $kit.FullName pack /o /d $layout /p $output
if ($LASTEXITCODE -ne 0) { throw "makeappx 실패" }

if ($Sign) {
    $signtool = Join-Path $kit.DirectoryName "signtool.exe"
    & $signtool sign /fd SHA256 /f $CertPath /p $CertPassword $output
    if ($LASTEXITCODE -ne 0) { throw "서명 실패" }
}
Write-Host "완료: $output"
