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
