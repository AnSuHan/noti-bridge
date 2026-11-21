//! Firebase 에뮬레이터로 하는 계정 로그인·기기 목록·필터 동기화 시험.
//!
//! cd firebase && npx firebase-tools emulators:start --only auth,firestore --project demo-noti-bridge
//! cargo test -p noti-bridge-core --test cloud_emulator -- --ignored

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use noti_bridge_core::cloud::CloudConfig;
use noti_bridge_core::{DEFAULT_RETENTION_MS, FilterKind, Node, NodeConfig, NodeEvent, NodeListener};

struct Quiet;

impl NodeListener for Quiet {
    fn on_event(&self, _event: NodeEvent) {}
}

fn emulator() -> CloudConfig {
    CloudConfig {
        api_key: "demo-key".into(),
        project_id: "demo-noti-bridge".into(),
        auth_emulator: Some("127.0.0.1:9099".into()),
        firestore_emulator: Some("127.0.0.1:8080".into()),
    }
}

async fn node(dir: &tempfile::TempDir, name: &str) -> Node {
    let config = NodeConfig {
        data_dir: dir.path().to_string_lossy().into_owned(),
        secret_key: noti_bridge_core::crypto::random_key().unwrap(),
        storage_key: vec![3u8; 32],
        device_name: name.into(),
        platform: "test".into(),
        relay_url: None,
        relay_disabled: true,
        mdns_enabled: false,
        retention_ms: DEFAULT_RETENTION_MS,
    };
    let node = Node::start(config, Arc::new(Quiet)).await.unwrap();
    node.cloud_set_config_full(&emulator()).unwrap();
    node
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Firebase 에뮬레이터가 필요하다"]
async fn sign_up_sync_devices_and_filters() {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    let email = format!("user{stamp}@example.com");
    let (d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let pc = node(&d1, "PC").await;
    let phone = node(&d2, "Phone").await;

    // 비밀번호 오류 메시지와 가입
    assert!(pc.cloud_sign_in(&email, "123", true).await.is_err());
    pc.add_filter(FilterKind::Keyword, "광고").unwrap();
    pc.cloud_sign_in(&email, "secret123", true).await.unwrap();
    let status = pc.cloud_status();
    assert!(status.signed_in && status.email == email && status.last_sync > 0);

    // 이미 가입된 이메일로 다시 가입하면 실패, 다른 기기에서 로그인하면 필터를 받아 온다.
    assert!(phone.cloud_sign_in(&email, "secret123", true).await.is_err());
    assert!(phone.cloud_sign_in(&email, "wrong-pass", false).await.is_err());
    phone.cloud_sign_in(&email, "secret123", false).await.unwrap();
    let filters = phone.filters().unwrap();
    assert_eq!(filters.len(), 1);
    assert_eq!(filters[0].pattern, "광고");

    let devices = phone.cloud_devices().await.unwrap();
    assert_eq!(devices.len(), 2);
    assert!(devices[0].is_self && devices[0].name == "Phone");

    // 폰에서 필터를 바꾸면 PC 가 다음 동기화 때 받아 온다(더 최근 쪽이 이긴다).
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    phone.add_filter(FilterKind::App, "com.kakao.talk").unwrap();
    phone.cloud_sync().await.unwrap();
    pc.cloud_sync().await.unwrap();
    assert_eq!(pc.filters().unwrap().len(), 2);

    // 로그아웃하면 계정의 기기 목록에서 빠진다.
    pc.cloud_sign_out().await.unwrap();
    assert!(!pc.cloud_status().signed_in);
    assert_eq!(phone.cloud_devices().await.unwrap().len(), 1);

    // 다른 사용자의 데이터는 보안 규칙으로 막힌다.
    let other = format!("other{stamp}@example.com");
    pc.cloud_sign_in(&other, "secret123", true).await.unwrap();
    assert_eq!(pc.cloud_devices().await.unwrap().len(), 1);
    pc.stop().await;
    phone.stop().await;
}
