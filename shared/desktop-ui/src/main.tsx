import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { StrictMode, createContext, useCallback, useContext, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { type Lang, STRINGS, type Strings } from "./i18n";
import "./style.css";

type Action = { id: string; title: string; accepts_reply: boolean };
type Notification = {
  nid: string;
  origin: string;
  app_pkg: string;
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
type FilterKind = "app" | "keyword";
type Filter = { id: number; kind: FilterKind; pattern: string };
type Cloud = { configured: boolean; signed_in: boolean; email: string; project_id: string; last_sync: number };
type State = {
  version: string;
  language: Lang;
  language_setting: Lang | null;
  toast_close_dismisses: boolean;
  filters: Filter[];
  cloud: Cloud;
  device_id: string;
  platform: string;
  can_capture: boolean;
  capture_state: "unsupported" | "pending" | "allowed" | "denied";
  dnd: boolean;
  relay_url: string;
  notifications: Notification[];
  peers: Peer[];
  rules: Rule[];
  pairing: { device_name: string; code: string } | null;
};
type CloudDevice = {
  device_id: string;
  name: string;
  platform: string;
  version: string;
  updated_at: number;
  is_self: boolean;
  paired: boolean;
};
type ReleaseNote = { version: string; date: string; items: string[] };
type Invite = { uri: string; qr_svg: string; expires_at: number };
type UiEvent =
  | { type: "changed" }
  | { type: "pairing_code"; device_name: string; code: string }
  | { type: "pairing_finished"; device_name: string; accepted: boolean }
  | { type: "pairing_failed"; reason: string };

const I18n = createContext<{ t: Strings; lang: Lang }>({ t: STRINGS.ko, lang: "ko" });
const useT = () => useContext(I18n);

const locale = (lang: Lang) => (lang === "ko" ? "ko-KR" : "en-US");

// 같은 앱의 알림은 아이콘이 같으므로 해시마다 한 번만 받아 Blob URL 하나를 같이 쓴다.
// 창을 닫으면 WebView 째로 사라지므로 따로 해제하지 않는다.
const iconUrls = new Map<string, Promise<string | undefined>>();

function iconUrl(hash: string) {
  let url = iconUrls.get(hash);
  if (!url) {
    url = invoke<ArrayBuffer>("icon", { hash })
      .then((buf) => (buf.byteLength ? URL.createObjectURL(new Blob([buf], { type: "image/png" })) : undefined))
      .catch(() => undefined);
    iconUrls.set(hash, url);
  }
  return url;
}

function Icon({ hash }: { hash: string }) {
  const [src, setSrc] = useState<string>();
  useEffect(() => {
    if (!hash) return;
    let live = true;
    iconUrl(hash).then((url) => live && setSrc(url));
    return () => {
      live = false;
    };
  }, [hash]);
  return src ? <img className="icon" src={src} alt="" /> : <div className="icon" />;
}

function NotificationCard({ n, myId }: { n: Notification; myId: string }) {
  const { t, lang } = useT();
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
        <time>{new Date(n.updated_at).toLocaleTimeString(locale(lang), { hour: "2-digit", minute: "2-digit" })}</time>
      </header>
      <h3>{n.title}</h3>
      <p className={n.body_hidden ? "muted" : ""}>{n.body_hidden ? t.bodyHidden : n.body}</p>
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
        <span className="grow" />
        <button className="quiet" title={t.filterAppTitle} onClick={() => invoke("add_filter", { kind: "app", pattern: n.app_pkg })}>
          {t.filterApp}
        </button>
        <button className="quiet" title={t.hideTitle} onClick={() => invoke("hide_local", { nid: n.nid })}>
          {t.hide}
        </button>
        <button className="quiet" title={t.dismissTitle} onClick={() => invoke("dismiss", { nid: n.nid })}>
          {t.dismiss}
        </button>
      </footer>
      {replyTo && (
        <div className="row">
          <input
            autoFocus
            value={text}
            placeholder={t.replyPlaceholder}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && send()}
          />
          <button onClick={send}>{t.send}</button>
        </div>
      )}
    </article>
  );
}

function AccountDevices({ state }: { state: State }) {
  const { t } = useT();
  const [devices, setDevices] = useState<CloudDevice[]>([]);
  const [error, setError] = useState<string>();
  useEffect(() => {
    if (!state.cloud.signed_in) return;
    invoke<CloudDevice[]>("cloud_devices").then(setDevices).catch(setError);
  }, [state.cloud.signed_in, state.cloud.last_sync, state.peers.length]);
  if (!state.cloud.signed_in) return null;
  return (
    <>
      <h2>{t.accountDevices}</h2>
      {error && <p className="error">{error}</p>}
      {devices.map((d) => (
        <article className="card row" key={d.device_id}>
          <div className="grow">
            <h3>
              {d.name} {d.is_self && <span className="badge">{t.thisDevice}</span>}
            </h3>
            <p className="muted">
              {d.platform} · v{d.version}
              {!d.is_self && ` · ${d.paired ? t.paired : t.notPaired}`}
            </p>
          </div>
        </article>
      ))}
    </>
  );
}

function Devices({ state }: { state: State }) {
  const { t } = useT();
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
  const pathLabel = { direct: t.pathDirect, relay: t.pathRelay, disconnected: t.pathDisconnected };
  return (
    <section>
      {state.peers.map((p) => (
        <article className="card row" key={p.device_id}>
          <span className={`dot ${p.path}`} />
          <div className="grow">
            <h3>{p.name}</h3>
            <p className="muted">
              {p.platform} · {pathLabel[p.path]}
            </p>
          </div>
          <button className="quiet" onClick={() => invoke("unpair", { deviceId: p.device_id })}>
            {t.unpair}
          </button>
        </article>
      ))}
      {state.peers.length === 0 && <p className="muted">{t.noPeers}</p>}
      <button onClick={add}>{t.addDevice}</button>
      {error && <p className="error">{error}</p>}
      {invite && (
        <div className="qr">
          <div dangerouslySetInnerHTML={{ __html: invite.qr_svg }} />
          <p className="muted">{t.scanHint}</p>
        </div>
      )}
      <AccountDevices state={state} />
    </section>
  );
}

function Filters({ state }: { state: State }) {
  const { t } = useT();
  const [kind, setKind] = useState<FilterKind>("keyword");
  const [pattern, setPattern] = useState("");
  // 지금 목록에 있는 앱을 고를 수 있게 한다. 직접 입력도 된다.
  const apps = [...new Set(state.notifications.map((n) => n.app_name).filter(Boolean))];
  const add = () => {
    if (!pattern.trim()) return;
    invoke("add_filter", { kind, pattern: pattern.trim() });
    setPattern("");
  };
  return (
    <>
      <h2>{t.filters}</h2>
      <p className="muted small">{t.filtersHint}</p>
      <div className="row">
        <select value={kind} onChange={(e) => setKind(e.target.value as FilterKind)}>
          <option value="keyword">{t.filterKindKeyword}</option>
          <option value="app">{t.filterKindApp}</option>
        </select>
        <input
          className="grow"
          list={kind === "app" ? "filter-apps" : undefined}
          value={pattern}
          placeholder={kind === "app" ? t.filterAppPlaceholder : t.filterKeywordPlaceholder}
          onChange={(e) => setPattern(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
        />
        <datalist id="filter-apps">
          {apps.map((a) => (
            <option key={a} value={a} />
          ))}
        </datalist>
        <button onClick={add}>{t.add}</button>
      </div>
      {state.filters.length === 0 && <p className="muted small">{t.noFilters}</p>}
      {state.filters.map((f) => {
        const name = state.notifications.find((n) => n.app_pkg === f.pattern)?.app_name;
        return (
          <div className="row" key={f.id}>
            <span className="badge">{f.kind === "app" ? t.filterKindApp : t.filterKindKeyword}</span>
            <span className="grow">{name ? `${name} (${f.pattern})` : f.pattern}</span>
            <button className="quiet" onClick={() => invoke("remove_filter", { id: f.id })}>
              {t.remove}
            </button>
          </div>
        );
      })}
    </>
  );
}

function Account({ state, onError }: { state: State; onError: (m: string) => void }) {
  const { t, lang } = useT();
  const cloud = state.cloud;
  const [apiKey, setApiKey] = useState("");
  const [projectId, setProjectId] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState(false);
  const run = (cmd: string, args?: Record<string, unknown>) => {
    setBusy(true);
    invoke(cmd, args)
      .then(() => {
        setPassword("");
        setEditing(false);
      })
      .catch(onError)
      .finally(() => setBusy(false));
  };
  return (
    <>
      <h2>{t.account}</h2>
      <p className="muted small">{t.accountHint}</p>
      {(!cloud.configured || editing) && !cloud.signed_in ? (
        <>
          {!cloud.configured && <p className="muted small">{t.firebaseNotConfigured}</p>}
          <div className="row">
            <input className="grow" value={apiKey} placeholder={t.apiKey} onChange={(e) => setApiKey(e.target.value)} />
          </div>
          <div className="row">
            <input className="grow" value={projectId} placeholder={t.projectId} onChange={(e) => setProjectId(e.target.value)} />
            <button disabled={busy} onClick={() => run("cloud_set_config", { apiKey, projectId })}>
              {t.save}
            </button>
          </div>
        </>
      ) : cloud.signed_in ? (
        <>
          <p>{t.signedInAs(cloud.email)}</p>
          <p className="muted small">
            {t.lastSync(cloud.last_sync ? new Date(cloud.last_sync).toLocaleString(locale(lang)) : t.never)} · {cloud.project_id}
          </p>
          <div className="row">
            <button disabled={busy} onClick={() => run("cloud_sync")}>
              {busy ? t.working : t.syncNow}
            </button>
            <button className="quiet" disabled={busy} onClick={() => run("cloud_sign_out")}>
              {t.signOut}
            </button>
          </div>
        </>
      ) : (
        <>
          <div className="row">
            <input className="grow" type="email" value={email} placeholder={t.email} onChange={(e) => setEmail(e.target.value)} />
          </div>
          <div className="row">
            <input
              className="grow"
              type="password"
              value={password}
              placeholder={t.password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </div>
          <div className="row">
            <button disabled={busy} onClick={() => run("cloud_sign_in", { email, password, create: false })}>
              {busy ? t.working : t.signIn}
            </button>
            <button className="quiet" disabled={busy} onClick={() => run("cloud_sign_in", { email, password, create: true })}>
              {t.signUp}
            </button>
            <span className="grow" />
            <button className="quiet small" onClick={() => setEditing(true)}>
              {t.changeProject}
            </button>
          </div>
          <p className="muted small">{cloud.project_id}</p>
        </>
      )}
    </>
  );
}

function Settings({ state, onError }: { state: State; onError: (m: string) => void }) {
  const { t } = useT();
  const [relay, setRelay] = useState(state.relay_url);
  const [pkg, setPkg] = useState("");
  const policies: [Policy, string][] = [
    ["allow", t.policyAllow],
    ["hide_body", t.policyHide],
    ["block", t.policyBlock],
  ];
  return (
    <section>
      <h2>{t.language}</h2>
      <select
        value={state.language_setting ?? ""}
        onChange={(e) => invoke("set_language", { language: e.target.value }).catch(onError)}
      >
        <option value="">{t.langAuto}</option>
        <option value="ko">한국어</option>
        <option value="en">English</option>
      </select>
      <h2>{t.toastClose}</h2>
      <label className="row">
        <input
          type="checkbox"
          checked={state.toast_close_dismisses}
          onChange={(e) => invoke("set_toast_close_dismisses", { enabled: e.target.checked })}
        />
        {t.toastCloseDismisses}
      </label>
      <p className="muted small">{t.toastCloseHint}</p>
      <Filters state={state} />
      <h2>{t.dnd}</h2>
      <label className="row">
        <input type="checkbox" checked={state.dnd} onChange={(e) => invoke("set_dnd", { enabled: e.target.checked })} />
        {t.dndSync}
      </label>
      <Account state={state} onError={onError} />
      <h2>{t.relay}</h2>
      <div className="row">
        <input className="grow" value={relay} placeholder={t.relayPlaceholder} onChange={(e) => setRelay(e.target.value)} />
        <button onClick={() => invoke("set_relay", { relayUrl: relay })}>{t.saveRestart}</button>
      </div>
      {state.capture_state === "denied" && (
        <div className="notice">
          <p>{t.captureDenied}</p>
          <button onClick={() => invoke("open_capture_settings")}>{t.openWindowsSettings}</button>
        </div>
      )}
      {state.can_capture && (
        <>
          <h2>{t.appRules}</h2>
          <div className="row">
            <input className="grow" value={pkg} placeholder={t.appId} onChange={(e) => setPkg(e.target.value)} />
            <button
              onClick={() => {
                if (pkg.trim()) invoke("set_app_rule", { appPkg: pkg.trim(), policy: "block" });
                setPkg("");
              }}
            >
              {t.addBlock}
            </button>
          </div>
          {state.rules.map((r) => (
            <div className="row" key={r.app_pkg}>
              <span className="grow">{r.app_pkg}</span>
              <select value={r.policy} onChange={(e) => invoke("set_app_rule", { appPkg: r.app_pkg, policy: e.target.value })}>
                {policies.map(([value, label]) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </select>
            </div>
          ))}
        </>
      )}
      {!state.can_capture && <p className="muted">{t.receiveOnly}</p>}
      <p className="muted small">
        {t.deviceId} {state.device_id.slice(0, 16)}…
      </p>
    </section>
  );
}

function About({ state }: { state: State }) {
  const { t } = useT();
  const [notes, setNotes] = useState<ReleaseNote[]>([]);
  useEffect(() => {
    invoke<ReleaseNote[]>("release_notes").then(setNotes);
  }, [state.language]);
  return (
    <section>
      <h2>
        noti-bridge {t.version} {state.version}
      </h2>
      <h2>{t.releaseNotes}</h2>
      {notes.map((n) => (
        <article className="card" key={n.version}>
          <h3>
            v{n.version} <span className="muted small">{n.date}</span>
          </h3>
          <ul>
            {n.items.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        </article>
      ))}
    </section>
  );
}

type Tab = "notifications" | "devices" | "settings" | "about";
const TABS: Tab[] = ["notifications", "devices", "settings", "about"];

function App() {
  const [state, setState] = useState<State>();
  const [tab, setTab] = useState<Tab>("notifications");
  const [code, setCode] = useState<{ device_name: string; code: string }>();
  // "일치"를 누른 뒤 상대의 확인을 기다리는 중인지. 누른 뒤 아무 반응이 없어 보이던 문제를 막는다.
  const [waiting, setWaiting] = useState(false);
  const [message, setMessage] = useState<string>();
  const refresh = useCallback(() => {
    invoke<State>("get_state").then(setState).catch(setMessage);
  }, []);
  const lang = state?.language ?? (navigator.language.startsWith("ko") ? "ko" : "en");
  const t = STRINGS[lang];

  useEffect(() => {
    // 창이 닫혀 있는 동안 페어링 코드가 왔으면 처음 읽은 상태로 이어서 보여준다.
    invoke<State>("get_state")
      .then((s) => {
        setState(s);
        if (s.pairing) setCode(s.pairing);
      })
      .catch(setMessage);
    const unlisten = listen<UiEvent>("node-changed", ({ payload }) => {
      if (payload.type === "pairing_code") {
        setCode(payload);
        setWaiting(false);
      }
      if (payload.type === "pairing_finished") {
        setCode(undefined);
        setWaiting(false);
        setMessage(payload.accepted ? STRINGS[lang].pairDone(payload.device_name) : STRINGS[lang].pairCancelled);
      }
      if (payload.type === "pairing_failed") {
        setCode(undefined);
        setWaiting(false);
        setMessage(payload.reason);
      }
      refresh();
    });
    // Windows 설정에서 돌아왔을 때 알림 접근 상태 같은 바깥 변화를 다시 읽는다.
    window.addEventListener("focus", refresh);
    return () => {
      window.removeEventListener("focus", refresh);
      void unlisten.then((stop) => stop());
    };
  }, [refresh, lang]);

  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);

  if (!state) return <main className="muted">{t.loading}</main>;
  const tabLabel = { notifications: t.tabNotifications, devices: t.tabDevices, settings: t.tabSettings, about: t.tabAbout };
  const confirm = (accepted: boolean) => {
    if (accepted) setWaiting(true);
    invoke("confirm_pairing", { accepted });
  };
  return (
    <I18n.Provider value={{ t, lang }}>
      <main>
        <nav>
          {TABS.map((id) => (
            <button key={id} className={id === tab ? "active" : "quiet"} onClick={() => setTab(id)}>
              {tabLabel[id]}
            </button>
          ))}
        </nav>
        {tab === "notifications" && (
          <section>
            {state.notifications.length === 0 && <p className="muted">{t.noNotifications}</p>}
            {state.notifications.map((n) => (
              <NotificationCard key={n.nid} n={n} myId={state.device_id} />
            ))}
          </section>
        )}
        {tab === "devices" && <Devices state={state} />}
        {tab === "settings" && <Settings state={state} key={state.relay_url} onError={setMessage} />}
        {tab === "about" && <About state={state} />}
        {code && (
          <div className="modal">
            <div className="sheet">
              <h2>{t.confirmCode}</h2>
              <p>{t.confirmCodeHint(code.device_name)}</p>
              <p className="code">{code.code}</p>
              {waiting ? (
                <p className="waiting">
                  <span className="spinner" /> {t.waitingPeer(code.device_name)}
                </p>
              ) : (
                <div className="row">
                  <button onClick={() => confirm(true)}>{t.match}</button>
                  <button className="quiet" onClick={() => confirm(false)}>
                    {t.mismatch}
                  </button>
                </div>
              )}
            </div>
          </div>
        )}
        {message && (
          <div className="modal">
            <div className="sheet">
              <p>{message}</p>
              <button onClick={() => setMessage(undefined)}>{t.ok}</button>
            </div>
          </div>
        )}
      </main>
    </I18n.Provider>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
