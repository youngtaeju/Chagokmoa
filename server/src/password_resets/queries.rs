use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

use super::{PasswordReset, ResetStatus};
use crate::{auth::token, error::AppResult};

/// 관리자 링크 유효 시간. 메일 본문 표기와 SQL 만료 공용.
pub const ADMIN_RESET_HOURS: u32 = 24;
/// 셀프 링크 유효 시간. 메일 링크 관례상 관리자 링크보다 짧게.
pub const SELF_RESET_HOURS: u32 = 1;

/// `now`: DB 시각. 만료 판정을 `expires_at` 과 같은 시계로.
struct ResetRow {
    id: Uuid,
    user_id: Uuid,
    expires_at: DateTime<Utc>,
    used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
}

impl From<ResetRow> for PasswordReset {
    fn from(r: ResetRow) -> Self {
        PasswordReset {
            status: ResetStatus::at(r.expires_at, r.used_at, r.revoked_at, r.now),
            id: r.id,
            user_id: r.user_id,
            expires_at: r.expires_at,
        }
    }
}

/// 살아 있는 링크 전부 회수. 정지, 본인 비밀번호 변경, 새 링크 발급 공용.
pub async fn revoke_for_user(conn: &mut PgConnection, user_id: Uuid) -> AppResult<()> {
    sqlx::query!(
        "UPDATE password_resets SET revoked_at = now()
         WHERE user_id = $1 AND used_at IS NULL AND revoked_at IS NULL",
        user_id,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// 같은 사람의 살아 있는 링크 전부 회수 후 `ADMIN_RESET_HOURS` 링크 발급. 호출자 트랜잭션 안에서 실행.
/// 원문 토큰과 만료 시각 반환.
pub async fn create(
    conn: &mut PgConnection,
    user_id: Uuid,
    created_by: Uuid,
) -> AppResult<(String, DateTime<Utc>)> {
    revoke_for_user(&mut *conn, user_id).await?;
    let (_, raw, expires_at) = insert(conn, user_id, created_by, ADMIN_RESET_HOURS).await?;
    Ok((raw, expires_at))
}

/// 셀프 링크(`created_by = user_id`) 발급. 회수 없음, 이전 셀프 링크는 발송 성공 후 `revoke_self_except`.
/// (행 ID, 원문 토큰, 만료 시각).
pub async fn create_self(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> AppResult<(Uuid, String, DateTime<Utc>)> {
    insert(conn, user_id, user_id, SELF_RESET_HOURS).await
}

/// DB 에는 SHA-256 해시만 저장.
async fn insert(
    conn: &mut PgConnection,
    user_id: Uuid,
    created_by: Uuid,
    hours: u32,
) -> AppResult<(Uuid, String, DateTime<Utc>)> {
    let (raw, token_hash) = token::generate();
    let row = sqlx::query!(
        r#"INSERT INTO password_resets (id, user_id, token_hash, created_by, expires_at)
           VALUES ($1, $2, $3, $4, now() + make_interval(hours => $5))
           RETURNING id, expires_at"#,
        Uuid::now_v7(),
        user_id,
        token_hash,
        created_by,
        hours as i32,
    )
    .fetch_one(conn)
    .await?;
    Ok((row.id, raw, row.expires_at))
}

/// 같은 사람의 살아 있는 셀프 링크 중 `keep` 제외 회수. 관리자 링크는 유지.
pub async fn revoke_self_except(
    exec: impl PgExecutor<'_>,
    user_id: Uuid,
    keep: Uuid,
) -> AppResult<()> {
    sqlx::query!(
        "UPDATE password_resets SET revoked_at = now()
         WHERE user_id = $1 AND created_by = $1 AND id <> $2
           AND used_at IS NULL AND revoked_at IS NULL",
        user_id,
        keep,
    )
    .execute(exec)
    .await?;
    Ok(())
}

/// 셀프 재설정 대기 시간 15분 안의 셀프 행 존재 여부. 메일 폭탄 방지.
/// 발송 실패 행은 삭제되므로 성공한 발송만 대상.
pub async fn self_sent_recently(exec: impl PgExecutor<'_>, user_id: Uuid) -> AppResult<bool> {
    let recent = sqlx::query_scalar!(
        r#"SELECT EXISTS(
               SELECT 1 FROM password_resets
               WHERE user_id = $1 AND created_by = $1
                 AND created_at > now() - interval '15 minutes'
           ) AS "recent!""#,
        user_id,
    )
    .fetch_one(exec)
    .await?;
    Ok(recent)
}

pub async fn delete(exec: impl PgExecutor<'_>, id: Uuid) -> AppResult<()> {
    sqlx::query!("DELETE FROM password_resets WHERE id = $1", id)
        .execute(exec)
        .await?;
    Ok(())
}

pub async fn find_by_raw(conn: &mut PgConnection, raw: &str) -> AppResult<Option<PasswordReset>> {
    let row = sqlx::query_as!(
        ResetRow,
        r#"SELECT id, user_id, expires_at, used_at, revoked_at, now() AS "now!"
           FROM password_resets WHERE token_hash = $1"#,
        token::hash(raw),
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(Into::into))
}

/// 행 잠금 조회. 트랜잭션 내 사용 경합 차단용.
pub async fn find_for_update(
    conn: &mut PgConnection,
    id: Uuid,
) -> AppResult<Option<PasswordReset>> {
    let row = sqlx::query_as!(
        ResetRow,
        r#"SELECT id, user_id, expires_at, used_at, revoked_at, now() AS "now!"
           FROM password_resets WHERE id = $1 FOR UPDATE"#,
        id,
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(Into::into))
}

pub async fn mark_used(conn: &mut PgConnection, id: Uuid) -> AppResult<()> {
    sqlx::query!(
        "UPDATE password_resets SET used_at = now() WHERE id = $1",
        id
    )
    .execute(conn)
    .await?;
    Ok(())
}
