//! 미러링한 알림을 Windows 토스트로 표시하고, 닫기·액션·답장을 코어로 되돌린다.
//!
//! 패키지 없이 실행할 때는 원래 앱(카카오톡, Slack 등)마다 AUMID 를 따로 등록해, 토스트 머리글에
//! noti-bridge 대신 그 앱 이름과 아이콘이 보이게 한다. 패키지(MSIX) 실행은 자기 AUMID 만 쓸 수 있어
//! 토스트 안 머리글(header)로 앱 이름을 묶어 보여준다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use noti_bridge_desktop::noti_bridge_core::crypto::sha256_hex;
use noti_bridge_desktop::noti_bridge_core::i18n::tr;
use noti_bridge_desktop::noti_bridge_core::{Node, NotificationRecord};
use noti_bridge_desktop::{DesktopPlatform, Secrets};
use tauri::Manager;
use windows::ApplicationModel::Activation::ActivationKind;
use windows::ApplicationModel::AppInstance;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::{IReference, TypedEventHandler};
use windows::UI::Notifications::{
    ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs, ToastNotification,
    ToastNotificationManager, ToastNotifier,
};
use windows::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegSetValueExW,
};
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows::core::{HSTRING, IInspectable, Interface, w};

use crate::{secrets, user_listener};

pub const AUMID: &str = "com.notibridge.desktop";
/// 원래 앱별로 등록하는 AUMID 의 접두사. PC 알림 읽기에서 우리 토스트를 다시 읽지 않도록 거를 때도 쓴다.
pub const AUMID_PREFIX: &str = "com.notibridge.";
const GROUP: &str = "mirror";
const REPLY_INPUT: &str = "reply";
const MAX_ACTIONS: usize = 5;

pub fn is_packaged() -> bool {
    let mut len = 0u32;
    let status = unsafe { GetCurrentPackageFullName(&mut len, None) };
    status != APPMODEL_ERROR_NO_PACKAGE
}

fn reg_sz(text: &str) -> Vec<u8> {
    text.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect()
}

/// 토스트 머리글에 보일 이름과 아이콘으로 AUMID 를 사용자 레지스트리에 등록한다.
fn register_aumid(aumid: &str, display_name: &str, icon: Option<&Path>) -> windows::core::Result<()> {
    let subkey = HSTRING::from(format!("Software\\Classes\\AppUserModelId\\{aumid}"));
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
        let mut result = RegSetValueExW(key, w!("DisplayName"), None, REG_SZ, Some(&reg_sz(display_name))).ok();
        if let (Ok(()), Some(icon)) = (&result, icon) {
            result = RegSetValueExW(key, w!("IconUri"), None, REG_SZ, Some(&reg_sz(&icon.to_string_lossy()))).ok();
        }
        let _ = RegCloseKey(key);
        result
    }
}

/// MSIX 없이 실행할 때도 토스트가 앱 이름으로 표시되도록 AUMID 를 사용자 레지스트리에 등록한다.
fn register_unpackaged_aumid() -> windows::core::Result<()> {
    register_aumid(AUMID, "noti-bridge", None)?;
    unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID)) }
}

/// 원래 앱별 AUMID. 같은 앱이면 어느 기기에서 왔든 같은 값이다.
fn app_aumid(app_pkg: &str) -> String {
    format!("{AUMID_PREFIX}mirror.{}", &sha256_hex(app_pkg.as_bytes())[..16])
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

/// header 가 true 면(패키지 실행) 토스트 안 머리글로 원래 앱 이름을 보여준다.
fn toast_xml(n: &NotificationRecord, icon: Option<&Path>, header: bool) -> String {
    let title = if n.title.is_empty() { &n.app_name } else { &n.title };
    let hidden = tr("내용 숨겨짐", "Content hidden");
    let body = if n.body_hidden { hidden.as_str() } else { n.body.as_str() };
    let placeholder = escape(&tr("답장 입력", "Type a reply"));
    let logo = icon
        .map(|p| format!(r#"<image placement="appLogoOverride" hint-crop="circle" src="{}"/>"#, escape(&p.to_string_lossy())))
        .unwrap_or_default();
    let mut actions = String::new();
    for action in n.actions.iter().take(MAX_ACTIONS) {
        let content = escape(&action.title);
        let id = escape(&action.id);
        if action.accepts_reply {
            actions.push_str(&format!(r#"<input id="{REPLY_INPUT}" type="text" placeHolderContent="{placeholder}"/>"#));
            actions.push_str(&format!(
                r#"<action content="{content}" arguments="reply:{id}" hint-inputId="{REPLY_INPUT}" activationType="foreground"/>"#
            ));
        } else {
            actions.push_str(&format!(
                r#"<action content="{content}" arguments="action:{id}" activationType="foreground"/>"#
            ));
        }
    }
    let header = if header && !n.app_name.is_empty() {
        let id = escape(&n.app_pkg);
        format!(r#"<header id="{id}" title="{}" arguments="app:{id}"/>"#, escape(&n.app_name))
    } else {
        String::new()
    };
    format!(
        r#"<toast>{header}<visual><binding template="ToastGeneric"><text>{}</text><text>{}</text><text placement="attribution">{}</text>{logo}</binding></visual><actions>{actions}</actions></toast>"#,
        escape(title),
        escape(body),
        escape(&n.app_name),
    )
}

struct LiveToast {
    /// 닫기·누르기 이벤트 처리기가 살아 있도록 토스트 객체를 붙잡아 둔다.
    _toast: ToastNotification,
    aumid: String,
    /// 마지막으로 띄운 제목·본문. 같으면 갱신만 하고 팝업하지 않는다.
    content: (String, String),
}

pub struct WindowsPlatform {
    packaged: bool,
    notifier: Mutex<Option<ToastNotifier>>,
    /// 원래 앱별 AUMID 의 알림기와 등록한 이름·아이콘.
    app_notifiers: Mutex<HashMap<String, (ToastNotifier, String, Option<PathBuf>)>>,
    live: Mutex<HashMap<String, LiveToast>>,
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
            app_notifiers: Mutex::new(HashMap::new()),
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
        doc.LoadXml(&HSTRING::from(toast_xml(n, icon, self.packaged)))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        toast.SetTag(&tag_of(&n.nid))?;
        toast.SetGroup(&HSTRING::from(GROUP))?;

        let dismiss_node = node.clone();
        let nid = n.nid.clone();
        toast.Dismissed(&TypedEventHandler::new(
            move |_: windows::core::Ref<ToastNotification>, args: windows::core::Ref<ToastDismissedEventArgs>| {
                // 시간 초과는 알림 센터로 옮겨간 것뿐이라 사용자가 닫은 경우만 처리한다.
                // 기본은 이 PC 에서만 내린다. 폰 알림을 지우면 음악 재생 같은 앱이 멈출 수 있다.
                if let Some(args) = args.as_ref() {
                    if args.Reason()? == ToastDismissalReason::UserCanceled {
                        let _ = if noti_bridge_desktop::toast_close_dismisses() {
                            dismiss_node.dismiss(&nid)
                        } else {
                            dismiss_node.hide_local(&nid)
                        };
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

    /// 원래 앱 이름·아이콘으로 등록한 AUMID 와 그 알림기. 실패하면 noti-bridge 자신의 것을 쓴다.
    fn notifier_for(&self, n: &NotificationRecord, icon: Option<&Path>) -> Option<(ToastNotifier, String)> {
        if !self.packaged && !n.app_name.is_empty() {
            let aumid = app_aumid(&n.app_pkg);
            let mut notifiers = self.app_notifiers.lock().unwrap();
            let stale = match notifiers.get(&aumid) {
                Some((_, name, path)) => name != &n.app_name || (icon.is_some() && path.as_deref() != icon),
                None => true,
            };
            if stale {
                let registered = register_aumid(&aumid, &n.app_name, icon)
                    .and_then(|_| ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(&aumid)));
                if let Ok(notifier) = registered {
                    notifiers.insert(aumid.clone(), (notifier, n.app_name.clone(), icon.map(Path::to_path_buf)));
                }
            }
            if let Some((notifier, _, _)) = notifiers.get(&aumid) {
                return Some((notifier.clone(), aumid));
            }
        }
        self.notifier.lock().unwrap().clone().map(|n| (n, AUMID.to_owned()))
    }

    fn remove_from_history(&self, nid: &str, aumid: &str) {
        if let Ok(history) = ToastNotificationManager::History() {
            let group = HSTRING::from(GROUP);
            let _ = if self.packaged {
                history.RemoveGroupedTag(&tag_of(nid), &group)
            } else {
                history.RemoveGroupedTagWithId(&tag_of(nid), &group, &HSTRING::from(aumid))
            };
        }
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

    /// 매니페스트의 StartupTask 로 켜졌으면 활성화 종류가 StartupTask 다. 패키지 없이 실행하면 항상 false.
    fn capture_state(&self) -> &'static str {
        if self.packaged { self.listener.state() } else { "unsupported" }
    }

    fn open_capture_settings(&self) {
        // 설정 > 개인정보 및 보안 > 알림. 사용자가 여기서 noti-bridge 의 알림 접근을 켠다.
        let _ = std::process::Command::new("explorer.exe").arg("ms-settings:privacy-notifications").spawn();
    }

    fn launched_at_login(&self) -> bool {
        self.packaged
            && AppInstance::GetActivatedEventArgs()
                .and_then(|args| args.Kind())
                .is_ok_and(|kind| kind == ActivationKind::StartupTask)
    }

    fn started(&self, _app: &tauri::AppHandle, node: Node) {
        if self.packaged {
            self.listener.start(node);
        }
    }

    fn system_language(&self) -> String {
        let mut buf = [0u16; 85];
        let len = unsafe { GetUserDefaultLocaleName(&mut buf) };
        if len > 1 { String::from_utf16_lossy(&buf[..len as usize - 1]) } else { "en".to_owned() }
    }

    fn show(&self, app: &tauri::AppHandle, node: &Node, n: &NotificationRecord, alert: bool) {
        let icon = Self::write_icon(app, node, &n.icon_hash);
        let Ok(toast) = self.build(n, icon.as_deref(), node) else { return };
        let content = (n.title.clone(), n.body.clone());
        let mut live = self.live.lock().unwrap();
        // 카카오톡·Slack 은 대화방마다 알림 하나를 갱신하므로, 내용이 바뀐 갱신은 새 메시지로 보고 다시 띄운다.
        // 내용이 같은 갱신, 쌓여 있던 알림(alert 아님)은 알림 센터에만 조용히 넣는다.
        let unchanged = live.get(&n.nid).is_some_and(|l| l.content == content);
        if !alert || unchanged {
            let _ = toast.SetSuppressPopup(true);
        }
        let Some((notifier, aumid)) = self.notifier_for(n, icon.as_deref()) else { return };
        // 앱 이름이 바뀌어 AUMID 가 달라졌으면 이전 토스트를 지워 둘이 남지 않게 한다.
        if let Some(prev) = live.get(&n.nid).filter(|l| l.aumid != aumid) {
            self.remove_from_history(&n.nid, &prev.aumid);
        }
        if notifier.Show(&toast).is_ok() {
            live.insert(n.nid.clone(), LiveToast { _toast: toast, aumid, content });
        }
    }

    fn hide(&self, _app: &tauri::AppHandle, node: &Node, nid: &str) {
        // 앱을 다시 시작하면 live 는 비어 있어도 이전 토스트는 알림 센터에 남아 있어, 기록에서 AUMID 를 다시 구한다.
        let aumid = match self.live.lock().unwrap().remove(nid) {
            Some(l) => l.aumid,
            None => match node.notification(nid) {
                Ok(Some(n)) if !self.packaged && !n.app_name.is_empty() => app_aumid(&n.app_pkg),
                _ => AUMID.to_owned(),
            },
        };
        self.remove_from_history(nid, &aumid);
        if aumid != AUMID {
            self.remove_from_history(nid, AUMID);
        }
    }

    fn cancel_local(&self, key: &str) {
        self.listener.remove(key);
    }

    fn app_rule_changed(&self, _app_pkg: &str) {
        self.listener.rescan();
    }

    fn open_url(&self, url: &str) {
        // cmd 의 start 는 & 를 명령 구분자로 읽어 주소가 잘린다. url.dll 은 주소를 그대로 넘긴다.
        let _ = std::process::Command::new("rundll32.exe").arg("url.dll,FileProtocolHandler").arg(url).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use noti_bridge_desktop::noti_bridge_core::{ActionInfo, HiddenReason, NotificationState};

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
            hidden_reason: HiddenReason::None,
            ongoing: false,
            pinned: false,
        }
    }

    #[test]
    fn xml_with_special_characters_and_actions_is_accepted_by_winrt() {
        let actions = vec![
            ActionInfo { id: "0".into(), title: "읽음 \"표시\"".into(), accepts_reply: false },
            ActionInfo { id: "1".into(), title: "답장".into(), accepts_reply: true },
        ];
        let n = sample("<제목> & 'x'", "본문\n둘째 줄 ]]> &amp;", actions);
        let xml = toast_xml(&n, Some(Path::new(r"C:\tmp\a&b.png")), true);
        let doc = XmlDocument::new().unwrap();
        doc.LoadXml(&HSTRING::from(xml)).unwrap();
        ToastNotification::CreateToastNotification(&doc).unwrap();
    }

    #[test]
    fn each_source_app_gets_its_own_stable_aumid() {
        assert_eq!(app_aumid("com.kakao.talk"), app_aumid("com.kakao.talk"));
        assert_ne!(app_aumid("com.kakao.talk"), app_aumid("com.Slack"));
        assert!(app_aumid("com.kakao.talk").starts_with(AUMID_PREFIX));
        assert!(app_aumid("x").len() < 129);
    }

    #[test]
    fn tag_fits_toast_limit_and_is_stable() {
        let long = "x".repeat(500);
        assert_eq!(tag_of(&long), tag_of(&long));
        assert!(tag_of(&long).len() <= 64);
    }

    /// 실제 토스트를 원래 앱 이름의 AUMID 로 띄우고 알림 센터 기록에서 지운다. 화면에 토스트가 잠깐 뜬다.
    #[test]
    #[ignore = "실제 Windows 토스트를 띄운다"]
    fn mirror_toast_uses_source_app_identity() {
        let mut n = sample("철수", "점심 먹었어?", vec![]);
        n.app_pkg = "com.kakao.talk".into();
        n.app_name = "카카오톡".into();
        let aumid = app_aumid(&n.app_pkg);
        register_aumid(&aumid, &n.app_name, None).unwrap();
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(&aumid)).unwrap();
        let doc = XmlDocument::new().unwrap();
        doc.LoadXml(&HSTRING::from(toast_xml(&n, None, false))).unwrap();
        let toast = ToastNotification::CreateToastNotification(&doc).unwrap();
        toast.SetTag(&tag_of(&n.nid)).unwrap();
        toast.SetGroup(&HSTRING::from(GROUP)).unwrap();
        notifier.Show(&toast).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(4));
        let history = ToastNotificationManager::History().unwrap();
        let count = || history.GetHistoryWithId(&HSTRING::from(&aumid)).unwrap().Size().unwrap();
        assert_eq!(count(), 1, "원래 앱 AUMID 로 기록돼야 한다");
        history.RemoveGroupedTagWithId(&tag_of(&n.nid), &HSTRING::from(GROUP), &HSTRING::from(&aumid)).unwrap();
        assert_eq!(count(), 0);
    }
}
