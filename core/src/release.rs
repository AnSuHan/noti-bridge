//! 앱 안 "버전 정보"에 보여 줄 변경 사항. 새 버전을 낼 때 맨 위에 추가하고 CHANGELOG.md 와 맞춘다.

use crate::i18n::is_korean;
use crate::types::ReleaseNote;

/// 지금 빌드의 버전. 모든 플랫폼이 Cargo.toml 의 작업 공간 버전을 따른다.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Entry {
    version: &'static str,
    date: &'static str,
    /// (한국어, 영어)
    items: &'static [(&'static str, &'static str)],
}

const NOTES: &[Entry] = &[
    Entry {
        version: "0.0.2",
        date: "2026-11-21",
        items: &[
            (
                "PC 알림이 noti-bridge 대신 원래 앱 이름(카카오톡, Slack 등)과 아이콘으로 표시됩니다.",
                "PC notifications now show the original app name (KakaoTalk, Slack, …) and icon instead of noti-bridge.",
            ),
            (
                "카카오톡·Slack 처럼 같은 대화방 알림을 갱신하는 앱도 새 메시지마다 PC 에 팝업이 뜹니다.",
                "Apps that update one notification per chat (KakaoTalk, Slack) now pop up on the PC for every new message.",
            ),
            (
                "카카오톡·Slack 알림 내용이 '내용 숨겨짐'으로 나오던 문제를 고쳤습니다(대화형 알림의 메시지를 읽음).",
                "Fixed KakaoTalk/Slack notifications showing as 'Content hidden' (messages in conversation-style notifications are now read).",
            ),
            (
                "앱·키워드 필터를 추가했습니다. 걸린 알림은 이 기기에 표시되지 않습니다.",
                "Added app and keyword filters. Matching notifications are not shown on this device.",
            ),
            (
                "'숨기기'로 이 기기 화면에서만 알림을 내릴 수 있습니다. PC 토스트를 닫아도 폰 알림은 지워지지 않아 음악 재생이 끊기지 않습니다.",
                "'Hide' removes a notification from this device only. Closing a PC toast no longer clears the phone notification, so music keeps playing.",
            ),
            (
                "음악 재생 같은 미디어 알림은 미러링하지 않고, 원격 삭제 요청으로도 지우지 않습니다.",
                "Media notifications (music playback) are no longer mirrored and are never cleared remotely.",
            ),
            (
                "앱을 새로 켜거나 다시 연결됐을 때, 그동안 쌓인 알림은 팝업 없이 목록에만 들어갑니다.",
                "After a restart or reconnection, notifications that piled up meanwhile go to the list without popping up.",
            ),
            (
                "페어링 확인 코드에서 '일치'를 누르면 상대 기기의 확인을 기다리는 중임을 보여줍니다.",
                "After tapping 'Match' on the pairing code, the app shows that it is waiting for the other device.",
            ),
            (
                "Firebase 계정 로그인을 추가했습니다. 같은 계정의 기기 목록과 필터가 동기화됩니다. 알림 내용은 계정에 올리지 않습니다.",
                "Added Firebase account sign-in. Your device list and filters sync across the account. Notification contents are never uploaded.",
            ),
            ("한국어·영어를 지원합니다.", "Korean and English are supported."),
            ("앱 안에서 버전과 변경 사항을 볼 수 있습니다.", "The version and release notes are shown in the app."),
        ],
    },
    Entry {
        version: "0.0.1",
        date: "2026-11-01",
        items: &[
            ("첫 공개 버전입니다.", "First public version."),
            ("QR 과 6자리 코드 페어링, 신뢰 목록.", "QR and 6-digit code pairing, trusted device list."),
            (
                "폰 알림을 PC 토스트와 앱 목록에 표시, 양방향 삭제, 액션과 답장.",
                "Phone notifications on PC toasts and in the app list, two-way dismissal, actions and replies.",
            ),
            (
                "같은 망 직접 연결과 공개 릴레이 경유, 오프라인 변경의 재연결 후 전달.",
                "Direct LAN connection or public relay, offline changes delivered after reconnecting.",
            ),
        ],
    },
];

pub fn release_notes() -> Vec<ReleaseNote> {
    let korean = is_korean();
    NOTES
        .iter()
        .map(|e| ReleaseNote {
            version: e.version.to_owned(),
            date: e.date.to_owned(),
            items: e.items.iter().map(|(ko, en)| if korean { *ko } else { *en }.to_owned()).collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_note_matches_build_version() {
        assert_eq!(NOTES[0].version, VERSION, "새 버전의 변경 사항을 release.rs 맨 위에 추가하세요");
    }
}
