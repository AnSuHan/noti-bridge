//! 기기 키를 DPAPI 로 현재 사용자 계정에 묶어 파일로 보관한다.

use std::path::Path;

use noti_bridge_desktop::Secrets;
use noti_bridge_desktop::noti_bridge_core::crypto;
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData};

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
}

/// DPAPI 가 할당한 출력 버퍼를 복사하고 해제한다.
unsafe fn take(output: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    unsafe {
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(Some(HLOCAL(output.pbData.cast())));
        bytes
    }
}

fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(&blob(data), None, None, None, None, 0, &mut output).map_err(|e| e.to_string())?;
        Ok(take(output))
    }
}

fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(&blob(data), None, None, None, None, 0, &mut output).map_err(|e| e.to_string())?;
        Ok(take(output))
    }
}

fn load_key(path: &Path) -> Result<Vec<u8>, String> {
    if let Ok(sealed) = std::fs::read(path) {
        return unprotect(&sealed);
    }
    let key = crypto::random_key().map_err(|e| e.to_string())?;
    std::fs::write(path, protect(&key)?).map_err(|e| e.to_string())?;
    Ok(key)
}

pub fn load(dir: &Path) -> Result<Secrets, String> {
    Ok(Secrets {
        secret_key: load_key(&dir.join("device_key.dpapi"))?,
        storage_key: load_key(&dir.join("storage_key.dpapi"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpapi_roundtrip_and_key_is_stable_across_loads() {
        let sealed = protect(b"secret").unwrap();
        assert!(!sealed.windows(6).any(|w| w == b"secret"));
        assert_eq!(unprotect(&sealed).unwrap(), b"secret");

        let dir = std::env::temp_dir().join(format!("nb-dpapi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = load(&dir).unwrap();
        let second = load(&dir).unwrap();
        assert_eq!(first.secret_key, second.secret_key);
        assert_ne!(first.secret_key, first.storage_key);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
