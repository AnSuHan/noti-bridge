import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { StrictMode, useCallback, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "./style.css";

type Action = { id: string; title: string; accepts_reply: boolean };
type Notification = {
  nid: string;
  origin: string;
  app_name: string;
  icon_hash: string;
  title: string;
  body: string;
  body_hidden: boolean;
  updated_at: number;
  actions: Action[];
};
type Peer = { device_id: string; name: string; platform: string; path: "direct" | "relay" | "disconnected" };
type Policy = "allow" | "block" | "hide_body";
type Rule = { app_pkg: string; policy: Policy };
type State = {
  device_id: string;
  platform: string;
  can_capture: boolean;
  dnd: boolean;
  relay_url: string;
  notifications: Notification[];
  peers: Peer[];
  rules: Rule[];
};
type Invite = { uri: string; qr_svg: string; expires_at: number };
type UiEvent =
  | { type: "changed" }
  | { type: "pairing_code"; device_name: string; code: string }
  | { type: "pairing_finished"; device_name: string; accepted: boolean }
  | { type: "pairing_failed"; reason: string };

const PATH_LABEL = { direct: "직접 연결", relay: "중계 연결", disconnected: "연결 끊김" };
const POLICIES: [Policy, string][] = [
  ["allow", "허용"],
  ["hide_body", "본문 숨김"],
  ["block", "차단"],
];

function Icon({ hash }: { hash: string }) {
  const [src, setSrc] = useState<string>();
  useEffect(() => {
    if (!hash) return;
    invoke<number[] | null>("icon", { hash }).then((bytes) => {
      if (bytes) setSrc(`data:image/png;base64,${btoa(String.fromCharCode(...bytes))}`);
    });
  }, [hash]);
  return src ? <img className="icon" src={src} alt="" /> : <div className="icon" />;
}

function NotificationCard({ n, myId }: { n: Notification; myId: string }) {
  const [replyTo, setReplyTo] = useState<Action>();
  const [text, setText] = useState("");
  const send = () => {
    if (!replyTo || !text.trim()) return;
    invoke("send_reply", { nid: n.nid, actionId: replyTo.id, text });
    setReplyTo(undefined);
    setText("");
  };
  return (
    <article className="card">
      <header>
        <Icon hash={n.icon_hash} />
        <span className="app">{n.app_name}</span>
        <time>{new Date(n.updated_at).toLocaleTimeString("ko-KR", { hour: "2-digit", minute: "2-digit" })}</time>
      </header>
      <h3>{n.title}</h3>
      <p className={n.body_hidden ? "muted" : ""}>{n.body_hidden ? "내용 숨겨짐" : n.body}</p>
      <footer>
        {n.origin !== myId &&
          n.actions.map((a) => (
            <button
              key={a.id}
              onClick={() => (a.accepts_reply ? setReplyTo(a) : invoke("invoke_action", { nid: n.nid, actionId: a.id }))}
            >
              {a.title}
            </button>
          ))}
        <button className="quiet" onClick={() => invoke("dismiss", { nid: n.nid })}>
          지우기
        </button>
      </footer>
      {replyTo && (
        <div className="row">
          <input
            autoFocus
            value={text}
            placeholder="답장 입력"
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && send()}
          />
          <button onClick={send}>보내기</button>
        </div>
      )}
    </article>
  );
}

function Devices({ state }: { state: State }) {
  const [invite, setInvite] = useState<Invite>();
  const [error, setError] = useState<string>();
  const add = () => {
    setError(undefined);
    invoke<Invite>("create_invite").then(setInvite).catch(setError);
  };
  useEffect(() => {
    if (!invite) return;
    const timer = setTimeout(() => setInvite(undefined), Math.max(invite.expires_at - Date.now(), 0));
    return () => clearTimeout(timer);
  }, [invite]);
  // 페어링이 끝나면 이미 쓴 QR 을 치운다.
  useEffect(() => setInvite(undefined), [state.peers.length]);
  return (
    <section>
      {state.peers.map((p) => (
        <article className="card row" key={p.device_id}>
          <span className={`dot ${p.path}`} />
          <div className="grow">
            <h3>{p.name}</h3>
            <p className="muted">
              {p.platform} · {PATH_LABEL[p.path]}
            </p>
          </div>
          <button className="quiet" onClick={() => invoke("unpair", { deviceId: p.device_id })}>
            해제
          </button>
        </article>
      ))}
      {state.peers.length === 0 && <p className="muted">아직 연결된 기기가 없습니다.</p>}
      <button onClick={add}>기기 추가</button>
      {error && <p className="error">{error}</p>}
      {invite && (
        <div className="qr">
          <div dangerouslySetInnerHTML={{ __html: invite.qr_svg }} />
          <p className="muted">폰의 noti-bridge 앱에서 5분 안에 스캔하세요.</p>
        </div>
      )}
    </section>
  );
}

function Settings({ state }: { state: State }) {
  const [relay, setRelay] = useState(state.relay_url);
  const [pkg, setPkg] = useState("");
  return (
    <section>
      <h2>방해 금지</h2>
      <label className="row">
        <input
          type="checkbox"
          checked={state.dnd}
          onChange={(e) => invoke("set_dnd", { enabled: e.target.checked })}
        />
        폰과 방해 금지 상태 맞추기
      </label>
      <h2>릴레이</h2>
      <div className="row">
        <input
          className="grow"
          value={relay}
          placeholder="비우면 iroh 공개 릴레이"
          onChange={(e) => setRelay(e.target.value)}
        />
        <button onClick={() => invoke("set_relay", { relayUrl: relay })}>저장 후 재시작</button>
      </div>
      {state.can_capture && (
        <>
          <h2>이 PC 앱별 미러링</h2>
          <div className="row">
            <input className="grow" value={pkg} placeholder="앱 ID" onChange={(e) => setPkg(e.target.value)} />
            <button
              onClick={() => {
                if (pkg.trim()) invoke("set_app_rule", { appPkg: pkg.trim(), policy: "block" });
                setPkg("");
              }}
            >
              차단 추가
            </button>
          </div>
          {state.rules.map((r) => (
            <div className="row" key={r.app_pkg}>
              <span className="grow">{r.app_pkg}</span>
              <select
                value={r.policy}
                onChange={(e) => invoke("set_app_rule", { appPkg: r.app_pkg, policy: e.target.value })}
              >
                {POLICIES.map(([value, label]) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </select>
            </div>
          ))}
        </>
      )}
      {!state.can_capture && <p className="muted">이 기기는 알림 수신 전용입니다.</p>}
      <p className="muted small">기기 ID {state.device_id.slice(0, 16)}…</p>
    </section>
  );
}

const TABS = ["알림", "기기", "설정"] as const;

function App() {
  const [state, setState] = useState<State>();
  const [tab, setTab] = useState<(typeof TABS)[number]>("알림");
  const [code, setCode] = useState<{ device_name: string; code: string }>();
  const [message, setMessage] = useState<string>();
  const refresh = useCallback(() => {
    invoke<State>("get_state").then(setState).catch(setMessage);
  }, []);

  useEffect(() => {
    refresh();
    const unlisten = listen<UiEvent>("node-changed", ({ payload }) => {
      if (payload.type === "pairing_code") setCode(payload);
      if (payload.type === "pairing_finished") {
        setCode(undefined);
        setMessage(payload.accepted ? `${payload.device_name} 연결 완료` : "페어링이 취소되었습니다.");
      }
      if (payload.type === "pairing_failed") {
        setCode(undefined);
        setMessage(payload.reason);
      }
      refresh();
    });
    return () => void unlisten.then((stop) => stop());
  }, [refresh]);

  if (!state) return <main className="muted">불러오는 중…</main>;
  return (
    <main>
      <nav>
        {TABS.map((t) => (
          <button key={t} className={t === tab ? "active" : "quiet"} onClick={() => setTab(t)}>
            {t}
          </button>
        ))}
      </nav>
      {tab === "알림" && (
        <section>
          {state.notifications.length === 0 && <p className="muted">표시할 알림이 없습니다.</p>}
          {state.notifications.map((n) => (
            <NotificationCard key={n.nid} n={n} myId={state.device_id} />
          ))}
        </section>
      )}
      {tab === "기기" && <Devices state={state} />}
      {tab === "설정" && <Settings state={state} key={state.relay_url} />}
      {code && (
        <div className="modal">
          <div className="sheet">
            <h2>확인 코드</h2>
            <p>{code.device_name} 화면의 코드와 같은지 확인하세요.</p>
            <p className="code">{code.code}</p>
            <div className="row">
              <button onClick={() => invoke("confirm_pairing", { accepted: true })}>일치</button>
              <button className="quiet" onClick={() => invoke("confirm_pairing", { accepted: false })}>
                다름
              </button>
            </div>
          </div>
        </div>
      )}
      {message && (
        <div className="modal">
          <div className="sheet">
            <p>{message}</p>
            <button onClick={() => setMessage(undefined)}>확인</button>
          </div>
        </div>
      )}
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
