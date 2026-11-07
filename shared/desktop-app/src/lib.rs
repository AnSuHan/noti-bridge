//! Windows·macOS 앱이 공유하는 Tauri 셸: 노드 수명, 트레이, UI 명령.

mod commands;
mod tray;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use noti_bridge_core::{DEFAULT_RETENTION_MS, Node, NodeConfig, NodeEvent, NodeListener, NotificationRecord, NotificationState};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, RunEvent};

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
    /// 로그인 시 자동 시작으로 켜졌는지. 그렇다면 창 없이 트레이에만 둔다.
    fn launched_at_login(&self) -> bool {
        false
    }
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
    /// 창이 없는 동안 도착한 확인 코드. 창을 다시 열면 UI 가 상태와 함께 읽어 간다.
    pub pairing: Mutex<Option<(String, String)>>,
}

impl NodeListener for Bridge {
    fn on_event(&self, event: NodeEvent) {
        let Some(node) = self.node.get() else { return };
        match &event {
            NodeEvent::PairingCode { device_name, code, .. } => {
                *self.pairing.lock().unwrap() = Some((device_name.clone(), code.clone()));
                // 창을 닫아 둔 상태여도 사용자가 코드를 확인할 수 있게 띄운다.
                tray::show_window(&self.app);
            }
            NodeEvent::PairingFinished { .. } | NodeEvent::PairingFailed { .. } => {
                *self.pairing.lock().unwrap() = None;
                tray::refresh(&self.app, node);
            }
            NodeEvent::NotificationChanged { notification }
                if notification.origin != node.device_id() && notification.state == NotificationState::Active =>
            {
                self.platform.show(&self.app, node, notification);
            }
            NodeEvent::NotificationRemoved { nid, .. } => self.platform.hide(&self.app, nid),
            NodeEvent::CancelLocal { key } => self.platform.cancel_local(key),
            NodeEvent::ConnectionChanged { .. } | NodeEvent::PeerUnpaired { .. } => {
                tray::refresh(&self.app, node);
            }
            _ => {}
        }
        let _ = self.app.emit("node-changed", commands::UiEvent::from(&event));
    }
}

pub fn run(platform: impl DesktopPlatform, context: tauri::Context, plugins: impl FnOnce(tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry>) {
    // Tauri 기본 런타임은 CPU 코어 수만큼 작업 스레드를 띄운다. 이 앱의 일은 가벼운 네트워크·DB 처리라 둘이면 충분하다.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("noti-bridge")
        .enable_all()
        .build()
        .expect("tokio 런타임 생성 실패");
    tauri::async_runtime::set(runtime.handle().clone());

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
                pairing: Mutex::new(None),
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
            // 창은 설정에서 자동 생성을 꺼 두고 여기서 만든다. 자동 시작이면 WebView 를 띄우지 않는다.
            if !platform.launched_at_login() {
                tray::show_window(app.handle());
            }
            platform.started(app.handle(), node);
            Ok(())
        });
    builder
        .build(context)
        .expect("Tauri 앱 초기화 실패")
        .run(move |app, event| match event {
            // 창을 닫으면 WebView 프로세스까지 내려 메모리를 돌려준다. 앱은 트레이에 남아 알림을 계속 받고,
            // 트레이에서 열면 창을 새로 만든다. 트레이의 종료(exit 코드 있음)만 실제로 끝낸다.
            RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
            RunEvent::Exit => {
                if let Some(bridge) = app.try_state::<Arc<Bridge>>() {
                    if let Some(node) = bridge.node.get() {
                        runtime.block_on(node.stop());
                    }
                }
            }
            _ => {}
        });
}

fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "PC".to_owned())
}
