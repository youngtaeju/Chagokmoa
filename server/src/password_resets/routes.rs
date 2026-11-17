use axum::{
    Json as AxumJson,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use axum_extra::extract::CookieJar;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::{PasswordReset, queries};
use crate::{
    auth::{
        extract::CurrentUser, password, routes::UserResponse, session, validate::validate_password,
    },
    error::{AppError, AppResult, ErrorBody},
    request::{Json, Path},
    state::AppState,
    users::{self, Role, User},
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(create_password_reset))
        .routes(routes!(preview_password_reset))
        .routes(routes!(accept_password_reset))
}

#[derive(Serialize, ToSchema)]
pub struct CreatePasswordResetResponse {
    /// `{PUBLIC_URL}/reset/{원문 토큰}`. 원문 토큰은 이 응답에서만 노출.
    url: String,
    expires_at: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
pub struct PasswordResetPreview {
    display_name: String,
    username: String,
    expires_at: DateTime<Utc>,
}

#[derive(Deserialize, ToSchema)]
pub struct AcceptPasswordResetRequest {
    password: String,
}

/// 원문 토큰으로 링크와 대상 조회 후 사용 가능 여부 확인. 대상 정지 시 회수와 같은 410.
async fn find_active(conn: &mut sqlx::PgConnection, raw: &str) -> AppResult<(PasswordReset, User)> {
    let reset = queries::find_by_raw(conn, raw)
        .await?
        .ok_or(AppError::NotFound("passwordReset.notFound"))?;
    reset.ensure_active()?;
    let user = users::queries::find_by_id(&mut *conn, reset.user_id)
        .await?
        .ok_or(AppError::NotFound("passwordReset.notFound"))?;
    if user.disabled_at.is_some() {
        return Err(AppError::Gone("passwordReset.revoked"));
    }
    Ok((reset, user))
}

#[utoipa::path(
    post,
    path = "/api/users/{id}/password-reset",
    params(("id" = Uuid, Path, description = "대상 사용자 ID")),
    responses(
        (status = 201, description = "재설정 링크 발급 (이전 링크는 회수)", body = CreatePasswordResetResponse),
        (status = 400, description = "자기 자신 또는 잘못된 ID 형식", body = ErrorBody),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
        (status = 404, description = "사용자 없음", body = ErrorBody),
        (status = 409, description = "정지된 사용자", body = ErrorBody),
    )
)]
async fn create_password_reset(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> AppResult<(StatusCode, AxumJson<CreatePasswordResetResponse>)> {
    current.require(Role::Owner)?;
    if id == current.user.id {
        return Err(AppError::bad_request("passwordReset.self"));
    }
    let mut tx = state.pool.begin().await?;
    let target = users::queries::find_by_id(&mut *tx, id)
        .await?
        .ok_or(AppError::NotFound("users.notFound"))?;
    if target.disabled_at.is_some() {
        return Err(AppError::conflict("passwordReset.userDisabled"));
    }
    let (raw, expires_at) = queries::create(&mut tx, target.id, current.user.id).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        AxumJson(CreatePasswordResetResponse {
            url: format!("{}/reset/{raw}", state.config.public_url),
            expires_at,
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/api/password-resets/{raw}/preview",
    params(("raw" = String, Path, description = "재설정 원문 토큰")),
    responses(
        (status = 200, description = "재설정 화면용 대상 정보", body = PasswordResetPreview),
        (status = 404, description = "링크 없음", body = ErrorBody),
        (status = 410, description = "만료, 사용됨, 회수됨 (대상 정지 포함)", body = ErrorBody),
    )
)]
async fn preview_password_reset(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> AppResult<AxumJson<PasswordResetPreview>> {
    let mut conn = state.pool.acquire().await?;
    let (reset, user) = find_active(&mut conn, &raw).await?;
    Ok(AxumJson(PasswordResetPreview {
        display_name: user.display_name,
        username: user.username,
        expires_at: reset.expires_at,
    }))
}

#[utoipa::path(
    post,
    path = "/api/password-resets/{raw}/accept",
    params(("raw" = String, Path, description = "재설정 원문 토큰")),
    request_body = AcceptPasswordResetRequest,
    responses(
        (status = 200, description = "비밀번호 변경, 다른 세션 전부 삭제, 새 세션 쿠키 발급", body = UserResponse),
        (status = 400, description = "비밀번호 검증 실패 (링크는 유지)", body = ErrorBody),
        (status = 404, description = "링크 없음", body = ErrorBody),
        (status = 410, description = "만료, 사용됨, 회수됨 (대상 정지 포함)", body = ErrorBody),
    )
)]
async fn accept_password_reset(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Path(raw): Path<String>,
    Json(req): Json<AcceptPasswordResetRequest>,
) -> AppResult<(CookieJar, AxumJson<UserResponse>)> {
    validate_password(&req.password)?;

    // 무효 링크의 해시 비용 회피. 확정 검사는 트랜잭션 내 행 잠금 후.
    let (reset, _) = find_active(&mut *state.pool.acquire().await?, &raw).await?;
    let password_hash = password::hash_async(req.password).await?;

    let mut tx = state.pool.begin().await?;
    let locked = queries::find_for_update(&mut tx, reset.id)
        .await?
        .ok_or(AppError::NotFound("passwordReset.notFound"))?;
    locked.ensure_active()?;
    let user = users::queries::find_by_id(&mut *tx, locked.user_id)
        .await?
        .ok_or(AppError::NotFound("passwordReset.notFound"))?;
    if user.disabled_at.is_some() {
        return Err(AppError::Gone("passwordReset.revoked"));
    }
    users::queries::update_password(&mut *tx, user.id, &password_hash).await?;
    session::delete_all_for_user(&mut *tx, user.id).await?;
    queries::mark_used(&mut tx, locked.id).await?;
    let (session_raw, _) =
        session::create(&mut *tx, user.id, session::user_agent(&headers)).await?;
    tx.commit().await?;

    Ok((
        jar.add(session::cookie(&state.config, session_raw)),
        AxumJson(UserResponse { user }),
    ))
}
