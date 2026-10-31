//! 미러링한 알림을 Windows 토스트로 표시하고, 닫기·액션·답장을 코어로 되돌린다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use noti_bridge_desktop::noti_bridge_core::crypto::sha256_hex;
use noti_bridge_desktop::noti_bridge_core::{Node, NotificationRecord};
use noti_bridge_desktop::{DesktopPlatform, Secrets};
use tauri::Manager;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::{IReference, TypedEventHandler};
use windows::UI::Notifications::{
    ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs, ToastNotification,
    ToastNotificationManager, ToastNotifier,
};
use windows::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegSetValueExW,
};
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows::core::{HSTRING, IInspectable, Interface, w};

use crate::{secrets, user_listener};

const AUMID: &str = "com.notibridge.desktop";
const GROUP: &str = "mirror";
const REPLY_INPUT: &str = "reply";
const MAX_ACTIONS: usize = 5;

pub fn is_packaged() -> bool {
    let mut len = 0u32;
    let status = unsafe { GetCurrentPackageFullName(&mut len, None) };
    status != APPMODEL_ERROR_NO_PACKAGE
}

/// MSIX 없이 실행할 때도 토스트가 앱 이름으로 표시되도록 AUMID 를 사용자 레지스트리에 등록한다.
fn register_unpackaged_aumid() -> windows::core::Result<()> {
    let subkey = HSTRING::from(format!("Software\\Classes\\AppUserModelId\\{AUMID}"));
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &subkey,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
        .ok()?;
        let name: Vec<u8> = "noti-bridge\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let result = RegSetValueExW(key, w!("DisplayName"), None, REG_SZ, Some(&name)).ok();
        let _ = RegCloseKey(key);
        result?;
        SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID))
    }
}

/// 토스트 태그는 64자 제한이 있어 nid 해시를 쓴다.
fn tag_of(nid: &str) -> HSTRING {
    HSTRING::from(&sha256_hex(nid.as_bytes())[..32])
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn toast_xml(n: &NotificationRecord, icon: Option<&Path>) -> String {
    let title = if n.title.is_empty() { &n.app_name } else { &n.title };
    let body = if n.body_hidden { "내용 숨겨짐" } else { n.body.as_str() };
    let logo = icon
        .map(|p| format!(r#"<image placement="appLogoOverride" hint-crop="circle" src="{}"/>"#, escape(&p.to_string_lossy())))
        .unwrap_or_default();
    let mut actions = String::new();
    for action in n.actions.iter().take(MAX_ACTIONS) {
        let content = escape(&action.title);
        let id = escape(&action.id);
        if action.accepts_reply {
            actions.push_str(&format!(r#"<input id="{REPLY_INPUT}" type="text" placeHolderContent="답장 입력"/>"#));
            actions.push_str(&format!(
                r#"<action content="{content}" arguments="reply:{id}" hint-inputId="{REPLY_INPUT}" activationType="foreground"/>"#
            ));
        } else {
            actions.push_str(&format!(
                r#"<action content="{content}" arguments="action:{id}" activationType="foreground"/>"#
            ));
        }
    }
    format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text><text placement="attribution">{}</text>{logo}</binding></visual><actions>{actions}</actions></toast>"#,
        escape(title),
        escape(body),
        escape(&n.app_name),
    )
}

pub struct WindowsPlatform {
    packaged: bool,
    notifier: Mutex<Option<ToastNotifier>>,
    live: Mutex<HashMap<String, ToastNotification>>,
    listener: user_listener::CaptureHandle,
}

// ToastNotifier 와 ToastNotification 은 agile WinRT 객체라 스레드 간 이동이 안전하다.
unsafe impl Send for WindowsPlatform {}
unsafe impl Sync for WindowsPlatform {}

impl WindowsPlatform {
    pub fn new() -> Self {
        let packaged = is_packaged();
        if !packaged {
            let _ = register_unpackaged_aumid();
        }
        let notifier = if packaged {
            ToastNotificationManager::CreateToastNotifier()
        } else {
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
        };
        Self {
            packaged,
            notifier: Mutex::new(notifier.ok()),
            live: Mutex::new(HashMap::new()),
            listener: user_listener::CaptureHandle::default(),
        }
    }

    fn write_icon(app: &tauri::AppHandle, node: &Node, hash: &str) -> Option<PathBuf> {
        if hash.is_empty() {
            return None;
        }
        let dir = app.path().app_cache_dir().ok()?.join("icons");
        let path = dir.join(format!("{hash}.png"));
        if !path.exists() {
            let png = node.icon(hash).ok()??;
            std::fs::create_dir_all(&dir).ok()?;
            std::fs::write(&path, png).ok()?;
        }
        Some(path)
    }

    fn build(&self, n: &NotificationRecord, icon: Option<&Path>, node: &Node) -> windows::core::Result<ToastNotification> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(toast_xml(n, icon)))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        toast.SetTag(&tag_of(&n.nid))?;
        toast.SetGroup(&HSTRING::from(GROUP))?;

        let dismiss_node = node.clone();
        let nid = n.nid.clone();
        toast.Dismissed(&TypedEventHandler::new(
            move |_: windows::core::Ref<ToastNotification>, args: windows::core::Ref<ToastDismissedEventArgs>| {
                // 시간 초과는 알림 센터로 옮겨간 것뿐이라 사용자가 닫은 경우만 삭제한다.
                if let Some(args) = args.as_ref() {
                    if args.Reason()? == ToastDismissalReason::UserCanceled {
                        let _ = dismiss_node.dismiss(&nid);
                    }
                }
                Ok(())
            },
        ))?;

        let activate_node = node.clone();
        let nid = n.nid.clone();
        toast.Activated(&TypedEventHandler::new(
            move |_: windows::core::Ref<ToastNotification>, args: windows::core::Ref<IInspectable>| {
                let Some(args) = args.as_ref() else { return Ok(()) };
                let args: ToastActivatedEventArgs = args.cast()?;
                let arguments = args.Arguments()?.to_string();
                if let Some(id) = arguments.strip_prefix("action:") {
                    let _ = activate_node.invoke_action(&nid, id);
                } else if let Some(id) = arguments.strip_prefix("reply:") {
                    let input = args.UserInput()?;
                    let value = input.Lookup(&HSTRING::from(REPLY_INPUT))?;
                    let text = value.cast::<IReference<HSTRING>>()?.Value()?;
                    let _ = activate_node.send_reply(&nid, id, &text.to_string());
                }
                Ok(())
            },
        ))?;
        Ok(toast)
    }
}

impl DesktopPlatform for WindowsPlatform {
    fn platform(&self) -> &'static str {
        "windows"
    }

    fn can_capture(&self) -> bool {
        self.packaged
    }

    fn load_secrets(&self, dir: &Path) -> Result<Secrets, String> {
        secrets::load(dir)
    }

    fn started(&self, _app: &tauri::AppHandle, node: Node) {
        if self.packaged {
            self.listener.start(node);
        }
    }

    fn show(&self, app: &tauri::AppHandle, node: &Node, n: &NotificationRecord) {
        let icon = Self::write_icon(app, node, &n.icon_hash);
        let Ok(toast) = self.build(n, icon.as_deref(), node) else { return };
        let mut live = self.live.lock().unwrap();
        // 같은 알림의 갱신은 팝업 없이 내용만 바꾼다.
        if live.contains_key(&n.nid) {
            let _ = toast.SetSuppressPopup(true);
        }
        if let Some(notifier) = self.notifier.lock().unwrap().as_ref() {
            if notifier.Show(&toast).is_ok() {
                live.insert(n.nid.clone(), toast);
            }
        }
    }

    fn hide(&self, _app: &tauri::AppHandle, nid: &str) {
        if self.live.lock().unwrap().remove(nid).is_none() {
            return;
        }
        if let Ok(history) = ToastNotificationManager::History() {
            let group = HSTRING::from(GROUP);
            let _ = if self.packaged {
                history.RemoveGroupedTag(&tag_of(nid), &group)
            } else {
                history.RemoveGroupedTagWithId(&tag_of(nid), &group, &HSTRING::from(AUMID))
            };
        }
    }

    fn cancel_local(&self, key: &str) {
        self.listener.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use noti_bridge_desktop::noti_bridge_core::{ActionInfo, NotificationState};

    fn sample(title: &str, body: &str, actions: Vec<ActionInfo>) -> NotificationRecord {
        NotificationRecord {
            nid: "origin:key".into(),
            origin: "origin".into(),
            app_pkg: "com.example".into(),
            app_name: "A&B <앱>".into(),
            icon_hash: String::new(),
            title: title.into(),
            body: body.into(),
            actions,
            state: NotificationState::Active,
            version: 1,
            updated_at: 0,
            body_hidden: false,
        }
    }

    #[test]
    fn xml_with_special_characters_and_actions_is_accepted_by_winrt() {
        let actions = vec![
            ActionInfo { id: "0".into(), title: "읽음 \"표시\"".into(), accepts_reply: false },
            ActionInfo { id: "1".into(), title: "답장".into(), accepts_reply: true },
        ];
        let n = sample("<제목> & 'x'", "본문\n둘째 줄 ]]> &amp;", actions);
        let xml = toast_xml(&n, Some(Path::new(r"C:\tmp\a&b.png")));
        let doc = XmlDocument::new().unwrap();
        doc.LoadXml(&HSTRING::from(xml)).unwrap();
        ToastNotification::CreateToastNotification(&doc).unwrap();
    }

    #[test]
    fn tag_fits_toast_limit_and_is_stable() {
        let long = "x".repeat(500);
        assert_eq!(tag_of(&long), tag_of(&long));
        assert!(tag_of(&long).len() <= 64);
    }
}
