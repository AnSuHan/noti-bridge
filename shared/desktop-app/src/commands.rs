//! 웹 UI 가 호출하는 Tauri 명령.

use std::sync::Arc;

use noti_bridge_core::pairing::PairingInvite;
use noti_bridge_core::{AppPolicy, ConnectionPath, CoreError, Node, NodeEvent, NotificationRecord};
use qrcode::QrCode;
use qrcode::render::svg;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::{Bridge, Settings, settings_path};

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
    app_name: String,
    icon_hash: String,
    title: String,
    body: String,
    body_hidden: bool,
    updated_at: i64,
    actions: Vec<ActionDto>,
}

impl From<NotificationRecord> for NotificationDto {
    fn from(n: NotificationRecord) -> Self {
        Self {
            nid: n.nid,
            origin: n.origin,
            app_name: n.app_name,
            icon_hash: n.icon_hash,
            title: n.title,
            body: n.body,
            body_hidden: n.body_hidden,
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
pub struct StateDto {
    device_id: String,
    platform: &'static str,
    can_capture: bool,
    dnd: bool,
    relay_url: String,
    notifications: Vec<NotificationDto>,
    peers: Vec<PeerDto>,
    rules: Vec<RuleDto>,
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
    let settings: Settings = std::fs::read(settings_path(&dir))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    Ok(StateDto {
        device_id: node.device_id(),
        platform: bridge.platform.platform(),
        can_capture: bridge.platform.can_capture(),
        dnd: node.dnd().map_err(err)?,
        relay_url: settings.relay_url.unwrap_or_default(),
        notifications: node.active_notifications().map_err(err)?.into_iter().map(Into::into).collect(),
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

#[tauri::command]
pub fn icon(bridge: BridgeState<'_>, hash: String) -> Result<Option<Vec<u8>>, String> {
    node(&bridge)?.icon(&hash).map_err(err)
}

/// 릴레이 주소를 저장하고 앱을 다시 시작해 적용한다.
#[tauri::command]
pub fn set_relay(app: AppHandle, relay_url: String) -> Result<(), String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let trimmed = relay_url.trim();
    let settings = Settings { relay_url: (!trimmed.is_empty()).then(|| trimmed.to_owned()) };
    let bytes = serde_json::to_vec(&settings).map_err(|e| e.to_string())?;
    std::fs::write(settings_path(&dir), bytes).map_err(|e| e.to_string())?;
    app.restart()
}
