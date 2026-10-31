//! Windows 앱. 토스트 표시, DPAPI 키 보관, PC 알림 캡처를 담당한다.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod secrets;
#[cfg(windows)]
mod toast;
#[cfg(windows)]
mod user_listener;

#[cfg(windows)]
fn main() {
    noti_bridge_desktop::run(toast::WindowsPlatform::new(), tauri::generate_context!(), |builder| builder);
}

#[cfg(not(windows))]
fn main() {
    panic!("이 크레이트는 Windows 전용입니다");
}
