//! 로컬 DB 저장 데이터 암호화. 키 자체의 보호는 플랫폼(Keystore, DPAPI, Keychain)이 맡는다.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use sha2::{Digest, Sha256};

use crate::error::{CoreError, Result};

const NONCE_LEN: usize = 12;

pub struct StorageCipher {
    cipher: ChaCha20Poly1305,
}

impl StorageCipher {
    pub fn new(key: &[u8; 32]) -> Self {
        Self { cipher: ChaCha20Poly1305::new(Key::from_slice(key)) }
    }

    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0u8; NONCE_LEN];
        fill_random(&mut nonce)?;
        let sealed = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|e| CoreError::Crypto(e.to_string()))?;
        let mut out = Vec::with_capacity(NONCE_LEN + sealed.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    pub fn open(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < NONCE_LEN {
            return Err(CoreError::Crypto("암호문이 너무 짧습니다".into()));
        }
        let (nonce, sealed) = data.split_at(NONCE_LEN);
        self.cipher
            .decrypt(Nonce::from_slice(nonce), sealed)
            .map_err(|e| CoreError::Crypto(e.to_string()))
    }
}

pub fn fill_random(buf: &mut [u8]) -> Result<()> {
    getrandom::fill(buf).map_err(|e| CoreError::Crypto(e.to_string()))
}

pub fn random_key() -> Result<Vec<u8>> {
    let mut key = vec![0u8; 32];
    fill_random(&mut key)?;
    Ok(key)
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_roundtrip_and_tamper() {
        let cipher = StorageCipher::new(&[7u8; 32]);
        let mut sealed = cipher.seal(b"hello").unwrap();
        assert_eq!(cipher.open(&sealed).unwrap(), b"hello");
        assert!(!sealed.windows(5).any(|w| w == b"hello"));
        *sealed.last_mut().unwrap() ^= 1;
        assert!(cipher.open(&sealed).is_err());
    }
}
