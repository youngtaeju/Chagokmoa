//! 앱 안 알림. 기록은 각 쓰기 지점의 트랜잭션 안에서.

use serde::Serialize;
use utoipa::ToSchema;

pub mod record;

/// 알림 종류. DB 는 `text` 소문자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, ToSchema)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum NotificationKind {
    Upload,
    Comment,
    Mention,
    Story,
    Milestone,
}
