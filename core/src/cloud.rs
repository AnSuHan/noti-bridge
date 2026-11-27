//! Firebase 계정(이메일·Google·GitHub 로그인)과 Firestore 저장. 공식 REST API 를 직접 부른다.
//!
//! 계정에는 알림 내용을 올리지 않는다. 올리는 것은 기기 목록(이름·플랫폼·버전)과 필터 목록뿐이다.
//! Firestore 경로:
//! - `users/{uid}/devices/{device_id}`: name, platform, version, updatedAt
//! - `users/{uid}/settings/filters`: items[{kind, pattern, label}], updatedAt

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{CoreError, Result};
use crate::i18n::tr;
use crate::types::{FilterKind, FilterSpec};

/// Firebase 프로젝트 값. 빌드 때 환경 변수로 넣거나 앱 설정에서 입력한다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudConfig {
    pub api_key: String,
    pub project_id: String,
    /// 에뮬레이터 주소(예: "127.0.0.1:9099"). 비우면 실제 Google 서버를 쓴다.
    #[serde(default)]
    pub auth_emulator: Option<String>,
    #[serde(default)]
    pub firestore_emulator: Option<String>,
    /// Google 로그인(데스크톱): Google Cloud 콘솔의 OAuth "데스크톱 앱" 클라이언트 ID 와 비밀번호.
    /// 데스크톱 클라이언트의 비밀번호는 앱에 들어가는 값이라 비밀이 아니다(Google 문서 기준).
    #[serde(default)]
    pub google_client_id: String,
    #[serde(default)]
    pub google_client_secret: String,
    /// Google 로그인(Android): Firebase 가 만든 "웹 클라이언트" ID. Credential Manager 의 serverClientId 로 쓴다.
    #[serde(default)]
    pub google_web_client_id: String,
    /// GitHub 로그인: Device Flow 를 켠 GitHub OAuth 앱의 클라이언트 ID. 같은 앱을 Firebase 의 GitHub 제공업체에 등록한다.
    #[serde(default)]
    pub github_client_id: String,
}

fn env_value(value: Option<&'static str>) -> String {
    value.map(str::trim).unwrap_or_default().to_owned()
}

impl CloudConfig {
    /// 빌드 때 넣은 기본 프로젝트. API 키와 프로젝트 ID 가 둘 다 있어야 쓴다.
    pub fn built_in() -> Option<Self> {
        let api_key = env_value(option_env!("NOTIBRIDGE_FIREBASE_API_KEY"));
        let project_id = env_value(option_env!("NOTIBRIDGE_FIREBASE_PROJECT_ID"));
        (!api_key.is_empty() && !project_id.is_empty()).then(|| Self {
            api_key,
            project_id,
            ..Self::social_built_in()
        })
    }

    /// 빌드 때 넣은 소셜 로그인 값. 앱 설정에서 Firebase 프로젝트만 바꿔 넣어도 이 값은 쓴다.
    pub fn social_built_in() -> Self {
        Self {
            google_client_id: env_value(option_env!("NOTIBRIDGE_GOOGLE_CLIENT_ID")),
            google_client_secret: env_value(option_env!("NOTIBRIDGE_GOOGLE_CLIENT_SECRET")),
            google_web_client_id: env_value(option_env!("NOTIBRIDGE_GOOGLE_WEB_CLIENT_ID")),
            github_client_id: env_value(option_env!("NOTIBRIDGE_GITHUB_CLIENT_ID")),
            ..Self::default()
        }
    }

    /// 비어 있는 소셜 값은 빌드 때 넣은 값으로 채운다.
    pub fn with_social_defaults(mut self) -> Self {
        let d = Self::social_built_in();
        for (mine, default) in [
            (&mut self.google_client_id, d.google_client_id),
            (&mut self.google_client_secret, d.google_client_secret),
            (&mut self.google_web_client_id, d.google_web_client_id),
            (&mut self.github_client_id, d.github_client_id),
        ] {
            if mine.trim().is_empty() {
                *mine = default;
            }
        }
        self
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

fn password_provider() -> String {
    "password".to_owned()
}

/// 로그인 상태. 갱신 토큰만 있으면 다시 로그인하지 않고 ID 토큰을 받을 수 있다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub uid: String,
    /// 이메일. GitHub 처럼 이메일을 주지 않는 계정은 표시 이름을 넣는다.
    pub email: String,
    /// "password", "google.com", "github.com". 예전 세션은 이메일 로그인이었다.
    #[serde(default = "password_provider")]
    pub provider: String,
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
    pub items: Vec<FilterSpec>,
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
            "Firebase 콘솔의 Authentication > 로그인 방법에서 이 로그인 방법을 켜 주세요.",
            "Enable this sign-in method in Firebase console > Authentication > Sign-in method.",
        ),
        "INVALID_IDP_RESPONSE" => tr(
            "소셜 로그인 응답을 확인하지 못했습니다. Firebase 의 제공업체 설정(클라이언트 ID)을 확인해 주세요.",
            "Could not verify the social sign-in. Check the provider settings (client ID) in Firebase.",
        ),
        "FEDERATED_USER_ID_ALREADY_LINKED" | "EMAIL_EXISTS_FOR_PROVIDER" | "NEED_CONFIRMATION" => tr(
            "이 이메일은 다른 로그인 방법으로 이미 가입돼 있습니다. 처음 가입한 방법으로 로그인해 주세요.",
            "This email is already registered with another sign-in method. Please use that method.",
        ),
        "RESET_PASSWORD_EXCEED_LIMIT" => tr(
            "비밀번호 재설정 메일을 너무 많이 요청했습니다. 잠시 후 다시 해 주세요.",
            "Too many password reset requests. Try again later.",
        ),
        other => other.to_owned(),
    }
}

fn session_from(v: &Value, provider: &str, fallback_email: &str) -> Result<Session> {
    let expires: i64 = v["expiresIn"].as_str().and_then(|s| s.parse().ok()).unwrap_or(3600);
    let email = [v["email"].as_str(), v["displayName"].as_str(), v["screenName"].as_str()]
        .into_iter()
        .flatten()
        .find(|s| !s.is_empty())
        .unwrap_or(fallback_email);
    let session = Session {
        uid: v["localId"].as_str().unwrap_or_default().to_owned(),
        email: email.to_owned(),
        provider: provider.to_owned(),
        refresh_token: v["refreshToken"].as_str().unwrap_or_default().to_owned(),
        id_token: v["idToken"].as_str().unwrap_or_default().to_owned(),
        expires_at: now_ms() + expires * 1000,
    };
    if session.uid.is_empty() || session.refresh_token.is_empty() {
        return Err(CoreError::Cloud(auth_message("INVALID_IDP_RESPONSE")));
    }
    Ok(session)
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

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// 이메일·비밀번호로 로그인한다. create 가 true 면 새로 가입한다.
    pub async fn sign_in(&self, config: &CloudConfig, email: &str, password: &str, create: bool) -> Result<Session> {
        let path = if create { "accounts:signUp" } else { "accounts:signInWithPassword" };
        let body = json!({ "email": email.trim(), "password": password, "returnSecureToken": true });
        let resp = self.http.post(config.auth_url(path)).json(&body).send().await.map_err(network)?;
        let v = read_json(resp).await?;
        session_from(&v, "password", email.trim())
    }

    /// Google ID 토큰이나 GitHub 액세스 토큰으로 로그인한다. 처음이면 계정이 만들어진다.
    /// post_body 예: "id_token=...&providerId=google.com", "access_token=...&providerId=github.com".
    pub async fn sign_in_with_idp(&self, config: &CloudConfig, provider: &str, post_body: &str) -> Result<Session> {
        let body = json!({
            "postBody": post_body,
            "requestUri": "http://localhost",
            "returnIdpCredential": true,
            "returnSecureToken": true,
        });
        let resp = self.http.post(config.auth_url("accounts:signInWithIdp")).json(&body).send().await.map_err(network)?;
        let v = read_json(resp).await?;
        // 같은 이메일이 다른 방법(예: 이메일/비밀번호)으로 가입돼 있으면 토큰 없이 확인 요청만 온다.
        if v["needConfirmation"].as_bool() == Some(true) && v["idToken"].as_str().is_none_or(str::is_empty) {
            return Err(CoreError::Cloud(auth_message("NEED_CONFIRMATION")));
        }
        if let Some(message) = v["errorMessage"].as_str().filter(|m| !m.is_empty()) {
            return Err(CoreError::Cloud(auth_message(message)));
        }
        session_from(&v, provider, "")
    }

    /// 비밀번호 재설정 메일을 보낸다.
    pub async fn send_password_reset(&self, config: &CloudConfig, email: &str) -> Result<()> {
        let body = json!({ "requestType": "PASSWORD_RESET", "email": email.trim() });
        let resp = self.http.post(config.auth_url("accounts:sendOobCode")).json(&body).send().await.map_err(network)?;
        read_json(resp).await.map(|_| ())
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

pub fn filters_fields(filters: &RemoteFilters) -> Value {
    let values: Vec<Value> = filters
        .items
        .iter()
        .map(|f| {
            json!({ "mapValue": { "fields": {
                "kind": { "stringValue": f.kind.name() },
                "pattern": { "stringValue": f.pattern },
                "label": { "stringValue": f.label },
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
                    let kind = FilterKind::from_name(f["kind"]["stringValue"].as_str()?)?;
                    let pattern = f["pattern"]["stringValue"].as_str()?.trim().to_owned();
                    let label = f["label"]["stringValue"].as_str().unwrap_or_default().to_owned();
                    (!pattern.is_empty()).then_some(FilterSpec { kind, pattern, label })
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
        let spec = |kind, pattern: &str, label: &str| FilterSpec { kind, pattern: pattern.into(), label: label.into() };
        let filters = RemoteFilters {
            items: vec![
                spec(FilterKind::App, "com.kakao.talk", "카카오톡"),
                spec(FilterKind::Keyword, "광고", ""),
                spec(FilterKind::HiddenContent, "*", ""),
            ],
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
            ..CloudConfig::default()
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

    #[test]
    fn old_sessions_without_provider_are_password_logins() {
        let s: Session = serde_json::from_str(r#"{"uid":"u","email":"a@b.c","refresh_token":"r"}"#).unwrap();
        assert_eq!(s.provider, "password");
    }

    #[test]
    fn idp_session_falls_back_to_display_name_without_email() {
        let v = json!({ "localId": "u", "refreshToken": "r", "idToken": "i", "displayName": "octocat" });
        assert_eq!(session_from(&v, "github.com", "").unwrap().email, "octocat");
        assert!(session_from(&json!({}), "github.com", "").is_err());
    }
}
