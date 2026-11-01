# noti-bridge

폰과 PC 사이에서 알림과 그 상태(도착, 갱신, 삭제, 액션)를 맞추는 앱입니다. 한쪽에서 지우면 양쪽에서 사라집니다.
자체 서버 없이 iroh P2P 로 연결하며, 같은 망이면 직접, 다른 망이면 홀펀칭 또는 공개 릴레이 중계를 씁니다.
프로토콜은 [docs/PROTOCOL.md](docs/PROTOCOL.md), 버전 기록은 [docs/VERSIONS.md](docs/VERSIONS.md) 에 있습니다.

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
| Windows | MSIX 설치 시 가능 | 토스트 | 양방향 |
| macOS | 불가 | 알림 센터 | 앱 안 목록에서 삭제 시 전송, 수신 삭제는 앱 목록에만 반영 |
| iOS | 불가 | 로컬 알림 | 양방향, 단 앱이 실행 중일 때 |

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
./scripts/build-windows.ps1                     # target/noti-bridge.msix
./scripts/build-windows.ps1 -Sign -CertPath x.pfx -CertPassword ...
```

서명 없이 쓰려면 `target/release/noti-bridge.exe` 를 바로 실행할 수 있습니다. 이 경우 패키지 ID 가 없어 PC 알림 읽기만 꺼지고 나머지는 동작합니다.

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
