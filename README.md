# noti-bridge

폰과 PC 사이에서 알림과 그 상태(도착, 갱신, 삭제, 액션)를 맞추는 앱입니다. 한쪽에서 지우면 양쪽에서 사라집니다.
자체 서버 없이 iroh P2P 로 연결하며, 같은 망이면 직접, 다른 망이면 홀펀칭 또는 공개 릴레이 중계를 씁니다.
프로토콜은 [docs/PROTOCOL.md](docs/PROTOCOL.md), 버전 기록은 [docs/VERSIONS.md](docs/VERSIONS.md), 개인정보 처리방침은 [docs/PRIVACY.md](docs/PRIVACY.md) 에 있습니다.

## 구조

| 경로 | 내용 |
|---|---|
| `core/` | Rust 코어: 프로토콜, 동기화 엔진, SQLite 저장, iroh 네트워크 |
| `proto/` | Protobuf 스키마 |
| `bindings/` | Kotlin·Swift 용 UniFFI 동적 라이브러리 |
| `android/` | Kotlin + Compose 앱 (알림 원본) |
| `ios/` | SwiftUI 앱 (수신 전용) |
| `windows/` | Tauri 앱 (토스트, DPAPI, MSIX) |
| `mac/` | Tauri 앱 (수신 전용) |
| `shared/desktop-app/` | Windows·macOS 공용 Tauri 셸 |
| `shared/desktop-ui/` | Windows·macOS 공용 React UI |

## 플랫폼별 역할

| 플랫폼 | 알림 읽기 | 알림 표시 | 삭제 동기화 |
|---|---|---|---|
| Android | 가능 | 가능 | 양방향 |
| Windows | 스토어(MSIX) 설치 시 가능 | 토스트 | 양방향 |
| macOS | 불가 | 알림 센터 | 앱 안 목록에서 삭제 시 전송, 수신 삭제는 앱 목록에만 반영 |
| iOS | 불가 | 로컬 알림 | 양방향, 단 앱이 실행 중일 때 |

Windows 앱은 Microsoft Store 로 배포합니다. 스토어로 설치하면 로그인할 때 트레이로 자동 시작하며, Windows 설정의 시작 프로그램에서 끌 수 있습니다.
Windows·macOS 앱은 창을 닫아도 트레이에 남아 알림을 받습니다. 창을 닫으면 화면(WebView)을 내려 메모리를 돌려주고, 트레이에서 열면 다시 만듭니다. 완전히 끝내려면 트레이 메뉴의 종료를 누릅니다.

## 빌드

Rust 1.91 이상이 필요합니다.

```bash
cargo test -p noti-bridge-core                                  # 코어 테스트
cargo test -p noti-bridge-core --test sync_e2e -- --ignored     # 공개 릴레이 경유 테스트
```

Android (Android SDK, NDK, `cargo install cargo-ndk`, `rustup target add aarch64-linux-android x86_64-linux-android`):

```bash
bash scripts/build-android-core.sh
cd android && ./gradlew assembleDebug
```

Windows (Node.js, Windows SDK):

```powershell
./scripts/build-windows.ps1     # target/noti-bridge.msix (서명 없음, 스토어 제출용)
```

- MSIX 버전은 `Cargo.toml` 의 `[workspace.package] version` 에 `.0` 을 붙여 넣습니다. `windows/src-tauri/tauri.conf.json` 버전이 다르면 빌드가 멈춥니다.
- 스토어에는 서명 없는 MSIX 를 올리면 스토어가 서명합니다. 매니페스트의 패키지 이름과 게시자는 개발용 값이라, 제출 빌드에는 Partner Center > 제품 ID 의 값을 넘깁니다.

  ```powershell
  ./scripts/build-windows.ps1 -IdentityName "<패키지 이름>" -Publisher "CN=<게시자 ID>" -PublisherDisplayName "<게시자 표시 이름>"
  ```

  CI 에서는 같은 값을 `NOTIBRIDGE_MSIX_NAME`, `NOTIBRIDGE_MSIX_PUBLISHER`, `NOTIBRIDGE_MSIX_PUBLISHER_DISPLAY` 환경 변수로 줄 수 있습니다.
- 패키지에는 UDP 수신 방화벽 규칙이 들어 있어, 설치하면 첫 실행 때 네트워크 허용 창이 뜨지 않습니다.
- 개발 PC 에서 패키지 환경으로 시험하려면 Windows 설정에서 개발자 모드를 켜고 `Add-AppxPackage -Register target\msix-layout\AppxManifest.xml` 로 등록합니다. 지울 때는 `Get-AppxPackage NotiBridge.Desktop | Remove-AppxPackage` 입니다.
- 스토어 밖에서 MSIX 를 직접 설치해 보려면 `-Sign -CertPath x.pfx -CertPassword ...` 로 서명합니다.
- `target/release/noti-bridge.exe` 를 바로 실행해도 됩니다. 이 경우 패키지 ID 가 없어 PC 알림 읽기와 자동 시작만 꺼지고 나머지는 동작합니다.

macOS:

```bash
npm --prefix shared/desktop-ui ci && npm --prefix shared/desktop-ui run build
cd mac/src-tauri && npx @tauri-apps/cli@2 build
```

iOS (Xcode, `brew install xcodegen`):

```bash
bash scripts/build-ios-core.sh
cd ios && xcodegen generate && open NotiBridge.xcodeproj
```
