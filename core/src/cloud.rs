//! Firebase 계정(이메일 로그인)과 Firestore 저장. 공식 REST API 를 직접 부른다.
//!
//! 계정에는 알림 내용을 올리지 않는다. 올리는 것은 기기 목록(이름·플랫폼·버전)과 필터 목록뿐이다.
//! Firestore 경로:
//! - `users/{uid}/devices/{device_id}`: name, platform, version, updatedAt
//! - `users/{uid}/settings/filters`: items[{kind, pattern}], updatedAt

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{CoreError, Result};
use crate::i18n::tr;
use crate::types::FilterKind;

/// Firebase 프로젝트 값. 빌드 때 환경 변수로 넣거나 앱 설정에서 입력한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudConfig {
    pub api_key: String,
    pub project_id: String,
    /// 에뮬레이터 주소(예: "127.0.0.1:9099"). 비우면 실제 Google 서버를 쓴다.
    #[serde(default)]
    pub auth_emulator: Option<String>,
    #[serde(default)]
    pub firestore_emulator: Option<String>,
}

impl CloudConfig {
    /// 빌드 때 넣은 기본 프로젝트. 둘 다 있어야 쓴다.
    pub fn built_in() -> Option<Self> {
        let api_key = option_env!("NOTIBRIDGE_FIREBASE_API_KEY")?.trim();
        let project_id = option_env!("NOTIBRIDGE_FIREBASE_PROJECT_ID")?.trim();
        (!api_key.is_empty() && !project_id.is_empty()).then(|| Self {
            api_key: api_key.to_owned(),
            project_id: project_id.to_owned(),
            auth_emulator: None,
            firestore_emulator: None,
        })
    }

    /// 실행 시 환경 변수 `NOTIBRIDGE_FIREBASE_EMULATOR=auth호스트:포트,firestore호스트:포트` 로 에뮬레이터를 쓴다(시험용).
    pub fn with_env_emulator(mut self) -> Self {
        if let Ok(value) = std::env::var("NOTIBRIDGE_FIREBASE_EMULATOR") {
            let mut parts = value.split(',').map(str::trim).filter(|s| !s.is_empty());
            self.auth_emulator = parts.next().map(str::to_owned);
            self.firestore_emulator = parts.next().map(str::to_owned);
        }
        self
    }

    fn auth_url(&self, path: &str) -> String {
        match &self.auth_emulator {
            Some(host) => format!("http://{host}/identitytoolkit.googleapis.com/v1/{path}?key={}", self.api_key),
            None => format!("https://identitytoolkit.googleapis.com/v1/{path}?key={}", self.api_key),
        }
    }

    fn token_url(&self) -> String {
        match &self.auth_emulator {
            Some(host) => format!("http://{host}/securetoken.googleapis.com/v1/token?key={}", self.api_key),
            None => format!("https://securetoken.googleapis.com/v1/token?key={}", self.api_key),
        }
    }

    fn doc_url(&self, path: &str) -> String {
        let base = match &self.firestore_emulator {
            Some(host) => format!("http://{host}"),
            None => "https://firestore.googleapis.com".to_owned(),
        };
        format!("{base}/v1/projects/{}/databases/(default)/documents/{path}", self.project_id)
    }
}

/// 로그인 상태. 갱신 토큰만 있으면 다시 로그인하지 않고 ID 토큰을 받을 수 있다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub uid: String,
    pub email: String,
    pub refresh_token: String,
    #[serde(skip)]
    pub id_token: String,
    /// id_token 만료 시각(ms).
    #[serde(skip)]
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDevice {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub version: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFilters {
    pub items: Vec<(FilterKind, String)>,
    pub updated_at: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// iroh 와 같은 ring 기반 rustls 에 공개 루트 인증서를 쓴다. Android 에서도 따로 초기화할 필요가 없다.
pub fn http_client() -> Result<reqwest::Client> {
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| CoreError::Cloud(e.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| CoreError::Cloud(e.to_string()))
}

fn network(e: reqwest::Error) -> CoreError {
    CoreError::Cloud(format!("{}: {e}", tr("서버에 연결하지 못했습니다", "Could not reach the server")))
}

/// Firebase 오류 코드를 사용자 문구로 바꾼다.
fn auth_message(code: &str) -> String {
    let code = code.split([' ', ':']).next().unwrap_or(code);
    match code {
        "EMAIL_EXISTS" => tr("이미 가입된 이메일입니다. 로그인을 눌러 주세요.", "This email is already registered. Please sign in."),
        "EMAIL_NOT_FOUND" | "INVALID_PASSWORD" | "INVALID_LOGIN_CREDENTIALS" => {
            tr("이메일 또는 비밀번호가 맞지 않습니다.", "Wrong email or password.")
        }
        "INVALID_EMAIL" | "MISSING_EMAIL" => tr("이메일 형식이 올바르지 않습니다.", "The email address is invalid."),
        "WEAK_PASSWORD" | "MISSING_PASSWORD" => tr("비밀번호는 6자 이상이어야 합니다.", "Password must be at least 6 characters."),
        "USER_DISABLED" => tr("사용이 중지된 계정입니다.", "This account has been disabled."),
        "TOO_MANY_ATTEMPTS_TRY_LATER" => tr("시도가 너무 많습니다. 잠시 후 다시 해 주세요.", "Too many attempts. Try again later."),
        "TOKEN_EXPIRED" | "INVALID_REFRESH_TOKEN" | "USER_NOT_FOUND" | "INVALID_ID_TOKEN" => {
            tr("로그인이 만료되었습니다. 다시 로그인해 주세요.", "Your session expired. Please sign in again.")
        }
        "API_KEY_INVALID" | "INVALID_API_KEY" => tr("Firebase API 키가 올바르지 않습니다.", "The Firebase API key is invalid."),
        "OPERATION_NOT_ALLOWED" | "CONFIGURATION_NOT_FOUND" => tr(
            "Firebase 콘솔에서 이메일/비밀번호 로그인을 켜 주세요.",
            "Enable Email/Password sign-in in the Firebase console.",
        ),
        other => other.to_owned(),
    }
}

async fn read_json(resp: reqwest::Response) -> Result<Value> {
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(body);
    }
    let message = body["error"]["message"].as_str().unwrap_or("").to_owned();
    let status_name = body["error"]["status"].as_str().unwrap_or("");
    let text = if status_name == "PERMISSION_DENIED" {
        tr(
            "계정 저장소 접근이 거부되었습니다. Firestore 보안 규칙을 확인해 주세요.",
            "Access to account storage was denied. Check the Firestore security rules.",
        )
    } else if message.is_empty() {
        format!("HTTP {status}")
    } else {
        auth_message(&message)
    };
    Err(CoreError::Cloud(text))
}

pub struct CloudClient {
    http: reqwest::Client,
}

impl CloudClient {
    pub fn new() -> Result<Self> {
        Ok(Self { http: http_client()? })
    }

    /// 이메일·비밀번호로 로그인한다. create 가 true 면 새로 가입한다.
    pub async fn sign_in(&self, config: &CloudConfig, email: &str, password: &str, create: bool) -> Result<Session> {
        let path = if create { "accounts:signUp" } else { "accounts:signInWithPassword" };
        let body = json!({ "email": email.trim(), "password": password, "returnSecureToken": true });
        let resp = self.http.post(config.auth_url(path)).json(&body).send().await.map_err(network)?;
        let v = read_json(resp).await?;
        let expires: i64 = v["expiresIn"].as_str().and_then(|s| s.parse().ok()).unwrap_or(3600);
        Ok(Session {
            uid: v["localId"].as_str().unwrap_or_default().to_owned(),
            email: v["email"].as_str().unwrap_or(email).to_owned(),
            refresh_token: v["refreshToken"].as_str().unwrap_or_default().to_owned(),
            id_token: v["idToken"].as_str().unwrap_or_default().to_owned(),
            expires_at: now_ms() + expires * 1000,
        })
    }

    /// 만료가 1분 안쪽이면 갱신 토큰으로 새 ID 토큰을 받는다.
    pub async fn ensure_token(&self, config: &CloudConfig, session: &mut Session) -> Result<()> {
        if !session.id_token.is_empty() && session.expires_at - now_ms() > 60_000 {
            return Ok(());
        }
        let form = format!("grant_type=refresh_token&refresh_token={}", session.refresh_token);
        let resp = self
            .http
            .post(config.token_url())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form)
            .send()
            .await
            .map_err(network)?;
        let v = read_json(resp).await?;
        let expires: i64 = v["expires_in"].as_str().and_then(|s| s.parse().ok()).unwrap_or(3600);
        session.id_token = v["id_token"].as_str().unwrap_or_default().to_owned();
        if let Some(refresh) = v["refresh_token"].as_str() {
            session.refresh_token = refresh.to_owned();
        }
        session.expires_at = now_ms() + expires * 1000;
        Ok(())
    }

    async fn get_doc(&self, config: &CloudConfig, session: &Session, path: &str) -> Result<Option<Value>> {
        let resp = self
            .http
            .get(config.doc_url(path))
            .bearer_auth(&session.id_token)
            .send()
            .await
            .map_err(network)?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        read_json(resp).await.map(Some)
    }

    async fn put_doc(&self, config: &CloudConfig, session: &Session, path: &str, fields: Value) -> Result<()> {
        let resp = self
            .http
            .patch(config.doc_url(path))
            .bearer_auth(&session.id_token)
            .json(&json!({ "fields": fields }))
            .send()
            .await
            .map_err(network)?;
        read_json(resp).await.map(|_| ())
    }

    pub async fn put_device(&self, config: &CloudConfig, session: &Session, device: &RemoteDevice) -> Result<()> {
        let path = format!("users/{}/devices/{}", session.uid, device.device_id);
        let fields = json!({
            "name": { "stringValue": device.name },
            "platform": { "stringValue": device.platform },
            "version": { "stringValue": device.version },
            "updatedAt": { "integerValue": device.updated_at.to_string() },
        });
        self.put_doc(config, session, &path, fields).await
    }

    pub async fn delete_device(&self, config: &CloudConfig, session: &Session, device_id: &str) -> Result<()> {
        let path = format!("users/{}/devices/{device_id}", session.uid);
        let resp = self
            .http
            .delete(config.doc_url(&path))
            .bearer_auth(&session.id_token)
            .send()
            .await
            .map_err(network)?;
        read_json(resp).await.map(|_| ())
    }

    pub async fn devices(&self, config: &CloudConfig, session: &Session) -> Result<Vec<RemoteDevice>> {
        let path = format!("users/{}/devices", session.uid);
        let Some(v) = self.get_doc(config, session, &path).await? else { return Ok(vec![]) };
        let docs = v["documents"].as_array().cloned().unwrap_or_default();
        Ok(docs.iter().map(parse_device).collect())
    }

    pub async fn filters(&self, config: &CloudConfig, session: &Session) -> Result<Option<RemoteFilters>> {
        let path = format!("users/{}/settings/filters", session.uid);
        Ok(self.get_doc(config, session, &path).await?.map(|doc| parse_filters(&doc)))
    }

    pub async fn put_filters(&self, config: &CloudConfig, session: &Session, filters: &RemoteFilters) -> Result<()> {
        let path = format!("users/{}/settings/filters", session.uid);
        self.put_doc(config, session, &path, filters_fields(filters)).await
    }
}

fn int_field(fields: &Value, name: &str) -> i64 {
    let v = &fields[name];
    v["integerValue"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| v["integerValue"].as_i64())
        .unwrap_or(0)
}

fn str_field(fields: &Value, name: &str) -> String {
    fields[name]["stringValue"].as_str().unwrap_or_default().to_owned()
}

fn parse_device(doc: &Value) -> RemoteDevice {
    let fields = &doc["fields"];
    RemoteDevice {
        device_id: doc["name"].as_str().and_then(|n| n.rsplit('/').next()).unwrap_or_default().to_owned(),
        name: str_field(fields, "name"),
        platform: str_field(fields, "platform"),
        version: str_field(fields, "version"),
        updated_at: int_field(fields, "updatedAt"),
    }
}

fn kind_name(kind: FilterKind) -> &'static str {
    match kind {
        FilterKind::App => "app",
        FilterKind::Keyword => "keyword",
    }
}

pub fn filters_fields(filters: &RemoteFilters) -> Value {
    let values: Vec<Value> = filters
        .items
        .iter()
        .map(|(kind, pattern)| {
            json!({ "mapValue": { "fields": {
                "kind": { "stringValue": kind_name(*kind) },
                "pattern": { "stringValue": pattern },
            }}})
        })
        .collect();
    json!({
        "items": { "arrayValue": { "values": values } },
        "updatedAt": { "integerValue": filters.updated_at.to_string() },
    })
}

pub fn parse_filters(doc: &Value) -> RemoteFilters {
    let fields = &doc["fields"];
    let items = fields["items"]["arrayValue"]["values"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(|v| {
                    let f = &v["mapValue"]["fields"];
                    let kind = match f["kind"]["stringValue"].as_str()? {
                        "app" => FilterKind::App,
                        "keyword" => FilterKind::Keyword,
                        _ => return None,
                    };
                    let pattern = f["pattern"]["stringValue"].as_str()?.trim().to_owned();
                    (!pattern.is_empty()).then_some((kind, pattern))
                })
                .collect()
        })
        .unwrap_or_default();
    RemoteFilters { items, updated_at: int_field(fields, "updatedAt") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_roundtrip_through_firestore_fields() {
        let filters = RemoteFilters {
            items: vec![(FilterKind::App, "com.kakao.talk".into()), (FilterKind::Keyword, "광고".into())],
            updated_at: 1_700_000_000_123,
        };
        let doc = json!({ "fields": filters_fields(&filters) });
        assert_eq!(parse_filters(&doc), filters);
    }

    #[test]
    fn empty_or_unknown_filter_entries_are_skipped() {
        let doc = json!({ "fields": { "items": { "arrayValue": { "values": [
            { "mapValue": { "fields": { "kind": { "stringValue": "nope" }, "pattern": { "stringValue": "x" } } } },
            { "mapValue": { "fields": { "kind": { "stringValue": "keyword" }, "pattern": { "stringValue": " " } } } },
        ] } } } });
        assert!(parse_filters(&doc).items.is_empty());
    }

    #[test]
    fn device_id_comes_from_document_name() {
        let doc = json!({ "name": "projects/p/databases/(default)/documents/users/u/devices/abc", "fields": {
            "name": { "stringValue": "PC" }, "updatedAt": { "integerValue": "5" } } });
        let d = parse_device(&doc);
        assert_eq!((d.device_id.as_str(), d.name.as_str(), d.updated_at), ("abc", "PC", 5));
    }

    #[test]
    fn emulator_urls() {
        let c = CloudConfig {
            api_key: "k".into(),
            project_id: "p".into(),
            auth_emulator: Some("127.0.0.1:9099".into()),
            firestore_emulator: Some("127.0.0.1:8080".into()),
        };
        assert_eq!(
            c.auth_url("accounts:signUp"),
            "http://127.0.0.1:9099/identitytoolkit.googleapis.com/v1/accounts:signUp?key=k"
        );
        assert_eq!(c.doc_url("users/u"), "http://127.0.0.1:8080/v1/projects/p/databases/(default)/documents/users/u");
    }

    #[test]
    fn firebase_error_codes_become_messages() {
        let _guard = crate::i18n::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language("en");
        assert_eq!(auth_message("WEAK_PASSWORD : Password should be at least 6 characters"), "Password must be at least 6 characters.");
        crate::i18n::set_language("ko");
    }
}
