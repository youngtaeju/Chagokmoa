use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::{PasswordReset, ResetStatus};
use crate::{auth::token, error::AppResult};

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

/// 같은 사람의 살아 있는 링크 회수 후 24시간 링크 발급. 호출자 트랜잭션 안에서 실행.
/// 원문 토큰과 만료 시각 반환. DB 에는 SHA-256 해시만 저장.
pub async fn create(
    conn: &mut PgConnection,
    user_id: Uuid,
    created_by: Uuid,
) -> AppResult<(String, DateTime<Utc>)> {
    revoke_for_user(&mut *conn, user_id).await?;
    let (raw, token_hash) = token::generate();
    let expires_at = sqlx::query_scalar!(
        r#"INSERT INTO password_resets (id, user_id, token_hash, created_by, expires_at)
           VALUES ($1, $2, $3, $4, now() + interval '24 hours')
           RETURNING expires_at"#,
        Uuid::now_v7(),
        user_id,
        token_hash,
        created_by,
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok((raw, expires_at))
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
