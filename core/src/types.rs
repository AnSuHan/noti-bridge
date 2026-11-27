//! 플랫폼 앱과 주고받는 공개 타입.

use crate::pb;

pub const DEFAULT_RETENTION_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum NotificationState {
    Active,
    Dismissed,
    Expired,
}

/// 본문이 숨겨진 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum HiddenReason {
    /// 숨겨지지 않았거나, 이유를 알리지 않는 이전 버전 기기에서 왔다.
    None,
    /// 원본 기기의 앱 규칙(본문 숨김). 규칙을 "허용"으로 바꾸면 보인다.
    Policy,
    /// OS 가 가려서 넘겨줬다(Android 15 민감 알림 등). 앱 규칙으로는 풀 수 없다.
    System,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ActionInfo {
    pub id: String,
    pub title: String,
    pub accepts_reply: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NotificationRecord {
    pub nid: String,
    pub origin: String,
    pub app_pkg: String,
    pub app_name: String,
    pub icon_hash: String,
    pub title: String,
    pub body: String,
    pub actions: Vec<ActionInfo>,
    pub state: NotificationState,
    pub version: u64,
    pub updated_at: i64,
    pub body_hidden: bool,
    pub hidden_reason: HiddenReason,
    /// 음악·동영상 재생처럼 계속 떠 있는 알림. 목록 위에 고정하고 팝업하지 않는다.
    pub ongoing: bool,
    /// 이 기기에서 사용자가 고정했거나 ongoing 이라 목록 위에 둔다. 기기마다 다르며 전송하지 않는다.
    pub pinned: bool,
}

/// 플랫폼 알림 리스너가 캡처한 알림.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CapturedNotification {
    pub key: String,
    pub app_pkg: String,
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub actions: Vec<ActionInfo>,
    pub icon_png: Option<Vec<u8>>,
    /// OS 가 내용을 가린 경우(Android 민감 알림 등).
    pub body_hidden: bool,
    /// 음악·동영상 재생 같은 진행 중 알림.
    pub ongoing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum RemovalReason {
    Dismissed,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ConnectionPath {
    Direct,
    Relay,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AppPolicy {
    Allow,
    Block,
    HideBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AppRule {
    pub app_pkg: String,
    pub policy: AppPolicy,
}

/// 이 기기에서 감출 알림을 고르는 필터의 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum FilterKind {
    /// 앱 패키지 이름(앱 ID)이나 앱 표시 이름이 같으면 걸린다. 대소문자는 가리지 않는다.
    App,
    /// 제목이나 본문에 이 낱말이 들어 있으면 걸린다. 대소문자는 가리지 않는다.
    Keyword,
    /// 본문이 숨겨진("내용 숨겨짐") 알림이면 걸린다. pattern 은 쓰지 않는다.
    HiddenContent,
}

impl FilterKind {
    pub fn to_u32(self) -> u32 {
        match self {
            Self::App => 0,
            Self::Keyword => 1,
            Self::HiddenContent => 2,
        }
    }

    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::App),
            1 => Some(Self::Keyword),
            2 => Some(Self::HiddenContent),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Keyword => "keyword",
            Self::HiddenContent => "hidden_content",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "app" => Some(Self::App),
            "keyword" => Some(Self::Keyword),
            "hidden_content" => Some(Self::HiddenContent),
            _ => None,
        }
    }
}

/// HiddenContent 필터에 저장하는 pattern. 비어 있으면 안 되므로 고정 값을 쓴다.
pub const HIDDEN_CONTENT_PATTERN: &str = "*";

/// 기기·계정 사이에 주고받는 필터 한 개(로컬 id 없이).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterSpec {
    pub kind: FilterKind,
    pub pattern: String,
    pub label: String,
}

/// 필터에 걸린 알림은 이 기기에 보이지 않고, 이 기기가 원본이면 다른 기기로 보내지도 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NotificationFilter {
    pub id: i64,
    pub kind: FilterKind,
    pub pattern: String,
    /// 화면에 보여 줄 이름. 앱 필터는 앱 이름(없으면 pattern 을 보여준다).
    pub label: String,
}

impl NotificationFilter {
    pub fn matches(&self, app_pkg: &str, app_name: &str, title: &str, body: &str, body_hidden: bool) -> bool {
        let pattern = self.pattern.trim().to_lowercase();
        match self.kind {
            FilterKind::HiddenContent => body_hidden,
            _ if pattern.is_empty() => false,
            FilterKind::App => app_pkg.to_lowercase() == pattern || app_name.trim().to_lowercase() == pattern,
            FilterKind::Keyword => title.to_lowercase().contains(&pattern) || body.to_lowercase().contains(&pattern),
        }
    }

    pub fn matches_record(&self, r: &NotificationRecord) -> bool {
        self.matches(&r.app_pkg, &r.app_name, &r.title, &r.body, r.body_hidden)
    }
}

/// 이 기기 화면에 보이지 않는 알림과 그 이유.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct HiddenNotification {
    pub notification: NotificationRecord,
    /// 이 알림을 감춘 필터 id. 0 이면 사용자가 직접 숨긴 것이다.
    pub filter_id: i64,
}

/// GitHub 기기 인증(device flow)의 사용자 코드. 사용자는 verification_uri 를 열어 user_code 를 넣는다.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct DeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: i64,
}

/// Firebase 계정 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CloudStatus {
    /// Firebase 프로젝트 값(API 키, 프로젝트 ID)이 있어 로그인할 수 있는지.
    pub configured: bool,
    pub signed_in: bool,
    pub email: String,
    /// 로그인 방법: "password", "google.com", "github.com". 로그인 전에는 빈 값.
    pub provider: String,
    pub project_id: String,
    /// 마지막으로 동기화에 성공한 시각(ms). 없으면 0.
    pub last_sync: i64,
    /// 소셜 로그인에 필요한 OAuth 클라이언트 값이 있어 버튼을 보여 줄 수 있는지.
    pub google_available: bool,
    pub github_available: bool,
}

/// 같은 계정에 로그인한 기기. 알림 내용은 계정에 올리지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CloudDevice {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub version: String,
    pub updated_at: i64,
    /// 이 기기 자신인지.
    pub is_self: bool,
    /// 이 기기와 이미 페어링돼 있는지.
    pub paired: bool,
}

/// 앱 안에 보여 줄 버전별 변경 사항.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ReleaseNote {
    pub version: String,
    pub date: String,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PeerInfo {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub paired_at: i64,
    pub path: ConnectionPath,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NodeConfig {
    pub data_dir: String,
    /// iroh 기기 개인키(32바이트).
    pub secret_key: Vec<u8>,
    /// 로컬 DB 암호화 키(32바이트).
    pub storage_key: Vec<u8>,
    pub device_name: String,
    pub platform: String,
    /// 비어 있으면 iroh 기본 공개 릴레이를 쓴다.
    pub relay_url: Option<String>,
    pub relay_disabled: bool,
    pub mdns_enabled: bool,
    pub retention_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum NodeEvent {
    /// alert 가 false 면 소리·팝업 없이 목록과 알림 센터에만 넣는다.
    /// 앱을 꺼 둔 동안 쌓였던 알림, 필터를 풀어 다시 보이게 된 알림이 여기에 해당한다.
    NotificationChanged { notification: NotificationRecord, alert: bool },
    NotificationRemoved { nid: String, state: NotificationState },
    /// 이 기기가 원본인 알림을 OS 에서 지워야 한다.
    CancelLocal { key: String },
    InvokeAction { key: String, action_id: String },
    SendReply { key: String, action_id: String, text: String },
    DndChanged { enabled: bool },
    ConnectionChanged { device_id: String, path: ConnectionPath },
    PairingCode { device_id: String, device_name: String, code: String },
    PairingFinished { device_id: String, device_name: String, accepted: bool },
    PairingFailed { reason: String },
    PeerUnpaired { device_id: String },
    /// 계정 로그인 상태나 계정에서 받은 설정(필터)이 바뀌었다. UI 는 상태를 다시 읽는다.
    CloudChanged,
    /// 앱 규칙이 바뀌었다(이 기기 또는 연결된 기기에서). 알림을 캡처하는 기기는 그 앱의 현재 알림을 다시 읽어
    /// 본문 숨김 해제 같은 변화를 바로 반영한다.
    AppRuleChanged { app_pkg: String },
}

pub fn make_nid(origin: &str, key: &str) -> String {
    format!("{origin}:{key}")
}

pub fn nid_key<'a>(origin: &str, nid: &'a str) -> Option<&'a str> {
    nid.strip_prefix(origin)?.strip_prefix(':')
}

impl From<pb::NotificationState> for NotificationState {
    fn from(s: pb::NotificationState) -> Self {
        match s {
            pb::NotificationState::Dismissed => Self::Dismissed,
            pb::NotificationState::Expired => Self::Expired,
            _ => Self::Active,
        }
    }
}

impl From<NotificationState> for pb::NotificationState {
    fn from(s: NotificationState) -> Self {
        match s {
            NotificationState::Active => Self::Active,
            NotificationState::Dismissed => Self::Dismissed,
            NotificationState::Expired => Self::Expired,
        }
    }
}

impl From<HiddenReason> for pb::HiddenReason {
    fn from(r: HiddenReason) -> Self {
        match r {
            HiddenReason::None => Self::Unspecified,
            HiddenReason::Policy => Self::Policy,
            HiddenReason::System => Self::System,
        }
    }
}

impl From<pb::HiddenReason> for HiddenReason {
    fn from(r: pb::HiddenReason) -> Self {
        match r {
            pb::HiddenReason::Policy => Self::Policy,
            pb::HiddenReason::System => Self::System,
            pb::HiddenReason::Unspecified => Self::None,
        }
    }
}

impl AppPolicy {
    pub fn to_u32(self) -> u32 {
        match self {
            Self::Allow => 0,
            Self::Block => 1,
            Self::HideBody => 2,
        }
    }

    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Block,
            2 => Self::HideBody,
            _ => Self::Allow,
        }
    }
}

impl From<&NotificationRecord> for pb::Notification {
    fn from(r: &NotificationRecord) -> Self {
        Self {
            nid: r.nid.clone(),
            origin: r.origin.clone(),
            app_pkg: r.app_pkg.clone(),
            app_name: r.app_name.clone(),
            icon_hash: r.icon_hash.clone(),
            title: r.title.clone(),
            body: r.body.clone(),
            actions: r
                .actions
                .iter()
                .map(|a| pb::NotificationAction {
                    id: a.id.clone(),
                    title: a.title.clone(),
                    accepts_reply: a.accepts_reply,
                })
                .collect(),
            state: pb::NotificationState::from(r.state) as i32,
            version: r.version,
            updated_at: r.updated_at,
            body_hidden: r.body_hidden,
            ongoing: r.ongoing,
            hidden_reason: pb::HiddenReason::from(r.hidden_reason) as i32,
        }
    }
}

impl From<pb::Notification> for NotificationRecord {
    fn from(n: pb::Notification) -> Self {
        let state = n.state().into();
        let hidden_reason = if n.body_hidden { n.hidden_reason().into() } else { HiddenReason::None };
        Self {
            nid: n.nid,
            origin: n.origin,
            app_pkg: n.app_pkg,
            app_name: n.app_name,
            icon_hash: n.icon_hash,
            title: n.title,
            body: n.body,
            actions: n
                .actions
                .into_iter()
                .map(|a| ActionInfo { id: a.id, title: a.title, accepts_reply: a.accepts_reply })
                .collect(),
            state,
            version: n.version,
            updated_at: n.updated_at,
            body_hidden: n.body_hidden,
            hidden_reason,
            ongoing: n.ongoing,
            pinned: false,
        }
    }
}
