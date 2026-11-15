//! 알림 기록. 모두 호출자 트랜잭션 안에서 실행.
//! 받는 사람 공통: 정지되지 않은 사용자 중 행동자 제외. 멘션 대상은 같은 사건에서 `mention` 한 줄만.

use sqlx::PgConnection;
use uuid::Uuid;

use super::NotificationKind;
use crate::{comments::mention_ids, error::AppResult};

/// 댓글 알림 대상. 댓글 id 와 그 사진 또는 이야기 id.
pub enum CommentTarget {
    Asset { asset_id: Uuid, comment_id: Uuid },
    Story { story_id: Uuid, comment_id: Uuid },
}

/// 알림 줄의 대상 id. 종류별 조합은 테이블 CHECK 기준.
#[derive(Default, Clone, Copy)]
struct Target {
    asset_id: Option<Uuid>,
    story_id: Option<Uuid>,
    milestone_id: Option<Uuid>,
    asset_comment_id: Option<Uuid>,
    story_comment_id: Option<Uuid>,
}

/// 이야기 작성. 본문 멘션 대상은 `mention`, 나머지는 `story`.
pub async fn record_story(
    conn: &mut PgConnection,
    actor: Uuid,
    story_id: Uuid,
    body: &str,
) -> AppResult<()> {
    let (mentioned, rest) = split_mentions(active_recipients(conn, actor).await?, body);
    let target = Target {
        story_id: Some(story_id),
        ..Target::default()
    };
    insert(conn, &mentioned, NotificationKind::Mention, actor, target).await?;
    insert(conn, &rest, NotificationKind::Story, actor, target).await
}

/// 마일스톤 작성. 만든 사람 빼고 전원.
pub async fn record_milestone(
    conn: &mut PgConnection,
    actor: Uuid,
    milestone_id: Uuid,
) -> AppResult<()> {
    let recipients = active_recipients(conn, actor).await?;
    let target = Target {
        milestone_id: Some(milestone_id),
        ..Target::default()
    };
    insert(
        conn,
        &recipients,
        NotificationKind::Milestone,
        actor,
        target,
    )
    .await
}

/// 댓글 작성. 멘션 대상은 `mention`, 사진 올린 사람 또는 이야기 쓴 사람과 앞선 댓글 작성자는 `comment`.
pub async fn record_comment(
    conn: &mut PgConnection,
    actor: Uuid,
    target: CommentTarget,
    body: &str,
) -> AppResult<()> {
    let (mentioned, rest) = split_mentions(active_recipients(conn, actor).await?, body);
    let (followers, target) = match target {
        CommentTarget::Asset {
            asset_id,
            comment_id,
        } => {
            let followers = sqlx::query_scalar!(
                r#"SELECT uploaded_by AS "id!" FROM assets WHERE id = $1
                   UNION
                   SELECT author_id FROM asset_comments WHERE asset_id = $1 AND id <> $2"#,
                asset_id,
                comment_id
            )
            .fetch_all(&mut *conn)
            .await?;
            let target = Target {
                asset_id: Some(asset_id),
                asset_comment_id: Some(comment_id),
                ..Target::default()
            };
            (followers, target)
        }
        CommentTarget::Story {
            story_id,
            comment_id,
        } => {
            let followers = sqlx::query_scalar!(
                r#"SELECT author_id AS "id!" FROM stories WHERE id = $1
                   UNION
                   SELECT author_id FROM story_comments WHERE story_id = $1 AND id <> $2"#,
                story_id,
                comment_id
            )
            .fetch_all(&mut *conn)
            .await?;
            let target = Target {
                story_id: Some(story_id),
                story_comment_id: Some(comment_id),
                ..Target::default()
            };
            (followers, target)
        }
    };
    let rest: Vec<Uuid> = rest.into_iter().filter(|u| followers.contains(u)).collect();
    insert(conn, &mentioned, NotificationKind::Mention, actor, target).await?;
    insert(conn, &rest, NotificationKind::Comment, actor, target).await
}

/// 정지되지 않은 사용자 중 행동자 제외. 모든 종류 공용.
async fn active_recipients(conn: &mut PgConnection, actor: Uuid) -> AppResult<Vec<Uuid>> {
    let ids = sqlx::query_scalar!(
        "SELECT id FROM users WHERE disabled_at IS NULL AND id <> $1",
        actor
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(ids)
}

/// (본문 멘션 대상, 나머지).
fn split_mentions(recipients: Vec<Uuid>, body: &str) -> (Vec<Uuid>, Vec<Uuid>) {
    // mention_ids 결과는 정렬, 중복 제거 상태. binary_search 전제.
    let mentions = mention_ids(body);
    recipients
        .into_iter()
        .partition(|u| mentions.binary_search(u).is_ok())
}

/// 받는 사람마다 한 줄.
async fn insert(
    conn: &mut PgConnection,
    users: &[Uuid],
    kind: NotificationKind,
    actor: Uuid,
    t: Target,
) -> AppResult<()> {
    if users.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = users.iter().map(|_| Uuid::now_v7()).collect();
    sqlx::query!(
        "INSERT INTO notifications
           (id, user_id, kind, actor_id, asset_id, story_id, milestone_id,
            asset_comment_id, story_comment_id)
         SELECT r.id, r.user_id, $3::text, $4::uuid, $5::uuid, $6::uuid, $7::uuid, $8::uuid, $9::uuid
         FROM unnest($1::uuid[], $2::uuid[]) AS r(id, user_id)",
        &ids,
        users,
        kind as _,
        actor,
        t.asset_id,
        t.story_id,
        t.milestone_id,
        t.asset_comment_id,
        t.story_comment_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}
