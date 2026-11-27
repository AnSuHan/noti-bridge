//! PC 알림을 폰으로 보내기 위한 UserNotificationListener. 패키지 ID(MSIX)가 있어야 동작한다.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

use noti_bridge_desktop::noti_bridge_core::{CapturedNotification, Node, RemovalReason};
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationBinding, NotificationKinds, UserNotification};

const OWN_DISPLAY_NAME: &str = "noti-bridge";
const POLL_INTERVAL: Duration = Duration::from_secs(3);
/// 접근이 거부된 동안 사용자가 Windows 설정에서 허용했는지 다시 보는 간격.
const ACCESS_RECHECK: Duration = Duration::from_secs(5);

/// 0 은 아직 확인 전(pending)이다. AtomicU8 의 기본값.
const ALLOWED: u8 = 1;
const DENIED: u8 = 2;
const UNAVAILABLE: u8 = 3;

#[derive(Default)]
pub struct CaptureHandle {
    state: Arc<AtomicU8>,
    /// 앱 규칙이 바뀌면 이미 본 알림도 다시 읽어 새 규칙(본문 숨김 해제 등)을 적용한다.
    rescan: Arc<AtomicBool>,
}

/// ToastGeneric 이 아닌 예전 템플릿(ToastText02 등)으로 띄운 알림도 있어, 없으면 첫 바인딩을 쓴다.
fn binding_of(n: &UserNotification) -> windows::core::Result<Option<NotificationBinding>> {
    let visual = n.Notification()?.Visual()?;
    if let Ok(binding) = visual.GetBinding(&KnownNotificationBindings::ToastGeneric()?) {
        return Ok(Some(binding));
    }
    let bindings = visual.Bindings()?;
    Ok(if bindings.Size()? > 0 { Some(bindings.GetAt(0)?) } else { None })
}

fn describe(n: &UserNotification) -> windows::core::Result<Option<CapturedNotification>> {
    let app = n.AppInfo()?;
    let display = app.DisplayInfo()?;
    let app_name = display.DisplayName()?.to_string();
    let aumid = app.AppUserModelId()?.to_string();
    // 우리가 띄운 미러 토스트(원래 앱 이름으로 표시될 수 있다)를 다시 읽어 되돌려 보내지 않는다.
    if app_name == OWN_DISPLAY_NAME || aumid.starts_with(crate::toast::AUMID_PREFIX) {
        return Ok(None);
    }
    let mut lines = Vec::new();
    if let Some(binding) = binding_of(n)? {
        for element in binding.GetTextElements()? {
            let text = element.Text()?.to_string();
            if !text.trim().is_empty() {
                lines.push(text);
            }
        }
    }
    let title = lines.first().cloned().unwrap_or_default();
    let body = lines.get(1..).map(|rest| rest.join("\n")).unwrap_or_default();
    Ok(Some(CapturedNotification {
        key: n.Id()?.to_string(),
        app_pkg: aumid,
        app_name,
        title,
        body,
        actions: vec![],
        icon_png: None,
        body_hidden: false,
        ongoing: false,
    }))
}

fn poll(listener: &UserNotificationListener, node: &Node, seen: &mut HashSet<u32>) -> windows::core::Result<()> {
    let current = listener.GetNotificationsAsync(NotificationKinds::Toast)?.join()?;
    let mut now = HashSet::new();
    for n in current {
        let Ok(id) = n.Id() else { continue };
        now.insert(id);
        if seen.contains(&id) {
            continue;
        }
        // 알림 하나를 읽지 못해도(형식이 다른 토스트 등) 나머지는 계속 읽는다.
        match describe(&n) {
            Ok(Some(captured)) => {
                let _ = node.capture_posted(captured);
            }
            Ok(None) => {}
            Err(e) => tracing::debug!("PC 알림을 읽지 못함: {e}"),
        }
    }
    for gone in seen.difference(&now) {
        let _ = node.capture_removed(&gone.to_string(), RemovalReason::Dismissed);
    }
    *seen = now;
    Ok(())
}

impl CaptureHandle {
    /// 알림 변경 이벤트는 백그라운드 작업 등록이 필요해, 상주 앱에서는 주기적으로 비교한다.
    /// 접근이 거부되면 사용자가 Windows 설정에서 허용할 때까지 상태만 다시 확인한다.
    pub fn start(&self, node: Node) {
        let state = self.state.clone();
        let rescan = self.rescan.clone();
        std::thread::spawn(move || {
            let Ok(listener) = UserNotificationListener::Current() else {
                state.store(UNAVAILABLE, Ordering::Relaxed);
                return;
            };
            let is_allowed = |s: windows::core::Result<UserNotificationListenerAccessStatus>| {
                s.is_ok_and(|s| s == UserNotificationListenerAccessStatus::Allowed)
            };
            // 패키지 앱에서는 작업 스레드에서 요청해도 동작함을 확인했다(2026-11-07).
            if !is_allowed(listener.RequestAccessAsync().and_then(|op| op.join())) {
                state.store(DENIED, Ordering::Relaxed);
                while !is_allowed(listener.GetAccessStatus()) {
                    std::thread::sleep(ACCESS_RECHECK);
                }
            }
            state.store(ALLOWED, Ordering::Relaxed);
            let mut seen = HashSet::new();
            loop {
                if rescan.swap(false, Ordering::Relaxed) {
                    // 지운 것으로 처리하지 않도록 비교 대상만 비운다. 내용이 같으면 코어가 무시한다.
                    seen.clear();
                }
                if let Err(e) = poll(&listener, &node, &mut seen) {
                    tracing::debug!("PC 알림 목록을 읽지 못함: {e}");
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        });
    }

    pub fn rescan(&self) {
        self.rescan.store(true, Ordering::Relaxed);
    }

    /// UI 에 보여줄 PC 알림 읽기 상태.
    pub fn state(&self) -> &'static str {
        match self.state.load(Ordering::Relaxed) {
            ALLOWED => "allowed",
            DENIED => "denied",
            UNAVAILABLE => "unsupported",
            _ => "pending",
        }
    }

    pub fn remove(&self, key: &str) {
        let Ok(id) = key.parse::<u32>() else { return };
        if let Ok(listener) = UserNotificationListener::Current() {
            let _ = listener.RemoveNotification(id);
        }
    }
}
