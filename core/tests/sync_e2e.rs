//! 실제 iroh 연결로 두 노드를 붙여 검증한다. 기본은 릴레이 없이 루프백 직접 연결이다.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use noti_bridge_core::pairing::PairingInvite;
use noti_bridge_core::*;
use tokio::sync::Notify;

#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<NodeEvent>>,
    changed: Notify,
}

impl NodeListener for Recorder {
    fn on_event(&self, event: NodeEvent) {
        self.events.lock().unwrap().push(event);
        self.changed.notify_waiters();
    }
}

impl Recorder {
    async fn wait_for(&self, what: &str, pred: impl Fn(&NodeEvent) -> bool) -> NodeEvent {
        let wait = async {
            loop {
                let notified = self.changed.notified();
                if let Some(e) = self.events.lock().unwrap().iter().find(|e| pred(e)) {
                    return e.clone();
                }
                notified.await;
            }
        };
        match tokio::time::timeout(Duration::from_secs(30), wait).await {
            Ok(e) => e,
            Err(_) => panic!("시간 초과: {what}\n받은 이벤트: {:?}", self.events.lock().unwrap()),
        }
    }

    fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}

struct TestDevice {
    node: Node,
    rec: Arc<Recorder>,
    config: NodeConfig,
    _dir: Arc<tempfile::TempDir>,
}

async fn device(name: &str, seed: u8, relay: bool) -> TestDevice {
    device_with(name, seed, relay, false).await
}

async fn device_with(name: &str, seed: u8, relay: bool, mdns: bool) -> TestDevice {
    let dir = Arc::new(tempfile::tempdir().unwrap());
    let config = NodeConfig {
        data_dir: dir.path().to_string_lossy().into_owned(),
        secret_key: vec![seed; 32],
        storage_key: vec![seed.wrapping_add(100); 32],
        device_name: name.into(),
        platform: "test".into(),
        relay_url: None,
        relay_disabled: !relay,
        mdns_enabled: mdns,
        retention_ms: DEFAULT_RETENTION_MS,
    };
    let rec = Arc::new(Recorder::default());
    let node = Node::start(config.clone(), rec.clone()).await.unwrap();
    TestDevice { node, rec, config, _dir: dir }
}

impl TestDevice {
    async fn restart(&mut self) {
        self.rec = Arc::new(Recorder::default());
        self.node = Node::start(self.config.clone(), self.rec.clone()).await.unwrap();
    }

    /// 노드가 멈춘 상태에서 호출한다. 해제 유예 시간이 지난 것처럼 페어링 시각을 2분 앞당긴다.
    fn age_pairing(&self) {
        let db = rusqlite::Connection::open(format!("{}/noti-bridge.db", self.config.data_dir)).unwrap();
        db.execute("UPDATE peers SET paired_at = paired_at - 120000", []).unwrap();
    }
}

fn captured(key: &str, body: &str) -> CapturedNotification {
    CapturedNotification {
        key: key.into(),
        app_pkg: "com.example".into(),
        app_name: "Example".into(),
        title: "제목".into(),
        body: body.into(),
        actions: vec![ActionInfo { id: "reply".into(), title: "답장".into(), accepts_reply: true }],
        icon_png: Some(vec![1, 2, 3, 4]),
        body_hidden: false,
    }
}

/// 초대 URI 를 pc 가 만들고 phone 이 참여해, 양쪽 코드 확인까지 끝낸다.
async fn pair(pc: &TestDevice, phone: &TestDevice) {
    let invite: PairingInvite = pc.node.create_invite().await.unwrap();
    phone.node.join(&invite.to_uri()).unwrap();
    let code_a = pc.rec.wait_for("pc 코드", |e| matches!(e, NodeEvent::PairingCode { .. })).await;
    let code_b = phone.rec.wait_for("phone 코드", |e| matches!(e, NodeEvent::PairingCode { .. })).await;
    let (NodeEvent::PairingCode { code: a, .. }, NodeEvent::PairingCode { code: b, .. }) = (code_a, code_b) else {
        unreachable!()
    };
    assert_eq!(a, b, "양쪽 확인 코드가 같아야 한다");
    pc.node.confirm_pairing(true);
    phone.node.confirm_pairing(true);
    let done = |e: &NodeEvent| matches!(e, NodeEvent::PairingFinished { accepted: true, .. });
    pc.rec.wait_for("pc 페어링 완료", done).await;
    phone.rec.wait_for("phone 페어링 완료", done).await;
}

fn changed_with(body: &'static str) -> impl Fn(&NodeEvent) -> bool {
    move |e| matches!(e, NodeEvent::NotificationChanged { notification } if notification.body == body)
}

fn is_connected(e: &NodeEvent) -> bool {
    matches!(e, NodeEvent::ConnectionChanged { path: ConnectionPath::Direct | ConnectionPath::Relay, .. })
}

#[tokio::test(flavor = "multi_thread")]
async fn pair_mirror_dismiss_both_ways_action_and_reply() {
    let pc = device("pc", 1, false).await;
    let phone = device("phone", 2, false).await;
    pair(&pc, &phone).await;

    phone.node.capture_posted(captured("k1", "첫 알림")).unwrap();
    let event = pc.rec.wait_for("미러링", changed_with("첫 알림")).await;
    let NodeEvent::NotificationChanged { notification } = event else { unreachable!() };
    assert_eq!(notification.origin, phone.node.device_id());
    assert_eq!(pc.node.icon(&notification.icon_hash).unwrap(), Some(vec![1, 2, 3, 4]));

    // PC 에서 지우면 폰 OS 알림 삭제가 요청되고, 폰에도 반영된다.
    pc.node.dismiss(&notification.nid).unwrap();
    phone
        .rec
        .wait_for("원격 삭제", |e| matches!(e, NodeEvent::CancelLocal { key } if key == "k1"))
        .await;
    assert!(phone.node.active_notifications().unwrap().is_empty());

    // 폰에서 지우면 PC 목록에서 사라진다.
    phone.node.capture_posted(captured("k2", "둘째")).unwrap();
    pc.rec.wait_for("둘째 미러링", changed_with("둘째")).await;
    pc.rec.clear();
    phone.node.capture_removed("k2", RemovalReason::Dismissed).unwrap();
    pc.rec.wait_for("폰 삭제 반영", |e| matches!(e, NodeEvent::NotificationRemoved { .. })).await;
    assert!(pc.node.active_notifications().unwrap().is_empty());

    // 액션과 답장은 원본 기기에서 실행된다.
    phone.node.capture_posted(captured("k3", "셋째")).unwrap();
    pc.rec.wait_for("셋째 미러링", changed_with("셋째")).await;
    let nid = make_nid(&phone.node.device_id(), "k3");
    pc.node.invoke_action(&nid, "read").unwrap();
    pc.node.send_reply(&nid, "reply", "안녕").unwrap();
    phone
        .rec
        .wait_for("액션", |e| matches!(e, NodeEvent::InvokeAction { action_id, .. } if action_id == "read"))
        .await;
    phone
        .rec
        .wait_for("답장", |e| matches!(e, NodeEvent::SendReply { text, .. } if text == "안녕"))
        .await;

    // PC 알림도 같은 경로로 폰에 미러링된다.
    pc.node.capture_posted(captured("win", "pc 알림")).unwrap();
    phone.rec.wait_for("역방향 미러링", changed_with("pc 알림")).await;

    pc.node.stop().await;
    phone.node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_changes_are_delivered_after_restart_with_no_loss() {
    let mut pc = device("pc", 3, false).await;
    let phone = device("phone", 4, false).await;
    pair(&pc, &phone).await;
    phone.node.capture_posted(captured("before", "연결 중")).unwrap();
    pc.rec.wait_for("연결 중 미러링", changed_with("연결 중")).await;

    pc.node.stop().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    for i in 0..200 {
        phone.node.capture_posted(captured(&format!("off{i}"), "오프라인")).unwrap();
    }
    phone.node.capture_removed("before", RemovalReason::Dismissed).unwrap();
    phone.node.capture_posted(captured("last", "마지막")).unwrap();

    pc.restart().await;
    pc.rec.wait_for("재연결 후 마지막 이벤트", changed_with("마지막")).await;
    let active = pc.node.active_notifications().unwrap();
    assert_eq!(active.len(), 201, "오프라인 동안의 알림 200건 + 마지막 1건");
    assert!(active.iter().all(|n| !n.nid.ends_with(":before")));

    pc.node.stop().await;
    phone.node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_token_and_used_token_are_rejected() {
    let pc = device("pc", 5, false).await;
    let phone = device("phone", 6, false).await;
    let mut invite = pc.node.create_invite().await.unwrap();
    let real = invite.token.clone();
    invite.token = "wrong".into();
    phone.node.join(&invite.to_uri()).unwrap();
    phone.rec.wait_for("잘못된 토큰 거부", |e| matches!(e, NodeEvent::PairingFailed { .. })).await;
    assert!(pc.node.peers().unwrap().is_empty());

    // 틀린 시도가 진짜 토큰을 소모하지는 않는다. 진짜 토큰은 한 번만 쓸 수 있다.
    invite.token = real;
    phone.rec.clear();
    phone.node.join(&invite.to_uri()).unwrap();
    pc.rec.wait_for("코드", |e| matches!(e, NodeEvent::PairingCode { .. })).await;
    phone.rec.wait_for("코드", |e| matches!(e, NodeEvent::PairingCode { .. })).await;

    let other = device("other", 7, false).await;
    other.node.join(&invite.to_uri()).unwrap();
    other.rec.wait_for("재사용 거부", |e| matches!(e, NodeEvent::PairingFailed { .. })).await;
    pc.node.confirm_pairing(false);
    phone.node.confirm_pairing(true);
    let rejected = |e: &NodeEvent| matches!(e, NodeEvent::PairingFinished { accepted: false, .. });
    phone.rec.wait_for("코드 불일치 거부", rejected).await;
    assert!(phone.node.peers().unwrap().is_empty());

    pc.node.stop().await;
    phone.node.stop().await;
    other.node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unpaired_device_is_refused_and_data_removed() {
    let pc = device("pc", 8, false).await;
    let phone = device("phone", 9, false).await;
    pair(&pc, &phone).await;
    phone.node.capture_posted(captured("k", "내용")).unwrap();
    pc.rec.wait_for("미러링", changed_with("내용")).await;

    let phone_id = phone.node.device_id();
    pc.node.unpair(&phone_id).unwrap();
    assert!(pc.node.peers().unwrap().is_empty());
    assert!(pc.node.active_notifications().unwrap().is_empty());

    // 해제 후 폰의 알림은 PC 에 도달하지 않는다.
    pc.rec.clear();
    phone.node.capture_posted(captured("k2", "도달 금지")).unwrap();
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!pc.rec.events.lock().unwrap().iter().any(changed_with("도달 금지")));

    pc.node.stop().await;
    phone.node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn one_phone_mirrors_to_two_pcs_and_dismiss_reaches_both() {
    let pc1 = device("pc1", 10, false).await;
    let pc2 = device("pc2", 11, false).await;
    let phone = device("phone", 12, false).await;
    pair(&pc1, &phone).await;
    phone.rec.clear();
    pair(&pc2, &phone).await;

    phone.node.capture_posted(captured("k", "둘 다")).unwrap();
    pc1.rec.wait_for("pc1 미러링", changed_with("둘 다")).await;
    pc2.rec.wait_for("pc2 미러링", changed_with("둘 다")).await;

    pc1.node.dismiss(&make_nid(&phone.node.device_id(), "k")).unwrap();
    pc2.rec.wait_for("pc2 삭제 반영", |e| matches!(e, NodeEvent::NotificationRemoved { .. })).await;

    pc1.node.stop().await;
    pc2.node.stop().await;
    phone.node.stop().await;
}

fn is_unpaired(e: &NodeEvent) -> bool {
    matches!(e, NodeEvent::PeerUnpaired { .. })
}

#[tokio::test(flavor = "multi_thread")]
async fn connected_peer_learns_of_unpair_from_close_code() {
    let pc = device("pc", 13, false).await;
    let mut phone = device("phone", 14, false).await;
    pair(&pc, &phone).await;
    phone.node.stop().await;
    phone.age_pairing();
    phone.restart().await;
    pc.rec.clear();
    phone.rec.wait_for("재연결", is_connected).await;

    pc.node.unpair(&phone.node.device_id()).unwrap();
    phone.rec.wait_for("코드 4403 으로 해제", is_unpaired).await;
    assert!(phone.node.peers().unwrap().is_empty());

    pc.node.stop().await;
    phone.node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_peer_learns_of_unpair_on_next_connection() {
    let pc = device("pc", 15, false).await;
    let mut phone = device("phone", 16, false).await;
    pair(&pc, &phone).await;
    phone.node.capture_posted(captured("k", "내용")).unwrap();
    pc.rec.wait_for("미러링", changed_with("내용")).await;

    phone.node.stop().await;
    pc.node.unpair(&phone.node.device_id()).unwrap();
    phone.age_pairing();
    phone.restart().await;
    phone.rec.wait_for("다음 연결에서 해제", is_unpaired).await;
    assert!(phone.node.peers().unwrap().is_empty());

    pc.node.stop().await;
    phone.node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn refusal_within_grace_keeps_pairing_without_tight_redial() {
    let pc = device("pc", 17, false).await;
    let phone = device("phone", 18, false).await;
    pair(&pc, &phone).await;
    phone.rec.wait_for("연결", is_connected).await;

    pc.node.unpair(&phone.node.device_id()).unwrap();
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(phone.node.peers().unwrap().len(), 1, "유예 시간 안의 거부는 해제로 보지 않는다");
    let drops = phone
        .rec
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, NodeEvent::ConnectionChanged { path: ConnectionPath::Disconnected, .. }))
        .count();
    assert!(drops <= 4, "거부당한 뒤 재접속은 백오프해야 한다: 5초에 {drops}번 끊김");

    pc.node.stop().await;
    phone.node.stop().await;
}

/// 초대에서 주소를 지워 mDNS 로만 상대를 찾게 한다. 로컬 멀티캐스트가 막힌 환경에서는 실패한다.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "로컬 멀티캐스트 필요"]
async fn pair_and_sync_found_through_mdns_only() {
    let pc = device_with("pc", 22, false, true).await;
    let phone = device_with("phone", 23, false, true).await;
    let mut invite = pc.node.create_invite().await.unwrap();
    invite.addrs.clear();
    invite.relay_url.clear();
    phone.node.join(&invite.to_uri()).unwrap();
    pc.rec.wait_for("pc 코드", |e| matches!(e, NodeEvent::PairingCode { .. })).await;
    phone.rec.wait_for("phone 코드", |e| matches!(e, NodeEvent::PairingCode { .. })).await;
    pc.node.confirm_pairing(true);
    phone.node.confirm_pairing(true);
    pc.rec.wait_for("연결", is_connected).await;
    phone.node.capture_posted(captured("k", "mdns")).unwrap();
    pc.rec.wait_for("미러링", changed_with("mdns")).await;
    pc.node.stop().await;
    phone.node.stop().await;
}

/// n0 공개 릴레이를 실제로 거치는 경로. 인터넷이 필요해 기본 실행에서는 제외한다.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "인터넷과 n0 공개 릴레이 필요"]
async fn pair_and_sync_through_public_relay() {
    let pc = device("pc", 20, true).await;
    let phone = device("phone", 21, true).await;
    pair(&pc, &phone).await;
    pc.rec.wait_for("연결", is_connected).await;
    phone.node.capture_posted(captured("k", "릴레이")).unwrap();
    pc.rec.wait_for("릴레이 미러링", changed_with("릴레이")).await;
    eprintln!("peers: {:?}", pc.node.peers().unwrap());
    pc.node.stop().await;
    phone.node.stop().await;
}
