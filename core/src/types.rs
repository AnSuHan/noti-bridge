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
}

/// 필터에 걸린 알림은 이 기기에 보이지 않고, 이 기기가 원본이면 다른 기기로 보내지도 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NotificationFilter {
    pub id: i64,
    pub kind: FilterKind,
    pub pattern: String,
}

impl NotificationFilter {
    pub fn matches(&self, app_pkg: &str, app_name: &str, title: &str, body: &str) -> bool {
        let pattern = self.pattern.trim().to_lowercase();
        if pattern.is_empty() {
            return false;
        }
        match self.kind {
            FilterKind::App => app_pkg.to_lowercase() == pattern || app_name.trim().to_lowercase() == pattern,
            FilterKind::Keyword => title.to_lowercase().contains(&pattern) || body.to_lowercase().contains(&pattern),
        }
    }
}

/// Firebase 계정 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CloudStatus {
    /// Firebase 프로젝트 값(API 키, 프로젝트 ID)이 있어 로그인할 수 있는지.
    pub configured: bool,
    pub signed_in: bool,
    pub email: String,
    pub project_id: String,
    /// 마지막으로 동기화에 성공한 시각(ms). 없으면 0.
    pub last_sync: i64,
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
        }
    }
}

impl From<pb::Notification> for NotificationRecord {
    fn from(n: pb::Notification) -> Self {
        let state = n.state().into();
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
        }
    }
}
