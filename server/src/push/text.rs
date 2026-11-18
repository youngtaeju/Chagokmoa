//! 푸시 문장과 이동 주소. 문구는 웹 locale 파일 원본 그대로.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde_json::Value;
use uuid::Uuid;

use crate::notifications::{Notification, NotificationKind};

// 웹 앱 안 알림 목록과 같은 문구 원본.
static KO: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../web/src/locales/ko.json")).unwrap_or_default()
});
static EN: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../web/src/locales/en.json")).unwrap_or_default()
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    Ko,
    En,
}

/// 사용자 언어 → 인스턴스 기본 언어 → ko.
pub fn pick_locale(user_locale: &str, default_locale: &str) -> Locale {
    for l in [user_locale, default_locale] {
        match l {
            "ko" => return Locale::Ko,
            "en" => return Locale::En,
            _ => {}
        }
    }
    Locale::Ko
}

/// 점 경로 키 조회와 `{{이름}}` 치환. 없는 키는 키 그대로.
pub fn t(locale: Locale, key: &str, args: &[(&str, &str)]) -> String {
    let root = match locale {
        Locale::Ko => &*KO,
        Locale::En => &*EN,
    };
    let found = key.split('.').try_fold(root, |v, part| v.get(part));
    let Some(Value::String(s)) = found else {
        return key.to_string();
    };
    args.iter().fold(s.clone(), |acc, (name, value)| {
        acc.replace(&format!("{{{{{name}}}}}"), value)
    })
}

/// 멘션 토큰 → `@이름` 평문. 끝의 잘린 토큰은 버림, 모르는 id 는 `@unknown`.
pub fn plain_mentions(body: &str, names: &HashMap<Uuid, String>, unknown: &str) -> String {
    let mut body = body;
    if let Some(pos) = body.rfind("<@") {
        let rest = &body[pos + 2..];
        if rest.len() <= 36
            && rest
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'-'))
        {
            body = &body[..pos];
        }
    }
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("<@") {
        let after = &rest[start + 2..];
        let token = after
            .get(..36)
            .filter(|id| after[36..].starts_with('>') && is_lower_uuid(id));
        match token {
            Some(id) => {
                out.push_str(&rest[..start]);
                let name = Uuid::parse_str(id).ok().and_then(|u| names.get(&u));
                out.push('@');
                out.push_str(name.map_or(unknown, String::as_str));
                rest = &after[37..];
            }
            None => {
                out.push_str(&rest[..start + 2]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// 소문자 하이픈 UUID 36자 형식.
fn is_lower_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => matches!(b, b'0'..=b'9' | b'a'..=b'f'),
        })
}

/// 푸시 본문. 웹 알림 목록의 문장과 같은 분기. 사진은 `delta`(직전 푸시 이후 새 사진 수).
pub fn body(
    locale: Locale,
    n: &Notification,
    delta: i32,
    recipient: Uuid,
    names: &HashMap<Uuid, String>,
) -> String {
    let name = n.actor.display_name.as_str();
    let unknown = t(locale, "comments.unknownUser", &[]);
    let plain = |s: &str| plain_mentions(s, names, &unknown);
    match n.kind {
        NotificationKind::Upload => {
            if delta > 1 {
                t(
                    locale,
                    "notifications.uploadMany",
                    &[("name", name), ("count", &delta.to_string())],
                )
            } else {
                t(locale, "notifications.uploadOne", &[("name", name)])
            }
        }
        NotificationKind::Comment => {
            let text = plain(n.comment.as_ref().map_or("", |c| &c.excerpt));
            let args = [("name", name), ("text", text.as_str())];
            match &n.asset {
                None => t(locale, "notifications.commentStory", &args),
                Some(a) if a.uploaded_by == recipient => {
                    t(locale, "notifications.commentMyPhoto", &args)
                }
                Some(_) => t(locale, "notifications.commentPhoto", &args),
            }
        }
        NotificationKind::Mention => {
            // 이야기 본문 멘션은 댓글 없이 이야기 발췌.
            let src = n
                .comment
                .as_ref()
                .map(|c| c.excerpt.as_str())
                .or(n.story.as_ref().map(|s| s.excerpt.as_str()))
                .unwrap_or("");
            t(
                locale,
                "notifications.mention",
                &[("name", name), ("text", &plain(src))],
            )
        }
        NotificationKind::Story => {
            let text = plain(n.story.as_ref().map_or("", |s| &s.excerpt));
            t(
                locale,
                "notifications.story",
                &[("name", name), ("text", &text)],
            )
        }
        NotificationKind::Milestone => {
            let label = match &n.milestone {
                Some(m) => match (&m.preset_key, &m.custom_label) {
                    (Some(key), _) => {
                        let path = format!("milestones.presets.{key}");
                        let s = t(locale, &path, &[]);
                        if s == path { key.clone() } else { s }
                    }
                    (None, label) => label.clone().unwrap_or_default(),
                },
                None => String::new(),
            };
            t(
                locale,
                "notifications.milestone",
                &[("name", name), ("label", &label)],
            )
        }
        NotificationKind::Backup => {
            if n.count > 1 {
                t(
                    locale,
                    "notifications.backupFailedMany",
                    &[("count", &n.count.to_string())],
                )
            } else {
                t(locale, "notifications.backupFailed", &[])
            }
        }
    }
}

/// 알림 클릭 이동 주소. 대상 요약이 없으면 `None`.
pub fn url(n: &Notification, delta: i32) -> Option<String> {
    match n.kind {
        NotificationKind::Upload => {
            let a = n.asset.as_ref()?;
            Some(if delta > 1 {
                format!("/days/{}", a.local_date)
            } else {
                format!("/photos/{}", a.id)
            })
        }
        NotificationKind::Comment | NotificationKind::Mention => match (&n.asset, &n.story) {
            (Some(a), _) => Some(format!("/photos/{}?panel=comments", a.id)),
            (None, Some(s)) => Some(format!("/stories/{}", s.id)),
            _ => None,
        },
        NotificationKind::Story => n.story.as_ref().map(|s| format!("/stories/{}", s.id)),
        NotificationKind::Milestone => n
            .milestone
            .as_ref()
            .map(|m| format!("/milestones/{}", m.id)),
        NotificationKind::Backup => Some("/me/backup".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, Utc};
    use serde_json::json;

    use super::*;
    use crate::assets::{AssetKind, AssetStatus, AssetSummary};
    use crate::notifications::{
        NotificationActor, NotificationComment, NotificationMilestone, NotificationStory,
    };

    fn asset(uploaded_by: Uuid) -> AssetSummary {
        AssetSummary {
            id: Uuid::now_v7(),
            kind: AssetKind::Photo,
            status: AssetStatus::Ready,
            taken_at: Utc::now(),
            local_date: NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(),
            width: None,
            height: None,
            duration_ms: None,
            blurhash: None,
            dominant_color: None,
            uploaded_by,
            baby_ids: vec![],
            derivatives: json!({}),
        }
    }

    fn note(kind: NotificationKind) -> Notification {
        Notification {
            id: Uuid::now_v7(),
            kind,
            count: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            actor: NotificationActor {
                id: Uuid::now_v7(),
                display_name: "엄마".into(),
            },
            asset: None,
            story: None,
            milestone: None,
            comment: None,
        }
    }

    fn comment(n: &mut Notification, text: &str) {
        n.comment = Some(NotificationComment {
            excerpt: text.into(),
        });
    }

    #[test]
    fn locale_keys_exist() {
        for locale in [Locale::Ko, Locale::En] {
            for key in [
                "uploadOne",
                "uploadMany",
                "commentMyPhoto",
                "commentPhoto",
                "commentStory",
                "mention",
                "story",
                "milestone",
                "backupFailed",
                "backupFailedMany",
            ] {
                let path = format!("notifications.{key}");
                assert_ne!(t(locale, &path, &[]), path, "{path}");
            }
            assert_ne!(
                t(locale, "comments.unknownUser", &[]),
                "comments.unknownUser"
            );
        }
    }

    #[test]
    fn pick_locale_fallback() {
        assert_eq!(pick_locale("en", "ko"), Locale::En);
        assert_eq!(pick_locale("ja", "en"), Locale::En);
        assert_eq!(pick_locale("ja", "fr"), Locale::Ko);
    }

    #[test]
    fn upload_texts() {
        let me = Uuid::now_v7();
        let mut n = note(NotificationKind::Upload);
        n.asset = Some(asset(me));
        let names = HashMap::new();
        assert_eq!(
            body(Locale::Ko, &n, 12, me, &names),
            "엄마님이 사진 12장을 올렸어요"
        );
        assert_eq!(
            body(Locale::Ko, &n, 1, me, &names),
            "엄마님이 사진을 올렸어요"
        );
        assert_eq!(body(Locale::En, &n, 3, me, &names), "엄마 added 3 photos");
    }

    #[test]
    fn comment_my_photo_vs_other() {
        let owner = Uuid::now_v7();
        let mut n = note(NotificationKind::Comment);
        n.asset = Some(asset(owner));
        comment(&mut n, "귀엽다");
        let names = HashMap::new();
        assert_eq!(
            body(Locale::Ko, &n, 1, owner, &names),
            "엄마님이 회원님 사진에 댓글을 남겼어요: \"귀엽다\""
        );
        assert_eq!(
            body(Locale::Ko, &n, 1, Uuid::now_v7(), &names),
            "엄마님이 댓글을 남겼어요: \"귀엽다\""
        );
    }

    #[test]
    fn mention_names_rendered() {
        let dad = Uuid::now_v7();
        let mut n = note(NotificationKind::Mention);
        n.actor.display_name = "할머니".into();
        comment(&mut n, &format!("<@{dad}> 이 날 기억나?"));
        let names = HashMap::from([(dad, "아빠".to_string())]);
        assert_eq!(
            body(Locale::Ko, &n, 1, Uuid::now_v7(), &names),
            "할머니님이 회원님을 언급했어요: \"@아빠 이 날 기억나?\""
        );
        let out = body(Locale::Ko, &n, 1, Uuid::now_v7(), &HashMap::new());
        assert!(out.contains("@알 수 없는 가족"), "{out}");
    }

    #[test]
    fn story_mention_uses_story_excerpt() {
        let mut n = note(NotificationKind::Mention);
        n.story = Some(NotificationStory {
            id: Uuid::now_v7(),
            excerpt: "오늘은".into(),
            cover: None,
        });
        assert_eq!(
            body(Locale::Ko, &n, 1, Uuid::now_v7(), &HashMap::new()),
            "엄마님이 회원님을 언급했어요: \"오늘은\""
        );
    }

    #[test]
    fn cut_mention_token_dropped() {
        let mut n = note(NotificationKind::Comment);
        comment(&mut n, "보고 싶다 <@1234abcd");
        let out = body(Locale::Ko, &n, 1, Uuid::now_v7(), &HashMap::new());
        assert!(!out.contains("<@"), "{out}");
        assert!(out.contains("보고 싶다"), "{out}");
    }

    #[test]
    fn milestone_label() {
        let who = Uuid::now_v7();
        let mk = |preset: Option<&str>, custom: Option<&str>| {
            let mut n = note(NotificationKind::Milestone);
            n.milestone = Some(NotificationMilestone {
                id: Uuid::now_v7(),
                preset_key: preset.map(Into::into),
                custom_label: custom.map(Into::into),
                cover: None,
            });
            body(Locale::Ko, &n, 1, who, &HashMap::new())
        };
        assert_eq!(
            mk(Some("first_smile"), None),
            "엄마님이 마일스톤을 남겼어요: 첫 웃음"
        );
        assert_eq!(
            mk(Some("no_such_key"), None),
            "엄마님이 마일스톤을 남겼어요: no_such_key"
        );
        assert_eq!(
            mk(None, Some("첫 외출")),
            "엄마님이 마일스톤을 남겼어요: 첫 외출"
        );
    }

    #[test]
    fn backup_texts() {
        let me = Uuid::now_v7();
        let mut n = note(NotificationKind::Backup);
        let names = HashMap::new();
        assert_eq!(
            body(Locale::Ko, &n, 1, me, &names),
            "백업에 실패했어요. 백업 화면에서 확인해 주세요"
        );
        n.count = 3;
        assert_eq!(
            body(Locale::En, &n, 1, me, &names),
            "Backup failed 3 times today. Check the backup screen"
        );
        assert_eq!(url(&n, 1), Some("/me/backup".to_string()));
    }

    #[test]
    fn urls() {
        let me = Uuid::now_v7();
        let a = asset(me);
        let (aid, date) = (a.id, a.local_date);

        let mut up = note(NotificationKind::Upload);
        up.asset = Some(a.clone());
        assert_eq!(url(&up, 1), Some(format!("/photos/{aid}")));
        assert_eq!(url(&up, 2), Some(format!("/days/{date}")));
        up.asset = None;
        assert_eq!(url(&up, 1), None);

        for kind in [NotificationKind::Comment, NotificationKind::Mention] {
            let mut n = note(kind);
            n.asset = Some(a.clone());
            assert_eq!(url(&n, 1), Some(format!("/photos/{aid}?panel=comments")));
            let sid = Uuid::now_v7();
            n.asset = None;
            n.story = Some(NotificationStory {
                id: sid,
                excerpt: String::new(),
                cover: None,
            });
            assert_eq!(url(&n, 1), Some(format!("/stories/{sid}")));
            n.story = None;
            assert_eq!(url(&n, 1), None);
        }

        let sid = Uuid::now_v7();
        let mut st = note(NotificationKind::Story);
        st.story = Some(NotificationStory {
            id: sid,
            excerpt: String::new(),
            cover: None,
        });
        assert_eq!(url(&st, 1), Some(format!("/stories/{sid}")));

        let mid = Uuid::now_v7();
        let mut ms = note(NotificationKind::Milestone);
        ms.milestone = Some(NotificationMilestone {
            id: mid,
            preset_key: None,
            custom_label: None,
            cover: None,
        });
        assert_eq!(url(&ms, 1), Some(format!("/milestones/{mid}")));
    }
}
