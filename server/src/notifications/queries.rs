//! 알림 조회, 읽음, 정리. 언제나 받는 사람 `user_id` 기준.
//! 대상 사진이 휴지통이면 목록과 안 읽은 수 모두 제외.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

use super::{
    Notification, NotificationActor, NotificationComment, NotificationKind, NotificationMilestone,
    NotificationStory,
};
use crate::{
    assets::{AssetKind, AssetStatus, AssetSummary},
    error::AppResult,
    links::{self, LinkKind},
};

/// 이야기 본문, 댓글 발췌 길이(문자 수).
const EXCERPT_CHARS: i32 = 100;
/// 안 읽은 수 상한.
const UNREAD_CAP: i64 = 100;

/// 최신순(`updated_at DESC, id DESC`). `limit` 만큼 가져오며 호출 측이 +1 로 다음 페이지 판정.
pub async fn list(
    conn: &mut PgConnection,
    timezone: &str,
    user_id: Uuid,
    cursor: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> AppResult<Vec<Notification>> {
    let (cursor_ts, cursor_id) = cursor.unzip();
    let rows = sqlx::query!(
        r#"SELECT n.id, n.kind AS "kind: NotificationKind", n.count, n.created_at, n.updated_at,
                  u.id AS actor_id, u.display_name AS actor_name,
                  a.id AS "asset_id?", a.kind AS "asset_kind?: AssetKind",
                  a.status AS "asset_status?: AssetStatus", a.taken_at AS "asset_taken_at?",
                  (a.taken_at AT TIME ZONE $2)::date AS "asset_local_date?",
                  a.width AS "asset_width?", a.height AS "asset_height?",
                  a.duration_ms AS "asset_duration_ms?", a.blurhash AS "asset_blurhash?",
                  a.dominant_color AS "asset_dominant_color?",
                  a.uploaded_by AS "asset_uploaded_by?",
                  CASE WHEN a.id IS NOT NULL THEN
                    COALESCE((SELECT array_agg(b.id ORDER BY b.sort_order, b.created_at, b.id)
                              FROM asset_babies ab JOIN babies b ON b.id = ab.baby_id
                              WHERE ab.asset_id = a.id), '{}')
                  END AS "asset_baby_ids?: Vec<Uuid>",
                  a.derivatives AS "asset_derivatives?",
                  s.id AS "story_id?", left(s.body, $3) AS "story_excerpt?",
                  m.id AS "milestone_id?", m.preset_key AS "preset_key?",
                  m.custom_label AS "custom_label?",
                  left(COALESCE(ac.body, sc.body), $3) AS "comment_excerpt?"
           FROM notifications n
           JOIN users u ON u.id = n.actor_id
           LEFT JOIN assets a ON a.id = n.asset_id
           LEFT JOIN stories s ON s.id = n.story_id
           LEFT JOIN milestones m ON m.id = n.milestone_id
           LEFT JOIN asset_comments ac ON ac.id = n.asset_comment_id
           LEFT JOIN story_comments sc ON sc.id = n.story_comment_id
           WHERE n.user_id = $1
             AND (n.asset_id IS NULL OR a.deleted_at IS NULL)
             AND ($4::timestamptz IS NULL OR (n.updated_at, n.id) < ($4, $5::uuid))
           ORDER BY n.updated_at DESC, n.id DESC
           LIMIT $6"#,
        user_id,
        timezone,
        EXCERPT_CHARS,
        cursor_ts,
        cursor_id,
        limit,
    )
    .fetch_all(&mut *conn)
    .await?;

    let story_ids: Vec<Uuid> = rows.iter().filter_map(|r| r.story_id).collect();
    let milestone_ids: Vec<Uuid> = rows.iter().filter_map(|r| r.milestone_id).collect();
    // 같은 대상의 알림이 여러 줄일 수 있어 소비형 `links::take` 대신 조회용 맵.
    let story_covers = covers(&mut *conn, timezone, LinkKind::Story, &story_ids).await?;
    let milestone_covers =
        covers(&mut *conn, timezone, LinkKind::Milestone, &milestone_ids).await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let asset = (|| {
                Some(AssetSummary {
                    id: r.asset_id?,
                    kind: r.asset_kind?,
                    status: r.asset_status?,
                    taken_at: r.asset_taken_at?,
                    local_date: r.asset_local_date?,
                    width: r.asset_width,
                    height: r.asset_height,
                    duration_ms: r.asset_duration_ms,
                    blurhash: r.asset_blurhash,
                    dominant_color: r.asset_dominant_color,
                    uploaded_by: r.asset_uploaded_by?,
                    baby_ids: r.asset_baby_ids?,
                    derivatives: r.asset_derivatives?,
                })
            })();
            let story = r
                .story_id
                .zip(r.story_excerpt)
                .map(|(id, excerpt)| NotificationStory {
                    id,
                    excerpt,
                    cover: story_covers.get(&id).cloned(),
                });
            let milestone = r.milestone_id.map(|id| NotificationMilestone {
                id,
                preset_key: r.preset_key,
                custom_label: r.custom_label,
                cover: milestone_covers.get(&id).cloned(),
            });
            Notification {
                id: r.id,
                kind: r.kind,
                count: r.count,
                created_at: r.created_at,
                updated_at: r.updated_at,
                actor: NotificationActor {
                    id: r.actor_id,
                    display_name: r.actor_name,
                },
                asset,
                story,
                milestone,
                comment: r
                    .comment_excerpt
                    .map(|excerpt| NotificationComment { excerpt }),
            }
        })
        .collect())
}

/// 대상별 첫 사진. 휴지통 제외.
async fn covers(
    conn: &mut PgConnection,
    timezone: &str,
    kind: LinkKind,
    owner_ids: &[Uuid],
) -> AppResult<HashMap<Uuid, AssetSummary>> {
    if owner_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = links::assets_for(conn, timezone, kind, owner_ids, Some(1)).await?;
    Ok(rows.into_iter().collect())
}

/// 마지막으로 본 시각. 본 적 없으면 가입 시각.
pub async fn seen_at(exec: impl PgExecutor<'_>, user_id: Uuid) -> AppResult<DateTime<Utc>> {
    let seen = sqlx::query_scalar!(
        r#"SELECT COALESCE(notifications_seen_at, created_at) AS "seen_at!"
           FROM users WHERE id = $1"#,
        user_id
    )
    .fetch_one(exec)
    .await?;
    Ok(seen)
}

pub async fn mark_seen(exec: impl PgExecutor<'_>, user_id: Uuid) -> AppResult<()> {
    sqlx::query!(
        "UPDATE users SET notifications_seen_at = now() WHERE id = $1",
        user_id
    )
    .execute(exec)
    .await?;
    Ok(())
}

/// 본 시각 이후 갱신된 표시 대상 줄 수. 상한에서 멈춤.
pub async fn unread_count(exec: impl PgExecutor<'_>, user_id: Uuid) -> AppResult<i64> {
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM (
             SELECT 1 FROM notifications n
             LEFT JOIN assets a ON a.id = n.asset_id
             WHERE n.user_id = $1
               AND n.updated_at > (SELECT COALESCE(notifications_seen_at, created_at)
                                   FROM users WHERE id = $1)
               AND (n.asset_id IS NULL OR a.deleted_at IS NULL)
             LIMIT $2
           ) t"#,
        user_id,
        UNREAD_CAP
    )
    .fetch_one(exec)
    .await?;
    Ok(count)
}

/// `updated_at` 이 `days` 일보다 오래된 줄 삭제. 삭제 수 반환.
pub async fn delete_older_than(exec: impl PgExecutor<'_>, days: i32) -> AppResult<u64> {
    let result = sqlx::query!(
        "DELETE FROM notifications WHERE updated_at < now() - make_interval(days => $1)",
        days
    )
    .execute(exec)
    .await?;
    Ok(result.rows_affected())
}
