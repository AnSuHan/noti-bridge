//! 코어가 만드는 사용자용 문구의 언어. 플랫폼 앱이 시작할 때와 사용자가 바꿀 때 정한다.

use std::sync::atomic::{AtomicU8, Ordering};

const KO: u8 = 0;
const EN: u8 = 1;

static LANGUAGE: AtomicU8 = AtomicU8::new(KO);

/// "ko", "ko-KR" 처럼 한국어로 시작하면 한국어, 그 밖에는 영어로 정한다.
pub fn set_language(tag: &str) {
    let value = if tag.trim().to_ascii_lowercase().starts_with("ko") { KO } else { EN };
    LANGUAGE.store(value, Ordering::Relaxed);
}

/// 지금 쓰는 언어 코드("ko" 또는 "en").
pub fn language() -> &'static str {
    if LANGUAGE.load(Ordering::Relaxed) == KO { "ko" } else { "en" }
}

pub fn is_korean() -> bool {
    LANGUAGE.load(Ordering::Relaxed) == KO
}

/// 한국어·영어 문구 중 현재 언어에 맞는 쪽을 고른다.
pub fn tr(ko: &str, en: &str) -> String {
    if is_korean() { ko.to_owned() } else { en.to_owned() }
}

/// 언어는 전역 값이라, 언어를 바꾸는 테스트끼리 동시에 돌지 않게 막는다.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_tags_map_to_korean_or_english() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_language("en-US");
        assert_eq!(language(), "en");
        assert_eq!(tr("가", "a"), "a");
        set_language("ko-KR");
        assert_eq!(language(), "ko");
        assert_eq!(tr("가", "a"), "가");
    }
}
