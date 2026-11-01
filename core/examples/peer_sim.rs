//! 실기기 점검용 PC 흉내 노드. 초대 URI 를 출력하고, 코드를 자동 수락하고, 받은 알림을 지운다.
//!
//!   cargo run -p noti-bridge-core --example peer_sim -- <데이터 폴더> [초대 URI]
//!
//! 초대 URI 를 주면 그 기기에 참여한다. 표준 입력으로 폰의 알림 원본 역할도 한다.
//!   post <키> <제목> <본문>   답장 액션이 붙은 알림 게시
//!   remove <키>               사용자가 지운 것처럼 삭제

use std::sync::{Arc, OnceLock};

use noti_bridge_core::{
    ActionInfo, CapturedNotification, DEFAULT_RETENTION_MS, Node, NodeConfig, NodeEvent, NodeListener, NotificationState,
    RemovalReason,
};

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

fn run_command(node: &Node, line: &str) {
    let mut parts = line.trim().splitn(4, ' ');
    let result = match (parts.next(), parts.next()) {
        (Some("post"), Some(key)) => node.capture_posted(CapturedNotification {
            key: key.into(),
            app_pkg: "com.example.sim".into(),
            app_name: "Sim".into(),
            title: parts.next().unwrap_or_default().into(),
            body: parts.next().unwrap_or_default().into(),
            actions: vec![ActionInfo { id: "0".into(), title: "답장".into(), accepts_reply: true }],
            icon_png: None,
            body_hidden: false,
        }),
        (Some("remove"), Some(key)) => node.capture_removed(key, RemovalReason::Dismissed),
        _ => return,
    };
    if let Err(e) = result {
        println!("ERROR {e}");
    }
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("데이터 폴더를 지정하세요");
    let invite = args.next();
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
    match invite {
        Some(uri) => node.join(&uri).unwrap(),
        None => println!("INVITE {}", node.create_invite().await.unwrap().to_uri()),
    }
    let input = node.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines().map_while(Result::ok) {
            run_command(&input, &line);
        }
    });
    tokio::signal::ctrl_c().await.unwrap();
    node.stop().await;
}
