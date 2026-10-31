//! Windows·macOS 앱이 공유하는 Tauri 셸: 노드 수명, 트레이, UI 명령.

mod commands;
mod tray;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use noti_bridge_core::{DEFAULT_RETENTION_MS, Node, NodeConfig, NodeEvent, NodeListener, NotificationRecord, NotificationState};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, RunEvent, WindowEvent};

pub use noti_bridge_core;

pub struct Secrets {
    pub secret_key: Vec<u8>,
    pub storage_key: Vec<u8>,
}

/// OS 마다 다른 부분. 나머지 동작은 이 크레이트가 공통으로 처리한다.
pub trait DesktopPlatform: Send + Sync + 'static {
    fn platform(&self) -> &'static str;
    /// 이 OS 에서 다른 앱의 알림을 읽을 수 있는지.
    fn can_capture(&self) -> bool {
        false
    }
    fn load_secrets(&self, dir: &Path) -> Result<Secrets, String>;
    /// 노드가 뜬 뒤 한 번 호출된다.
    fn started(&self, _app: &tauri::AppHandle, _node: Node) {}
    fn show(&self, app: &tauri::AppHandle, node: &Node, notification: &NotificationRecord);
    fn hide(&self, app: &tauri::AppHandle, nid: &str);
    /// 이 기기가 원본인 알림을 OS 에서 지운다.
    fn cancel_local(&self, _key: &str) {}
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Settings {
    pub relay_url: Option<String>,
}

pub(crate) fn settings_path(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

fn load_settings(dir: &Path) -> Settings {
    std::fs::read(settings_path(dir))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub(crate) struct Bridge {
    pub app: tauri::AppHandle,
    pub platform: Arc<dyn DesktopPlatform>,
    pub node: OnceLock<Node>,
}

impl NodeListener for Bridge {
    fn on_event(&self, event: NodeEvent) {
        let Some(node) = self.node.get() else { return };
        match &event {
            NodeEvent::NotificationChanged { notification }
                if notification.origin != node.device_id() && notification.state == NotificationState::Active =>
            {
                self.platform.show(&self.app, node, notification);
            }
            NodeEvent::NotificationRemoved { nid, .. } => self.platform.hide(&self.app, nid),
            NodeEvent::CancelLocal { key } => self.platform.cancel_local(key),
            NodeEvent::ConnectionChanged { .. } | NodeEvent::PeerUnpaired { .. } | NodeEvent::PairingFinished { .. } => {
                tray::refresh(&self.app, node);
            }
            _ => {}
        }
        let _ = self.app.emit("node-changed", commands::UiEvent::from(&event));
    }
}

pub fn run(platform: impl DesktopPlatform, context: tauri::Context, plugins: impl FnOnce(tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry>) {
    let platform: Arc<dyn DesktopPlatform> = Arc::new(platform);
    let builder = plugins(tauri::Builder::default())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| tray::show_window(app)))
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::create_invite,
            commands::confirm_pairing,
            commands::dismiss,
            commands::invoke_action,
            commands::send_reply,
            commands::unpair,
            commands::set_app_rule,
            commands::set_relay,
            commands::set_dnd,
            commands::icon,
        ])
        .setup(move |app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let secrets = platform.load_secrets(&dir).map_err(std::io::Error::other)?;
            let settings = load_settings(&dir);
            let bridge = Arc::new(Bridge {
                app: app.handle().clone(),
                platform: platform.clone(),
                node: OnceLock::new(),
            });
            let config = NodeConfig {
                data_dir: dir.join("core").to_string_lossy().into_owned(),
                secret_key: secrets.secret_key,
                storage_key: secrets.storage_key,
                device_name: device_name(),
                platform: platform.platform().to_owned(),
                relay_url: settings.relay_url,
                relay_disabled: false,
                mdns_enabled: true,
                retention_ms: DEFAULT_RETENTION_MS,
            };
            let node = tauri::async_runtime::block_on(Node::start(config, bridge.clone()))?;
            let _ = bridge.node.set(node.clone());
            app.manage(bridge);
            tray::create(app.handle(), &node)?;
            platform.started(app.handle(), node);
            Ok(())
        })
        .on_window_event(|window, event| {
            // 창을 닫아도 트레이에 남아 알림을 계속 받는다.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        });
    builder
        .build(context)
        .expect("Tauri 앱 초기화 실패")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                if let Some(bridge) = app.try_state::<Arc<Bridge>>() {
                    if let Some(node) = bridge.node.get() {
                        tauri::async_runtime::block_on(node.stop());
                    }
                }
            }
        });
}

fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "PC".to_owned())
}
