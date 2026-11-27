# 개발 PC 에서 Windows 앱을 패키지(MSIX 레이아웃)로 등록해 실행한다.
# 패키지 ID 가 있어야 PC 알림 읽기(UserNotificationListener)와 자동 시작이 동작한다.
# Windows 설정 > 시스템 > 개발자용 > 개발자 모드가 켜져 있어야 한다(서명 없이 등록).
#   ./scripts/install-windows-dev.ps1            # 빌드 후 등록
#   ./scripts/install-windows-dev.ps1 -NoBuild   # 이미 만든 target\msix-layout 만 다시 등록
param([switch]$NoBuild)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$unlock = Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock" -ErrorAction SilentlyContinue
if ($unlock.AllowDevelopmentWithoutDevLicense -ne 1) {
    throw "개발자 모드가 꺼져 있습니다. Windows 설정 > 시스템 > 개발자용에서 켜 주세요."
}

if (-not $NoBuild) {
    & (Join-Path $PSScriptRoot "build-windows.ps1")
}
$manifest = Resolve-Path "target\msix-layout\AppxManifest.xml"

# 실행 중이면 파일이 잠겨 등록이 실패한다. 패키지판과 exe 판 모두 끈다.
Get-Process noti-bridge -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500

$name = ([xml](Get-Content $manifest -Raw -Encoding UTF8)).Package.Identity.Name
Get-AppxPackage $name | Remove-AppxPackage
Add-AppxPackage -Register $manifest
$package = Get-AppxPackage $name
Write-Host "등록됨: $($package.PackageFullName)"

# 시작 메뉴와 같은 방식으로 패키지 앱을 실행한다(exe 를 직접 실행하면 패키지 ID 가 없다).
Start-Process "shell:AppsFolder\$($package.PackageFamilyName)!App"
