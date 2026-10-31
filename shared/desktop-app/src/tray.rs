//! 트레이 아이콘. 색으로 연결 상태(직접/중계/끊김)를 보여준다.

use noti_bridge_core::{ConnectionPath, Node};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

const TRAY_ID: &str = "status";
const ICON_SIZE: u32 = 32;

fn status(node: &Node) -> (&'static str, [u8; 3]) {
    let peers = node.peers().unwrap_or_default();
    if peers.iter().any(|p| p.path == ConnectionPath::Direct) {
        ("noti-bridge: 직접 연결", [46, 160, 67])
    } else if peers.iter().any(|p| p.path == ConnectionPath::Relay) {
        ("noti-bridge: 중계 연결", [219, 154, 4])
    } else {
        ("noti-bridge: 연결 끊김", [139, 148, 158])
    }
}

fn circle(color: [u8; 3]) -> Image<'static> {
    let center = (ICON_SIZE as f32 - 1.0) / 2.0;
    let radius = ICON_SIZE as f32 / 2.0 - 1.0;
    let mut rgba = Vec::with_capacity((ICON_SIZE * ICON_SIZE * 4) as usize);
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let distance = ((x as f32 - center).powi(2) + (y as f32 - center).powi(2)).sqrt();
            let alpha = ((radius - distance + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
            rgba.extend_from_slice(&[color[0], color[1], color[2], alpha]);
        }
    }
    Image::new_owned(rgba, ICON_SIZE, ICON_SIZE)
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn create(app: &AppHandle, node: &Node) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "열기", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let (tooltip, color) = status(node);
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(circle(color))
        .tooltip(tooltip)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn refresh(app: &AppHandle, node: &Node) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let (tooltip, color) = status(node);
        let _ = tray.set_icon(Some(circle(color)));
        let _ = tray.set_tooltip(Some(tooltip));
    }
}
