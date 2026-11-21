//! 트레이 아이콘. 색으로 연결 상태(직접/중계/끊김)를 보여준다.

use noti_bridge_core::i18n::tr;
use noti_bridge_core::{ConnectionPath, Node};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewWindowBuilder};

const TRAY_ID: &str = "status";
const ICON_SIZE: u32 = 32;

fn status(node: &Node) -> (String, [u8; 3]) {
    let peers = node.peers().unwrap_or_default();
    let (label, color) = if peers.iter().any(|p| p.path == ConnectionPath::Direct) {
        (tr("직접 연결", "Direct connection"), [46, 160, 67])
    } else if peers.iter().any(|p| p.path == ConnectionPath::Relay) {
        (tr("중계 연결", "Relay connection"), [219, 154, 4])
    } else {
        (tr("연결 끊김", "Disconnected"), [139, 148, 158])
    };
    (format!("noti-bridge: {label}"), color)
}

fn menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let open = MenuItem::with_id(app, "open", tr("열기", "Open"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", tr("종료", "Quit"), true, None::<&str>)?;
    Menu::with_items(app, &[&open, &quit])
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

const MAIN_WINDOW: &str = "main";

/// 창은 닫힐 때 WebView 째로 없어지므로, 없으면 설정 파일의 정의대로 새로 만든다.
pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let Some(config) = app.config().app.windows.iter().find(|w| w.label == MAIN_WINDOW) else { return };
    match WebviewWindowBuilder::from_config(app, config).and_then(|b| b.build()) {
        Ok(window) => {
            let _ = window.set_focus();
        }
        Err(e) => eprintln!("창을 만들지 못했습니다: {e}"),
    }
}

pub fn create(app: &AppHandle, node: &Node) -> tauri::Result<()> {
    let menu = menu(app)?;
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

/// 언어를 바꾼 뒤 메뉴 문구를 다시 만든다.
pub fn relabel(app: &AppHandle, node: &Node) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
    refresh(app, node);
}
