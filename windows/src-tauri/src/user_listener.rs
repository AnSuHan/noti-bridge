//! PC 알림을 폰으로 보내기 위한 UserNotificationListener. 패키지 ID(MSIX)가 있어야 동작한다.

use std::collections::HashSet;
use std::time::Duration;

use noti_bridge_desktop::noti_bridge_core::{CapturedNotification, Node, RemovalReason};
use windows::UI::Notifications::KnownNotificationBindings;
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{NotificationKinds, UserNotification};

const OWN_DISPLAY_NAME: &str = "noti-bridge";
const POLL_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct CaptureHandle;

fn describe(n: &UserNotification) -> windows::core::Result<Option<CapturedNotification>> {
    let app = n.AppInfo()?;
    let display = app.DisplayInfo()?;
    let app_name = display.DisplayName()?.to_string();
    if app_name == OWN_DISPLAY_NAME {
        return Ok(None);
    }
    let toast = n.Notification()?;
    let visual = toast.Visual()?;
    let binding = visual.GetBinding(&KnownNotificationBindings::ToastGeneric()?)?;
    let texts = binding.GetTextElements()?;
    let mut lines = Vec::new();
    for element in texts {
        lines.push(element.Text()?.to_string());
    }
    let title = lines.first().cloned().unwrap_or_default();
    let body = lines.get(1..).map(|rest| rest.join("\n")).unwrap_or_default();
    Ok(Some(CapturedNotification {
        key: n.Id()?.to_string(),
        app_pkg: app.AppUserModelId()?.to_string(),
        app_name,
        title,
        body,
        actions: vec![],
        icon_png: None,
        body_hidden: false,
    }))
}

fn poll(listener: &UserNotificationListener, node: &Node, seen: &mut HashSet<u32>) -> windows::core::Result<()> {
    let current = listener.GetNotificationsAsync(NotificationKinds::Toast)?.join()?;
    let mut now = HashSet::new();
    for n in current {
        let id = n.Id()?;
        now.insert(id);
        if !seen.contains(&id) {
            if let Some(captured) = describe(&n)? {
                let _ = node.capture_posted(captured);
            }
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
    pub fn start(&self, node: Node) {
        std::thread::spawn(move || {
            let Ok(listener) = UserNotificationListener::Current() else { return };
            let allowed = listener
                .RequestAccessAsync()
                .and_then(|op| op.join())
                .is_ok_and(|s| s == UserNotificationListenerAccessStatus::Allowed);
            if !allowed {
                return;
            }
            let mut seen = HashSet::new();
            loop {
                let _ = poll(&listener, &node, &mut seen);
                std::thread::sleep(POLL_INTERVAL);
            }
        });
    }

    pub fn remove(&self, key: &str) {
        let Ok(id) = key.parse::<u32>() else { return };
        if let Ok(listener) = UserNotificationListener::Current() {
            let _ = listener.RemoveNotification(id);
        }
    }
}
