//! 소셜 로그인용 OAuth 흐름. 받은 Google ID 토큰·GitHub 액세스 토큰은 Firebase signInWithIdp 로 넘긴다.
//!
//! - Google(데스크톱): 설치형 앱 표준인 루프백 리디렉션 + PKCE. 브라우저에서 로그인하면 127.0.0.1 의 임시 포트로
//!   코드가 돌아오고, 그 코드를 ID 토큰으로 바꾼다. Google OAuth "데스크톱 앱" 클라이언트가 필요하다.
//! - Google(Android): 앱이 Credential Manager 로 ID 토큰을 받아 코어에 넘긴다(여기서는 하지 않음).
//! - GitHub: 기기 인증(device flow). 사용자는 github.com/login/device 에 코드를 넣는다. 모든 플랫폼에서 같다.

use std::time::Duration;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::error::{CoreError, Result};
use crate::i18n::tr;

const GOOGLE_AUTH: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN: &str = "https://oauth2.googleapis.com/token";
const GITHUB_DEVICE: &str = "https://github.com/login/device/code";
const GITHUB_TOKEN: &str = "https://github.com/login/oauth/access_token";
/// 브라우저 로그인을 기다리는 최대 시간.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

fn enc(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}={}", enc(v))).collect::<Vec<_>>().join("&")
}

/// 패딩 없는 base64url.
pub fn base64url(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        let len = chunk.len() + 1;
        for i in 0..len {
            out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn random_token() -> Result<String> {
    Ok(base64url(&crate::crypto::random_key()?))
}

fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == name).then(|| percent_encoding::percent_decode_str(&v.replace('+', " ")).decode_utf8_lossy().into_owned())
    })
}

fn oauth_err(e: impl std::fmt::Display) -> CoreError {
    CoreError::Cloud(format!("{}: {e}", tr("소셜 로그인에 실패했습니다", "Social sign-in failed")))
}

/// 브라우저 로그인을 기다리는 Google 루프백 로그인 한 번.
pub struct GoogleLogin {
    listener: TcpListener,
    verifier: String,
    state: String,
    redirect_uri: String,
    pub auth_url: String,
}

impl GoogleLogin {
    pub async fn begin(client_id: &str) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(oauth_err)?;
        let port = listener.local_addr().map_err(oauth_err)?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}");
        let verifier = random_token()?;
        let state = random_token()?;
        let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
        let auth_url = format!(
            "{GOOGLE_AUTH}?{}",
            form(&[
                ("client_id", client_id),
                ("redirect_uri", &redirect_uri),
                ("response_type", "code"),
                ("scope", "openid email profile"),
                ("code_challenge", &challenge),
                ("code_challenge_method", "S256"),
                ("state", &state),
                ("prompt", "select_account"),
            ])
        );
        Ok(Self { listener, verifier, state, redirect_uri, auth_url })
    }

    /// 브라우저가 돌아올 때까지 기다린 뒤 코드를 ID 토큰으로 바꾼다.
    pub async fn finish(self, http: &reqwest::Client, client_id: &str, client_secret: &str) -> Result<String> {
        let code = tokio::time::timeout(LOGIN_TIMEOUT, self.wait_code())
            .await
            .map_err(|_| CoreError::Cloud(tr("로그인 시간이 지났습니다. 다시 시도해 주세요.", "Sign-in timed out. Please try again.")))??;
        let body = form(&[
            ("code", &code),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", &self.redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", &self.verifier),
        ]);
        let resp = http
            .post(GOOGLE_TOKEN)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(oauth_err)?;
        let v: Value = resp.json().await.map_err(oauth_err)?;
        match v["id_token"].as_str() {
            Some(token) => Ok(token.to_owned()),
            None => Err(oauth_err(v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("no id_token"))),
        }
    }

    async fn wait_code(&self) -> Result<String> {
        loop {
            let (mut sock, _) = self.listener.accept().await.map_err(oauth_err)?;
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.map_err(oauth_err)?;
            let request = String::from_utf8_lossy(&buf[..n]);
            let target = request.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");
            let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let code = query_param(query, "code");
            let error = query_param(query, "error");
            if code.is_none() && error.is_none() {
                // favicon 같은 다른 요청은 무시하고 계속 기다린다.
                let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                continue;
            }
            let ok = code.is_some() && query_param(query, "state").as_deref() == Some(self.state.as_str());
            let page = if ok {
                tr("로그인했습니다. 이 창을 닫고 noti-bridge 로 돌아가세요.", "Signed in. You can close this window and return to noti-bridge.")
            } else {
                tr("로그인하지 못했습니다. noti-bridge 에서 다시 시도해 주세요.", "Sign-in failed. Please try again in noti-bridge.")
            };
            let html = format!(
                "<!doctype html><meta charset=utf-8><title>noti-bridge</title><body style=\"font-family:sans-serif;padding:3em\"><p>{page}</p>"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                html.len()
            );
            let _ = sock.write_all(response.as_bytes()).await;
            let _ = sock.shutdown().await;
            return match (ok, code, error) {
                (true, Some(code), _) => Ok(code),
                (_, _, Some(error)) if error == "access_denied" => {
                    Err(CoreError::Cloud(tr("로그인을 취소했습니다.", "Sign-in was cancelled.")))
                }
                (_, _, error) => Err(oauth_err(error.unwrap_or_else(|| "state mismatch".into()))),
            };
        }
    }
}

/// GitHub 기기 인증 시작 결과.
#[derive(Debug, Clone)]
pub struct GithubDevice {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: Duration,
    pub expires_in: Duration,
}

pub async fn github_begin(http: &reqwest::Client, client_id: &str) -> Result<GithubDevice> {
    let resp = http
        .post(GITHUB_DEVICE)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form(&[("client_id", client_id), ("scope", "read:user user:email")]))
        .send()
        .await
        .map_err(oauth_err)?;
    let v: Value = resp.json().await.map_err(oauth_err)?;
    let field = |name: &str| v[name].as_str().map(str::to_owned);
    match (field("device_code"), field("user_code")) {
        (Some(device_code), Some(user_code)) => Ok(GithubDevice {
            device_code,
            user_code,
            verification_uri: field("verification_uri").unwrap_or_else(|| "https://github.com/login/device".into()),
            interval: Duration::from_secs(v["interval"].as_u64().unwrap_or(5).max(1)),
            expires_in: Duration::from_secs(v["expires_in"].as_u64().unwrap_or(900)),
        }),
        _ => Err(oauth_err(match v["error"].as_str() {
            Some("device_flow_disabled") => tr(
                "GitHub OAuth 앱에서 Device Flow 를 켜 주세요",
                "Enable Device Flow in the GitHub OAuth app",
            ),
            Some(e) => e.to_owned(),
            None => "device code".to_owned(),
        })),
    }
}

/// 사용자가 코드를 넣을 때까지 기다렸다가 액세스 토큰을 받는다.
pub async fn github_poll(http: &reqwest::Client, client_id: &str, device: &GithubDevice) -> Result<String> {
    let deadline = tokio::time::Instant::now() + device.expires_in.min(LOGIN_TIMEOUT * 3);
    let mut interval = device.interval;
    loop {
        tokio::time::sleep(interval).await;
        if tokio::time::Instant::now() > deadline {
            return Err(CoreError::Cloud(tr("코드가 만료되었습니다. 다시 시도해 주세요.", "The code expired. Please try again.")));
        }
        let resp = http
            .post(GITHUB_TOKEN)
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form(&[
                ("client_id", client_id),
                ("device_code", &device.device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ]))
            .send()
            .await
            .map_err(oauth_err)?;
        let v: Value = resp.json().await.map_err(oauth_err)?;
        if let Some(token) = v["access_token"].as_str() {
            return Ok(token.to_owned());
        }
        match v["error"].as_str() {
            Some("authorization_pending") => {}
            Some("slow_down") => interval += Duration::from_secs(5),
            Some("expired_token") => {
                return Err(CoreError::Cloud(tr("코드가 만료되었습니다. 다시 시도해 주세요.", "The code expired. Please try again.")));
            }
            Some("access_denied") => return Err(CoreError::Cloud(tr("로그인을 취소했습니다.", "Sign-in was cancelled."))),
            Some(e) => return Err(oauth_err(e)),
            None => return Err(oauth_err("no access_token")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_matches_rfc4648_vectors() {
        let cases = [("", ""), ("f", "Zg"), ("fo", "Zm8"), ("foo", "Zm9v"), ("foob", "Zm9vYg"), ("fooba", "Zm9vYmE")];
        for (input, expected) in cases {
            assert_eq!(base64url(input.as_bytes()), expected);
        }
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(base64url(&Sha256::digest(verifier.as_bytes())), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn query_params_are_decoded() {
        assert_eq!(query_param("code=4%2F0Ab&state=x", "code").as_deref(), Some("4/0Ab"));
        assert_eq!(query_param("error=access_denied", "code"), None);
    }

    #[tokio::test]
    async fn loopback_receives_code_and_checks_state() {
        let login = GoogleLogin::begin("client").await.unwrap();
        assert!(login.auth_url.contains("code_challenge_method=S256"));
        let url = format!("{}/?code=abc&state={}", login.redirect_uri, login.state);
        let port = login.listener.local_addr().unwrap().port();
        let browser = tokio::spawn(async move {
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            // 먼저 favicon 요청이 와도 무시해야 한다.
            s.write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n").await.unwrap();
            let mut sink = Vec::new();
            let _ = s.read_to_end(&mut sink).await;
            let path = url.split_once("127.0.0.1").unwrap().1.split_once('/').unwrap().1;
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(format!("GET /{path} HTTP/1.1\r\n\r\n").as_bytes()).await.unwrap();
            let mut page = String::new();
            let _ = s.read_to_string(&mut page).await;
            page
        });
        assert_eq!(login.wait_code().await.unwrap(), "abc");
        assert!(browser.await.unwrap().starts_with("HTTP/1.1 200"));
    }
}
