use sqlx::PgExecutor;
use uuid::Uuid;

use super::{NewUser, ProfileUpdate, Role, User, UserRecord};
use crate::error::{AppError, AppResult};

pub async fn count(exec: impl PgExecutor<'_>) -> AppResult<i64> {
    let n = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM users"#)
        .fetch_one(exec)
        .await?;
    Ok(n)
}

pub async fn create(exec: impl PgExecutor<'_>, new: NewUser) -> AppResult<User> {
    sqlx::query_as!(
        User,
        r#"INSERT INTO users (id, username, password_hash, display_name, role, email, locale)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id, username, display_name, role AS "role: Role", email, locale,
                     created_at, disabled_at"#,
        Uuid::now_v7(),
        new.username,
        new.password_hash,
        new.display_name,
        new.role as Role,
        new.email,
        new.locale,
    )
    .fetch_one(exec)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.constraint() == Some("users_username_lower_idx") => {
            AppError::conflict("auth.usernameTaken")
        }
        _ => AppError::Db(e),
    })
}

pub async fn find_by_username(
    exec: impl PgExecutor<'_>,
    username: &str,
) -> AppResult<Option<UserRecord>> {
    let row = sqlx::query!(
        r#"SELECT id, username, password_hash, display_name, role AS "role: Role", email,
                  locale, created_at, disabled_at
           FROM users WHERE lower(username) = lower($1)"#,
        username,
    )
    .fetch_optional(exec)
    .await?;
    Ok(row.map(|r| UserRecord {
        user: User {
            id: r.id,
            username: r.username,
            display_name: r.display_name,
            role: r.role,
            email: r.email,
            locale: r.locale,
            created_at: r.created_at,
            disabled_at: r.disabled_at,
        },
        password_hash: r.password_hash,
    }))
}

pub async fn find_by_id(exec: impl PgExecutor<'_>, id: Uuid) -> AppResult<Option<User>> {
    let user = sqlx::query_as!(
        User,
        r#"SELECT id, username, display_name, role AS "role: Role", email, locale,
                  created_at, disabled_at
           FROM users WHERE id = $1"#,
        id,
    )
    .fetch_optional(exec)
    .await?;
    Ok(user)
}

/// 대소문자 무시 일치. 이메일은 사용자 간 유일하지 않음.
pub async fn find_by_email(exec: impl PgExecutor<'_>, email: &str) -> AppResult<Vec<User>> {
    let users = sqlx::query_as!(
        User,
        r#"SELECT id, username, display_name, role AS "role: Role", email, locale,
                  created_at, disabled_at
           FROM users WHERE lower(email) = lower($1) ORDER BY created_at, id"#,
        email,
    )
    .fetch_all(exec)
    .await?;
    Ok(users)
}

pub async fn list(exec: impl PgExecutor<'_>) -> AppResult<Vec<User>> {
    let users = sqlx::query_as!(
        User,
        r#"SELECT id, username, display_name, role AS "role: Role", email, locale,
                  created_at, disabled_at
           FROM users ORDER BY created_at, id"#,
    )
    .fetch_all(exec)
    .await?;
    Ok(users)
}

/// 갱신 여부 반환(대상 없음 → false).
pub async fn update_role(exec: impl PgExecutor<'_>, id: Uuid, role: Role) -> AppResult<bool> {
    let result = sqlx::query!("UPDATE users SET role = $2 WHERE id = $1", id, role as Role)
        .execute(exec)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// 비활성화 시 최초 시각 유지. 갱신 여부 반환(대상 없음 → false).
pub async fn set_disabled(exec: impl PgExecutor<'_>, id: Uuid, disabled: bool) -> AppResult<bool> {
    let result = sqlx::query!(
        "UPDATE users
         SET disabled_at = CASE WHEN $2 THEN COALESCE(disabled_at, now()) ELSE NULL END
         WHERE id = $1",
        id,
        disabled,
    )
    .execute(exec)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// `None` 필드는 유지. 대상 없음 → `None`.
pub async fn update_profile(
    exec: impl PgExecutor<'_>,
    id: Uuid,
    update: ProfileUpdate,
) -> AppResult<Option<User>> {
    let email_set = update.email.is_some();
    let email = update.email.flatten();
    let user = sqlx::query_as!(
        User,
        r#"UPDATE users
           SET display_name = COALESCE($2, display_name),
               locale = COALESCE($3, locale),
               email = CASE WHEN $4 THEN $5 ELSE email END
           WHERE id = $1
           RETURNING id, username, display_name, role AS "role: Role", email, locale,
                     created_at, disabled_at"#,
        id,
        update.display_name,
        update.locale,
        email_set,
        email,
    )
    .fetch_optional(exec)
    .await?;
    Ok(user)
}

/// 사용자 없음 → `None`, 비밀번호 없음 → `Some(None)`.
pub async fn password_hash(
    exec: impl PgExecutor<'_>,
    id: Uuid,
) -> AppResult<Option<Option<String>>> {
    let hash = sqlx::query_scalar!("SELECT password_hash FROM users WHERE id = $1", id)
        .fetch_optional(exec)
        .await?;
    Ok(hash)
}

/// 비밀번호 유무와 저장된 OIDC issuer. 사용자 없음 → `None`.
pub async fn login_method(
    exec: impl PgExecutor<'_>,
    id: Uuid,
) -> AppResult<Option<(bool, Option<String>)>> {
    let row = sqlx::query!(
        r#"SELECT password_hash IS NOT NULL AS "has_password!", oidc_issuer
           FROM users WHERE id = $1"#,
        id,
    )
    .fetch_optional(exec)
    .await?;
    Ok(row.map(|r| (r.has_password, r.oidc_issuer)))
}

/// `issuer` 로 연결된 사용자 수와 그중 비밀번호 없는 수. 정지 사용자 포함.
pub async fn oidc_link_counts(exec: impl PgExecutor<'_>, issuer: &str) -> AppResult<(i64, i64)> {
    let row = sqlx::query!(
        r#"SELECT count(*) AS "linked!",
                  count(*) FILTER (WHERE password_hash IS NULL) AS "passwordless!"
           FROM users WHERE oidc_issuer = $1"#,
        issuer,
    )
    .fetch_one(exec)
    .await?;
    Ok((row.linked, row.passwordless))
}

/// IdP 계정(issuer + sub)으로 연결된 사용자.
pub async fn find_by_oidc(
    exec: impl PgExecutor<'_>,
    issuer: &str,
    subject: &str,
) -> AppResult<Option<User>> {
    let user = sqlx::query_as!(
        User,
        r#"SELECT id, username, display_name, role AS "role: Role", email, locale,
                  created_at, disabled_at
           FROM users WHERE oidc_issuer = $1 AND oidc_subject = $2"#,
        issuer,
        subject,
    )
    .fetch_optional(exec)
    .await?;
    Ok(user)
}

/// 기존 연결은 덮어씀. 같은 IdP 계정이 다른 사용자에 연결돼 있으면 `oidc.alreadyLinked`.
pub async fn set_oidc_link(
    exec: impl PgExecutor<'_>,
    id: Uuid,
    issuer: &str,
    subject: &str,
) -> AppResult<()> {
    sqlx::query!(
        "UPDATE users SET oidc_issuer = $2, oidc_subject = $3 WHERE id = $1",
        id,
        issuer,
        subject,
    )
    .execute(exec)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.constraint() == Some("users_oidc_link_idx") => {
            AppError::conflict("oidc.alreadyLinked")
        }
        _ => AppError::Db(e),
    })?;
    Ok(())
}

/// 비밀번호 있는 계정만 해제. 해제(또는 원래 연결 없음) 여부 반환. 비밀번호 없으면 false.
pub async fn clear_oidc_link(exec: impl PgExecutor<'_>, id: Uuid) -> AppResult<bool> {
    let result = sqlx::query!(
        "UPDATE users SET oidc_issuer = NULL, oidc_subject = NULL
         WHERE id = $1 AND password_hash IS NOT NULL",
        id,
    )
    .execute(exec)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_password(
    exec: impl PgExecutor<'_>,
    id: Uuid,
    password_hash: &str,
) -> AppResult<()> {
    sqlx::query!(
        "UPDATE users SET password_hash = $2 WHERE id = $1",
        id,
        password_hash,
    )
    .execute(exec)
    .await?;
    Ok(())
}
