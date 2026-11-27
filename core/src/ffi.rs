//! Kotlin·Swift 에서 쓰는 UniFFI 인터페이스.

use std::sync::Arc;

use tokio::runtime::Runtime;

use crate::error::{CoreError, Result};
use crate::node::{Node, NodeListener};
use crate::pairing::PairingInvite;
use crate::types::*;

#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {
    fn on_event(&self, event: NodeEvent);
}

struct ListenerBridge(Arc<dyn EventListener>);

impl NodeListener for ListenerBridge {
    fn on_event(&self, event: NodeEvent) {
        self.0.on_event(event);
    }
}

#[derive(uniffi::Object)]
pub struct NotiBridgeNode {
    runtime: Runtime,
    node: Node,
}

/// 기기 개인키나 저장소 키로 쓸 32바이트 난수를 만든다.
#[uniffi::export]
pub fn generate_key() -> Result<Vec<u8>> {
    crate::crypto::random_key()
}

#[uniffi::export]
pub fn invite_uri(invite: PairingInvite) -> String {
    invite.to_uri()
}

/// 코어가 만드는 문구(오류, 변경 사항)의 언어. "ko" 로 시작하면 한국어, 그 밖에는 영어.
#[uniffi::export]
pub fn set_language(tag: String) {
    crate::i18n::set_language(&tag);
}

#[uniffi::export]
pub fn core_version() -> String {
    crate::release::VERSION.to_owned()
}

#[uniffi::export]
pub fn release_notes() -> Vec<ReleaseNote> {
    crate::release::release_notes()
}

#[uniffi::export]
impl NotiBridgeNode {
    #[uniffi::constructor]
    pub fn start(config: NodeConfig, listener: Arc<dyn EventListener>) -> Result<Arc<Self>> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| CoreError::Network(e.to_string()))?;
        let node = runtime.block_on(Node::start(config, Arc::new(ListenerBridge(listener))))?;
        Ok(Arc::new(Self { runtime, node }))
    }

    pub fn device_id(&self) -> String {
        self.node.device_id()
    }

    pub fn create_invite(&self) -> Result<PairingInvite> {
        self.runtime.block_on(self.node.create_invite())
    }

    pub fn join(&self, uri: String) -> Result<()> {
        let _guard = self.runtime.enter();
        self.node.join(&uri)
    }

    pub fn confirm_pairing(&self, accepted: bool) {
        self.node.confirm_pairing(accepted);
    }

    pub fn capture_posted(&self, notification: CapturedNotification) -> Result<()> {
        self.node.capture_posted(notification)
    }

    pub fn capture_removed(&self, key: String, reason: RemovalReason) -> Result<()> {
        self.node.capture_removed(&key, reason)
    }

    pub fn dismiss(&self, nid: String) -> Result<()> {
        self.node.dismiss(&nid)
    }

    pub fn invoke_action(&self, nid: String, action_id: String) -> Result<()> {
        self.node.invoke_action(&nid, &action_id)
    }

    pub fn send_reply(&self, nid: String, action_id: String, text: String) -> Result<()> {
        self.node.send_reply(&nid, &action_id, &text)
    }

    pub fn set_dnd(&self, enabled: bool) -> Result<()> {
        self.node.set_dnd(enabled)
    }

    pub fn active_notifications(&self) -> Result<Vec<NotificationRecord>> {
        self.node.active_notifications()
    }

    pub fn hide_local(&self, nid: String) -> Result<()> {
        self.node.hide_local(&nid)
    }

    /// 숨기거나(true) 다시 보인다(false). 연결된 기기도 같이 바뀐다.
    pub fn set_hidden(&self, nid: String, hidden: bool) -> Result<()> {
        self.node.set_hidden(&nid, hidden)
    }

    pub fn hidden_notifications(&self) -> Result<Vec<HiddenNotification>> {
        self.node.hidden_notifications()
    }

    /// app_pkg 가 None 이면 전체. 고정한 알림은 남긴다. dismiss 가 true 면 원본 기기의 알림까지 지운다.
    pub fn clear(&self, app_pkg: Option<String>, dismiss: bool) -> Result<()> {
        self.node.clear(app_pkg.as_deref(), dismiss)
    }

    pub fn set_pinned(&self, nid: String, pinned: bool) -> Result<()> {
        self.node.set_pinned(&nid, pinned)
    }

    pub fn filters(&self) -> Result<Vec<NotificationFilter>> {
        self.node.filters()
    }

    pub fn add_filter(&self, kind: FilterKind, pattern: String, label: String) -> Result<()> {
        let _guard = self.runtime.enter();
        self.node.add_filter(kind, &pattern, &label)
    }

    pub fn remove_filter(&self, id: i64) -> Result<()> {
        let _guard = self.runtime.enter();
        self.node.remove_filter(id)
    }

    pub fn cloud_status(&self) -> CloudStatus {
        self.node.cloud_status()
    }

    pub fn cloud_set_config(&self, api_key: String, project_id: String) -> Result<()> {
        self.node.cloud_set_config(&api_key, &project_id)
    }

    pub fn cloud_sign_in(&self, email: String, password: String, create: bool) -> Result<()> {
        self.runtime.block_on(self.node.cloud_sign_in(&email, &password, create))
    }

    pub fn cloud_set_social(
        &self,
        google_client_id: String,
        google_client_secret: String,
        google_web_client_id: String,
        github_client_id: String,
    ) -> Result<()> {
        self.node.cloud_set_social(&google_client_id, &google_client_secret, &google_web_client_id, &github_client_id)
    }

    pub fn cloud_google_web_client_id(&self) -> String {
        self.node.cloud_google_web_client_id()
    }

    /// Android Credential Manager 로 받은 Google ID 토큰으로 로그인한다.
    pub fn cloud_sign_in_google(&self, id_token: String) -> Result<()> {
        self.runtime.block_on(self.node.cloud_sign_in_google_id_token(&id_token))
    }

    pub fn cloud_github_begin(&self) -> Result<DeviceCodeInfo> {
        self.runtime.block_on(self.node.cloud_github_begin())
    }

    /// 사용자가 GitHub 에 코드를 넣을 때까지 막힌다. UI 스레드가 아닌 곳에서 부른다.
    pub fn cloud_github_finish(&self) -> Result<()> {
        self.runtime.block_on(self.node.cloud_github_finish())
    }

    pub fn cloud_send_password_reset(&self, email: String) -> Result<()> {
        self.runtime.block_on(self.node.cloud_send_password_reset(&email))
    }

    pub fn cloud_sign_out(&self) -> Result<()> {
        self.runtime.block_on(self.node.cloud_sign_out())
    }

    pub fn cloud_sync(&self) -> Result<()> {
        self.runtime.block_on(self.node.cloud_sync())
    }

    pub fn cloud_devices(&self) -> Result<Vec<CloudDevice>> {
        self.runtime.block_on(self.node.cloud_devices())
    }

    pub fn icon(&self, hash: String) -> Result<Option<Vec<u8>>> {
        self.node.icon(&hash)
    }

    pub fn peers(&self) -> Result<Vec<PeerInfo>> {
        self.node.peers()
    }

    pub fn app_rules(&self) -> Result<Vec<AppRule>> {
        self.node.app_rules()
    }

    pub fn set_app_rule(&self, app_pkg: String, policy: AppPolicy) -> Result<()> {
        let _guard = self.runtime.enter();
        self.node.set_app_rule(&app_pkg, policy)
    }

    pub fn unpair(&self, device_id: String) -> Result<()> {
        self.node.unpair(&device_id)
    }

    pub fn network_changed(&self) {
        self.runtime.block_on(self.node.network_changed());
    }

    pub fn stop(&self) {
        self.runtime.block_on(self.node.stop());
    }
}
