//! iroh 위에서 페어링과 동기화 세션을 돌리는 노드.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::{Connection, ConnectionError, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey, TransportAddr};
use iroh_mdns_address_lookup::MdnsAddressLookup;
use n0_future::StreamExt;
use prost::Message;
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::crypto::StorageCipher;
use crate::engine::Engine;
use crate::error::{CoreError, Result};
use crate::pairing::{self, PairingInvite};
use crate::pb::{self, frame, log_event::Kind, pair_frame};
use crate::store::PeerRow;
use crate::types::*;

pub const ALPN_SYNC: &[u8] = b"noti-bridge/sync/1";
pub const ALPN_PAIR: &[u8] = b"noti-bridge/pair/1";

const MAX_FRAME: usize = 1024 * 1024;
const EVENT_BATCH: usize = 64;
const DIAL_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(120);
const MAINTAIN_INTERVAL: Duration = Duration::from_secs(600);
/// 페어링 직후 상대의 신뢰 저장이 끝나기 전에 받은 거부는 해제로 취급하지 않는다.
const UNPAIR_GRACE_MS: i64 = 60_000;

/// 신뢰 목록에 없는 키의 연결을 끊을 때 쓴다. 받는 쪽은 페어링 해제로 처리한다.
const CLOSE_UNTRUSTED: u32 = 4403;
/// 중복 연결 중 진 쪽을 끊을 때 쓴다.
const CLOSE_DUPLICATE: u32 = 4409;

pub trait NodeListener: Send + Sync {
    fn on_event(&self, event: NodeEvent);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn net_err(e: impl std::fmt::Display) -> CoreError {
    CoreError::Network(e.to_string())
}

async fn write_msg(send: &mut SendStream, msg: &impl Message) -> Result<()> {
    let body = msg.encode_to_vec();
    if body.len() > MAX_FRAME {
        return Err(CoreError::Protocol("프레임이 너무 큽니다".into()));
    }
    send.write_all(&(body.len() as u32).to_be_bytes()).await.map_err(net_err)?;
    send.write_all(&body).await.map_err(net_err)
}

async fn read_msg<M: Message + Default>(recv: &mut RecvStream) -> Result<M> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await.map_err(net_err)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(CoreError::Protocol("프레임이 너무 큽니다".into()));
    }
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body).await.map_err(net_err)?;
    Ok(M::decode(body.as_slice())?)
}

struct ActiveSession {
    id: u64,
    /// id 가 작은 쪽이 건 연결이 중복 시 살아남는다.
    preferred: bool,
    cancel: CancellationToken,
    conn: Connection,
    path: ConnectionPath,
}

struct Invite {
    token: String,
    expires_at: i64,
}

#[derive(Default)]
struct PairingState {
    invite: Option<Invite>,
    confirm: Option<oneshot::Sender<bool>>,
}

struct Inner {
    engine: Mutex<Engine>,
    endpoint: Endpoint,
    listener: Arc<dyn NodeListener>,
    device_id: String,
    device_name: String,
    platform: String,
    sessions: Mutex<HashMap<String, ActiveSession>>,
    dialers: Mutex<HashMap<String, CancellationToken>>,
    pairing: Mutex<PairingState>,
    log_changed: watch::Sender<u64>,
    wake: Notify,
    next_session: AtomicU64,
    shutdown: CancellationToken,
}

#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
}

fn key32(bytes: &[u8], what: &str) -> Result<[u8; 32]> {
    bytes.try_into().map_err(|_| CoreError::InvalidInput(format!("{what} 는 32바이트여야 합니다")))
}

fn relay_hint(addr: &EndpointAddr) -> String {
    addr.relay_urls().next().map(ToString::to_string).unwrap_or_default()
}

fn ip_hints(addr: &EndpointAddr) -> Vec<String> {
    addr.ip_addrs().map(ToString::to_string).collect()
}

impl Node {
    pub async fn start(config: NodeConfig, listener: Arc<dyn NodeListener>) -> Result<Self> {
        let secret = SecretKey::from_bytes(&key32(&config.secret_key, "secret_key")?);
        let storage_key = key32(&config.storage_key, "storage_key")?;
        let device_id = secret.public().to_string();

        std::fs::create_dir_all(&config.data_dir)?;
        let db_path = Path::new(&config.data_dir).join("noti-bridge.db");
        let engine = Engine::open(&db_path, StorageCipher::new(&storage_key), device_id.clone(), config.retention_ms)?;

        let builder = if config.relay_disabled {
            Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled)
        } else {
            let builder = Endpoint::builder(presets::N0);
            match config.relay_url.as_deref().filter(|u| !u.is_empty()) {
                Some(url) => {
                    let url = RelayUrl::from_str(url).map_err(|e| CoreError::InvalidInput(e.to_string()))?;
                    builder.relay_mode(RelayMode::custom([url]))
                }
                None => builder,
            }
        };
        let endpoint = builder
            .secret_key(secret)
            .alpns(vec![ALPN_SYNC.to_vec(), ALPN_PAIR.to_vec()])
            .bind()
            .await
            .map_err(net_err)?;

        if config.mdns_enabled {
            // mDNS 를 못 쓰는 망이어도 릴레이 경로로 동작하므로 실패해도 계속한다.
            let mdns = MdnsAddressLookup::builder().build(endpoint.id());
            match (mdns, endpoint.address_lookup()) {
                (Ok(mdns), Ok(services)) => services.add(mdns),
                _ => tracing::warn!("mDNS 로컬 탐색을 시작하지 못했습니다"),
            }
        }

        let inner = Arc::new(Inner {
            engine: Mutex::new(engine),
            endpoint,
            listener,
            device_id,
            device_name: config.device_name,
            platform: config.platform,
            sessions: Mutex::new(HashMap::new()),
            dialers: Mutex::new(HashMap::new()),
            pairing: Mutex::new(PairingState::default()),
            log_changed: watch::Sender::new(0),
            wake: Notify::new(),
            next_session: AtomicU64::new(1),
            shutdown: CancellationToken::new(),
        });

        tokio::spawn(accept_loop(inner.clone()));
        tokio::spawn(maintain_loop(inner.clone()));
        let peers = inner.engine.lock().unwrap().store.peers()?;
        for peer in peers {
            spawn_dialer(&inner, peer.device_id);
        }
        Ok(Self { inner })
    }

    pub fn device_id(&self) -> String {
        self.inner.device_id.clone()
    }

    /// 주소 정보. 릴레이를 쓰는 경우 연결될 때까지 잠시 기다린다.
    async fn local_addr(&self) -> EndpointAddr {
        let _ = tokio::time::timeout(Duration::from_secs(5), self.inner.endpoint.online()).await;
        self.inner.endpoint.addr()
    }

    pub async fn create_invite(&self) -> Result<PairingInvite> {
        let addr = self.local_addr().await;
        let invite = PairingInvite {
            endpoint_id: self.inner.device_id.clone(),
            relay_url: relay_hint(&addr),
            token: pairing::new_token()?,
            expires_at: now_ms() + pairing::TOKEN_TTL_MS,
            addrs: ip_hints(&addr),
        };
        self.inner.pairing.lock().unwrap().invite =
            Some(Invite { token: invite.token.clone(), expires_at: invite.expires_at });
        Ok(invite)
    }

    /// 초대 URI 로 페어링을 시작한다. 결과는 이벤트로 전달된다.
    pub fn join(&self, uri: &str) -> Result<()> {
        let invite = PairingInvite::parse(uri)?;
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this.join_inner(invite).await {
                this.inner.emit(NodeEvent::PairingFailed { reason: e.to_string() });
            }
        });
        Ok(())
    }

    async fn join_inner(&self, invite: PairingInvite) -> Result<()> {
        let inner = &self.inner;
        let peer_addr = dial_addr(&invite.endpoint_id, &invite.relay_url, &invite.addrs)?;
        let conn = tokio::time::timeout(DIAL_TIMEOUT, inner.endpoint.connect(peer_addr, ALPN_PAIR))
            .await
            .map_err(|_| CoreError::Pairing("상대 기기에 연결하지 못했습니다".into()))?
            .map_err(net_err)?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(net_err)?;
        let addr = self.local_addr().await;
        let request = pb::PairRequest {
            token: invite.token,
            device_id: inner.device_id.clone(),
            device_name: inner.device_name.clone(),
            platform: inner.platform.clone(),
            relay_url: relay_hint(&addr),
            addrs: ip_hints(&addr),
        };
        let frame = pb::PairFrame { kind: Some(pair_frame::Kind::Request(request)) };
        write_msg(&mut send, &frame).await?;
        let reply: pb::PairFrame = read_msg(&mut recv)
            .await
            .map_err(|_| CoreError::Pairing("QR 코드가 만료되었거나 이미 사용되었습니다".into()))?;
        let Some(pair_frame::Kind::Accepted(accepted)) = reply.kind else {
            return Err(CoreError::Pairing("상대가 페어링을 거부했습니다".into()));
        };
        if accepted.device_id != conn.remote_id().to_string() {
            return Err(CoreError::Pairing("상대 기기 신원이 일치하지 않습니다".into()));
        }
        let peer = PeerRow {
            device_id: accepted.device_id,
            name: accepted.device_name,
            platform: accepted.platform,
            paired_at: now_ms(),
            relay_url: accepted.relay_url,
            addrs: accepted.addrs,
            last_seen_seq: 0,
            acked_seq: 0,
        };
        finish_pairing(inner, peer, send, recv, &conn).await
    }

    /// 화면의 6자리 코드를 사용자가 확인한 결과를 넘긴다.
    pub fn confirm_pairing(&self, accepted: bool) {
        if let Some(tx) = self.inner.pairing.lock().unwrap().confirm.take() {
            let _ = tx.send(accepted);
        }
    }

    pub fn capture_posted(&self, c: CapturedNotification) -> Result<()> {
        self.inner.with_engine_logged(|e| e.capture_posted(c, now_ms()))
    }

    pub fn capture_removed(&self, key: &str, reason: RemovalReason) -> Result<()> {
        self.inner.with_engine_logged(|e| e.capture_removed(key, reason, now_ms()))
    }

    pub fn dismiss(&self, nid: &str) -> Result<()> {
        self.inner.with_engine_logged(|e| e.request_dismiss(nid, now_ms()))
    }

    pub fn invoke_action(&self, nid: &str, action_id: &str) -> Result<()> {
        self.inner.with_engine_logged(|e| e.request_action(nid, action_id, now_ms()))
    }

    pub fn send_reply(&self, nid: &str, action_id: &str, text: &str) -> Result<()> {
        self.inner.with_engine_logged(|e| e.request_reply(nid, action_id, text, now_ms()))
    }

    pub fn set_dnd(&self, enabled: bool) -> Result<()> {
        self.inner.with_engine_logged(|e| e.set_dnd(enabled, now_ms()).map(|_| vec![]))
    }

    pub fn dnd(&self) -> Result<bool> {
        self.inner.engine.lock().unwrap().dnd()
    }

    pub fn active_notifications(&self) -> Result<Vec<NotificationRecord>> {
        self.inner.engine.lock().unwrap().store.active_notifications()
    }

    pub fn icon(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        self.inner.engine.lock().unwrap().store.icon(hash)
    }

    pub fn peers(&self) -> Result<Vec<PeerInfo>> {
        let rows = self.inner.engine.lock().unwrap().store.peers()?;
        let sessions = self.inner.sessions.lock().unwrap();
        Ok(rows
            .into_iter()
            .map(|p| PeerInfo {
                path: sessions.get(&p.device_id).map_or(ConnectionPath::Disconnected, |s| s.path),
                device_id: p.device_id,
                name: p.name,
                platform: p.platform,
                paired_at: p.paired_at,
            })
            .collect())
    }

    pub fn app_rules(&self) -> Result<Vec<AppRule>> {
        self.inner.engine.lock().unwrap().store.app_rules()
    }

    pub fn set_app_rule(&self, app_pkg: &str, policy: AppPolicy) -> Result<()> {
        self.inner.engine.lock().unwrap().store.set_app_rule(app_pkg, policy)
    }

    /// 신뢰 목록에서 빼고 데이터를 지운다. 상대는 다음 연결 때 거부 코드로 해제를 알게 된다.
    pub fn unpair(&self, device_id: &str) -> Result<()> {
        remove_peer(&self.inner, device_id)
    }

    /// 네트워크 전환(Wi-Fi↔LTE, 절전 복귀) 시 호출하면 즉시 재연결을 시도한다.
    pub async fn network_changed(&self) {
        self.inner.endpoint.network_change().await;
        self.inner.wake.notify_waiters();
    }

    pub async fn stop(&self) {
        self.inner.shutdown.cancel();
        self.inner.wake.notify_waiters();
        self.inner.endpoint.close().await;
    }
}

impl Inner {
    fn emit(&self, event: NodeEvent) {
        self.listener.on_event(event);
    }

    fn emit_all(&self, events: Vec<NodeEvent>) {
        for event in events {
            self.emit(event);
        }
    }

    fn with_engine_logged(&self, f: impl FnOnce(&Engine) -> Result<Vec<NodeEvent>>) -> Result<()> {
        let events = f(&self.engine.lock().unwrap())?;
        self.notify_log();
        self.emit_all(events);
        Ok(())
    }

    fn notify_log(&self) {
        self.log_changed.send_modify(|v| *v = v.wrapping_add(1));
    }

    fn is_trusted(&self, device_id: &str) -> bool {
        matches!(self.engine.lock().unwrap().store.peer(device_id), Ok(Some(_)))
    }

    fn set_path(&self, device_id: &str, session_id: u64, path: ConnectionPath) {
        let changed = {
            let mut sessions = self.sessions.lock().unwrap();
            match sessions.get_mut(device_id) {
                Some(s) if s.id == session_id && s.path != path => {
                    s.path = path;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.emit(NodeEvent::ConnectionChanged { device_id: device_id.to_owned(), path });
        }
    }
}

fn dial_addr(device_id: &str, relay_url: &str, addrs: &[String]) -> Result<EndpointAddr> {
    let id = EndpointId::from_str(device_id).map_err(|e| CoreError::InvalidInput(e.to_string()))?;
    let mut parts: Vec<TransportAddr> = addrs
        .iter()
        .filter_map(|a| a.parse().ok())
        .map(TransportAddr::Ip)
        .collect();
    if let Ok(url) = RelayUrl::from_str(relay_url) {
        parts.push(TransportAddr::Relay(url));
    }
    Ok(EndpointAddr::from_parts(id, parts))
}

async fn accept_loop(inner: Arc<Inner>) {
    loop {
        let incoming = tokio::select! {
            _ = inner.shutdown.cancelled() => return,
            incoming = inner.endpoint.accept() => incoming,
        };
        let Some(incoming) = incoming else { return };
        let inner = inner.clone();
        tokio::spawn(async move {
            let Ok(conn) = incoming.await else { return };
            let remote = conn.remote_id().to_string();
            if conn.alpn() == ALPN_PAIR {
                if let Err(e) = host_pairing(&inner, conn).await {
                    inner.emit(NodeEvent::PairingFailed { reason: e.to_string() });
                }
            } else if inner.is_trusted(&remote) {
                install_session(&inner, remote, conn, false);
            } else {
                tracing::info!(%remote, "신뢰 목록에 없는 연결 거부");
                conn.close(CLOSE_UNTRUSTED.into(), b"untrusted");
            }
        });
    }
}

async fn maintain_loop(inner: Arc<Inner>) {
    let mut interval = tokio::time::interval(MAINTAIN_INTERVAL);
    loop {
        tokio::select! {
            _ = inner.shutdown.cancelled() => return,
            _ = interval.tick() => {}
        }
        let result = inner.engine.lock().unwrap().maintain(now_ms());
        match result {
            Ok(events) => inner.emit_all(events),
            Err(e) => tracing::warn!("저장소 정리 실패: {e}"),
        }
    }
}

fn spawn_dialer(inner: &Arc<Inner>, device_id: String) {
    let cancel = inner.shutdown.child_token();
    inner.dialers.lock().unwrap().insert(device_id.clone(), cancel.clone());
    tokio::spawn(dial_loop(inner.clone(), device_id, cancel));
}

async fn dial_loop(inner: Arc<Inner>, device_id: String, cancel: CancellationToken) {
    let mut backoff = Duration::from_secs(1);
    // 양쪽이 동시에 걸면 id 가 작은 쪽 연결이 이기므로, 큰 쪽은 조금 늦게 시작한다.
    if inner.device_id > device_id {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    }
    while !cancel.is_cancelled() {
        // wake 를 먼저 구독해야 세션 종료 알림을 놓치지 않는다.
        let notified = inner.wake.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        let connected = inner.sessions.lock().unwrap().contains_key(&device_id);
        if connected {
            backoff = Duration::from_secs(1);
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = notified => {}
            }
            continue;
        }
        let row = inner.engine.lock().unwrap().store.peer(&device_id);
        let Ok(Some(row)) = row else { return };
        let dialed = match dial_addr(&row.device_id, &row.relay_url, &row.addrs) {
            Ok(addr) => tokio::time::timeout(DIAL_TIMEOUT, inner.endpoint.connect(addr, ALPN_SYNC)).await,
            Err(_) => return,
        };
        if let Ok(Ok(conn)) = dialed {
            backoff = Duration::from_secs(1);
            install_session(&inner, device_id.clone(), conn, true);
            continue;
        }
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = notified => backoff = Duration::from_secs(1),
            _ = tokio::time::sleep(backoff) => backoff = (backoff * 2).min(MAX_BACKOFF),
        }
    }
}

/// 연결을 세션으로 올린다. 같은 상대의 연결이 이미 있으면 id 가 작은 쪽이 건 연결만 남긴다.
fn install_session(inner: &Arc<Inner>, device_id: String, conn: Connection, i_dialed: bool) {
    let dialer_is_lower = if i_dialed { inner.device_id < device_id } else { device_id < inner.device_id };
    let id = inner.next_session.fetch_add(1, Ordering::Relaxed);
    let cancel = inner.shutdown.child_token();
    {
        let mut sessions = inner.sessions.lock().unwrap();
        if let Some(active) = sessions.get(&device_id) {
            if active.preferred && !dialer_is_lower {
                conn.close(CLOSE_DUPLICATE.into(), b"duplicate");
                return;
            }
            active.cancel.cancel();
            active.conn.close(CLOSE_DUPLICATE.into(), b"replaced");
        }
        sessions.insert(
            device_id.clone(),
            ActiveSession {
                id,
                preferred: dialer_is_lower,
                cancel: cancel.clone(),
                conn: conn.clone(),
                path: ConnectionPath::Disconnected,
            },
        );
    }
    let inner = inner.clone();
    tokio::spawn(async move {
        let result = run_session(&inner, &device_id, id, &conn, i_dialed, &cancel).await;
        if let Err(e) = result {
            tracing::debug!(%device_id, "세션 종료: {e}");
        }
        let was_current = {
            let mut sessions = inner.sessions.lock().unwrap();
            let current = sessions.get(&device_id).is_some_and(|s| s.id == id);
            if current {
                sessions.remove(&device_id);
            }
            current
        };
        if !was_current {
            return;
        }
        inner.emit(NodeEvent::ConnectionChanged {
            device_id: device_id.clone(),
            path: ConnectionPath::Disconnected,
        });
        let reason = tokio::time::timeout(Duration::from_secs(1), conn.closed()).await;
        if let Ok(ConnectionError::ApplicationClosed(close)) = reason {
            if close.error_code == CLOSE_UNTRUSTED.into() {
                handle_remote_unpair(&inner, &device_id);
            }
        }
        inner.wake.notify_waiters();
    });
}

fn handle_remote_unpair(inner: &Arc<Inner>, device_id: &str) {
    let peer = inner.engine.lock().unwrap().store.peer(device_id);
    if let Ok(Some(peer)) = peer {
        if now_ms() - peer.paired_at > UNPAIR_GRACE_MS {
            if let Err(e) = remove_peer(inner, device_id) {
                tracing::warn!("페어링 해제 처리 실패: {e}");
            }
        }
    }
}

fn remove_peer(inner: &Arc<Inner>, device_id: &str) -> Result<()> {
    let events = inner.engine.lock().unwrap().remove_peer(device_id)?;
    if let Some(cancel) = inner.dialers.lock().unwrap().remove(device_id) {
        cancel.cancel();
    }
    if let Some(session) = inner.sessions.lock().unwrap().remove(device_id) {
        session.cancel.cancel();
        session.conn.close(CLOSE_UNTRUSTED.into(), b"unpaired");
    }
    inner.emit_all(events);
    inner.emit(NodeEvent::PeerUnpaired { device_id: device_id.to_owned() });
    Ok(())
}

fn selected_path(conn: &Connection) -> ConnectionPath {
    let paths = conn.paths();
    match paths.iter().find(|p| p.is_selected()) {
        Some(p) if p.is_ip() => ConnectionPath::Direct,
        Some(_) => ConnectionPath::Relay,
        None => ConnectionPath::Disconnected,
    }
}

async fn watch_paths(inner: Arc<Inner>, device_id: String, session_id: u64, conn: Connection, cancel: CancellationToken) {
    let mut stream = conn.paths_stream();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            item = stream.next() => {
                let Some(_) = item else { return };
                inner.set_path(&device_id, session_id, selected_path(&conn));
            }
        }
    }
}

async fn run_session(
    inner: &Arc<Inner>,
    device_id: &str,
    session_id: u64,
    conn: &Connection,
    i_dialed: bool,
    cancel: &CancellationToken,
) -> Result<()> {
    let (mut send, mut recv) = tokio::select! {
        _ = cancel.cancelled() => return Ok(()),
        streams = async {
            if i_dialed { conn.open_bi().await } else { conn.accept_bi().await }
        } => streams.map_err(net_err)?,
    };
    let last_seen = inner
        .engine
        .lock()
        .unwrap()
        .store
        .peer(device_id)?
        .map_or(0, |p| p.last_seen_seq);
    let hello = frame::Kind::Hello(pb::Hello { device_id: inner.device_id.clone(), last_seen_seq: last_seen });
    write_msg(&mut send, &pb::Frame { kind: Some(hello) }).await?;

    tokio::spawn(watch_paths(inner.clone(), device_id.to_owned(), session_id, conn.clone(), cancel.clone()));
    inner.set_path(device_id, session_id, selected_path(conn));

    let (ctrl_tx, ctrl_rx) = mpsc::channel::<pb::Frame>(256);
    let (cursor_tx, cursor_rx) = oneshot::channel::<u64>();
    let pending_icons = Arc::new(Mutex::new(Vec::<(u64, String)>::new()));
    let writer = write_loop(inner, device_id, send, ctrl_rx, cursor_rx, pending_icons.clone());
    let reader = read_loop(inner, device_id, &mut recv, ctrl_tx, cursor_tx, pending_icons);
    tokio::select! {
        _ = cancel.cancelled() => Ok(()),
        r = writer => r,
        r = reader => r,
    }
}

async fn write_loop(
    inner: &Arc<Inner>,
    device_id: &str,
    mut send: SendStream,
    mut ctrl_rx: mpsc::Receiver<pb::Frame>,
    cursor_rx: oneshot::Receiver<u64>,
    pending_icons: Arc<Mutex<Vec<(u64, String)>>>,
) -> Result<()> {
    let mut cursor = cursor_rx.await.map_err(|_| CoreError::Protocol("Hello 없이 종료".into()))?;
    let mut log_rx = inner.log_changed.subscribe();
    let mut sent_icons = HashSet::new();
    loop {
        let batch = inner.engine.lock().unwrap().store.events_for(device_id, cursor, EVENT_BATCH)?;
        let full = batch.len() == EVENT_BATCH;
        for event in batch {
            if let Some(hash) = icon_hash_of(&event) {
                let icon = {
                    let engine = inner.engine.lock().unwrap();
                    if sent_icons.contains(&hash) || engine.store.peer_has_icon(device_id, &hash)? {
                        None
                    } else {
                        engine.store.icon(&hash)?
                    }
                };
                if let Some(png) = icon {
                    let blob = frame::Kind::Icon(pb::IconBlob { hash: hash.clone(), png });
                    write_msg(&mut send, &pb::Frame { kind: Some(blob) }).await?;
                    pending_icons.lock().unwrap().push((event.seq, hash.clone()));
                    sent_icons.insert(hash);
                }
            }
            cursor = event.seq;
            write_msg(&mut send, &pb::Frame { kind: Some(frame::Kind::Event(event)) }).await?;
        }
        while let Ok(ctrl) = ctrl_rx.try_recv() {
            write_msg(&mut send, &ctrl).await?;
        }
        if full {
            continue;
        }
        tokio::select! {
            changed = log_rx.changed() => { changed.map_err(net_err)?; }
            ctrl = ctrl_rx.recv() => match ctrl {
                Some(ctrl) => write_msg(&mut send, &ctrl).await?,
                None => return Ok(()),
            },
        }
    }
}

fn icon_hash_of(event: &pb::LogEvent) -> Option<String> {
    let n = match &event.kind {
        Some(Kind::Posted(p)) => p.notification.as_ref(),
        Some(Kind::Updated(u)) => u.notification.as_ref(),
        _ => None,
    }?;
    (!n.icon_hash.is_empty()).then(|| n.icon_hash.clone())
}

async fn read_loop(
    inner: &Arc<Inner>,
    device_id: &str,
    recv: &mut RecvStream,
    ctrl_tx: mpsc::Sender<pb::Frame>,
    cursor_tx: oneshot::Sender<u64>,
    pending_icons: Arc<Mutex<Vec<(u64, String)>>>,
) -> Result<()> {
    let mut cursor_tx = Some(cursor_tx);
    loop {
        let frame: pb::Frame = read_msg(recv).await?;
        match frame.kind {
            Some(frame::Kind::Hello(hello)) => {
                let acked = inner.engine.lock().unwrap().store.peer(device_id)?.map_or(0, |p| p.acked_seq);
                if let Some(tx) = cursor_tx.take() {
                    let _ = tx.send(hello.last_seen_seq.max(acked));
                }
            }
            Some(frame::Kind::Event(event)) => {
                let seq = event.seq;
                let events = inner.engine.lock().unwrap().apply_event(device_id, event, now_ms())?;
                inner.notify_log();
                inner.emit_all(events);
                let ack = frame::Kind::Ack(pb::Ack { seq });
                ctrl_tx.send(pb::Frame { kind: Some(ack) }).await.map_err(net_err)?;
            }
            Some(frame::Kind::Ack(ack)) => {
                let engine = inner.engine.lock().unwrap();
                engine.store.set_acked(device_id, ack.seq)?;
                // 전달이 확인된 아이콘만 "보냄"으로 기록해, 끊겨도 다시 보낼 수 있게 한다.
                pending_icons.lock().unwrap().retain(|(seq, hash)| {
                    if *seq <= ack.seq {
                        let _ = engine.store.mark_peer_icon(device_id, hash);
                        false
                    } else {
                        true
                    }
                });
            }
            Some(frame::Kind::Icon(icon)) => {
                inner.engine.lock().unwrap().store.put_icon(&icon.hash, &icon.png)?;
            }
            None => {}
        }
    }
}

async fn host_pairing(inner: &Arc<Inner>, conn: Connection) -> Result<()> {
    let (mut send, mut recv) = conn.accept_bi().await.map_err(net_err)?;
    let frame: pb::PairFrame = read_msg(&mut recv).await?;
    let Some(pair_frame::Kind::Request(request)) = frame.kind else {
        return Err(CoreError::Pairing("잘못된 페어링 요청".into()));
    };
    let valid = {
        let mut state = inner.pairing.lock().unwrap();
        let ok = state
            .invite
            .as_ref()
            .is_some_and(|i| i.token == request.token && now_ms() <= i.expires_at);
        if ok {
            state.invite = None;
        }
        ok
    };
    if !valid || request.device_id != conn.remote_id().to_string() {
        conn.close(CLOSE_UNTRUSTED.into(), b"invalid token");
        return Err(CoreError::Pairing("유효하지 않은 페어링 토큰".into()));
    }
    let addr = inner.endpoint.addr();
    let accepted = pb::PairAccepted {
        device_id: inner.device_id.clone(),
        device_name: inner.device_name.clone(),
        platform: inner.platform.clone(),
        relay_url: relay_hint(&addr),
        addrs: ip_hints(&addr),
    };
    let frame = pb::PairFrame { kind: Some(pair_frame::Kind::Accepted(accepted)) };
    write_msg(&mut send, &frame).await?;
    let peer = PeerRow {
        device_id: request.device_id,
        name: request.device_name,
        platform: request.platform,
        paired_at: now_ms(),
        relay_url: request.relay_url,
        addrs: request.addrs,
        last_seen_seq: 0,
        acked_seq: 0,
    };
    finish_pairing(inner, peer, send, recv, &conn).await
}

/// 양쪽 사용자가 6자리 코드를 확인하면 서로를 신뢰 목록에 넣는다.
async fn finish_pairing(
    inner: &Arc<Inner>,
    peer: PeerRow,
    mut send: SendStream,
    mut recv: RecvStream,
    conn: &Connection,
) -> Result<()> {
    let (tx, rx) = oneshot::channel();
    inner.pairing.lock().unwrap().confirm = Some(tx);
    inner.emit(NodeEvent::PairingCode {
        device_id: peer.device_id.clone(),
        device_name: peer.name.clone(),
        code: pairing::confirmation_code(&inner.device_id, &peer.device_id),
    });
    let mine = tokio::time::timeout(CONFIRM_TIMEOUT, rx).await.ok().and_then(|r| r.ok()).unwrap_or(false);
    let frame = pb::PairFrame { kind: Some(pair_frame::Kind::Confirm(pb::PairConfirm { accepted: mine })) };
    write_msg(&mut send, &frame).await?;
    let theirs = if mine {
        let frame = tokio::time::timeout(CONFIRM_TIMEOUT, read_msg::<pb::PairFrame>(&mut recv)).await;
        matches!(
            frame,
            Ok(Ok(pb::PairFrame { kind: Some(pair_frame::Kind::Confirm(pb::PairConfirm { accepted: true })) }))
        )
    } else {
        false
    };
    let _ = send.finish();
    let _ = tokio::time::timeout(Duration::from_secs(3), recv.read_to_end(1024)).await;
    conn.close(0u32.into(), b"done");

    let accepted = mine && theirs;
    let (device_id, device_name) = (peer.device_id.clone(), peer.name.clone());
    if accepted {
        inner.engine.lock().unwrap().add_peer(peer, now_ms())?;
        inner.notify_log();
        spawn_dialer(inner, device_id.clone());
    }
    inner.emit(NodeEvent::PairingFinished { device_id, device_name, accepted });
    Ok(())
}
