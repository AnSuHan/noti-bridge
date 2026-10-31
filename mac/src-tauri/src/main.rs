//! macOS 앱. 다른 앱 알림을 읽는 공개 API 가 없어 수신 전용으로 동작한다.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::Path;

use noti_bridge_desktop::noti_bridge_core::{NotificationRecord, Node, crypto};
use noti_bridge_desktop::{DesktopPlatform, Secrets};
use tauri_plugin_notification::NotificationExt;

struct MacPlatform;

#[cfg(target_os = "macos")]
fn load_key(name: &str, _dir: &Path) -> Result<Vec<u8>, String> {
    use security_framework::passwords::{get_generic_password, set_generic_password};
    const SERVICE: &str = "com.notibridge.mac";
    if let Ok(key) = get_generic_password(SERVICE, name) {
        return Ok(key);
    }
    let key = crypto::random_key().map_err(|e| e.to_string())?;
    set_generic_password(SERVICE, name, &key).map_err(|e| e.to_string())?;
    Ok(key)
}

/// 다른 OS 에서 이 크레이트를 검사할 때만 쓰는 대체 구현이다.
#[cfg(not(target_os = "macos"))]
fn load_key(name: &str, dir: &Path) -> Result<Vec<u8>, String> {
    let path = dir.join(name);
    if let Ok(key) = std::fs::read(&path) {
        return Ok(key);
    }
    let key = crypto::random_key().map_err(|e| e.to_string())?;
    std::fs::write(path, &key).map_err(|e| e.to_string())?;
    Ok(key)
}

impl DesktopPlatform for MacPlatform {
    fn platform(&self) -> &'static str {
        "macos"
    }

    fn load_secrets(&self, dir: &Path) -> Result<Secrets, String> {
        Ok(Secrets { secret_key: load_key("device_key", dir)?, storage_key: load_key("storage_key", dir)? })
    }

    fn show(&self, app: &tauri::AppHandle, _node: &Node, n: &NotificationRecord) {
        let body = if n.body_hidden { "내용 숨겨짐" } else { n.body.as_str() };
        let title = if n.title.is_empty() { n.app_name.as_str() } else { n.title.as_str() };
        let _ = app.notification().builder().title(title).body(body).show();
    }

    // 알림 센터에서 개별 알림을 지우는 API 가 플러그인에 없어, 앱 안 목록에서만 사라진다.
    fn hide(&self, _app: &tauri::AppHandle, _nid: &str) {}
}

fn main() {
    noti_bridge_desktop::run(MacPlatform, tauri::generate_context!(), |builder| {
        builder.plugin(tauri_plugin_notification::init())
    });
}
