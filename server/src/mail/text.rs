//! 메일 제목과 본문. 일반 텍스트, `en` 아니면 ko. 유효 기간은 시각 대신 길이(시간대 변환 회피).

/// 보내는 이름. 본문과 같은 언어.
pub fn sender(locale: &str) -> &'static str {
    if locale == "en" {
        "Chagokmoa"
    } else {
        "차곡모아"
    }
}

/// 초대 메일 (제목, 본문).
pub fn invite(locale: &str, instance: &str, url: &str, hours: u32) -> (String, String) {
    if locale == "en" {
        (
            format!("You're invited to {instance}"),
            format!(
                "You've been invited to {instance}.\n\
                 Open the link below to join.\n\n\
                 {url}\n\n\
                 The link is valid for {}.",
                en_hours(hours)
            ),
        )
    } else {
        (
            format!("{instance}에 초대해요"),
            format!(
                "{instance}에 초대받았어요.\n\
                 아래 링크를 열어 참여해 주세요.\n\n\
                 {url}\n\n\
                 링크는 {hours}시간 동안 쓸 수 있어요."
            ),
        )
    }
}

/// 비밀번호 재설정 메일 (제목, 본문). 같은 이메일의 여러 계정 구분용 아이디 포함.
pub fn reset(
    locale: &str,
    instance: &str,
    username: &str,
    url: &str,
    hours: u32,
) -> (String, String) {
    if locale == "en" {
        (
            format!("Reset your {instance} password"),
            format!(
                "Open the link below to set a new password for {instance} account {username}.\n\n\
                 {url}\n\n\
                 The link can be used once within {}.\n\
                 If you don't want to change your password, you can ignore this email.",
                en_hours(hours)
            ),
        )
    } else {
        (
            format!("{instance} 비밀번호 재설정"),
            format!(
                "아래 링크를 열어 {instance} 아이디 {username}의 새 비밀번호를 정해 주세요.\n\n\
                 {url}\n\n\
                 링크는 {hours}시간 안에 한 번만 쓸 수 있어요.\n\
                 비밀번호를 바꾸지 않으려면 이 메일은 무시해 주세요."
            ),
        )
    }
}

fn en_hours(hours: u32) -> String {
    if hours == 1 {
        "1 hour".to_owned()
    } else {
        format!("{hours} hours")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_fallback_and_hours() {
        let (subject, body) = invite("ja", "우리집", "http://x/invite/r", 72);
        assert_eq!(subject, "우리집에 초대해요");
        assert!(body.contains("http://x/invite/r") && body.contains("72시간"));

        let (_, body) = reset("en", "Home", "dad", "http://x/reset/r", 1);
        assert!(body.contains("account dad") && body.contains("within 1 hour."));

        assert_eq!(sender("en"), "Chagokmoa");
        assert_eq!(sender("ja"), "차곡모아");
    }
}
