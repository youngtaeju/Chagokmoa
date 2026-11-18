//! 알림 기록. 모두 호출자 트랜잭션 안에서 실행.
//! 받는 사람 공통: 정지되지 않은 사용자 중 행동자 제외. 멘션 대상은 같은 사건에서 `mention` 한 줄만.

use sqlx::PgConnection;
use uuid::Uuid;

use super::NotificationKind;
use crate::{comments::mention_ids, error::AppResult, settings};

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

/// 사진 알림 묶음 창. SQL interval 문자열.
const UPLOAD_GROUP_WINDOW: &str = "30 minutes";

/// 사진 푸시 발송 지연. 업로드마다 다시 시작. SQL interval 문자열.
pub const PUSH_UPLOAD_DELAY: &str = "2 minutes";

/// 지연 없는 푸시.
const PUSH_NOW: &str = "0 seconds";

/// 사진 업로드. 받는 사람마다 같은 행동자의 창 안 최신 줄이 있으면 count 증가와 대표 사진 교체, 없으면 새 줄.
/// 묶음 대상이 휴지통 사진이어도 갱신(대표가 새 사진으로 교체).
pub async fn record_upload(conn: &mut PgConnection, actor: Uuid, asset_id: Uuid) -> AppResult<()> {
    // 같은 행동자의 동시 finalize 직렬화. 줄 갈라짐 방지.
    sqlx::query!(
        "SELECT pg_advisory_xact_lock(hashtextextended('notifications.upload:' || $1::uuid::text, 0))",
        actor
    )
    .execute(&mut *conn)
    .await?;
    let recipients = active_recipients(conn, actor).await?;
    let grouped = sqlx::query!(
        "WITH latest AS (
           SELECT DISTINCT ON (user_id) id FROM notifications
           WHERE user_id = ANY($1) AND actor_id = $2 AND kind = 'upload'
             AND updated_at > now() - $4::text::interval
           ORDER BY user_id, updated_at DESC, id DESC
         )
         UPDATE notifications n
         SET count = n.count + 1, asset_id = $3, updated_at = now()
         FROM latest WHERE n.id = latest.id
         RETURNING n.id, n.user_id",
        &recipients,
        actor,
        asset_id,
        UPLOAD_GROUP_WINDOW
    )
    .fetch_all(&mut *conn)
    .await?;
    let grouped_users: Vec<Uuid> = grouped.iter().map(|g| g.user_id).collect();
    let grouped_ids: Vec<Uuid> = grouped.iter().map(|g| g.id).collect();
    // 묶음 줄의 대기 중 잡은 2분 연장. 이미 나갔거나 실행 중이면 새 잡.
    let extended = sqlx::query_scalar!(
        r#"UPDATE jobs SET run_after = now() + $2::text::interval
           WHERE kind = 'send_push' AND status = 'queued'
             AND payload->>'notification_id' = ANY($1)
           RETURNING payload->>'notification_id' AS "note!""#,
        &grouped_ids.iter().map(Uuid::to_string).collect::<Vec<_>>(),
        PUSH_UPLOAD_DELAY
    )
    .fetch_all(&mut *conn)
    .await?;
    let unextended: Vec<Uuid> = grouped_ids
        .into_iter()
        .filter(|id| !extended.contains(&id.to_string()))
        .collect();
    enqueue_pushes(conn, &unextended, PUSH_UPLOAD_DELAY).await?;
    let fresh: Vec<Uuid> = recipients
        .into_iter()
        .filter(|u| !grouped_users.contains(u))
        .collect();
    let target = Target {
        asset_id: Some(asset_id),
        ..Target::default()
    };
    insert(
        conn,
        &fresh,
        NotificationKind::Upload,
        actor,
        target,
        PUSH_UPLOAD_DELAY,
    )
    .await
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
    insert(
        conn,
        &mentioned,
        NotificationKind::Mention,
        actor,
        target,
        PUSH_NOW,
    )
    .await?;
    insert(
        conn,
        &rest,
        NotificationKind::Story,
        actor,
        target,
        PUSH_NOW,
    )
    .await
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
        PUSH_NOW,
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
    insert(
        conn,
        &mentioned,
        NotificationKind::Mention,
        actor,
        target,
        PUSH_NOW,
    )
    .await?;
    insert(
        conn,
        &rest,
        NotificationKind::Comment,
        actor,
        target,
        PUSH_NOW,
    )
    .await
}

/// 백업 실패. 행동자는 받는 소유자 자신.
/// 인스턴스 시간대 같은 날 만든 줄이 있으면 count 증가만(푸시는 그날 첫 실패 한 번).
pub async fn record_backup_failed(conn: &mut PgConnection, owners: &[Uuid]) -> AppResult<()> {
    if owners.is_empty() {
        return Ok(());
    }
    let timezone = settings::queries::load(&mut *conn).await?.timezone;
    let grouped = sqlx::query_scalar!(
        "UPDATE notifications SET count = count + 1, updated_at = now()
         WHERE user_id = ANY($1) AND kind = 'backup'
           AND (created_at AT TIME ZONE $2)::date = (now() AT TIME ZONE $2)::date
         RETURNING user_id",
        owners,
        timezone
    )
    .fetch_all(&mut *conn)
    .await?;
    let fresh: Vec<Uuid> = owners
        .iter()
        .copied()
        .filter(|u| !grouped.contains(u))
        .collect();
    if fresh.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = fresh.iter().map(|_| Uuid::now_v7()).collect();
    sqlx::query!(
        "INSERT INTO notifications (id, user_id, kind, actor_id)
         SELECT r.id, r.user_id, 'backup', r.user_id
         FROM unnest($1::uuid[], $2::uuid[]) AS r(id, user_id)",
        &ids,
        &fresh
    )
    .execute(&mut *conn)
    .await?;
    enqueue_pushes(conn, &ids, PUSH_NOW).await
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

/// 받는 사람마다 한 줄과 푸시 잡 하나. `delay` 는 SQL interval 문자열.
async fn insert(
    conn: &mut PgConnection,
    users: &[Uuid],
    kind: NotificationKind,
    actor: Uuid,
    t: Target,
    delay: &str,
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
    enqueue_pushes(conn, &ids, delay).await
}

/// 알림 줄마다 `send_push` 잡 하나. 구독 유무는 발송 때 확인.
async fn enqueue_pushes(conn: &mut PgConnection, notes: &[Uuid], delay: &str) -> AppResult<()> {
    if notes.is_empty() {
        return Ok(());
    }
    let job_ids: Vec<Uuid> = notes.iter().map(|_| Uuid::now_v7()).collect();
    sqlx::query!(
        "INSERT INTO jobs (id, kind, payload, run_after)
         SELECT j.id, 'send_push', jsonb_build_object('notification_id', j.note), now() + $3::text::interval
         FROM unnest($1::uuid[], $2::uuid[]) AS j(id, note)",
        &job_ids,
        notes,
        delay
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}
