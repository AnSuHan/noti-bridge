//! 실기기 점검용 PC 흉내 노드. 초대 URI 를 출력하고, 코드를 자동 수락하고, 받은 알림을 지운다.
//!
//!   cargo run -p noti-bridge-core --example peer_sim -- <데이터 폴더>

use std::sync::{Arc, OnceLock};

use noti_bridge_core::{DEFAULT_RETENTION_MS, Node, NodeConfig, NodeEvent, NodeListener, NotificationState};

#[derive(Default)]
struct Printer {
    node: OnceLock<Node>,
}

impl NodeListener for Printer {
    fn on_event(&self, event: NodeEvent) {
        println!("EVENT {event:?}");
        let Some(node) = self.node.get() else { return };
        match event {
            NodeEvent::PairingCode { .. } => node.confirm_pairing(true),
            NodeEvent::NotificationChanged { notification } if notification.state == NotificationState::Active => {
                if notification.title.contains("dismiss-me") {
                    node.dismiss(&notification.nid).unwrap();
                }
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() {
    let dir = std::env::args().nth(1).expect("데이터 폴더를 지정하세요");
    let config = NodeConfig {
        data_dir: dir,
        secret_key: vec![0x42; 32],
        storage_key: vec![0x43; 32],
        device_name: "peer-sim".into(),
        platform: "sim".into(),
        relay_url: None,
        relay_disabled: false,
        mdns_enabled: true,
        retention_ms: DEFAULT_RETENTION_MS,
    };
    let printer = Arc::new(Printer::default());
    let node = Node::start(config, printer.clone()).await.unwrap();
    let _ = printer.node.set(node.clone());
    println!("INVITE {}", node.create_invite().await.unwrap().to_uri());
    tokio::signal::ctrl_c().await.unwrap();
    node.stop().await;
}
