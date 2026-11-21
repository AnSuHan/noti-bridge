# noti-bridge

폰과 PC 사이에서 알림과 그 상태(도착, 갱신, 삭제, 액션)를 맞추는 앱입니다. 한쪽에서 지우면 양쪽에서 사라집니다.
자체 서버 없이 iroh P2P 로 연결하며, 같은 망이면 직접, 다른 망이면 홀펀칭 또는 공개 릴레이 중계를 씁니다.
프로토콜은 [docs/PROTOCOL.md](docs/PROTOCOL.md), 버전별 변경 사항은 [CHANGELOG.md](CHANGELOG.md)(자세한 개발 기록은 [docs/VERSIONS.md](docs/VERSIONS.md)), 개인정보 처리방침은 [docs/PRIVACY.md](docs/PRIVACY.md) 에 있습니다.

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

## 주요 기능

- **원래 앱 이름으로 표시**: 폰의 카카오톡·Slack 알림이 PC 토스트에 원래 앱 이름과 아이콘으로 뜹니다(패키지 없는 exe). 스토어(MSIX) 설치본은 토스트 안 머리글로 앱 이름을 보여줍니다.
- **지우기와 숨기기**: "지우기" 는 원본 기기까지 지우고, "숨기기" 는 이 기기 화면에서만 내립니다. PC 토스트를 닫으면 기본은 숨기기라 폰의 음악 재생 알림 등이 꺼지지 않습니다. 음악 재생 같은 미디어 알림은 미러링하지 않습니다.
- **필터**: 앱(이름 또는 앱 ID)·키워드(제목·본문)로 거릅니다. 걸린 알림은 이 기기에 표시되지 않고, 이 기기의 알림이면 다른 기기로 보내지 않습니다.
- **쌓인 알림은 조용히**: 앱을 새로 켜거나 다시 연결되면 끊겨 있던 동안의 알림은 팝업 없이 목록과 알림 센터에만 들어갑니다.
- **계정(Firebase, 선택)**: 이메일로 로그인하면 같은 계정의 기기 목록과 필터가 동기화됩니다. 알림 내용은 계정에 올리지 않습니다.
- **한국어·영어**: 기본은 시스템 언어, 설정에서 바꿀 수 있습니다. "정보" 탭에서 버전과 변경 사항을 봅니다.

## Firebase 계정 설정 (선택)

계정 기능은 Firebase 프로젝트가 있어야 켜집니다. 없어도 나머지 기능은 그대로 동작합니다.

1. Firebase 콘솔에서 프로젝트를 만들고 Authentication > 이메일/비밀번호 로그인을 켭니다.
2. Firestore 데이터베이스를 만들고 [firebase/firestore.rules](firebase/firestore.rules) 를 보안 규칙으로 배포합니다(`cd firebase && npx firebase-tools deploy --only firestore:rules --project <프로젝트 ID>`). 사용자는 자기 `users/{uid}` 아래만 읽고 씁니다.
3. 프로젝트 설정의 웹 API 키와 프로젝트 ID 를 앱 설정 > 계정에 넣거나, 빌드할 때 환경 변수 `NOTIBRIDGE_FIREBASE_API_KEY`, `NOTIBRIDGE_FIREBASE_PROJECT_ID` 로 넣습니다.

에뮬레이터 시험:

```bash
cd firebase && npx firebase-tools emulators:start --only auth,firestore --project demo-noti-bridge
cargo test -p noti-bridge-core --test cloud_emulator -- --ignored
```

## 빌드

Rust 1.91 이상이 필요합니다.

```bash
cargo test -p noti-bridge-core                                  # 코어 테스트
cargo test -p noti-bridge-core --test sync_e2e -- --ignored     # 공개 릴레이 경유 테스트
cargo test -p noti-bridge-windows -- --ignored                  # 실제 Windows 토스트를 띄우는 시험
```

새 버전을 낼 때는 `Cargo.toml` 버전과 함께 `core/src/release.rs` 맨 위에 변경 사항을 추가하고 `CHANGELOG.md` 를 맞춥니다. `release.rs` 맨 위 버전이 `Cargo.toml` 과 다르면 코어 테스트가 실패합니다.

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
