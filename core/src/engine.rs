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
/// 필터 목록을 마지막으로 바꾼 시각(ms). 계정·기기 동기화에서 더 최근 쪽을 고를 때 쓴다.
pub const META_FILTERS_UPDATED: &str = "filters_updated_at";
/// 이보다 오래된 알림은 받아도 소리·팝업 없이 넣는다. 기기 사이 시계 차이를 감안한 값이다.
pub const FRESH_MS: i64 = 60_000;

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

fn filter_specs(filters: &[NotificationFilter]) -> Vec<FilterSpec> {
    filters
        .iter()
        .map(|f| FilterSpec { kind: f.kind, pattern: f.pattern.clone(), label: f.label.clone() })
        .collect()
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

    /// 이 기기 화면용 표시 정보(고정 여부)를 채운다.
    fn decorate(&self, mut rec: NotificationRecord) -> Result<NotificationRecord> {
        rec.pinned = rec.ongoing || self.store.is_pinned(&rec.nid)?;
        Ok(rec)
    }

    /// OS 알림 리스너가 알림을 받았을 때 호출한다.
    pub fn capture_posted(&self, c: CapturedNotification, now: i64) -> Result<Vec<NodeEvent>> {
        self.store.transaction(|store| {
            let policy = store.app_policy(&c.app_pkg)?;
            if policy == AppPolicy::Block {
                return Ok(vec![]);
            }
            let hide = policy == AppPolicy::HideBody || c.body_hidden;
            let filters = store.filters()?;
            if filters.iter().any(|f| f.matches(&c.app_pkg, &c.app_name, &c.title, &c.body, hide)) {
                return Ok(vec![]);
            }
            let nid = make_nid(&self.device_id, &c.key);
            let existing = store.notification(&nid)?;
            let body = if hide { String::new() } else { c.body };
            let hidden_reason = match (c.body_hidden, hide) {
                (true, _) => HiddenReason::System,
                (false, true) => HiddenReason::Policy,
                _ => HiddenReason::None,
            };
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
                    && e.hidden_reason == hidden_reason
                    && e.app_name == c.app_name
                    && e.actions == c.actions
                    && e.icon_hash == icon_hash
                    && e.ongoing == c.ongoing;
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
                hidden_reason,
                ongoing: c.ongoing,
                pinned: false,
            };
            store.upsert_notification(&rec)?;
            // 내용이 바뀌면 숨겼던 알림도 다시 보인다(새 메시지). 재생 중 알림은 곡이 바뀌어도 숨긴 채로 둔다.
            if !rec.ongoing {
                store.set_hidden(&rec.nid, false)?;
            }
            let wire = Some(pb::Notification::from(&rec));
            let kind = if was_active {
                Kind::Updated(pb::Updated { notification: wire })
            } else {
                Kind::Posted(pb::Posted { notification: wire })
            };
            self.log(kind, None, now)?;
            let alert = !rec.ongoing;
            Ok(vec![NodeEvent::NotificationChanged { notification: self.decorate(rec)?, alert }])
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
        self.store.transaction(|_| self.dismiss_in(nid, now))
    }

    fn dismiss_in(&self, nid: &str, now: i64) -> Result<Vec<NodeEvent>> {
        let store = &self.store;
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
        self.apply_event_with(peer, event, now, false)
    }

    /// backlog 가 true 면 끊겨 있던 동안 쌓인 이벤트다. 저장과 삭제 동기화는 그대로 하고, 알림은 소리·팝업 없이 넣는다.
    pub fn apply_event_with(&self, peer: &str, event: pb::LogEvent, now: i64, backlog: bool) -> Result<Vec<NodeEvent>> {
        self.store.transaction(|store| {
            let Some(row) = store.peer(peer)? else { return Ok(vec![]) };
            if event.seq <= row.last_seen_seq {
                return Ok(vec![]);
            }
            let events = match event.kind {
                Some(Kind::Posted(m)) => self.apply_notification(peer, m.notification, now, backlog)?,
                Some(Kind::Updated(m)) => self.apply_notification(peer, m.notification, now, backlog)?,
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
                Some(Kind::HideChanged(m)) => {
                    self.observe(m.version)?;
                    self.hide_in(&m.nid, m.hidden, now)?
                }
                Some(Kind::FiltersChanged(m)) => self.apply_filters(m)?,
                Some(Kind::AppRuleChanged(m)) => self.set_rule_in(&m.app_pkg, AppPolicy::from_u32(m.policy), now)?,
                None => vec![],
            };
            store.set_last_seen(peer, event.seq)?;
            Ok(events)
        })
    }

    fn apply_notification(
        &self,
        peer: &str,
        n: Option<pb::Notification>,
        now: i64,
        backlog: bool,
    ) -> Result<Vec<NodeEvent>> {
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
        let local = self.store.notification(&rec.nid)?;
        if let Some(local) = &local {
            if !incoming_wins(rec.version, rec.state, local) {
                return Ok(vec![]);
            }
        }
        let was_visible = match &local {
            Some(l) => l.state == NotificationState::Active && self.is_visible(l)?,
            None => false,
        };
        self.store.upsert_notification(&rec)?;
        if rec.state == NotificationState::Active && !rec.ongoing {
            self.store.set_hidden(&rec.nid, false)?;
        }
        if !self.is_visible(&rec)? {
            // 필터에 걸리거나 숨긴 알림은 저장만 하고 보여주지 않는다. 이미 떠 있던 알림이면 내린다.
            return Ok(if was_visible {
                vec![NodeEvent::NotificationRemoved { nid: rec.nid, state: rec.state }]
            } else {
                vec![]
            });
        }
        let alert =
            !backlog && !rec.ongoing && rec.state == NotificationState::Active && now - rec.updated_at <= FRESH_MS;
        Ok(vec![NodeEvent::NotificationChanged { notification: self.decorate(rec)?, alert }])
    }

    fn filter_of(&self, rec: &NotificationRecord, filters: &[NotificationFilter]) -> Option<i64> {
        filters.iter().find(|f| f.matches_record(rec)).map(|f| f.id)
    }

    /// 필터에 걸리지 않고, 이 기기에서 숨기지도 않은 알림인지.
    fn is_visible(&self, rec: &NotificationRecord) -> Result<bool> {
        Ok(self.filter_of(rec, &self.store.filters()?).is_none() && !self.store.is_hidden(&rec.nid)?)
    }

    /// 화면에 보여 줄 활성 알림: 필터에 걸린 것과 숨긴 것을 뺀다. 고정한 알림이 위에 오고, 그 안에서는 최근 순이다.
    pub fn visible_notifications(&self) -> Result<Vec<NotificationRecord>> {
        let filters = self.store.filters()?;
        let mut out = Vec::new();
        for rec in self.store.active_notifications()? {
            if self.filter_of(&rec, &filters).is_none() && !self.store.is_hidden(&rec.nid)? {
                out.push(self.decorate(rec)?);
            }
        }
        // 정렬은 안정적이라 같은 그룹 안에서는 최근 순이 유지된다.
        out.sort_by_key(|r| !r.pinned);
        Ok(out)
    }

    /// 화면에 보이지 않는 활성 알림: 직접 숨긴 것(filter_id 0)과 필터에 걸린 것.
    pub fn hidden_notifications(&self) -> Result<Vec<HiddenNotification>> {
        let filters = self.store.filters()?;
        let mut out = Vec::new();
        for rec in self.store.active_notifications()? {
            let filter_id = if self.store.is_hidden(&rec.nid)? {
                Some(0)
            } else {
                self.filter_of(&rec, &filters)
            };
            if let Some(filter_id) = filter_id {
                out.push(HiddenNotification { notification: self.decorate(rec)?, filter_id });
            }
        }
        Ok(out)
    }

    /// 화면에서 숨기거나 다시 보인다. 연결된 기기도 같이 숨긴다. 원본 OS 알림은 그대로 둔다
    /// (원본에서 지우면 음악 재생 같은 앱이 멈춘다). 원본이 내용을 바꾸면 다시 보인다.
    pub fn set_hidden(&self, nid: &str, hidden: bool, now: i64) -> Result<Vec<NodeEvent>> {
        self.store.transaction(|_| self.hide_in(nid, hidden, now))
    }

    pub fn hide_local(&self, nid: &str, now: i64) -> Result<Vec<NodeEvent>> {
        self.set_hidden(nid, true, now)
    }

    /// 숨김 상태를 바꾸고, 바뀌었으면 모든 상대에게 알린다. 상대에게서 받은 변경도 다시 퍼뜨려
    /// 직접 연결되지 않은 기기(폰을 사이에 둔 PC 두 대)까지 전해진다. 바뀌지 않으면 퍼뜨리지 않아 되돌아와도 멈춘다.
    fn hide_in(&self, nid: &str, hidden: bool, now: i64) -> Result<Vec<NodeEvent>> {
        let store = &self.store;
        if store.is_hidden(nid)? == hidden {
            return Ok(vec![]);
        }
        let rec = store.notification(nid)?;
        let was_visible = match &rec {
            Some(r) => r.state == NotificationState::Active && self.is_visible(r)?,
            None => false,
        };
        store.set_hidden(nid, hidden)?;
        let version = self.tick(0)?;
        self.log(Kind::HideChanged(pb::HideChanged { nid: nid.to_owned(), hidden, version }), None, now)?;
        let Some(rec) = rec.filter(|r| r.state == NotificationState::Active) else { return Ok(vec![]) };
        let visible = self.is_visible(&rec)?;
        Ok(match (was_visible, visible) {
            (true, false) => vec![NodeEvent::NotificationRemoved { nid: rec.nid, state: rec.state }],
            (false, true) => vec![NodeEvent::NotificationChanged { notification: self.decorate(rec)?, alert: false }],
            _ => vec![],
        })
    }

    /// 보이는 알림을 한꺼번에 지운다. app_pkg 가 있으면 그 앱 것만. 고정한 알림은 남긴다.
    /// dismiss 가 false 면 숨기기(원본 OS 알림 유지), true 면 원본까지 지운다.
    pub fn clear(&self, app_pkg: Option<&str>, dismiss: bool, now: i64) -> Result<Vec<NodeEvent>> {
        self.store.transaction(|_| {
            let mut events = Vec::new();
            for rec in self.visible_notifications()? {
                if rec.pinned || app_pkg.is_some_and(|p| p != rec.app_pkg) {
                    continue;
                }
                events.extend(if dismiss { self.dismiss_in(&rec.nid, now)? } else { self.hide_in(&rec.nid, true, now)? });
            }
            Ok(events)
        })
    }

    /// 이 기기 목록 위에 고정한다. 기기마다 따로 정한다.
    pub fn set_pinned(&self, nid: &str, pinned: bool) -> Result<Vec<NodeEvent>> {
        self.store.transaction(|store| {
            store.set_pinned(nid, pinned)?;
            let Some(rec) = store.notification(nid)? else { return Ok(vec![]) };
            if rec.state != NotificationState::Active || !self.is_visible(&rec)? {
                return Ok(vec![]);
            }
            Ok(vec![NodeEvent::NotificationChanged { notification: self.decorate(rec)?, alert: false }])
        })
    }

    pub fn filters(&self) -> Result<Vec<NotificationFilter>> {
        self.store.filters()
    }

    /// 필터 목록이 바뀐 뒤, 보이던 알림 중 새로 걸린 것은 내리고 풀린 것은 조용히 다시 띄운다.
    fn refilter(&self, before: &[NotificationFilter]) -> Result<Vec<NodeEvent>> {
        let after = self.store.filters()?;
        let mut events = Vec::new();
        for rec in self.store.active_notifications()? {
            if self.store.is_hidden(&rec.nid)? {
                continue;
            }
            match (self.filter_of(&rec, before).is_some(), self.filter_of(&rec, &after).is_some()) {
                (false, true) => events.push(NodeEvent::NotificationRemoved { nid: rec.nid, state: rec.state }),
                (true, false) => {
                    events.push(NodeEvent::NotificationChanged { notification: self.decorate(rec)?, alert: false })
                }
                _ => {}
            }
        }
        Ok(events)
    }

    /// 필터를 바꾸고 모든 상대에게 목록 전체를 알린다.
    fn change_filters_in(&self, now: i64, f: impl FnOnce(&Store) -> Result<()>) -> Result<Vec<NodeEvent>> {
        let store = &self.store;
        let before = store.filters()?;
        f(store)?;
        store.set_meta_u64(META_FILTERS_UPDATED, now.max(0) as u64)?;
        let items = store
            .filters()?
            .into_iter()
            .map(|f| pb::FilterItem { kind: f.kind.to_u32(), pattern: f.pattern, label: f.label })
            .collect();
        self.log(Kind::FiltersChanged(pb::FiltersChanged { items, updated_at: now }), None, now)?;
        self.refilter(&before)
    }

    pub fn add_filter(&self, kind: FilterKind, pattern: &str, label: &str, now: i64) -> Result<Vec<NodeEvent>> {
        let pattern = if kind == FilterKind::HiddenContent { HIDDEN_CONTENT_PATTERN } else { pattern.trim() };
        if pattern.is_empty() {
            return Ok(vec![]);
        }
        let label = label.trim();
        self.store
            .transaction(|_| self.change_filters_in(now, |store| store.add_filter(kind, pattern, label).map(|_| ())))
    }

    pub fn remove_filter(&self, id: i64, now: i64) -> Result<Vec<NodeEvent>> {
        self.store.transaction(|_| self.change_filters_in(now, |store| store.remove_filter(id)))
    }

    /// 계정이나 상대에게서 받은 목록으로 바꾼다. 바꾼 시각은 받은 값을 그대로 쓴다.
    pub fn replace_filters(&self, filters: &[FilterSpec], updated_at: i64) -> Result<Vec<NodeEvent>> {
        self.store.transaction(|_| self.replace_filters_in(filters, updated_at))
    }

    fn replace_filters_in(&self, filters: &[FilterSpec], updated_at: i64) -> Result<Vec<NodeEvent>> {
        self.change_filters_in(updated_at, |store| {
            store.clear_filters()?;
            for f in filters {
                store.add_filter(f.kind, &f.pattern, &f.label)?;
            }
            Ok(())
        })
    }

    /// 상대의 필터 목록이 더 최근이면 받아들인다. 받아들이면 다시 퍼뜨린다(replace 가 기록한다).
    fn apply_filters(&self, m: pb::FiltersChanged) -> Result<Vec<NodeEvent>> {
        if m.updated_at <= self.filters_updated_at()? {
            return Ok(vec![]);
        }
        let items: Vec<FilterSpec> = m
            .items
            .into_iter()
            .filter_map(|i| {
                let kind = FilterKind::from_u32(i.kind)?;
                let pattern = i.pattern.trim().to_owned();
                (!pattern.is_empty()).then_some(FilterSpec { kind, pattern, label: i.label })
            })
            .collect();
        let mut events = self.replace_filters_in(&items, m.updated_at)?;
        events.push(NodeEvent::CloudChanged);
        Ok(events)
    }

    pub fn filters_updated_at(&self) -> Result<i64> {
        Ok(self.store.meta_u64(META_FILTERS_UPDATED)? as i64)
    }

    /// 계정에 올릴 필터 목록.
    pub fn filter_specs(&self) -> Result<Vec<FilterSpec>> {
        Ok(filter_specs(&self.store.filters()?))
    }

    /// 앱 규칙을 바꾸고 연결된 기기에 알린다. 규칙은 그 앱의 알림을 캡처하는 기기(대개 폰)가 적용한다.
    pub fn set_app_rule(&self, app_pkg: &str, policy: AppPolicy, now: i64) -> Result<Vec<NodeEvent>> {
        let app_pkg = app_pkg.trim();
        if app_pkg.is_empty() {
            return Ok(vec![]);
        }
        self.store.transaction(|_| self.set_rule_in(app_pkg, policy, now))
    }

    fn set_rule_in(&self, app_pkg: &str, policy: AppPolicy, now: i64) -> Result<Vec<NodeEvent>> {
        let store = &self.store;
        if store.app_rule(app_pkg)? == Some(policy) {
            return Ok(vec![]);
        }
        store.set_app_rule(app_pkg, policy)?;
        self.log(
            Kind::AppRuleChanged(pb::AppRuleChanged { app_pkg: app_pkg.to_owned(), policy: policy.to_u32() }),
            None,
            now,
        )?;
        let mut events = Vec::new();
        // 차단하면 이 기기가 원본인 그 앱의 알림을 다른 기기에서 내린다. OS 알림은 그대로 둔다.
        if policy == AppPolicy::Block {
            for mut rec in store.active_from_origin(&self.device_id)? {
                if rec.app_pkg != app_pkg {
                    continue;
                }
                rec.state = NotificationState::Expired;
                rec.version = self.tick(rec.version)?;
                rec.updated_at = now;
                store.upsert_notification(&rec)?;
                self.log_removed(&rec)?;
                events.push(NodeEvent::NotificationRemoved { nid: rec.nid, state: rec.state });
            }
        }
        events.push(NodeEvent::AppRuleChanged { app_pkg: app_pkg.to_owned() });
        Ok(events)
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

    /// 페어링 완료 처리. 이전 로그는 건너뛰고 현재 활성 알림과 필터 목록만 새 상대에게 보낸다.
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
            let updated_at = self.filters_updated_at()?;
            if updated_at > 0 {
                let items = store
                    .filters()?
                    .into_iter()
                    .map(|f| pb::FilterItem { kind: f.kind.to_u32(), pattern: f.pattern, label: f.label })
                    .collect();
                self.log(Kind::FiltersChanged(pb::FiltersChanged { items, updated_at }), Some(&device_id), now)?;
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
            ongoing: false,
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
        assert!(matches!(&events[0], NodeEvent::NotificationChanged { notification, .. } if notification.body == "본문"));
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

    #[test]
    fn keyword_and_app_filters_hide_mirrored_and_block_origin() {
        let (a, b) = pair();
        b.add_filter(FilterKind::Keyword, "광고", "", NOW).unwrap();
        let mut ad = captured("ad", "오늘만 (광고) 할인");
        ad.title = "쇼핑".into();
        a.capture_posted(ad, NOW).unwrap();
        a.capture_posted(captured("ok", "회의 10시"), NOW).unwrap();
        let events = sync(&a, &b);
        // 걸린 알림은 저장만 하고 띄우지 않는다.
        assert_eq!(events.len(), 1);
        assert_eq!(b.visible_notifications().unwrap().len(), 1);
        assert_eq!(b.store.active_notifications().unwrap().len(), 2);

        // 원본 쪽 앱 필터는 아예 보내지 않는다(앱 표시 이름, 대소문자 무시).
        a.add_filter(FilterKind::App, "example", "", NOW).unwrap();
        assert!(a.capture_posted(captured("k2", "x"), NOW).unwrap().is_empty());
    }

    #[test]
    fn adding_and_removing_filters_hides_and_restores_silently() {
        let (a, b) = pair();
        a.capture_posted(captured("k", "결제 완료"), NOW).unwrap();
        sync(&a, &b);
        let added = b.add_filter(FilterKind::Keyword, "결제", "", NOW).unwrap();
        assert!(matches!(added[..], [NodeEvent::NotificationRemoved { .. }]));
        assert!(b.visible_notifications().unwrap().is_empty());
        let id = b.filters().unwrap()[0].id;
        let removed = b.remove_filter(id, NOW + 1).unwrap();
        assert!(matches!(removed[..], [NodeEvent::NotificationChanged { alert: false, .. }]));
        assert_eq!(b.filters_updated_at().unwrap(), NOW + 1);
    }

    #[test]
    fn hide_local_keeps_origin_untouched_and_update_reveals_again() {
        let (a, b) = pair();
        a.capture_posted(captured("music", "재생 중"), NOW).unwrap();
        sync(&a, &b);
        let nid = make_nid("phone", "music");
        let events = b.hide_local(&nid, NOW).unwrap();
        assert!(matches!(events[..], [NodeEvent::NotificationRemoved { .. }]));
        assert!(b.visible_notifications().unwrap().is_empty());
        // 원본은 목록에서만 같이 숨기고 OS 알림은 지우지 않는다: 원본에서 꺼지면 음악이 멈춘다.
        let events = sync(&b, &a);
        assert!(!events.iter().any(|e| matches!(e, NodeEvent::CancelLocal { .. })));
        assert_eq!(state(&a, &nid), NotificationState::Active);
        assert!(a.visible_notifications().unwrap().is_empty());
        // 내용이 바뀌면 다시 보인다.
        a.capture_posted(captured("music", "다음 곡"), NOW).unwrap();
        sync(&a, &b);
        assert_eq!(b.visible_notifications().unwrap().len(), 1);
    }

    #[test]
    fn backlog_and_stale_notifications_do_not_alert() {
        let (a, b) = pair();
        a.capture_posted(captured("old", "x"), NOW - 5 * 60_000).unwrap();
        a.capture_posted(captured("new", "y"), NOW).unwrap();
        a.capture_posted(captured("queued", "z"), NOW).unwrap();
        let log = a.store.events_for("pc", 0, 10).unwrap();
        let alerts: Vec<bool> = log
            .into_iter()
            .enumerate()
            .flat_map(|(i, ev)| b.apply_event_with("phone", ev, NOW, i == 2).unwrap())
            .map(|e| match e {
                NodeEvent::NotificationChanged { alert, .. } => alert,
                _ => panic!(),
            })
            .collect();
        assert_eq!(alerts, vec![false, true, false]);
    }

    fn third(id: &str) -> Engine {
        let e = engine(id);
        e.add_peer(peer("phone"), NOW).unwrap();
        e
    }

    #[test]
    fn hide_and_unhide_propagate_through_the_phone_to_another_pc() {
        let (phone, pc1) = pair();
        let pc2 = third("pc2");
        phone.add_peer(peer("pc2"), NOW).unwrap();
        phone.capture_posted(captured("k", "x"), NOW).unwrap();
        sync(&phone, &pc1);
        sync(&phone, &pc2);
        let nid = make_nid("phone", "k");

        pc1.hide_local(&nid, NOW).unwrap();
        sync(&pc1, &phone);
        // 폰이 받은 숨김을 다시 퍼뜨려, 직접 연결되지 않은 pc2 도 숨긴다.
        sync(&phone, &pc2);
        assert!(pc2.visible_notifications().unwrap().is_empty());
        assert_eq!(pc2.hidden_notifications().unwrap()[0].filter_id, 0);
        // 되돌아온 같은 변경은 다시 퍼뜨리지 않는다.
        let before = phone.last_seq().unwrap();
        sync(&phone, &pc1);
        sync(&pc1, &phone);
        assert_eq!(phone.last_seq().unwrap(), before);

        let events = pc2.set_hidden(&nid, false, NOW).unwrap();
        assert!(matches!(events[..], [NodeEvent::NotificationChanged { alert: false, .. }]));
        sync(&pc2, &phone);
        sync(&phone, &pc1);
        assert_eq!(pc1.visible_notifications().unwrap().len(), 1);
    }

    #[test]
    fn ongoing_media_is_pinned_silent_and_stays_hidden_across_track_changes() {
        let (a, b) = pair();
        let mut media = captured("yt", "영상 1");
        media.ongoing = true;
        let events = a.capture_posted(media.clone(), NOW).unwrap();
        assert!(matches!(&events[..], [NodeEvent::NotificationChanged { alert: false, notification }] if notification.pinned));
        a.capture_posted(captured("chat", "안녕"), NOW).unwrap();
        let events = sync(&a, &b);
        assert!(events.iter().all(|e| match e {
            NodeEvent::NotificationChanged { notification, alert } => *alert != notification.ongoing,
            _ => true,
        }));
        // 고정 알림이 위로 온다.
        let list = b.visible_notifications().unwrap();
        assert!(list[0].ongoing && list[0].pinned && !list[1].pinned);

        let nid = make_nid("phone", "yt");
        b.hide_local(&nid, NOW).unwrap();
        media.body = "영상 2".into();
        a.capture_posted(media, NOW).unwrap();
        sync(&a, &b);
        assert!(b.hidden_notifications().unwrap().iter().any(|h| h.notification.nid == nid));
    }

    #[test]
    fn clear_hides_everything_except_pinned_or_only_one_app() {
        let (a, b) = pair();
        a.capture_posted(captured("k1", "x"), NOW).unwrap();
        let mut other = captured("k2", "y");
        other.app_pkg = "com.other".into();
        a.capture_posted(other, NOW).unwrap();
        a.capture_posted(captured("k3", "z"), NOW).unwrap();
        sync(&a, &b);
        b.set_pinned(&make_nid("phone", "k3"), true).unwrap();

        b.clear(Some("com.other"), false, NOW).unwrap();
        assert_eq!(b.visible_notifications().unwrap().len(), 2);
        let events = b.clear(None, false, NOW).unwrap();
        assert_eq!(events.len(), 1);
        let left = b.visible_notifications().unwrap();
        assert_eq!(left.len(), 1);
        assert!(left[0].pinned);
        // 숨기기로 지웠으므로 원본 알림은 살아 있다.
        sync(&b, &a);
        assert_eq!(a.store.active_notifications().unwrap().len(), 3);

        // 지우기(dismiss)로 하면 원본에 삭제를 요청한다.
        b.set_pinned(&make_nid("phone", "k3"), false).unwrap();
        b.clear(None, true, NOW).unwrap();
        let events = sync(&b, &a);
        assert!(events.contains(&NodeEvent::CancelLocal { key: "k3".into() }));
    }

    #[test]
    fn filters_sync_between_peers_with_latest_winning() {
        let (a, b) = pair();
        a.capture_posted(captured("k", "광고 문자"), NOW).unwrap();
        sync(&a, &b);
        b.add_filter(FilterKind::Keyword, "광고", "", NOW + 10).unwrap();
        let events = sync(&b, &a);
        assert!(events.contains(&NodeEvent::CloudChanged));
        assert_eq!(a.filters().unwrap().len(), 1);
        assert_eq!(a.filters_updated_at().unwrap(), NOW + 10);
        // 더 오래된 목록은 무시한다.
        let stale = pb::LogEvent {
            seq: 999,
            kind: Some(Kind::FiltersChanged(pb::FiltersChanged { items: vec![], updated_at: NOW })),
        };
        assert!(a.apply_event("pc", stale, NOW).unwrap().is_empty());
        assert_eq!(a.filters().unwrap().len(), 1);
    }

    #[test]
    fn hidden_content_filter_and_app_label() {
        let (a, b) = pair();
        let mut otp = captured("otp", "");
        otp.body_hidden = true;
        a.capture_posted(otp.clone(), NOW).unwrap();
        a.capture_posted(captured("ok", "보임"), NOW).unwrap();
        sync(&a, &b);
        let shown = b.visible_notifications().unwrap();
        assert!(shown.iter().any(|n| n.hidden_reason == HiddenReason::System));

        b.add_filter(FilterKind::HiddenContent, "", "", NOW).unwrap();
        assert_eq!(b.visible_notifications().unwrap().len(), 1);
        let hidden = b.hidden_notifications().unwrap();
        assert_eq!(hidden.len(), 1);
        assert_ne!(hidden[0].filter_id, 0);

        b.add_filter(FilterKind::App, "com.example", "Example", NOW + 1).unwrap();
        let app = b.filters().unwrap().into_iter().find(|f| f.kind == FilterKind::App).unwrap();
        assert_eq!(app.label, "Example");
        // 원본도 필터를 받아, 내용이 숨겨진 알림은 더 보내지 않는다.
        sync(&b, &a);
        otp.key = "otp2".into();
        assert!(a.capture_posted(otp, NOW).unwrap().is_empty());
    }

    #[test]
    fn app_rule_set_on_pc_reaches_phone_and_reveals_body() {
        let (phone, pc) = pair();
        let mut bank = captured("b", "잔액 1000원");
        bank.app_pkg = "com.kbstar.kbbank".into();
        phone.capture_posted(bank.clone(), NOW).unwrap();
        sync(&phone, &pc);
        let rec = pc.visible_notifications().unwrap().remove(0);
        assert_eq!(rec.hidden_reason, HiddenReason::Policy);

        pc.set_app_rule("com.kbstar.kbbank", AppPolicy::Allow, NOW).unwrap();
        let events = sync(&pc, &phone);
        assert!(events.contains(&NodeEvent::AppRuleChanged { app_pkg: "com.kbstar.kbbank".into() }));
        // 폰 앱은 AppRuleChanged 를 받으면 현재 알림을 다시 캡처한다.
        phone.capture_posted(bank, NOW + 1).unwrap();
        sync(&phone, &pc);
        let rec = pc.visible_notifications().unwrap().remove(0);
        assert_eq!((rec.body.as_str(), rec.hidden_reason), ("잔액 1000원", HiddenReason::None));

        // 차단하면 이미 보낸 알림도 내린다.
        pc.set_app_rule("com.kbstar.kbbank", AppPolicy::Block, NOW + 2).unwrap();
        sync(&pc, &phone);
        sync(&phone, &pc);
        assert!(pc.visible_notifications().unwrap().is_empty());
    }
}
