//! 메일 설정과 테스트 발송 API. 모두 owner. 응답에 SMTP 비밀번호 없음(설정 여부만).

use axum::{Json as AxumJson, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::{
    MailError, Outgoing,
    settings::{self, MailConfig, MailSecurity},
};
use crate::{
    auth::extract::CurrentUser,
    error::{AppError, AppResult, ErrorBody},
    request::Json,
    state::AppState,
    users::Role,
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            get_mail_settings,
            update_mail_settings,
            delete_mail_settings
        ))
        .routes(routes!(send_test_mail))
}

#[derive(Serialize, ToSchema)]
pub struct MailSettingsResponse {
    /// 미설정이면 null.
    #[schema(required = true)]
    config: Option<MailConfigView>,
}

#[derive(Serialize, ToSchema)]
pub struct MailConfigView {
    host: String,
    port: u16,
    security: MailSecurity,
    /// 없으면 인증 없음.
    #[schema(required = true)]
    username: Option<String>,
    /// 비밀번호 저장됨. 값은 응답하지 않음.
    password_set: bool,
    from_address: String,
}

impl From<&MailConfig> for MailConfigView {
    fn from(c: &MailConfig) -> Self {
        Self {
            host: c.host.clone(),
            port: c.port,
            security: c.security,
            username: c.username.clone(),
            password_set: c.password.is_some(),
            from_address: c.from_address.clone(),
        }
    }
}

/// 문자열은 앞뒤 공백 제거.
#[derive(Deserialize, ToSchema)]
pub struct MailSettingsRequest {
    host: String,
    /// 1~65535.
    port: u16,
    security: MailSecurity,
    /// 비었거나 없으면 인증 없음. 저장된 비밀번호도 삭제.
    username: Option<String>,
    /// 비었거나 없으면 저장된 값 유지.
    password: Option<String>,
    from_address: String,
}

#[derive(Deserialize, ToSchema)]
pub struct TestMailRequest {
    /// 받는 주소.
    to: String,
}

/// 앞뒤 공백 제거 후 빈 값은 None.
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// 테스트 메일 제목과 본문. `en` 아니면 ko.
fn test_text(locale: &str) -> (&'static str, &'static str) {
    if locale == "en" {
        (
            "Test email",
            "Your Chagokmoa mail settings work.\nThis test email was sent from the mail settings screen.",
        )
    } else {
        (
            "테스트 메일",
            "차곡모아 메일 설정이 잘 동작해요.\n메일 설정 화면에서 보낸 테스트 메일이에요.",
        )
    }
}

#[utoipa::path(
    get,
    path = "/api/mail",
    responses(
        (status = 200, description = "메일 설정. 비밀번호는 설정 여부만", body = MailSettingsResponse),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님", body = ErrorBody),
    )
)]
async fn get_mail_settings(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<AxumJson<MailSettingsResponse>> {
    current.require(Role::Owner)?;
    let config = settings::load(&state.pool).await?;
    Ok(AxumJson(MailSettingsResponse {
        config: config.as_ref().map(MailConfigView::from),
    }))
}

#[utoipa::path(
    put,
    path = "/api/mail",
    request_body = MailSettingsRequest,
    responses(
        (status = 200, description = "저장 후 설정. 연결 시험은 하지 않음", body = MailSettingsResponse),
        (status = 400, description = "본문 형식 오류, `mail.hostRequired`, `mail.invalidPort`, `mail.invalidAddress`", body = ErrorBody),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
    )
)]
async fn update_mail_settings(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(req): Json<MailSettingsRequest>,
) -> AppResult<AxumJson<MailSettingsResponse>> {
    current.require(Role::Owner)?;
    let host = req.host.trim();
    if host.is_empty() {
        return Err(AppError::bad_request("mail.hostRequired"));
    }
    if req.port == 0 {
        return Err(AppError::bad_request("mail.invalidPort"));
    }
    let from_address = req.from_address.trim();
    if !super::valid_address(from_address) {
        return Err(AppError::bad_request("mail.invalidAddress"));
    }
    let username = non_empty(req.username.as_deref());
    let new_password = non_empty(req.password.as_deref());

    let mut tx = state.pool.begin().await?;
    let password = match (username, new_password) {
        (None, _) => None,
        (Some(_), Some(p)) => Some(p.to_owned()),
        (Some(_), None) => settings::load(&mut *tx).await?.and_then(|c| c.password),
    };
    let config = MailConfig {
        host: host.to_owned(),
        port: req.port,
        security: req.security,
        username: username.map(str::to_owned),
        password,
        from_address: from_address.to_owned(),
    };
    settings::save(&mut *tx, &config).await?;
    tx.commit().await?;
    Ok(AxumJson(MailSettingsResponse {
        config: Some(MailConfigView::from(&config)),
    }))
}

#[utoipa::path(
    delete,
    path = "/api/mail",
    responses(
        (status = 204, description = "메일 설정 삭제. 메일 기능 꺼짐"),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
    )
)]
async fn delete_mail_settings(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<StatusCode> {
    current.require(Role::Owner)?;
    settings::remove(&state.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/mail/test",
    request_body = TestMailRequest,
    responses(
        (status = 204, description = "저장된 설정으로 발송 성공"),
        (status = 400, description = "본문 형식 오류, `mail.invalidAddress`", body = ErrorBody),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
        (status = 409, description = "`mail.notConfigured`, `mail.sendFailed`(`message`, 비밀번호 제거 후)", body = ErrorBody),
    )
)]
async fn send_test_mail(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(req): Json<TestMailRequest>,
) -> AppResult<StatusCode> {
    current.require(Role::Owner)?;
    let to = req.to.trim();
    if !super::valid_address(to) {
        return Err(AppError::bad_request("mail.invalidAddress"));
    }
    let config = settings::load(&state.pool)
        .await?
        .ok_or(AppError::conflict("mail.notConfigured"))?;
    let (subject, body) = test_text(&current.user.locale);
    let msg = Outgoing {
        to: to.to_owned(),
        subject: subject.to_owned(),
        body: body.to_owned(),
    };
    match super::send(&state, &config, msg).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(MailError::Send(message)) => Err(AppError::Conflict {
            key: "mail.sendFailed",
            params: json!({ "message": message }),
        }),
        Err(MailError::App(e)) => Err(e),
    }
}
