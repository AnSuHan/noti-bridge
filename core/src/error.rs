use std::fmt;

use crate::i18n::tr;

#[derive(Debug)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
#[cfg_attr(feature = "ffi", uniffi(flat_error))]
pub enum CoreError {
    Storage(String),
    Crypto(String),
    Network(String),
    Protocol(String),
    Pairing(String),
    InvalidInput(String),
    /// Firebase 로그인·동기화 오류. 메시지는 사용자에게 그대로 보여준다.
    Cloud(String),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (label, detail) = match self {
            Self::Storage(m) => (tr("저장소 오류", "Storage error"), m),
            Self::Crypto(m) => (tr("암호화 오류", "Encryption error"), m),
            Self::Network(m) => (tr("네트워크 오류", "Network error"), m),
            Self::Protocol(m) => (tr("프로토콜 오류", "Protocol error"), m),
            Self::Pairing(m) => (tr("페어링 오류", "Pairing error"), m),
            Self::InvalidInput(m) => (tr("잘못된 입력", "Invalid input"), m),
            Self::Cloud(m) => (tr("계정 오류", "Account error"), m),
        };
        write!(f, "{label}: {detail}")
    }
}

impl std::error::Error for CoreError {}

pub type Result<T> = std::result::Result<T, CoreError>;

impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        CoreError::Storage(e.to_string())
    }
}

impl From<prost::DecodeError> for CoreError {
    fn from(e: prost::DecodeError) -> Self {
        CoreError::Protocol(e.to_string())
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::Network(e.to_string())
    }
}
