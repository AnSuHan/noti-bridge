# noti-bridge 버전 기록

## v0.0.3 (2026-11-27)

사용자에게 보이는 변경 요약은 [CHANGELOG.md](../CHANGELOG.md) 에 있다.

### PC 알림 읽기
- 보고된 증상(PC 알림이 폰·앱 목록에 없음)의 직접 원인은 패키지 없는 exe 실행이었다. `UserNotificationListener` 는 패키지 ID 가 있어야 해서 exe 판은 `capture_state = unsupported`. 알림 탭에 안내를 넣었다.
- 폴링에서 `describe(&n)?` 가 알림 하나의 실패로 그 주기 전체를 버리던 버그를 고쳤다. ToastGeneric 바인딩이 없는 예전 템플릿 토스트는 첫 바인딩을 읽는다.
- `%LOCALAPPDATA%\Microsoft\Windows\Notifications\wpndatabase.db` 를 직접 읽는 대안은 Windows 11 26200 에서 알림 센터의 토스트(Docker 등)가 DB 에 없어 쓰지 않았다.
- `scripts/install-windows-dev.ps1`: 빌드 → MSIX 레이아웃 등록(개발자 모드) → 패키지로 실행. 등록본에서 PC 토스트가 앱 목록에 들어오는 것을 확인했다.

### 표시·숨김·동기화
- 코어: `pinned` 표(기기별 고정), `NotificationRecord.{ongoing, pinned, hidden_reason}`, `visible_notifications` 는 고정 먼저 정렬, `hidden_notifications`(직접 숨김은 filter_id 0), `clear(app, dismiss)`.
- 숨기기·필터·앱 규칙을 로그 이벤트로 보내고, 바뀐 경우에만 다시 퍼뜨린다(PROTOCOL.md). 상대에게서 받은 필터는 계정에도 올린다.
- 진행 중 알림(ongoing)은 alert 없음, 갱신돼도 숨김 유지. Android 는 미디어 알림만 진행 중이어도 보낸다(그 밖의 진행 중 알림은 여전히 제외).
- 필터 종류 `HiddenContent`(본문 숨김 알림), 필터 `label`(DB 열 추가, 이전 DB 자동 변환, Firestore `label` 필드).

### Android 본문 추출
- 후보(긴 본문, 본문, 목록형 줄, 요약, ticker) 중 "새 메시지 N개" 같은 요약 문구가 아닌 첫 값을 쓴다. 같은 그룹에 내용 있는 알림이 있으면 요약 문구만 든 알림은 보내지 않고, 이미 보냈으면 내린다.

### 소셜 로그인
- `core/src/oauth.rs`: Google 루프백(127.0.0.1 임시 포트) + PKCE(S256), GitHub device flow. `cloud.rs` 의 `signInWithIdp`, `sendOobCode(PASSWORD_RESET)`. 세션에 `provider`(이전 세션은 password).
- Android 는 Credential Manager(`GetGoogleIdOption`, serverClientId = 웹 클라이언트 ID)로 받은 ID 토큰을 코어에 넘긴다.
- OAuth 클라이언트 값: 빌드 환경 변수 `NOTIBRIDGE_GOOGLE_CLIENT_ID`, `NOTIBRIDGE_GOOGLE_CLIENT_SECRET`, `NOTIBRIDGE_GOOGLE_WEB_CLIENT_ID`, `NOTIBRIDGE_GITHUB_CLIENT_ID` 또는 데스크톱 설정. 값이 있는 버튼만 보인다.
- 검증: PKCE RFC 7636 예제, base64url 벡터, 루프백 수신(favicon 무시·state 확인) 단위 테스트. 에뮬레이터용 Google ID 토큰 테스트는 추가했지만 이번에 돌리지 않았다. 실제 OAuth 클라이언트로는 시험하지 않았다.

## v0.0.2 (2026-11-21)

사용자에게 보이는 변경 요약은 [CHANGELOG.md](../CHANGELOG.md) 에 있다.

### 알림 표시와 동기화
- Windows 토스트: 패키지 없는 실행은 원래 앱마다 AUMID(`com.notibridge.mirror.<앱 해시>`)를 HKCU 에 등록해 머리글에 원래 앱 이름·아이콘을 보인다. 패키지 실행은 자기 AUMID 만 쓸 수 있어 토스트 `<header>` 로 앱 이름을 묶는다. PC 알림 읽기는 `com.notibridge.` 로 시작하는 AUMID 를 건너뛰어 미러 토스트를 되돌려 보내지 않는다.
- 같은 알림의 갱신: 제목·본문이 바뀌면 다시 팝업, 같으면 조용히 교체. 예전에는 갱신을 모두 조용히 처리해 카카오톡·Slack 의 두 번째 메시지부터 팝업이 없었다.
- Android 본문 추출: `EXTRA_BIG_TEXT` 가 빈 문자열이면 그대로 쓰던 버그를 고치고, MessagingStyle 메시지(최근 3개, 단체 대화는 보낸 사람 포함) → 긴 본문 → 본문 → 목록형 줄 → 요약 → ticker 순으로 첫 값을 쓴다. 제목은 대화방 이름 → 큰 제목 → 제목.
- 코어 `NodeEvent::NotificationChanged` 에 `alert` 를 추가했다. 끊겨 있던 동안 쌓인 이벤트(상대 `Hello.head_seq` 이하)와 1분보다 오래된 알림은 `alert=false` 로, PC 는 알림 센터에만(SuppressPopup), Android 는 무음 채널로, macOS 는 앱 목록에만 넣는다. 데스크톱은 3초에 3개를 넘는 팝업도 조용히 넣는다.
- "숨기기"(`hide_local`): 로컬 `hidden` 표에만 기록하고 원본에 아무것도 보내지 않는다. 원본이 내용을 바꾸면 다시 보인다. PC 토스트를 닫으면 기본으로 숨기기, 설정 "PC 알림을 닫으면 폰 알림도 지우기" 를 켜면 예전처럼 삭제.
- Android: 미디어 알림(`EXTRA_MEDIA_SESSION`, `CATEGORY_TRANSPORT`, MediaStyle)은 미러링하지 않고, 원격 삭제 요청이 와도 미디어·고정 알림은 지우지 않는다.

### 필터
- 코어 `filters` 표. 앱(패키지 또는 표시 이름 일치), 키워드(제목·본문 포함), 대소문자 무시. 받는 쪽은 저장하되 보여주지 않고, 원본 쪽은 보내지 않는다. 필터를 추가·삭제하면 보이던 알림을 내리거나 조용히 다시 띄운다.

### 계정(Firebase)
- `core/src/cloud.rs`: Identity Toolkit 이메일 가입·로그인, securetoken 갱신, Firestore REST. TLS 는 iroh 와 같은 rustls(ring) + webpki 루트라 Android 에서도 추가 초기화가 없다.
- 저장: `users/{uid}/devices/{device_id}`(이름·플랫폼·버전), `users/{uid}/settings/filters`(필터, 바꾼 시각). 필터는 더 최근에 바꾼 쪽이 이긴다. 알림 내용은 올리지 않는다. 갱신 토큰은 로컬 DB 에 저장소 키로 암호화해 둔다.
- Firebase 프로젝트 값은 빌드 환경 변수 `NOTIBRIDGE_FIREBASE_API_KEY`, `NOTIBRIDGE_FIREBASE_PROJECT_ID` 또는 앱 설정 입력. 실행 시 `NOTIBRIDGE_FIREBASE_EMULATOR=auth호스트:포트,firestore호스트:포트` 로 에뮬레이터를 쓴다.
- 검증: Firebase 에뮬레이터(firebase-tools 13.29.1)로 `core/tests/cloud_emulator.rs` 통과(가입, 잘못된 비밀번호, 중복 가입, 다른 기기 로그인 시 필터 수신, 기기 목록, 최근 쪽 필터 반영, 로그아웃 시 기기 제거). 보안 규칙으로 다른 사용자 경로 접근이 403 인 것도 확인. 실제 Firebase 프로젝트로는 시험하지 않았다.

### 다국어와 버전 정보
- 한국어·영어. 코어 오류·변경 사항 문구(`i18n`), 데스크톱 트레이·토스트·UI, Android 리소스(`values`, `values-ko`)와 앱 안 언어 선택. iOS 는 코어 언어만 맞췄고 화면 문구는 그대로다.
- 앱 안 "정보" 탭: 버전과 변경 사항(`core/src/release.rs`, 맨 위 항목이 빌드 버전과 다르면 테스트가 실패한다).

### 프로토콜
- `Hello.head_seq` 추가(하위 호환).

### Microsoft Store 배포 준비 (v0.0.1 이후 미배포분)

- Windows 는 Microsoft Store 로 배포한다. 스토어가 서명하므로 코드 서명 인증서 없이 설치되고, 패키지 ID 가 생겨 PC 알림 읽기(PC → 폰)도 켜진다.
- 로그인 시 자동 시작을 추가했다(MSIX StartupTask). 자동 시작으로 켜지면 창 없이 트레이에만 둔다. 앱을 한 번 실행한 뒤부터 동작하고 Windows 설정에서 끌 수 있다.
- 빌드 스크립트가 MSIX 버전을 `Cargo.toml` 버전으로 채우고, `tauri.conf.json` 버전과 다르면 멈춘다.
- 스토어 제출용 패키지 이름·게시자를 빌드 인자나 환경 변수로 넣을 수 있게 했다.
- 패키지에 UDP 수신 방화벽 규칙을 선언해, 첫 실행 때 Windows 의 네트워크 허용 창이 뜨지 않는다. 이전에는 이 창에서 취소하면 같은 망 직접 연결이 막힐 수 있었다.
- PC 알림 접근이 꺼져 있으면 설정 화면에 안내와 Windows 설정 열기 버튼을 보여준다. 허용하면 다시 시작하지 않아도 알림 읽기가 시작된다.
- 개인정보 처리방침 초안([PRIVACY.md](PRIVACY.md))을 추가했다.

### 메모리 사용 줄이기

- PC 앱(Windows·macOS)은 창을 닫으면 숨기지 않고 WebView 까지 내린다. 트레이에 남아 알림은 계속 받고, 트레이에서 열면 창을 새로 만든다. 트레이 메뉴의 종료만 앱을 끝낸다.
  - Windows 릴리스 빌드 실측: 창을 연 상태 약 119MB(프로세스 7개), 창을 닫고 트레이에 둔 상태 약 8MB(프로세스 1개). 이전에는 창을 닫아도 119MB 가 그대로 남았다.
- 창이 닫혀 있을 때 페어링 확인 코드가 오면 창을 띄우고 코드를 이어서 보여준다.
- PC 앱의 비동기 작업 스레드를 CPU 코어 수만큼에서 2개로 줄였다.
- 알림 아이콘을 UI 로 넘길 때 JSON 숫자 배열 대신 원본 바이트를 쓰고, 같은 아이콘은 한 번만 받아 같이 쓴다.
- 코어 SQLite 페이지 캐시를 2MB 에서 512KB 로 줄였다.
- Android 아이콘 캐시를 개수 64개 대신 256KB 로 제한하고, 변환에 쓴 Bitmap 을 바로 해제한다.

## v0.0.1 (2026-11-01)

첫 공개 버전이다. Android 와 Windows 사이의 알림 미러링을 시험해 볼 수 있는 단계이며, 일상 사용을 보장하지는 않는다.

### 포함된 기능

- QR 과 6자리 코드 페어링, 신뢰 목록.
- 폰 알림을 PC 토스트와 앱 목록에 표시, 양방향 삭제, 앱 목록에서 액션과 답장.
- 같은 망 직접 연결과 공개 릴레이 경유, 오프라인 변경의 재연결 후 전달.
- 앱별 허용·차단·본문 숨김, 방해 금지 동기화, 알림 내용 저장 시 암호화와 24시간 보존.

### 이번 검증에서 고친 것

- 페어링 직후 상대에게 거부당한 기기가 끝없이 재접속하던 문제. 5초에 537번 다시 걸던 것을 간격을 두고 다시 걸도록 바꿨다.
- 앱을 다시 시작하면 이전 세션의 토스트가 원격 삭제로 지워지지 않던 문제.
- 페어링이 끝나도 사용한 QR 이 화면에 남던 문제.
- CI 에서 실패했을 `gradlew` 실행 권한 누락, iOS Swift 바인딩 생성 경로, 스크립트 줄바꿈.

### 배포 파일

| 파일 | 대상 | 비고 |
|---|---|---|
| `noti-bridge-0.0.1-windows-x64.exe` | Windows 10 이상 x64 | 서명 없음. WebView2 런타임 필요. PC 알림 읽기 꺼짐 |
| `noti-bridge-0.0.1-android.apk` | Android 8 이상 arm64, x86_64 | 디버그 키 서명 |

### 알려진 제한

- 토스트의 닫기 버튼과 토스트 안 답장은 시험하지 못했다. 삭제와 답장은 앱 목록에서 하면 동작한다.
- Android 는 에뮬레이터에서만 시험했다.
- macOS, iOS 는 배포하지 않는다. iOS 는 컴파일도 확인되지 않았다.
