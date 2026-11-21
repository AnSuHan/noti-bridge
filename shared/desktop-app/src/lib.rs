//! Windows·macOS 앱이 공유하는 Tauri 셸: 노드 수명, 트레이, UI 명령.

mod commands;
mod tray;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

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
    /// PC 알림 읽기 상태: "unsupported", "pending", "allowed", "denied".
    fn capture_state(&self) -> &'static str {
        "unsupported"
    }
    /// 알림 접근을 허용하는 OS 설정 화면을 연다.
    fn open_capture_settings(&self) {}
    fn load_secrets(&self, dir: &Path) -> Result<Secrets, String>;
    /// 로그인 시 자동 시작으로 켜졌는지. 그렇다면 창 없이 트레이에만 둔다.
    fn launched_at_login(&self) -> bool {
        false
    }
    /// OS 표시 언어(BCP 47, 예: "ko-KR"). 사용자가 언어를 고르지 않았을 때 쓴다.
    fn system_language(&self) -> String {
        std::env::var("LANG").unwrap_or_else(|_| "en".to_owned())
    }
    /// 노드가 뜬 뒤 한 번 호출된다.
    fn started(&self, _app: &tauri::AppHandle, _node: Node) {}
    /// alert 가 false 면 소리·팝업 없이 알림 센터에만 넣는다.
    fn show(&self, app: &tauri::AppHandle, node: &Node, notification: &NotificationRecord, alert: bool);
    fn hide(&self, app: &tauri::AppHandle, node: &Node, nid: &str);
    /// 이 기기가 원본인 알림을 OS 에서 지운다.
    fn cancel_local(&self, _key: &str) {}
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub(crate) struct Settings {
    pub relay_url: Option<String>,
    /// "ko", "en". 없으면 OS 언어를 따른다.
    #[serde(default)]
    pub language: Option<String>,
    /// 토스트를 닫으면 원본 기기의 알림도 지울지. 기본은 이 PC 에서만 내린다.
    #[serde(default)]
    pub toast_close_dismisses: bool,
}

static TOAST_CLOSE_DISMISSES: AtomicBool = AtomicBool::new(false);

/// 토스트를 사용자가 닫았을 때 원본 기기의 알림까지 지울지(설정값).
pub fn toast_close_dismisses() -> bool {
    TOAST_CLOSE_DISMISSES.load(Ordering::Relaxed)
}

pub(crate) fn apply_settings(settings: &Settings, platform: &dyn DesktopPlatform) {
    let language = settings.language.clone().unwrap_or_else(|| platform.system_language());
    noti_bridge_core::i18n::set_language(&language);
    TOAST_CLOSE_DISMISSES.store(settings.toast_close_dismisses, Ordering::Relaxed);
}

pub(crate) fn save_settings(dir: &Path, settings: &Settings) -> Result<(), String> {
    let bytes = serde_json::to_vec(settings).map_err(|e| e.to_string())?;
    std::fs::write(settings_path(dir), bytes).map_err(|e| e.to_string())
}

/// 짧은 시간에 팝업이 몰리면 나머지는 조용히 넣는다. 재연결 직후처럼 한꺼번에 오는 경우를 막는다.
const BURST_WINDOW: Duration = Duration::from_secs(3);
const BURST_MAX: usize = 3;

#[derive(Default)]
struct BurstLimiter {
    recent: VecDeque<Instant>,
}

impl BurstLimiter {
    fn allow(&mut self, now: Instant) -> bool {
        while self.recent.front().is_some_and(|t| now.duration_since(*t) > BURST_WINDOW) {
            self.recent.pop_front();
        }
        if self.recent.len() >= BURST_MAX {
            return false;
        }
        self.recent.push_back(now);
        true
    }
}

pub(crate) fn settings_path(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

pub(crate) fn load_settings(dir: &Path) -> Settings {
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
    burst: Mutex<BurstLimiter>,
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
            NodeEvent::NotificationChanged { notification, alert }
                if notification.origin != node.device_id() && notification.state == NotificationState::Active =>
            {
                let alert = *alert && self.burst.lock().unwrap().allow(Instant::now());
                self.platform.show(&self.app, node, notification, alert);
            }
            NodeEvent::NotificationRemoved { nid, .. } => self.platform.hide(&self.app, node, nid),
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
            commands::open_capture_settings,
            commands::hide_local,
            commands::add_filter,
            commands::remove_filter,
            commands::set_language,
            commands::set_toast_close_dismisses,
            commands::release_notes,
            commands::cloud_set_config,
            commands::cloud_sign_in,
            commands::cloud_sign_out,
            commands::cloud_sync,
            commands::cloud_devices,
        ])
        .setup(move |app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let secrets = platform.load_secrets(&dir).map_err(std::io::Error::other)?;
            let settings = load_settings(&dir);
            apply_settings(&settings, platform.as_ref());
            let bridge = Arc::new(Bridge {
                app: app.handle().clone(),
                platform: platform.clone(),
                node: OnceLock::new(),
                pairing: Mutex::new(None),
                burst: Mutex::new(BurstLimiter::default()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_limiter_allows_three_then_waits() {
        let mut limiter = BurstLimiter::default();
        let t = Instant::now();
        assert!((0..3).all(|_| limiter.allow(t)));
        assert!(!limiter.allow(t + Duration::from_millis(500)));
        assert!(limiter.allow(t + Duration::from_secs(4)));
    }
}
