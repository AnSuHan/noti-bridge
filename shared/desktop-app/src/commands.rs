//! 웹 UI 가 호출하는 Tauri 명령.

use std::sync::Arc;

use noti_bridge_core::pairing::PairingInvite;
use noti_bridge_core::{
    AppPolicy, CloudStatus, ConnectionPath, CoreError, FilterKind, HiddenReason, Node, NodeEvent, NotificationRecord,
    ReleaseNote,
};
use qrcode::QrCode;
use qrcode::render::svg;
use serde::{Deserialize, Serialize};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, State};

use crate::{Bridge, apply_settings, load_settings, save_settings, tray};

type BridgeState<'a> = State<'a, Arc<Bridge>>;

fn node(bridge: &Bridge) -> Result<Node, String> {
    bridge.node.get().cloned().ok_or_else(|| "노드가 아직 준비되지 않았습니다".to_owned())
}

fn err(e: CoreError) -> String {
    e.to_string()
}

#[derive(Serialize)]
pub struct ActionDto {
    id: String,
    title: String,
    accepts_reply: bool,
}

#[derive(Serialize)]
pub struct NotificationDto {
    nid: String,
    origin: String,
    app_pkg: String,
    app_name: String,
    icon_hash: String,
    title: String,
    body: String,
    body_hidden: bool,
    /// "none", "policy"(앱 규칙), "system"(OS 가 가림).
    hidden_reason: &'static str,
    ongoing: bool,
    pinned: bool,
    updated_at: i64,
    actions: Vec<ActionDto>,
}

#[derive(Serialize)]
pub struct HiddenDto {
    notification: NotificationDto,
    /// 0 이면 직접 숨긴 것, 그 밖에는 감춘 필터 id.
    filter_id: i64,
}

impl From<NotificationRecord> for NotificationDto {
    fn from(n: NotificationRecord) -> Self {
        Self {
            nid: n.nid,
            origin: n.origin,
            app_pkg: n.app_pkg,
            app_name: n.app_name,
            icon_hash: n.icon_hash,
            title: n.title,
            body: n.body,
            body_hidden: n.body_hidden,
            hidden_reason: match n.hidden_reason {
                HiddenReason::None => "none",
                HiddenReason::Policy => "policy",
                HiddenReason::System => "system",
            },
            ongoing: n.ongoing,
            pinned: n.pinned,
            updated_at: n.updated_at,
            actions: n
                .actions
                .into_iter()
                .map(|a| ActionDto { id: a.id, title: a.title, accepts_reply: a.accepts_reply })
                .collect(),
        }
    }
}

fn path_name(path: ConnectionPath) -> &'static str {
    match path {
        ConnectionPath::Direct => "direct",
        ConnectionPath::Relay => "relay",
        ConnectionPath::Disconnected => "disconnected",
    }
}

#[derive(Serialize)]
pub struct PeerDto {
    device_id: String,
    name: String,
    platform: String,
    path: &'static str,
}

#[derive(Serialize)]
pub struct RuleDto {
    app_pkg: String,
    policy: &'static str,
}

fn policy_name(p: AppPolicy) -> &'static str {
    match p {
        AppPolicy::Allow => "allow",
        AppPolicy::Block => "block",
        AppPolicy::HideBody => "hide_body",
    }
}

#[derive(Serialize)]
pub struct FilterDto {
    id: i64,
    kind: &'static str,
    pattern: String,
    label: String,
}

#[derive(Serialize)]
pub struct CloudDto {
    configured: bool,
    signed_in: bool,
    email: String,
    provider: String,
    project_id: String,
    last_sync: i64,
    google_available: bool,
    github_available: bool,
}

impl From<CloudStatus> for CloudDto {
    fn from(c: CloudStatus) -> Self {
        Self {
            configured: c.configured,
            signed_in: c.signed_in,
            email: c.email,
            provider: c.provider,
            project_id: c.project_id,
            last_sync: c.last_sync,
            google_available: c.google_available,
            github_available: c.github_available,
        }
    }
}

#[derive(Serialize)]
pub struct StateDto {
    version: &'static str,
    /// 실제로 쓰는 언어("ko", "en")와 사용자가 고른 값(없으면 자동).
    language: &'static str,
    language_setting: Option<String>,
    toast_close_dismisses: bool,
    filters: Vec<FilterDto>,
    cloud: CloudDto,
    device_id: String,
    platform: &'static str,
    can_capture: bool,
    capture_state: &'static str,
    dnd: bool,
    relay_url: String,
    notifications: Vec<NotificationDto>,
    /// 화면에 보이지 않는 알림(숨긴 것, 필터에 걸린 것).
    hidden: Vec<HiddenDto>,
    peers: Vec<PeerDto>,
    rules: Vec<RuleDto>,
    /// 창이 없을 때 도착해 아직 확인하지 않은 페어링 코드.
    pairing: Option<PairingDto>,
}

#[derive(Serialize)]
pub struct PairingDto {
    device_name: String,
    code: String,
}

/// UI 는 대부분 이벤트를 받으면 상태를 다시 읽고, 페어링 진행만 내용을 쓴다.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiEvent {
    Changed,
    PairingCode { device_name: String, code: String },
    PairingFinished { device_name: String, accepted: bool },
    PairingFailed { reason: String },
}

impl From<&NodeEvent> for UiEvent {
    fn from(event: &NodeEvent) -> Self {
        match event {
            NodeEvent::PairingCode { device_name, code, .. } => {
                Self::PairingCode { device_name: device_name.clone(), code: code.clone() }
            }
            NodeEvent::PairingFinished { device_name, accepted, .. } => {
                Self::PairingFinished { device_name: device_name.clone(), accepted: *accepted }
            }
            NodeEvent::PairingFailed { reason } => Self::PairingFailed { reason: reason.clone() },
            _ => Self::Changed,
        }
    }
}

#[tauri::command]
pub fn get_state(app: AppHandle, bridge: BridgeState<'_>) -> Result<StateDto, String> {
    let node = node(&bridge)?;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let settings = load_settings(&dir);
    Ok(StateDto {
        version: noti_bridge_core::release::VERSION,
        language: noti_bridge_core::i18n::language(),
        language_setting: settings.language.clone(),
        toast_close_dismisses: settings.toast_close_dismisses,
        filters: node
            .filters()
            .map_err(err)?
            .into_iter()
            .map(|f| FilterDto { id: f.id, kind: f.kind.name(), pattern: f.pattern, label: f.label })
            .collect(),
        cloud: node.cloud_status().into(),
        device_id: node.device_id(),
        platform: bridge.platform.platform(),
        can_capture: bridge.platform.can_capture(),
        capture_state: bridge.platform.capture_state(),
        dnd: node.dnd().map_err(err)?,
        relay_url: settings.relay_url.unwrap_or_default(),
        notifications: node.active_notifications().map_err(err)?.into_iter().map(Into::into).collect(),
        hidden: node
            .hidden_notifications()
            .map_err(err)?
            .into_iter()
            .map(|h| HiddenDto { notification: h.notification.into(), filter_id: h.filter_id })
            .collect(),
        peers: node
            .peers()
            .map_err(err)?
            .into_iter()
            .map(|p| PeerDto { device_id: p.device_id, name: p.name, platform: p.platform, path: path_name(p.path) })
            .collect(),
        rules: node
            .app_rules()
            .map_err(err)?
            .into_iter()
            .map(|r| RuleDto { app_pkg: r.app_pkg, policy: policy_name(r.policy) })
            .collect(),
        pairing: bridge
            .pairing
            .lock()
            .unwrap()
            .clone()
            .map(|(device_name, code)| PairingDto { device_name, code }),
    })
}

#[derive(Serialize)]
pub struct InviteDto {
    uri: String,
    qr_svg: String,
    expires_at: i64,
}

fn invite_dto(invite: PairingInvite) -> Result<InviteDto, String> {
    let uri = invite.to_uri();
    let code = QrCode::new(uri.as_bytes()).map_err(|e| e.to_string())?;
    let qr_svg = code.render::<svg::Color<'_>>().min_dimensions(280, 280).build();
    Ok(InviteDto { uri, qr_svg, expires_at: invite.expires_at })
}

#[tauri::command]
pub async fn create_invite(bridge: BridgeState<'_>) -> Result<InviteDto, String> {
    let node = node(&bridge)?;
    invite_dto(node.create_invite().await.map_err(err)?)
}

#[tauri::command]
pub fn confirm_pairing(bridge: BridgeState<'_>, accepted: bool) -> Result<(), String> {
    node(&bridge)?.confirm_pairing(accepted);
    Ok(())
}

#[tauri::command]
pub fn dismiss(bridge: BridgeState<'_>, nid: String) -> Result<(), String> {
    node(&bridge)?.dismiss(&nid).map_err(err)
}

#[tauri::command]
pub fn invoke_action(bridge: BridgeState<'_>, nid: String, action_id: String) -> Result<(), String> {
    node(&bridge)?.invoke_action(&nid, &action_id).map_err(err)
}

#[tauri::command]
pub fn send_reply(bridge: BridgeState<'_>, nid: String, action_id: String, text: String) -> Result<(), String> {
    node(&bridge)?.send_reply(&nid, &action_id, &text).map_err(err)
}

#[tauri::command]
pub fn unpair(bridge: BridgeState<'_>, device_id: String) -> Result<(), String> {
    node(&bridge)?.unpair(&device_id).map_err(err)
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDto {
    Allow,
    Block,
    HideBody,
}

#[tauri::command]
pub fn set_app_rule(bridge: BridgeState<'_>, app_pkg: String, policy: PolicyDto) -> Result<(), String> {
    let policy = match policy {
        PolicyDto::Allow => AppPolicy::Allow,
        PolicyDto::Block => AppPolicy::Block,
        PolicyDto::HideBody => AppPolicy::HideBody,
    };
    node(&bridge)?.set_app_rule(&app_pkg, policy).map_err(err)
}

#[tauri::command]
pub fn set_dnd(bridge: BridgeState<'_>, enabled: bool) -> Result<(), String> {
    node(&bridge)?.set_dnd(enabled).map_err(err)
}

/// PNG 를 JSON 숫자 배열이 아닌 원본 바이트로 넘긴다. 없으면 빈 응답이다.
#[tauri::command]
pub fn icon(bridge: BridgeState<'_>, hash: String) -> Result<Response, String> {
    let png = node(&bridge)?.icon(&hash).map_err(err)?;
    Ok(Response::new(png.unwrap_or_default()))
}

#[tauri::command]
pub fn open_capture_settings(bridge: BridgeState<'_>) {
    bridge.platform.open_capture_settings();
}

/// 릴레이 주소를 저장하고 앱을 다시 시작해 적용한다.
#[tauri::command]
pub fn set_relay(app: AppHandle, relay_url: String) -> Result<(), String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let trimmed = relay_url.trim();
    let mut settings = load_settings(&dir);
    settings.relay_url = (!trimmed.is_empty()).then(|| trimmed.to_owned());
    save_settings(&dir, &settings)?;
    app.restart()
}

/// 화면에서 숨긴다(연결된 기기도 같이). 폰의 원래 알림은 지우지 않는다.
#[tauri::command]
pub fn hide_local(bridge: BridgeState<'_>, nid: String) -> Result<(), String> {
    node(&bridge)?.hide_local(&nid).map_err(err)
}

/// 숨긴 알림을 다시 보인다.
#[tauri::command]
pub fn unhide(bridge: BridgeState<'_>, nid: String) -> Result<(), String> {
    node(&bridge)?.set_hidden(&nid, false).map_err(err)
}

/// 보이는 알림을 모두(app_pkg 가 있으면 그 앱 것만) 지운다. 고정한 알림은 남긴다.
/// "닫으면 폰 알림도 지우기" 설정을 따른다: 꺼져 있으면 숨기기만 한다.
#[tauri::command]
pub fn clear(bridge: BridgeState<'_>, app_pkg: Option<String>) -> Result<(), String> {
    node(&bridge)?.clear(app_pkg.as_deref(), crate::toast_close_dismisses()).map_err(err)
}

#[tauri::command]
pub fn set_pinned(bridge: BridgeState<'_>, nid: String, pinned: bool) -> Result<(), String> {
    node(&bridge)?.set_pinned(&nid, pinned).map_err(err)
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterKindDto {
    App,
    Keyword,
    HiddenContent,
}

#[tauri::command]
pub fn add_filter(bridge: BridgeState<'_>, kind: FilterKindDto, pattern: String, label: Option<String>) -> Result<(), String> {
    let kind = match kind {
        FilterKindDto::App => FilterKind::App,
        FilterKindDto::Keyword => FilterKind::Keyword,
        FilterKindDto::HiddenContent => FilterKind::HiddenContent,
    };
    node(&bridge)?.add_filter(kind, &pattern, label.as_deref().unwrap_or_default()).map_err(err)
}

#[tauri::command]
pub fn remove_filter(bridge: BridgeState<'_>, id: i64) -> Result<(), String> {
    node(&bridge)?.remove_filter(id).map_err(err)
}

/// "ko", "en", 또는 빈 값(자동). 트레이 메뉴와 이후 토스트·오류 문구에 바로 반영한다.
#[tauri::command]
pub fn set_language(app: AppHandle, bridge: BridgeState<'_>, language: String) -> Result<(), String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let mut settings = load_settings(&dir);
    settings.language = match language.as_str() {
        "ko" | "en" => Some(language),
        _ => None,
    };
    save_settings(&dir, &settings)?;
    apply_settings(&settings, bridge.platform.as_ref());
    tray::relabel(&app, &node(&bridge)?);
    Ok(())
}

#[tauri::command]
pub fn set_toast_close_dismisses(app: AppHandle, bridge: BridgeState<'_>, enabled: bool) -> Result<(), String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let mut settings = load_settings(&dir);
    settings.toast_close_dismisses = enabled;
    save_settings(&dir, &settings)?;
    apply_settings(&settings, bridge.platform.as_ref());
    Ok(())
}

#[derive(Serialize)]
pub struct ReleaseNoteDto {
    version: String,
    date: String,
    items: Vec<String>,
}

impl From<ReleaseNote> for ReleaseNoteDto {
    fn from(n: ReleaseNote) -> Self {
        Self { version: n.version, date: n.date, items: n.items }
    }
}

#[tauri::command]
pub fn release_notes() -> Vec<ReleaseNoteDto> {
    noti_bridge_core::release::release_notes().into_iter().map(Into::into).collect()
}

#[tauri::command]
pub fn cloud_set_config(bridge: BridgeState<'_>, api_key: String, project_id: String) -> Result<(), String> {
    node(&bridge)?.cloud_set_config(&api_key, &project_id).map_err(err)
}

#[tauri::command]
pub async fn cloud_sign_in(bridge: BridgeState<'_>, email: String, password: String, create: bool) -> Result<(), String> {
    node(&bridge)?.cloud_sign_in(&email, &password, create).await.map_err(err)
}

#[tauri::command]
pub fn cloud_set_social(
    bridge: BridgeState<'_>,
    google_client_id: String,
    google_client_secret: String,
    github_client_id: String,
) -> Result<(), String> {
    let node = node(&bridge)?;
    // Android 용 웹 클라이언트 ID 는 데스크톱에서 쓰지 않으므로 있던 값을 유지한다.
    let web = node.cloud_google_web_client_id();
    node.cloud_set_social(&google_client_id, &google_client_secret, &web, &github_client_id).map_err(err)
}

#[tauri::command]
pub async fn cloud_password_reset(bridge: BridgeState<'_>, email: String) -> Result<(), String> {
    node(&bridge)?.cloud_send_password_reset(&email).await.map_err(err)
}

/// 브라우저에서 Google 로그인을 열고, 끝날 때까지 기다린다.
#[tauri::command]
pub async fn cloud_google_sign_in(bridge: BridgeState<'_>) -> Result<(), String> {
    let node = node(&bridge)?;
    let url = node.cloud_google_begin().await.map_err(err)?;
    bridge.platform.open_url(&url);
    node.cloud_google_finish().await.map_err(err)
}

#[derive(Serialize)]
pub struct DeviceCodeDto {
    user_code: String,
    verification_uri: String,
    expires_at: i64,
}

/// GitHub 기기 인증을 시작하고 확인 주소를 브라우저로 연다. 화면에는 코드를 보여 준다.
#[tauri::command]
pub async fn cloud_github_begin(bridge: BridgeState<'_>) -> Result<DeviceCodeDto, String> {
    let info = node(&bridge)?.cloud_github_begin().await.map_err(err)?;
    bridge.platform.open_url(&info.verification_uri);
    Ok(DeviceCodeDto { user_code: info.user_code, verification_uri: info.verification_uri, expires_at: info.expires_at })
}

/// 사용자가 GitHub 에 코드를 넣을 때까지 기다린다.
#[tauri::command]
pub async fn cloud_github_finish(bridge: BridgeState<'_>) -> Result<(), String> {
    node(&bridge)?.cloud_github_finish().await.map_err(err)
}

#[tauri::command]
pub fn open_url(bridge: BridgeState<'_>, url: String) {
    if url.starts_with("https://") {
        bridge.platform.open_url(&url);
    }
}

#[tauri::command]
pub async fn cloud_sign_out(bridge: BridgeState<'_>) -> Result<(), String> {
    node(&bridge)?.cloud_sign_out().await.map_err(err)
}

#[tauri::command]
pub async fn cloud_sync(bridge: BridgeState<'_>) -> Result<(), String> {
    node(&bridge)?.cloud_sync().await.map_err(err)
}

#[derive(Serialize)]
pub struct CloudDeviceDto {
    device_id: String,
    name: String,
    platform: String,
    version: String,
    updated_at: i64,
    is_self: bool,
    paired: bool,
}

#[tauri::command]
pub async fn cloud_devices(bridge: BridgeState<'_>) -> Result<Vec<CloudDeviceDto>, String> {
    let devices = node(&bridge)?.cloud_devices().await.map_err(err)?;
    Ok(devices
        .into_iter()
        .map(|d| CloudDeviceDto {
            device_id: d.device_id,
            name: d.name,
            platform: d.platform,
            version: d.version,
            updated_at: d.updated_at,
            is_self: d.is_self,
            paired: d.paired,
        })
        .collect())
}
