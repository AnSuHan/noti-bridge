//! QR 초대 코드와 6자리 확인 코드.

use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use sha2::{Digest, Sha256};

use crate::crypto::fill_random;
use crate::error::{CoreError, Result};

pub const INVITE_SCHEME: &str = "notibridge://pair";
pub const TOKEN_TTL_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PairingInvite {
    pub endpoint_id: String,
    pub relay_url: String,
    pub token: String,
    pub expires_at: i64,
    /// 같은 망에서 바로 붙기 위한 직접 주소 힌트.
    pub addrs: Vec<String>,
}

fn enc(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

impl PairingInvite {
    pub fn to_uri(&self) -> String {
        format!(
            "{INVITE_SCHEME}?id={}&relay={}&token={}&exp={}&addrs={}",
            enc(&self.endpoint_id),
            enc(&self.relay_url),
            enc(&self.token),
            self.expires_at,
            enc(&self.addrs.join(","))
        )
    }

    pub fn parse(uri: &str) -> Result<Self> {
        let query = uri
            .strip_prefix(INVITE_SCHEME)
            .and_then(|r| r.strip_prefix('?'))
            .ok_or_else(|| CoreError::Pairing("noti-bridge QR 코드가 아닙니다".into()))?;
        let mut invite = PairingInvite {
            endpoint_id: String::new(),
            relay_url: String::new(),
            token: String::new(),
            expires_at: 0,
            addrs: vec![],
        };
        for part in query.split('&') {
            let Some((k, v)) = part.split_once('=') else { continue };
            let v = percent_decode_str(v).decode_utf8_lossy().into_owned();
            match k {
                "id" => invite.endpoint_id = v,
                "relay" => invite.relay_url = v,
                "token" => invite.token = v,
                "exp" => invite.expires_at = v.parse().unwrap_or(0),
                "addrs" => invite.addrs = v.split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect(),
                _ => {}
            }
        }
        if invite.endpoint_id.is_empty() || invite.token.is_empty() {
            return Err(CoreError::Pairing("QR 코드 내용이 불완전합니다".into()));
        }
        Ok(invite)
    }
}

pub fn new_token() -> Result<String> {
    let mut bytes = [0u8; 16];
    fill_random(&mut bytes)?;
    Ok(hex::encode(bytes))
}

/// 두 공개키의 해시로 만든 6자리 코드. 순서와 무관하게 양쪽이 같은 값을 얻는다.
pub fn confirmation_code(a: &str, b: &str) -> String {
    let (first, second) = if a <= b { (a, b) } else { (b, a) };
    let mut hasher = Sha256::new();
    hasher.update(first.as_bytes());
    hasher.update([0u8]);
    hasher.update(second.as_bytes());
    let digest = hasher.finalize();
    let n = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    format!("{:06}", n % 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_roundtrip() {
        let invite = PairingInvite {
            endpoint_id: "abc".into(),
            relay_url: "https://relay.example/?a=1&b=2".into(),
            token: "tok".into(),
            expires_at: 42,
            addrs: vec!["192.168.0.2:1234".into(), "[::1]:9".into()],
        };
        assert_eq!(PairingInvite::parse(&invite.to_uri()).unwrap(), invite);
    }

    #[test]
    fn rejects_foreign_uri() {
        assert!(PairingInvite::parse("https://example.com").is_err());
        assert!(PairingInvite::parse("notibridge://pair?id=a").is_err());
    }

    #[test]
    fn code_is_symmetric_six_digits() {
        let c = confirmation_code("aaa", "bbb");
        assert_eq!(c, confirmation_code("bbb", "aaa"));
        assert_eq!(c.len(), 6);
        assert_ne!(c, confirmation_code("aaa", "ccc"));
    }
}
