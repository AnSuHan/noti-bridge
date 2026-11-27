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
- **지우기와 숨기기**: "지우기" 는 원본 기기까지 지우고, "숨기기" 는 화면에서만 내립니다. 숨기기는 연결된 모든 기기에 같이 적용되고(폰을 사이에 둔 PC 끼리도), 원본 폰 알림은 그대로 둡니다. PC 토스트를 닫으면 기본은 숨기기라 폰의 음악 재생 알림 등이 꺼지지 않습니다.
- **앱별 묶음, 모두 지우기, 고정**: 알림을 앱별로 묶어 보여 주고, 앱별 지우기와 "모두 지우기"(기본은 숨기기)가 있습니다. 고정한 알림은 맨 위에 남고 모두 지우기에서도 남습니다.
- **재생 중 알림**: YouTube·음악 같은 미디어 알림은 다른 기기 목록 맨 위에 고정되고 팝업하지 않습니다. 재생/일시정지 버튼을 PC 에서 누를 수 있고, 원격에서 지워도 폰 알림은 지우지 않습니다.
- **숨긴 항목**: 알림 탭의 "숨긴 항목"에서 숨긴 앱·숨긴 알림·필터에 걸린 알림을 보고 다시 보이게 합니다.
- **필터**: 앱(이름 또는 앱 ID)·키워드(제목·본문)·"내용 숨겨진 알림"으로 거릅니다. 걸린 알림은 표시되지 않고, 원본 기기는 다른 기기로 보내지 않습니다. 필터와 앱 규칙은 연결된 기기끼리, 그리고 계정으로 동기화됩니다.
- **내용 숨겨짐 안내**: 본문이 숨겨진 이유(앱 규칙 / OS 가림)를 보여 줍니다. 앱 규칙 때문이면 "내용 표시"로 폰의 규칙까지 바로 풉니다. Android 15 이상의 OS 가림은 PC 에서 `adb shell appops set com.notibridge.app RECEIVE_SENSITIVE_NOTIFICATIONS allow` 를 한 번 실행하면 풀립니다.
- **쌓인 알림은 조용히**: 앱을 새로 켜거나 다시 연결되면 끊겨 있던 동안의 알림은 팝업 없이 목록과 알림 센터에만 들어갑니다.
- **계정(Firebase, 선택)**: 이메일·Google·GitHub 로 로그인하면 같은 계정의 기기 목록과 필터가 동기화됩니다. 알림 내용은 계정에 올리지 않습니다.
- **한국어·영어**: 기본은 시스템 언어, 설정에서 바꿀 수 있습니다. "정보" 탭에서 버전과 변경 사항을 봅니다.

## Firebase 계정 설정 (선택)

계정 기능은 Firebase 프로젝트가 있어야 켜집니다. 없어도 나머지 기능은 그대로 동작합니다.

1. Firebase 콘솔에서 프로젝트를 만들고 Authentication > 이메일/비밀번호 로그인을 켭니다.
2. Firestore 데이터베이스를 만들고 [firebase/firestore.rules](firebase/firestore.rules) 를 보안 규칙으로 배포합니다(`cd firebase && npx firebase-tools deploy --only firestore:rules --project <프로젝트 ID>`). 사용자는 자기 `users/{uid}` 아래만 읽고 씁니다.
3. 프로젝트 설정의 웹 API 키와 프로젝트 ID 를 앱 설정 > 계정에 넣거나, 빌드할 때 환경 변수 `NOTIBRIDGE_FIREBASE_API_KEY`, `NOTIBRIDGE_FIREBASE_PROJECT_ID` 로 넣습니다.

소셜 로그인(선택). 값이 있는 버튼만 앱에 나타납니다.

| 로그인 | Firebase 콘솔 | OAuth 클라이언트 | 빌드 환경 변수 |
|---|---|---|---|
| Google (Windows·macOS) | Authentication > 로그인 방법 > Google 사용 | Google Cloud 콘솔(같은 프로젝트) > 사용자 인증 정보 > OAuth 클라이언트 ID > **데스크톱 앱** | `NOTIBRIDGE_GOOGLE_CLIENT_ID`, `NOTIBRIDGE_GOOGLE_CLIENT_SECRET` |
| Google (Android) | 위와 같음 + 프로젝트 설정에 Android 앱(`com.notibridge.app`)과 서명 키 SHA-1 등록 | Firebase 가 만든 **웹 클라이언트** ID(Credential Manager 의 serverClientId) | `NOTIBRIDGE_GOOGLE_WEB_CLIENT_ID` |
| GitHub (전체) | Authentication > 로그인 방법 > GitHub 사용, 아래 OAuth 앱의 ID·비밀번호 입력 | GitHub > Settings > Developer settings > OAuth Apps 에서 앱을 만들고 **Enable Device Flow** 를 켬. 콜백 URL 은 Firebase 가 보여 주는 값 | `NOTIBRIDGE_GITHUB_CLIENT_ID` |

- 데스크톱 Google 로그인은 브라우저를 열고 `127.0.0.1` 임시 포트로 돌아오는 표준 방식(루프백 + PKCE)입니다. 데스크톱 클라이언트 비밀번호는 앱에 들어가는 값이라 비밀로 취급되지 않습니다(Google 문서 기준).
- GitHub 는 기기 인증 방식이라, 앱에 뜬 코드를 github.com/login/device 에 넣으면 됩니다.
- 데스크톱은 설정 > 계정 > 프로젝트 변경에서 같은 값을 넣을 수도 있습니다.
- 같은 이메일을 다른 방법으로 이미 가입했다면 처음 가입한 방법으로 로그인해야 합니다(Firebase 기본 설정 "이메일당 계정 1개").

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
- `target/release/noti-bridge.exe`(또는 릴리스의 `-windows-x64.exe`)를 바로 실행해도 됩니다. 이 경우 패키지 ID 가 없어 **PC 알림 읽기**(PC 알림을 폰과 앱 목록으로 보내기)와 자동 시작이 꺼지고 나머지는 동작합니다. 앱의 알림 탭에도 이 안내가 나옵니다.
- 개발 PC 에서 빌드부터 패키지 등록까지 한 번에: `./scripts/install-windows-dev.ps1` (개발자 모드 필요, 실행 중인 noti-bridge 를 끄고 다시 등록합니다).

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
