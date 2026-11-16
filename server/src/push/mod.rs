//! 웹푸시. VAPID 키와 발송.

use crate::notifications::NotificationKind;

pub mod text;
pub mod vapid;

/// 설정 화면 표시 순서.
pub const PUSH_KINDS: [NotificationKind; 5] = [
    NotificationKind::Upload,
    NotificationKind::Comment,
    NotificationKind::Mention,
    NotificationKind::Story,
    NotificationKind::Milestone,
];
