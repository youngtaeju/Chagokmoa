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
    jobs::{self, Job},
    mail::{self, MailOutcome},
    request::{Json, Path},
    settings,
    state::AppState,
    users::{self, Role, User},
};

/// 발송 실패 뒤 같은 사용자 재요청 대기. SMTP 장애 중 익명 요청 반복 차단.
const FAILED_RETRY_SECS: i64 = 60;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(create_password_reset))
        .routes(routes!(preview_password_reset))
        .routes(routes!(accept_password_reset))
        .routes(routes!(forgot_password))
}

#[derive(Deserialize, ToSchema)]
pub struct CreatePasswordResetRequest {
    /// 대상 이메일로 링크 발송. 기본 false.
    #[serde(default)]
    send_email: bool,
}

#[derive(Serialize, ToSchema)]
pub struct CreatePasswordResetResponse {
    /// `{PUBLIC_URL}/reset/{원문 토큰}`. 원문 토큰은 이 응답에서만 노출.
    url: String,
    expires_at: DateTime<Utc>,
    /// 메일 발송 결과. 요청 안 했으면 null. 미설정, 대상 이메일 없음, 실패는 false.
    #[schema(required = true)]
    mail_sent: Option<bool>,
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

#[derive(Deserialize, ToSchema)]
pub struct ForgotPasswordRequest {
    /// 아이디 또는 이메일(`@` 포함). 앞뒤 공백 무시, 이메일은 대소문자 무시.
    login: String,
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
    request_body = CreatePasswordResetRequest,
    responses(
        (status = 201, description = "재설정 링크 발급 (이전 링크는 회수)", body = CreatePasswordResetResponse),
        (status = 400, description = "본문 형식 오류, 자기 자신 또는 잘못된 ID 형식", body = ErrorBody),
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
    // 본문 없음(이전 버전 클라이언트)은 `send_email = false`.
    req: Option<Json<CreatePasswordResetRequest>>,
) -> AppResult<(StatusCode, AxumJson<CreatePasswordResetResponse>)> {
    current.require(Role::Owner)?;
    if id == current.user.id {
        return Err(AppError::bad_request("passwordReset.self"));
    }
    let send_email = req.is_some_and(|Json(r)| r.send_email);
    // 생성 뒤 조회 실패로 만든 링크를 잃지 않게 미리.
    let instance_name = if send_email {
        Some(settings::queries::load(&state.pool).await?.instance_name)
    } else {
        None
    };
    let mut tx = state.pool.begin().await?;
    let target = users::queries::find_by_id(&mut *tx, id)
        .await?
        .ok_or(AppError::NotFound("users.notFound"))?;
    if target.disabled_at.is_some() {
        return Err(AppError::conflict("passwordReset.userDisabled"));
    }
    let (raw, expires_at) = queries::create(&mut tx, target.id, current.user.id).await?;
    tx.commit().await?;
    let url = format!("{}/reset/{raw}", state.config.public_url);
    let mail_sent = match (instance_name, target.email.as_deref()) {
        (None, _) => None,
        (Some(_), None) => Some(false),
        (Some(instance), Some(to)) => {
            let text = mail::text::reset(
                &target.locale,
                &instance,
                &target.username,
                &url,
                queries::ADMIN_RESET_HOURS,
            );
            Some(mail::send_now(&state, to, &target.locale, text).await == MailOutcome::Sent)
        }
    };
    Ok((
        StatusCode::CREATED,
        AxumJson(CreatePasswordResetResponse {
            url,
            expires_at,
            mail_sent,
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
    // 셀프, 관리자 링크가 함께 살아 있을 수 있어 남은 링크 전부.
    queries::revoke_for_user(&mut tx, user.id).await?;
    let (session_raw, _) =
        session::create(&mut *tx, user.id, session::user_agent(&headers)).await?;
    tx.commit().await?;

    Ok((
        jar.add(session::cookie(&state.config, session_raw)),
        AxumJson(UserResponse { user }),
    ))
}

#[utoipa::path(
    post,
    path = "/api/auth/forgot-password",
    request_body = ForgotPasswordRequest,
    responses(
        (status = 204, description = "대상 유무, 메일 설정 여부와 무관하게 같은 응답. 대상마다 재설정 메일 작업 등록"),
        (status = 400, description = "본문 형식 오류", body = ErrorBody),
    )
)]
async fn forgot_password(
    State(state): State<AppState>,
    Json(req): Json<ForgotPasswordRequest>,
) -> AppResult<StatusCode> {
    // 대상 판별과 발송은 응답과 분리. 응답으로 계정, 이메일 존재 여부 비노출.
    let login = req.login.trim();
    if login.is_empty() || !mail::is_configured(&state.pool).await? {
        return Ok(StatusCode::NO_CONTENT);
    }
    let candidates = if login.contains('@') {
        users::queries::find_by_email(&state.pool, login).await?
    } else {
        users::queries::find_by_username(&state.pool, login)
            .await?
            .map(|r| r.user)
            .into_iter()
            .collect()
    };
    let mut queued = false;
    for user in candidates {
        if user.disabled_at.is_some()
            || user.email.is_none()
            || queries::self_sent_recently(&state.pool, user.id).await?
            || jobs::reset_mail_pending(&state.pool, user.id).await?
            || jobs::reset_mail_failed_within(&state.pool, user.id, FAILED_RETRY_SECS).await?
        {
            continue;
        }
        jobs::delete_failed_reset_mail(&state.pool, user.id).await?;
        jobs::enqueue(&state.pool, &Job::SendResetMail { user_id: user.id }).await?;
        queued = true;
    }
    if queued {
        state.push_wake.notify_one();
    }
    Ok(StatusCode::NO_CONTENT)
}
