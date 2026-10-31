//! noti-bridge 공통 코어: 프로토콜, 동기화 엔진, 로컬 저장, iroh 네트워크.

pub mod crypto;
pub mod engine;
pub mod error;
#[cfg(feature = "ffi")]
pub mod ffi;
pub mod node;
pub mod pairing;
pub mod store;
pub mod types;

pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/noti_bridge.v1.rs"));
}

pub use error::{CoreError, Result};
pub use node::{Node, NodeListener};
pub use types::*;

#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!();
