# noti-bridge 개인정보 처리방침

시행일: 2026-11-07 (초안)

noti-bridge(이하 "앱")는 사용자의 폰과 PC 사이에서 알림과 그 상태를 맞추는 앱입니다. 이 문서는 앱이 어떤 정보를 다루고, 어디로 보내며, 얼마나 보관하는지 설명합니다.

## 요약

- 앱 개발자는 서버를 운영하지 않으며, 사용자의 알림 내용을 받거나 저장하지 않습니다.
- 알림 내용은 사용자가 직접 페어링한 기기에만 암호화되어 전달됩니다.
- 기기에 저장되는 알림 내용은 암호화되며 24시간이 지나면 지워집니다.
- 광고, 분석, 사용 통계 수집이 없습니다.

## 앱이 다루는 정보

| 정보 | 용도 | 어디에 있나 |
|---|---|---|
| 알림 내용: 앱 이름, 앱 ID, 제목, 본문, 버튼 이름, 앱 아이콘 | 페어링한 기기에 같은 알림을 표시하고 삭제·버튼·답장을 맞추기 위해 | 알림이 생긴 기기, 페어링한 기기 |
| 답장 내용 | 다른 기기에서 쓴 답장을 원래 알림의 앱에 전달하기 위해 | 보낸 기기, 원래 알림이 있는 기기 |
| 기기 정보: 기기 이름, 운영체제 종류 | 페어링한 기기 목록에 표시하기 위해 | 페어링한 기기 |
| 기기 키: 기기마다 만드는 공개 키와 개인 키 | 기기를 식별하고 연결을 암호화하기 위해 | 개인 키는 그 기기에만 있고 밖으로 나가지 않음 |
| 연결 주소: IP 주소와 릴레이 주소 | 기기끼리 연결하기 위해 | 페어링한 기기, 아래의 연결 보조 서비스 |
| 앱별 규칙, 방해 금지 상태, 릴레이 설정 | 사용자가 정한 대로 동작하기 위해 | 그 기기 (방해 금지는 페어링한 기기와 공유) |

앱은 다른 앱의 알림을 읽기 위해 운영체제의 알림 접근 권한을 사용합니다. Android 에서는 알림 이름과 아이콘을 보여주기 위해 설치된 앱 목록을 조회합니다. 이 정보는 기기 밖으로 보내지 않고 알림을 표시하는 데에만 씁니다.

은행·인증 앱 등 민감한 앱은 기본으로 본문을 보내지 않습니다. 사용자는 앱별로 허용, 본문 숨김, 차단을 정할 수 있습니다.

## 전달 방식

- 기기끼리는 iroh(QUIC) 로 직접 연결하며, 연결은 종단 간 암호화됩니다.
- 직접 연결할 수 없을 때는 릴레이 서버가 암호화된 데이터를 중계합니다. 기본값은 iroh 개발사 n0 가 운영하는 공개 릴레이이며, 설정에서 직접 운영하는 릴레이로 바꿀 수 있습니다. 릴레이는 암호화된 내용을 풀 수 없지만, 연결에 쓰는 IP 주소와 기기 공개 키는 볼 수 있습니다.
- 기기를 찾기 위해 n0 의 주소 조회 서비스에 기기 공개 키와 연결용 주소 정보를 게시합니다. 알림 내용은 게시하지 않습니다.
- 같은 네트워크 안에서는 mDNS 로 기기를 찾습니다. 이때 같은 네트워크의 다른 기기가 기기 공개 키를 볼 수 있습니다.

## 보관과 삭제

- 알림 내용과 동기화 기록은 기기 안의 데이터베이스에 암호화해서 저장합니다. 암호화 키는 Windows 는 DPAPI, macOS·iOS 는 키체인, Android 는 Keystore 로 보호합니다.
- 알림 내용은 24시간이 지나면 지웁니다. 상대 기기가 받은 동기화 기록도 주기적으로 지웁니다.
- 앱 아이콘은 같은 아이콘을 다시 보내지 않도록 기기에 보관합니다.
- 페어링을 해제하면 그 기기와 관련된 기록을 지웁니다. 앱을 지우면 기기에 저장된 모든 데이터가 함께 지워집니다.

## 제3자 제공

앱 개발자는 사용자의 정보를 제3자에게 제공하거나 판매하지 않습니다. 위 "전달 방식"의 릴레이와 주소 조회 서비스 외에는 외부로 보내는 정보가 없습니다.

## 아동

앱은 아동을 대상으로 하지 않으며, 나이 정보를 수집하지 않습니다.

## 변경과 문의

이 방침이 바뀌면 이 문서를 고치고 시행일을 갱신합니다. 문의는 GitHub 저장소의 Issues 로 해 주세요: https://github.com/AnSuHan/noti-bridge/issues

---

# noti-bridge Privacy Policy (English summary)

Effective: 2026-11-07 (draft)

- The developer runs no server and never receives or stores your notifications.
- Notification content (app name and ID, title, body, action titles, app icon) and replies are sent only to devices you paired yourself, over end-to-end encrypted iroh (QUIC) connections.
- When a direct connection is not possible, an encrypted relay is used. By default this is the public relay run by n0, the developer of iroh; you can set your own relay. Relays cannot read the content but can see IP addresses and device public keys. The app also publishes its device public key and connection addresses to n0's address lookup service, and uses mDNS on the local network.
- Notification content is stored encrypted on the device and deleted after 24 hours. Unpairing deletes data for that device; uninstalling deletes all data.
- The app reads other apps' notifications using the operating system's notification access permission. Sensitive apps such as banking and authenticator apps have their message bodies hidden by default.
- No ads, analytics, or tracking. No data is sold or shared with third parties beyond the relay and address lookup services above.
- Contact: https://github.com/AnSuHan/noti-bridge/issues
