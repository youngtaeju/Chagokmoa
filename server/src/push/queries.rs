//! 푸시 구독과 종류별 끔 설정.

use sqlx::{PgConnection, PgExecutor, PgPool};
use uuid::Uuid;

use crate::{error::AppResult, notifications::NotificationKind};

/// 발송 대상 구독 한 건.
#[derive(Debug, Clone)]
pub struct Subscription {
    pub session_id: Uuid,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

/// 세션당 1건, 주소는 전체에서 유일. 같은 세션이나 같은 주소의 옛 행은 교체.
pub async fn replace(
    pool: &PgPool,
    session_id: Uuid,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    // 같은 주소 동시 요청 직렬화. 세션 충돌은 upsert 가 처리.
    sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", endpoint)
        .execute(&mut *tx)
        .await?;
    sqlx::query!(
        "DELETE FROM push_subscriptions WHERE endpoint = $1 AND session_id <> $2",
        endpoint,
        session_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO push_subscriptions (session_id, endpoint, p256dh, auth)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (session_id) DO UPDATE
         SET endpoint = EXCLUDED.endpoint, p256dh = EXCLUDED.p256dh,
             auth = EXCLUDED.auth, created_at = now()",
        session_id,
        endpoint,
        p256dh,
        auth
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn delete_by_session(exec: impl PgExecutor<'_>, session_id: Uuid) -> AppResult<()> {
    sqlx::query!(
        "DELETE FROM push_subscriptions WHERE session_id = $1",
        session_id
    )
    .execute(exec)
    .await?;
    Ok(())
}

/// 만료 응답(404/410) 정리용.
pub async fn delete_by_endpoint(exec: impl PgExecutor<'_>, endpoint: &str) -> AppResult<()> {
    sqlx::query!(
        "DELETE FROM push_subscriptions WHERE endpoint = $1",
        endpoint
    )
    .execute(exec)
    .await?;
    Ok(())
}

pub async fn endpoint_of_session(
    exec: impl PgExecutor<'_>,
    session_id: Uuid,
) -> AppResult<Option<String>> {
    Ok(sqlx::query_scalar!(
        "SELECT endpoint FROM push_subscriptions WHERE session_id = $1",
        session_id
    )
    .fetch_optional(exec)
    .await?)
}

/// 사용자의 구독. 만료된 세션 제외.
pub async fn endpoints(exec: impl PgExecutor<'_>, user_id: Uuid) -> AppResult<Vec<Subscription>> {
    Ok(sqlx::query_as!(
        Subscription,
        "SELECT p.session_id, p.endpoint, p.p256dh, p.auth
         FROM push_subscriptions p JOIN sessions s ON s.id = p.session_id
         WHERE s.user_id = $1 AND s.expires_at > now()
         ORDER BY p.created_at, p.session_id",
        user_id
    )
    .fetch_all(exec)
    .await?)
}

/// 끈 종류. 행이 없으면 켜짐.
pub async fn muted(exec: impl PgExecutor<'_>, user_id: Uuid) -> AppResult<Vec<NotificationKind>> {
    Ok(sqlx::query_scalar!(
        r#"SELECT kind AS "kind: NotificationKind" FROM push_mutes WHERE user_id = $1"#,
        user_id
    )
    .fetch_all(exec)
    .await?)
}

pub async fn set_muted(
    conn: &mut PgConnection,
    user_id: Uuid,
    kind: NotificationKind,
    mute: bool,
) -> AppResult<()> {
    if mute {
        sqlx::query!(
            "INSERT INTO push_mutes (user_id, kind) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            user_id,
            kind as NotificationKind
        )
        .execute(conn)
        .await?;
    } else {
        sqlx::query!(
            "DELETE FROM push_mutes WHERE user_id = $1 AND kind = $2",
            user_id,
            kind as NotificationKind
        )
        .execute(conn)
        .await?;
    }
    Ok(())
}
