//! 앱 안 알림. 기록은 각 쓰기 지점의 트랜잭션 안에서.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::assets::AssetSummary;

pub mod queries;
pub mod record;
pub mod routes;

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
    /// 백업 실패. 소유자만, 행동자는 받는 사람 자신, 대상 없음.
    Backup,
}

/// 알림 한 줄. 대상 요약은 종류별로 채움.
/// 사진 댓글은 `asset` + `comment`, 이야기 댓글은 `story` + `comment`, 이야기 본문 멘션은 `story` 만.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Notification {
    pub id: Uuid,
    pub kind: NotificationKind,
    /// 묶인 사진 수, 백업은 그날 실패 횟수. 그 밖의 종류는 1.
    pub count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub actor: NotificationActor,
    /// 대상 사진. 사진 묶음은 가장 최근 사진.
    #[schema(required = true)]
    pub asset: Option<AssetSummary>,
    #[schema(required = true)]
    pub story: Option<NotificationStory>,
    #[schema(required = true)]
    pub milestone: Option<NotificationMilestone>,
    #[schema(required = true)]
    pub comment: Option<NotificationComment>,
}

/// 알림을 만든 사용자.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct NotificationActor {
    pub id: Uuid,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct NotificationStory {
    pub id: Uuid,
    /// 본문 앞 100자(문자 수). 멘션 토큰 그대로.
    pub excerpt: String,
    /// 첫 사진. 휴지통 제외.
    #[schema(required = true)]
    pub cover: Option<AssetSummary>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct NotificationMilestone {
    pub id: Uuid,
    /// `preset_key` 와 `custom_label` 중 하나만 값.
    #[schema(required = true)]
    pub preset_key: Option<String>,
    #[schema(required = true)]
    pub custom_label: Option<String>,
    /// 첫 사진. 휴지통 제외.
    #[schema(required = true)]
    pub cover: Option<AssetSummary>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct NotificationComment {
    /// 본문 앞 100자(문자 수). 멘션 토큰 그대로.
    pub excerpt: String,
}

/// 알림 목록 페이지.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct NotificationPage {
    pub items: Vec<Notification>,
    /// 다음 페이지가 없으면 `null`.
    #[schema(required = true)]
    pub next_cursor: Option<String>,
    /// 마지막으로 알림을 본 시각. 본 적 없으면 가입 시각.
    pub seen_at: DateTime<Utc>,
}

/// 안 읽은 알림 수. 100 에서 멈춤.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UnreadCount {
    pub count: i64,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ListQuery {
    /// 이전 응답의 `next_cursor`.
    pub cursor: Option<String>,
    /// 1~100, 기본 30.
    pub limit: Option<i64>,
}
