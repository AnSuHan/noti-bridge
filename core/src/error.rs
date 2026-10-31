#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
#[cfg_attr(feature = "ffi", uniffi(flat_error))]
pub enum CoreError {
    #[error("저장소 오류: {0}")]
    Storage(String),
    #[error("암호화 오류: {0}")]
    Crypto(String),
    #[error("네트워크 오류: {0}")]
    Network(String),
    #[error("프로토콜 오류: {0}")]
    Protocol(String),
    #[error("페어링 오류: {0}")]
    Pairing(String),
    #[error("잘못된 입력: {0}")]
    InvalidInput(String),
}

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
