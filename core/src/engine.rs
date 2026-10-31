//! 네트워크와 분리된 동기화 엔진. 이벤트 로그, Lamport 시계, 충돌 해결을 담당한다.

use std::path::Path;

use crate::crypto::{StorageCipher, sha256_hex};
use crate::error::Result;
use crate::pb::{self, log_event::Kind};
use crate::store::{PeerRow, Store};
use crate::types::*;

const META_CLOCK: &str = "lamport";
const META_SEQ: &str = "seq";
const META_DND: &str = "dnd";
const META_DND_VERSION: &str = "dnd_version";

pub struct Engine {
    pub store: Store,
    device_id: String,
    retention_ms: i64,
}

/// 같은 알림의 동시 변경 해결: version 이 큰 쪽, 동률이면 dismissed 우선.
fn incoming_wins(in_version: u64, in_state: NotificationState, local: &NotificationRecord) -> bool {
    in_version > local.version
        || (in_version == local.version
            && in_state == NotificationState::Dismissed
            && local.state != NotificationState::Dismissed)
}

impl Engine {
    pub fn open(path: &Path, cipher: StorageCipher, device_id: String, retention_ms: i64) -> Result<Self> {
        Ok(Self { store: Store::open(path, cipher)?, device_id, retention_ms })
    }

    pub fn open_in_memory(cipher: StorageCipher, device_id: String, retention_ms: i64) -> Result<Self> {
        Ok(Self { store: Store::open_in_memory(cipher)?, device_id, retention_ms })
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// 관측한 값까지 시계를 당긴 뒤 1 증가시킨 값을 돌려준다.
    fn tick(&self, observed: u64) -> Result<u64> {
        let next = self.store.meta_u64(META_CLOCK)?.max(observed) + 1;
        self.store.set_meta_u64(META_CLOCK, next)?;
        Ok(next)
    }

    fn observe(&self, observed: u64) -> Result<()> {
        if observed > self.store.meta_u64(META_CLOCK)? {
            self.store.set_meta_u64(META_CLOCK, observed)?;
        }
        Ok(())
    }

    fn log(&self, kind: Kind, target: Option<&str>, now: i64) -> Result<()> {
        let seq = self.store.meta_u64(META_SEQ)? + 1;
        self.store.set_meta_u64(META_SEQ, seq)?;
        self.store.append_event(seq, now, target, &pb::LogEvent { seq, kind: Some(kind) })
    }

    pub fn last_seq(&self) -> Result<u64> {
        self.store.meta_u64(META_SEQ)
    }

    fn log_removed(&self, rec: &NotificationRecord) -> Result<()> {
        self.log(
            Kind::Removed(pb::Removed {
                nid: rec.nid.clone(),
                state: pb::NotificationState::from(rec.state) as i32,
                version: rec.version,
                updated_at: rec.updated_at,
            }),
            None,
            rec.updated_at,
        )
    }

    /// OS 알림 리스너가 알림을 받았을 때 호출한다.
    pub fn capture_posted(&self, c: CapturedNotification, now: i64) -> Result<Vec<NodeEvent>> {
        self.store.transaction(|store| {
            let policy = store.app_policy(&c.app_pkg)?;
            if policy == AppPolicy::Block {
                return Ok(vec![]);
            }
            let nid = make_nid(&self.device_id, &c.key);
            let existing = store.notification(&nid)?;
            let hide = policy == AppPolicy::HideBody || c.body_hidden;
            let body = if hide { String::new() } else { c.body };
            let icon_hash = match &c.icon_png {
                Some(png) => {
                    let hash = sha256_hex(png);
                    store.put_icon(&hash, png)?;
                    hash
                }
                None => existing.as_ref().map(|e| e.icon_hash.clone()).unwrap_or_default(),
            };
            let was_active = existing.as_ref().is_some_and(|e| e.state == NotificationState::Active);
            if let Some(e) = existing.as_ref().filter(|_| was_active) {
                let unchanged = e.title == c.title
                    && e.body == body
                    && e.body_hidden == hide
                    && e.app_name == c.app_name
                    && e.actions == c.actions
                    && e.icon_hash == icon_hash;
                if unchanged {
                    return Ok(vec![]);
                }
            }
            let version = self.tick(existing.as_ref().map_or(0, |e| e.version))?;
            let rec = NotificationRecord {
                nid,
                origin: self.device_id.clone(),
                app_pkg: c.app_pkg,
                app_name: c.app_name,
                icon_hash,
                title: c.title,
                body,
                actions: c.actions,
                state: NotificationState::Active,
                version,
                updated_at: now,
                body_hidden: hide,
            };
            store.upsert_notification(&rec)?;
            let wire = Some(pb::Notification::from(&rec));
            let kind = if was_active {
                Kind::Updated(pb::Updated { notification: wire })
            } else {
                Kind::Posted(pb::Posted { notification: wire })
            };
            self.log(kind, None, now)?;
            Ok(vec![NodeEvent::NotificationChanged { notification: rec }])
        })
    }

    /// OS 에서 알림이 사라졌을 때 호출한다.
    pub fn capture_removed(&self, key: &str, reason: RemovalReason, now: i64) -> Result<Vec<NodeEvent>> {
        let nid = make_nid(&self.device_id, key);
        self.store.transaction(|store| {
            let Some(mut rec) = store.notification(&nid)? else { return Ok(vec![]) };
            if rec.state != NotificationState::Active {
                return Ok(vec![]);
            }
            rec.state = match reason {
                RemovalReason::Dismissed => NotificationState::Dismissed,
                RemovalReason::Expired => NotificationState::Expired,
            };
            rec.version = self.tick(rec.version)?;
            rec.updated_at = now;
            store.upsert_notification(&rec)?;
            self.log_removed(&rec)?;
            Ok(vec![NodeEvent::NotificationRemoved { nid: rec.nid, state: rec.state }])
        })
    }

    /// 사용자가 이 기기에서 알림을 지웠을 때 호출한다.
    pub fn request_dismiss(&self, nid: &str, now: i64) -> Result<Vec<NodeEvent>> {
        self.store.transaction(|store| {
            let Some(mut rec) = store.notification(nid)? else { return Ok(vec![]) };
            if rec.state != NotificationState::Active {
                return Ok(vec![]);
            }
            rec.state = NotificationState::Dismissed;
            rec.version = self.tick(rec.version)?;
            rec.updated_at = now;
            store.upsert_notification(&rec)?;
            let mut events = vec![NodeEvent::NotificationRemoved { nid: rec.nid.clone(), state: rec.state }];
            if rec.origin == self.device_id {
                self.log_removed(&rec)?;
                if let Some(key) = nid_key(&self.device_id, nid) {
                    events.push(NodeEvent::CancelLocal { key: key.to_owned() });
                }
            } else {
                self.log(
                    Kind::DismissRequest(pb::DismissRequest { nid: rec.nid.clone(), version: rec.version }),
                    Some(&rec.origin),
                    now,
                )?;
            }
            Ok(events)
        })
    }

    fn origin_of(&self, nid: &str) -> Result<Option<String>> {
        Ok(self.store.notification(nid)?.map(|r| r.origin))
    }

    pub fn request_action(&self, nid: &str, action_id: &str, now: i64) -> Result<Vec<NodeEvent>> {
        let Some(origin) = self.origin_of(nid)? else { return Ok(vec![]) };
        if origin == self.device_id {
            let key = nid_key(&self.device_id, nid).unwrap_or_default().to_owned();
            return Ok(vec![NodeEvent::InvokeAction { key, action_id: action_id.to_owned() }]);
        }
        let kind = Kind::ActionRequest(pb::ActionRequest { nid: nid.to_owned(), action_id: action_id.to_owned() });
        self.store.transaction(|_| self.log(kind, Some(&origin), now))?;
        Ok(vec![])
    }

    pub fn request_reply(&self, nid: &str, action_id: &str, text: &str, now: i64) -> Result<Vec<NodeEvent>> {
        let Some(origin) = self.origin_of(nid)? else { return Ok(vec![]) };
        if origin == self.device_id {
            let key = nid_key(&self.device_id, nid).unwrap_or_default().to_owned();
            return Ok(vec![NodeEvent::SendReply {
                key,
                action_id: action_id.to_owned(),
                text: text.to_owned(),
            }]);
        }
        let kind = Kind::ReplyRequest(pb::ReplyRequest {
            nid: nid.to_owned(),
            action_id: action_id.to_owned(),
            text: text.to_owned(),
        });
        self.store.transaction(|_| self.log(kind, Some(&origin), now))?;
        Ok(vec![])
    }

    pub fn set_dnd(&self, enabled: bool, now: i64) -> Result<()> {
        self.store.transaction(|store| {
            if store.meta_u64(META_DND)? == enabled as u64 && store.meta_u64(META_DND_VERSION)? > 0 {
                return Ok(());
            }
            let version = self.tick(store.meta_u64(META_DND_VERSION)?)?;
            store.set_meta_u64(META_DND, enabled as u64)?;
            store.set_meta_u64(META_DND_VERSION, version)?;
            self.log(Kind::DndChanged(pb::DndChanged { enabled, version }), None, now)
        })
    }

    pub fn dnd(&self) -> Result<bool> {
        Ok(self.store.meta_u64(META_DND)? == 1)
    }

    /// 상대의 이벤트를 적용한다. 이미 받은 seq 는 무시하므로 재전송에 안전하다.
    pub fn apply_event(&self, peer: &str, event: pb::LogEvent, now: i64) -> Result<Vec<NodeEvent>> {
        self.store.transaction(|store| {
            let Some(row) = store.peer(peer)? else { return Ok(vec![]) };
            if event.seq <= row.last_seen_seq {
                return Ok(vec![]);
            }
            let events = match event.kind {
                Some(Kind::Posted(m)) => self.apply_notification(peer, m.notification, now)?,
                Some(Kind::Updated(m)) => self.apply_notification(peer, m.notification, now)?,
                Some(Kind::Removed(m)) => self.apply_removed(peer, m)?,
                Some(Kind::DismissRequest(m)) => self.apply_dismiss_request(m, now)?,
                Some(Kind::ActionRequest(m)) => match self.owned_key(&m.nid) {
                    Some(key) => vec![NodeEvent::InvokeAction { key, action_id: m.action_id }],
                    None => vec![],
                },
                Some(Kind::ReplyRequest(m)) => match self.owned_key(&m.nid) {
                    Some(key) => vec![NodeEvent::SendReply { key, action_id: m.action_id, text: m.text }],
                    None => vec![],
                },
                Some(Kind::DndChanged(m)) => self.apply_dnd(m)?,
                None => vec![],
            };
            store.set_last_seen(peer, event.seq)?;
            Ok(events)
        })
    }

    fn apply_notification(&self, peer: &str, n: Option<pb::Notification>, now: i64) -> Result<Vec<NodeEvent>> {
        let Some(n) = n else { return Ok(vec![]) };
        let rec = NotificationRecord::from(n);
        // 원본 기기만 자기 알림을 게시할 수 있다.
        if rec.origin != peer || nid_key(peer, &rec.nid).is_none() {
            return Ok(vec![]);
        }
        self.observe(rec.version)?;
        if rec.updated_at < now - self.retention_ms {
            return Ok(vec![]);
        }
        if let Some(local) = self.store.notification(&rec.nid)? {
            if !incoming_wins(rec.version, rec.state, &local) {
                return Ok(vec![]);
            }
        }
        self.store.upsert_notification(&rec)?;
        Ok(vec![NodeEvent::NotificationChanged { notification: rec }])
    }

    fn apply_removed(&self, peer: &str, m: pb::Removed) -> Result<Vec<NodeEvent>> {
        self.observe(m.version)?;
        let state = NotificationState::from(m.state());
        let Some(mut local) = self.store.notification(&m.nid)? else { return Ok(vec![]) };
        if local.origin != peer || !incoming_wins(m.version, state, &local) {
            return Ok(vec![]);
        }
        let was_active = local.state == NotificationState::Active;
        local.state = state;
        local.version = m.version;
        local.updated_at = m.updated_at;
        self.store.upsert_notification(&local)?;
        Ok(if was_active { vec![NodeEvent::NotificationRemoved { nid: m.nid, state }] } else { vec![] })
    }

    fn owned_key(&self, nid: &str) -> Option<String> {
        nid_key(&self.device_id, nid).map(str::to_owned)
    }

    fn apply_dismiss_request(&self, m: pb::DismissRequest, now: i64) -> Result<Vec<NodeEvent>> {
        self.observe(m.version)?;
        let Some(key) = self.owned_key(&m.nid) else { return Ok(vec![]) };
        let Some(mut local) = self.store.notification(&m.nid)? else { return Ok(vec![]) };
        // 삭제 요청 이후에 내용이 갱신됐으면 요청이 진다. 갱신본이 상대에서 알림을 되살린다.
        if local.state != NotificationState::Active || m.version < local.version {
            return Ok(vec![]);
        }
        local.state = NotificationState::Dismissed;
        local.version = self.tick(m.version)?;
        local.updated_at = now;
        self.store.upsert_notification(&local)?;
        self.log_removed(&local)?;
        Ok(vec![
            NodeEvent::NotificationRemoved { nid: m.nid, state: NotificationState::Dismissed },
            NodeEvent::CancelLocal { key },
        ])
    }

    fn apply_dnd(&self, m: pb::DndChanged) -> Result<Vec<NodeEvent>> {
        self.observe(m.version)?;
        if m.version <= self.store.meta_u64(META_DND_VERSION)? {
            return Ok(vec![]);
        }
        self.store.set_meta_u64(META_DND, m.enabled as u64)?;
        self.store.set_meta_u64(META_DND_VERSION, m.version)?;
        Ok(vec![NodeEvent::DndChanged { enabled: m.enabled }])
    }

    /// 페어링 완료 처리. 이전 로그는 건너뛰고 현재 활성 알림만 새 상대에게 보낸다.
    pub fn add_peer(&self, mut peer: PeerRow, now: i64) -> Result<()> {
        self.store.transaction(|store| {
            let device_id = peer.device_id.clone();
            peer.acked_seq = self.last_seq()?;
            peer.last_seen_seq = 0;
            store.put_peer(&peer)?;
            for rec in store.active_from_origin(&self.device_id)? {
                let kind = Kind::Posted(pb::Posted { notification: Some(pb::Notification::from(&rec)) });
                self.log(kind, Some(&device_id), now)?;
            }
            Ok(())
        })
    }

    /// 페어링 해제: 신뢰 목록과 그 상대에게서 받은 데이터를 지운다.
    pub fn remove_peer(&self, device_id: &str) -> Result<Vec<NodeEvent>> {
        self.store.transaction(|store| {
            let events = store
                .active_from_origin(device_id)?
                .into_iter()
                .map(|r| NodeEvent::NotificationRemoved { nid: r.nid, state: NotificationState::Expired })
                .collect();
            store.delete_from_origin(device_id)?;
            store.delete_peer(device_id)?;
            Ok(events)
        })
    }

    /// 보존 기간이 지난 알림과 로그를 정리한다.
    pub fn maintain(&self, now: i64) -> Result<Vec<NodeEvent>> {
        let cutoff = now - self.retention_ms;
        self.store.transaction(|store| {
            let expired = store.purge_notifications(cutoff)?;
            store.compact_events(cutoff)?;
            Ok(expired
                .into_iter()
                .map(|nid| NodeEvent::NotificationRemoved { nid, state: NotificationState::Expired })
                .collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pb::log_event::Kind;

    const NOW: i64 = 1_000_000_000;
    const HOUR: i64 = 3_600_000;

    fn engine(id: &str) -> Engine {
        Engine::open_in_memory(StorageCipher::new(&[1u8; 32]), id.into(), 24 * HOUR).unwrap()
    }

    fn peer(id: &str) -> PeerRow {
        PeerRow {
            device_id: id.into(),
            name: id.into(),
            platform: "test".into(),
            paired_at: NOW,
            relay_url: String::new(),
            addrs: vec![],
            last_seen_seq: 0,
            acked_seq: 0,
        }
    }

    fn pair() -> (Engine, Engine) {
        let (a, b) = (engine("phone"), engine("pc"));
        a.add_peer(peer("pc"), NOW).unwrap();
        b.add_peer(peer("phone"), NOW).unwrap();
        (a, b)
    }

    fn captured(key: &str, body: &str) -> CapturedNotification {
        CapturedNotification {
            key: key.into(),
            app_pkg: "com.example".into(),
            app_name: "Example".into(),
            title: "t".into(),
            body: body.into(),
            actions: vec![],
            icon_png: None,
            body_hidden: false,
        }
    }

    /// from 의 로그 중 to 가 아직 안 받은 것을 전달한다.
    fn sync(from: &Engine, to: &Engine) -> Vec<NodeEvent> {
        let last = to.store.peer(from.device_id()).unwrap().unwrap().last_seen_seq;
        let mut out = vec![];
        for ev in from.store.events_for(to.device_id(), last, 1000).unwrap() {
            out.extend(to.apply_event(from.device_id(), ev, NOW).unwrap());
        }
        out
    }

    fn state(e: &Engine, nid: &str) -> NotificationState {
        e.store.notification(nid).unwrap().unwrap().state
    }

    #[test]
    fn posted_reaches_mirror() {
        let (a, b) = pair();
        a.capture_posted(captured("k", "본문"), NOW).unwrap();
        let events = sync(&a, &b);
        assert!(matches!(&events[0], NodeEvent::NotificationChanged { notification } if notification.body == "본문"));
    }

    #[test]
    fn identical_repost_is_deduplicated() {
        let (a, _b) = pair();
        assert_eq!(a.capture_posted(captured("k", "x"), NOW).unwrap().len(), 1);
        assert!(a.capture_posted(captured("k", "x"), NOW + 1).unwrap().is_empty());
        assert_eq!(a.capture_posted(captured("k", "y"), NOW + 2).unwrap().len(), 1);
        let log = a.store.events_for("pc", 0, 10).unwrap();
        assert!(matches!(log[0].kind, Some(Kind::Posted(_))));
        assert!(matches!(log[1].kind, Some(Kind::Updated(_))));
    }

    #[test]
    fn dismiss_on_mirror_cancels_origin_and_syncs_back() {
        let (a, b) = pair();
        a.capture_posted(captured("k", "x"), NOW).unwrap();
        sync(&a, &b);
        let nid = make_nid("phone", "k");
        b.request_dismiss(&nid, NOW).unwrap();
        let events = sync(&b, &a);
        assert!(events.contains(&NodeEvent::CancelLocal { key: "k".into() }));
        assert_eq!(state(&a, &nid), NotificationState::Dismissed);
        // OS 가 뒤이어 보내는 제거 콜백은 중복 이벤트를 만들지 않는다.
        assert!(a.capture_removed("k", RemovalReason::Dismissed, NOW).unwrap().is_empty());
        sync(&a, &b);
        assert_eq!(state(&b, &nid), NotificationState::Dismissed);
    }

    #[test]
    fn dismiss_on_origin_removes_from_mirror() {
        let (a, b) = pair();
        a.capture_posted(captured("k", "x"), NOW).unwrap();
        sync(&a, &b);
        a.capture_removed("k", RemovalReason::Dismissed, NOW).unwrap();
        let events = sync(&a, &b);
        assert!(matches!(events[0], NodeEvent::NotificationRemoved { .. }));
    }

    #[test]
    fn version_tie_prefers_dismissed_on_both_sides() {
        let (a, b) = pair();
        a.capture_posted(captured("k", "x"), NOW).unwrap();
        sync(&a, &b);
        let nid = make_nid("phone", "k");
        // 양쪽이 동시에 변경: 원본은 갱신, 미러는 삭제. 같은 version 이 된다.
        a.capture_posted(captured("k", "y"), NOW).unwrap();
        b.request_dismiss(&nid, NOW).unwrap();
        assert_eq!(
            a.store.notification(&nid).unwrap().unwrap().version,
            b.store.notification(&nid).unwrap().unwrap().version
        );
        sync(&a, &b);
        sync(&b, &a);
        sync(&a, &b);
        assert_eq!(state(&a, &nid), NotificationState::Dismissed);
        assert_eq!(state(&b, &nid), NotificationState::Dismissed);
    }

    #[test]
    fn newer_update_beats_stale_dismiss() {
        let (a, b) = pair();
        a.capture_posted(captured("k", "x"), NOW).unwrap();
        sync(&a, &b);
        let nid = make_nid("phone", "k");
        b.request_dismiss(&nid, NOW).unwrap();
        a.capture_posted(captured("k", "y"), NOW).unwrap();
        a.capture_posted(captured("k", "z"), NOW).unwrap();
        sync(&b, &a);
        sync(&a, &b);
        assert_eq!(state(&a, &nid), NotificationState::Active);
        assert_eq!(state(&b, &nid), NotificationState::Active);
        assert_eq!(b.store.notification(&nid).unwrap().unwrap().body, "z");
    }

    #[test]
    fn replay_is_idempotent_for_actions() {
        let (a, b) = pair();
        a.capture_posted(captured("k", "x"), NOW).unwrap();
        sync(&a, &b);
        b.request_reply(&make_nid("phone", "k"), "reply", "hi", NOW).unwrap();
        let log = b.store.events_for("phone", 0, 10).unwrap();
        let first = a.apply_event("pc", log[0].clone(), NOW).unwrap();
        assert_eq!(first.len(), 1);
        assert!(a.apply_event("pc", log[0].clone(), NOW).unwrap().is_empty());
    }

    #[test]
    fn offline_queue_delivers_everything_after_reconnect() {
        let (a, b) = pair();
        for i in 0..50 {
            a.capture_posted(captured(&format!("k{i}"), "x"), NOW).unwrap();
        }
        for i in 0..10 {
            a.capture_removed(&format!("k{i}"), RemovalReason::Dismissed, NOW).unwrap();
        }
        sync(&a, &b);
        assert_eq!(b.store.active_notifications().unwrap().len(), 40);
    }

    #[test]
    fn peer_cannot_forge_other_origin() {
        let (a, b) = pair();
        b.capture_posted(captured("k", "x"), NOW).unwrap();
        let mut forged = b.store.events_for("phone", 0, 1).unwrap().remove(0);
        if let Some(Kind::Posted(p)) = &mut forged.kind {
            let n = p.notification.as_mut().unwrap();
            n.origin = "someone".into();
            n.nid = make_nid("someone", "k");
        }
        assert!(a.apply_event("pc", forged, NOW).unwrap().is_empty());
        assert!(a.store.active_notifications().unwrap().is_empty());
    }

    #[test]
    fn unknown_peer_events_are_ignored() {
        let (a, b) = pair();
        b.capture_posted(captured("k", "x"), NOW).unwrap();
        let ev = b.store.events_for("phone", 0, 1).unwrap().remove(0);
        assert!(a.apply_event("stranger", ev, NOW).unwrap().is_empty());
    }

    #[test]
    fn block_and_hide_policies_apply_at_origin() {
        let (a, _b) = pair();
        a.store.set_app_rule("com.example", AppPolicy::Block).unwrap();
        assert!(a.capture_posted(captured("k", "x"), NOW).unwrap().is_empty());
        a.store.set_app_rule("com.example", AppPolicy::HideBody).unwrap();
        a.capture_posted(captured("k", "x"), NOW).unwrap();
        let rec = a.store.notification(&make_nid("phone", "k")).unwrap().unwrap();
        assert!(rec.body.is_empty() && rec.body_hidden);
    }

    #[test]
    fn banking_apps_hide_body_by_default_until_user_allows() {
        let (a, _b) = pair();
        let mut c = captured("k", "잔액 1000원");
        c.app_pkg = "com.kbstar.kbbank".into();
        a.capture_posted(c.clone(), NOW).unwrap();
        assert!(a.store.notification(&make_nid("phone", "k")).unwrap().unwrap().body_hidden);
        a.store.set_app_rule("com.kbstar.kbbank", AppPolicy::Allow).unwrap();
        a.capture_posted(c, NOW).unwrap();
        assert_eq!(a.store.notification(&make_nid("phone", "k")).unwrap().unwrap().body, "잔액 1000원");
    }

    #[test]
    fn retention_purges_notifications_and_compacts_log() {
        let (a, b) = pair();
        a.capture_posted(captured("k", "x"), NOW).unwrap();
        sync(&a, &b);
        let events = b.maintain(NOW + 25 * HOUR).unwrap();
        assert!(matches!(events[0], NodeEvent::NotificationRemoved { state: NotificationState::Expired, .. }));
        assert!(b.store.notification(&make_nid("phone", "k")).unwrap().is_none());
        a.maintain(NOW + 25 * HOUR).unwrap();
        assert_eq!(a.store.event_count().unwrap(), 0);
    }

    #[test]
    fn acked_events_are_compacted_only_when_all_peers_have_them() {
        let (a, _b) = pair();
        a.add_peer(peer("pc2"), NOW).unwrap();
        a.capture_posted(captured("k", "x"), NOW).unwrap();
        let seq = a.last_seq().unwrap();
        a.store.set_acked("pc", seq).unwrap();
        a.maintain(NOW).unwrap();
        assert_eq!(a.store.event_count().unwrap(), 1);
        a.store.set_acked("pc2", seq).unwrap();
        a.maintain(NOW).unwrap();
        assert_eq!(a.store.event_count().unwrap(), 0);
    }

    #[test]
    fn new_peer_gets_snapshot_of_active_notifications_only() {
        let a = engine("phone");
        a.capture_posted(captured("old", "x"), NOW).unwrap();
        a.capture_posted(captured("gone", "x"), NOW).unwrap();
        a.capture_removed("gone", RemovalReason::Dismissed, NOW).unwrap();
        a.add_peer(peer("pc"), NOW).unwrap();
        let acked = a.store.peer("pc").unwrap().unwrap().acked_seq;
        let events = a.store.events_for("pc", acked, 10).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn unpair_deletes_peer_data() {
        let (a, b) = pair();
        a.capture_posted(captured("k", "x"), NOW).unwrap();
        sync(&a, &b);
        let events = b.remove_peer("phone").unwrap();
        assert_eq!(events.len(), 1);
        assert!(b.store.peer("phone").unwrap().is_none());
        assert!(b.store.notification(&make_nid("phone", "k")).unwrap().is_none());
    }

    #[test]
    fn dnd_syncs_with_lamport_order() {
        let (a, b) = pair();
        a.set_dnd(true, NOW).unwrap();
        let events = sync(&a, &b);
        assert_eq!(events, vec![NodeEvent::DndChanged { enabled: true }]);
        assert!(b.dnd().unwrap());
    }
}
