//! SQLite 로컬 저장소. 알림 내용과 이벤트 페이로드는 암호화해서 넣는다.

use std::path::Path;

use prost::Message;
use rusqlite::{Connection, OptionalExtension, params};

use crate::crypto::StorageCipher;
use crate::error::Result;
use crate::pb;
use crate::types::{AppPolicy, AppRule, NotificationRecord, NotificationState};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS peers (
  device_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  platform TEXT NOT NULL,
  paired_at INTEGER NOT NULL,
  relay_url TEXT NOT NULL DEFAULT '',
  addrs TEXT NOT NULL DEFAULT '',
  last_seen_seq INTEGER NOT NULL DEFAULT 0,
  acked_seq INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS notifications (
  nid TEXT PRIMARY KEY,
  origin TEXT NOT NULL,
  state INTEGER NOT NULL,
  version INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  payload BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_notifications_updated ON notifications(updated_at);
CREATE TABLE IF NOT EXISTS event_log (
  seq INTEGER PRIMARY KEY,
  created_at INTEGER NOT NULL,
  target TEXT,
  payload BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS icons (hash TEXT PRIMARY KEY, png BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS peer_icons (
  device_id TEXT NOT NULL,
  hash TEXT NOT NULL,
  PRIMARY KEY (device_id, hash)
);
CREATE TABLE IF NOT EXISTS app_rules (app_pkg TEXT PRIMARY KEY, policy INTEGER NOT NULL);
";

/// 사용자가 따로 정하지 않아도 본문을 숨기는 은행·인증 앱.
const SENSITIVE_PREFIXES: &[&str] = &[
    "com.kbstar.",
    "com.shinhan.",
    "com.wooribank.",
    "com.kakaobank.",
    "com.hanabank.",
    "com.ibk.",
    "nh.smart",
    "viva.republica.toss",
    "com.google.android.apps.authenticator",
    "com.azure.authenticator",
    "com.authy.",
    "com.lastpass.",
    "com.x8bit.bitwarden",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerRow {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub paired_at: i64,
    pub relay_url: String,
    pub addrs: Vec<String>,
    pub last_seen_seq: u64,
    pub acked_seq: u64,
}

pub struct Store {
    conn: Connection,
    cipher: StorageCipher,
}

fn policy_to_i64(p: AppPolicy) -> i64 {
    match p {
        AppPolicy::Allow => 0,
        AppPolicy::Block => 1,
        AppPolicy::HideBody => 2,
    }
}

fn policy_from_i64(v: i64) -> AppPolicy {
    match v {
        1 => AppPolicy::Block,
        2 => AppPolicy::HideBody,
        _ => AppPolicy::Allow,
    }
}

fn state_to_i64(s: NotificationState) -> i64 {
    match s {
        NotificationState::Active => 0,
        NotificationState::Dismissed => 1,
        NotificationState::Expired => 2,
    }
}

impl Store {
    pub fn open(path: &Path, cipher: StorageCipher) -> Result<Self> {
        Self::init(Connection::open(path)?, cipher)
    }

    pub fn open_in_memory(cipher: StorageCipher) -> Result<Self> {
        Self::init(Connection::open_in_memory()?, cipher)
    }

    fn init(conn: Connection, cipher: StorageCipher) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn, cipher })
    }

    /// 클로저 전체를 한 트랜잭션으로 실행한다.
    pub fn transaction<T>(&self, f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match f(self) {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                self.conn.execute_batch("ROLLBACK")?;
                Err(e)
            }
        }
    }

    pub fn meta_u64(&self, key: &str) -> Result<u64> {
        let value: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?;
        Ok(value.and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    pub fn set_meta_u64(&self, key: &str, value: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value.to_string()],
        )?;
        Ok(())
    }

    pub fn upsert_notification(&self, rec: &NotificationRecord) -> Result<()> {
        let payload = self.cipher.seal(&pb::Notification::from(rec).encode_to_vec())?;
        self.conn.execute(
            "INSERT INTO notifications(nid, origin, state, version, updated_at, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(nid) DO UPDATE SET state = excluded.state, version = excluded.version,
               updated_at = excluded.updated_at, payload = excluded.payload",
            params![
                rec.nid,
                rec.origin,
                state_to_i64(rec.state),
                rec.version as i64,
                rec.updated_at,
                payload
            ],
        )?;
        Ok(())
    }

    fn decode_notification(&self, payload: &[u8]) -> Result<NotificationRecord> {
        let plain = self.cipher.open(payload)?;
        Ok(pb::Notification::decode(plain.as_slice())?.into())
    }

    pub fn notification(&self, nid: &str) -> Result<Option<NotificationRecord>> {
        let payload: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT payload FROM notifications WHERE nid = ?1", [nid], |r| r.get(0))
            .optional()?;
        payload.map(|p| self.decode_notification(&p)).transpose()
    }

    fn notifications_where(&self, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<NotificationRecord>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(args, |r| r.get::<_, Vec<u8>>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(self.decode_notification(&row?)?);
        }
        Ok(out)
    }

    pub fn active_notifications(&self) -> Result<Vec<NotificationRecord>> {
        self.notifications_where(
            "SELECT payload FROM notifications WHERE state = 0 ORDER BY updated_at DESC",
            &[],
        )
    }

    pub fn active_from_origin(&self, origin: &str) -> Result<Vec<NotificationRecord>> {
        self.notifications_where(
            "SELECT payload FROM notifications WHERE state = 0 AND origin = ?1 ORDER BY updated_at",
            &[&origin],
        )
    }

    pub fn delete_from_origin(&self, origin: &str) -> Result<()> {
        self.conn.execute("DELETE FROM notifications WHERE origin = ?1", [origin])?;
        Ok(())
    }

    /// 보존 기간이 지난 행을 지우고, 활성 상태였던 알림의 nid 를 돌려준다.
    pub fn purge_notifications(&self, cutoff: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT nid FROM notifications WHERE state = 0 AND updated_at < ?1")?;
        let expired = stmt
            .query_map([cutoff], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        self.conn.execute("DELETE FROM notifications WHERE updated_at < ?1", [cutoff])?;
        Ok(expired)
    }

    pub fn append_event(&self, seq: u64, created_at: i64, target: Option<&str>, event: &pb::LogEvent) -> Result<()> {
        let payload = self.cipher.seal(&event.encode_to_vec())?;
        self.conn.execute(
            "INSERT INTO event_log(seq, created_at, target, payload) VALUES (?1, ?2, ?3, ?4)",
            params![seq as i64, created_at, target, payload],
        )?;
        Ok(())
    }

    pub fn events_for(&self, device_id: &str, after: u64, limit: usize) -> Result<Vec<pb::LogEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT payload FROM event_log WHERE seq > ?1 AND (target IS NULL OR target = ?2)
             ORDER BY seq LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![after as i64, device_id, limit as i64], |r| r.get::<_, Vec<u8>>(0))?;
        let mut out = Vec::new();
        for row in rows {
            let plain = self.cipher.open(&row?)?;
            out.push(pb::LogEvent::decode(plain.as_slice())?);
        }
        Ok(out)
    }

    pub fn event_count(&self) -> Result<u64> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM event_log", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    /// 모든 상대가 받은 이벤트와 보존 기간이 지난 이벤트를 지운다.
    pub fn compact_events(&self, cutoff: i64) -> Result<()> {
        self.conn.execute("DELETE FROM event_log WHERE created_at < ?1", [cutoff])?;
        let min_acked: Option<i64> = self.conn.query_row("SELECT MIN(acked_seq) FROM peers", [], |r| r.get(0))?;
        match min_acked {
            Some(seq) => self.conn.execute("DELETE FROM event_log WHERE seq <= ?1", [seq])?,
            None => self.conn.execute("DELETE FROM event_log", [])?,
        };
        Ok(())
    }

    pub fn put_peer(&self, peer: &PeerRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO peers(device_id, name, platform, paired_at, relay_url, addrs, last_seen_seq, acked_seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(device_id) DO UPDATE SET name = excluded.name, platform = excluded.platform,
               paired_at = excluded.paired_at, relay_url = excluded.relay_url, addrs = excluded.addrs,
               last_seen_seq = excluded.last_seen_seq, acked_seq = excluded.acked_seq",
            params![
                peer.device_id,
                peer.name,
                peer.platform,
                peer.paired_at,
                peer.relay_url,
                peer.addrs.join(","),
                peer.last_seen_seq as i64,
                peer.acked_seq as i64
            ],
        )?;
        Ok(())
    }

    fn peer_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PeerRow> {
        let addrs: String = r.get(5)?;
        Ok(PeerRow {
            device_id: r.get(0)?,
            name: r.get(1)?,
            platform: r.get(2)?,
            paired_at: r.get(3)?,
            relay_url: r.get(4)?,
            addrs: addrs.split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect(),
            last_seen_seq: r.get::<_, i64>(6)? as u64,
            acked_seq: r.get::<_, i64>(7)? as u64,
        })
    }

    pub fn peer(&self, device_id: &str) -> Result<Option<PeerRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT device_id, name, platform, paired_at, relay_url, addrs, last_seen_seq, acked_seq
                 FROM peers WHERE device_id = ?1",
                [device_id],
                Self::peer_from_row,
            )
            .optional()?)
    }

    pub fn peers(&self) -> Result<Vec<PeerRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT device_id, name, platform, paired_at, relay_url, addrs, last_seen_seq, acked_seq
             FROM peers ORDER BY paired_at",
        )?;
        let rows = stmt.query_map([], Self::peer_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn delete_peer(&self, device_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM peers WHERE device_id = ?1", [device_id])?;
        self.conn.execute("DELETE FROM peer_icons WHERE device_id = ?1", [device_id])?;
        self.conn.execute("DELETE FROM event_log WHERE target = ?1", [device_id])?;
        Ok(())
    }

    pub fn set_last_seen(&self, device_id: &str, seq: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE peers SET last_seen_seq = ?2 WHERE device_id = ?1",
            params![device_id, seq as i64],
        )?;
        Ok(())
    }

    pub fn set_acked(&self, device_id: &str, seq: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE peers SET acked_seq = MAX(acked_seq, ?2) WHERE device_id = ?1",
            params![device_id, seq as i64],
        )?;
        Ok(())
    }

    pub fn put_icon(&self, hash: &str, png: &[u8]) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO icons(hash, png) VALUES (?1, ?2)",
            params![hash, png],
        )?;
        Ok(())
    }

    pub fn icon(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row("SELECT png FROM icons WHERE hash = ?1", [hash], |r| r.get(0))
            .optional()?)
    }

    pub fn peer_has_icon(&self, device_id: &str, hash: &str) -> Result<bool> {
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM peer_icons WHERE device_id = ?1 AND hash = ?2",
                [device_id, hash],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    pub fn mark_peer_icon(&self, device_id: &str, hash: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO peer_icons(device_id, hash) VALUES (?1, ?2)",
            [device_id, hash],
        )?;
        Ok(())
    }

    pub fn app_policy(&self, app_pkg: &str) -> Result<AppPolicy> {
        let rule: Option<i64> = self
            .conn
            .query_row("SELECT policy FROM app_rules WHERE app_pkg = ?1", [app_pkg], |r| r.get(0))
            .optional()?;
        Ok(match rule {
            Some(v) => policy_from_i64(v),
            None if SENSITIVE_PREFIXES.iter().any(|p| app_pkg.starts_with(p)) => AppPolicy::HideBody,
            None => AppPolicy::Allow,
        })
    }

    pub fn set_app_rule(&self, app_pkg: &str, policy: AppPolicy) -> Result<()> {
        self.conn.execute(
            "INSERT INTO app_rules(app_pkg, policy) VALUES (?1, ?2)
             ON CONFLICT(app_pkg) DO UPDATE SET policy = excluded.policy",
            params![app_pkg, policy_to_i64(policy)],
        )?;
        Ok(())
    }

    pub fn app_rules(&self) -> Result<Vec<AppRule>> {
        let mut stmt = self.conn.prepare("SELECT app_pkg, policy FROM app_rules ORDER BY app_pkg")?;
        let rows = stmt.query_map([], |r| {
            Ok(AppRule { app_pkg: r.get(0)?, policy: policy_from_i64(r.get(1)?) })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
