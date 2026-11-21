//! OIDC 설정 API. 모두 owner. 응답에 client secret 없음(설정 여부만).

use axum::{Json as AxumJson, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::{
    OidcError,
    settings::{self, DEFAULT_LABEL, OidcConfig},
};
use crate::{
    auth::extract::CurrentUser,
    error::{AppError, AppResult, ErrorBody},
    request::Json,
    state::AppState,
    users::{self, Role},
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(
        get_oidc_settings,
        update_oidc_settings,
        delete_oidc_settings
    ))
}

#[derive(Serialize, ToSchema)]
pub struct OidcSettingsResponse {
    /// 미설정이면 null.
    #[schema(required = true)]
    config: Option<OidcConfigView>,
    /// IdP 에 등록할 redirect URI.
    callback_url: String,
    /// 현재 설정 issuer 로 연결된 계정 수. 미설정이면 0.
    linked: i64,
    /// `linked` 중 비밀번호 없는 계정 수. 설정 삭제나 issuer 변경 시 로그인 불가.
    passwordless: i64,
}

#[derive(Serialize, ToSchema)]
pub struct OidcConfigView {
    /// discovery 문서의 issuer.
    issuer_url: String,
    client_id: String,
    /// client secret 저장됨. 값은 응답하지 않음.
    secret_set: bool,
    button_label: String,
}

impl From<&OidcConfig> for OidcConfigView {
    fn from(c: &OidcConfig) -> Self {
        Self {
            issuer_url: c.issuer_url.clone(),
            client_id: c.client_id.clone(),
            secret_set: c.client_secret.is_some(),
            button_label: c.button_label.clone(),
        }
    }
}

/// 문자열은 앞뒤 공백 제거.
#[derive(Deserialize, ToSchema)]
pub struct OidcSettingsRequest {
    /// discovery 기준 주소. 저장 값은 discovery 문서의 issuer(끝 슬래시 하나 차이 허용).
    issuer_url: String,
    client_id: String,
    /// 없으면 저장된 값 유지(issuer 나 client_id 가 바뀌면 삭제). 빈 문자열이면 삭제(public client).
    client_secret: Option<String>,
    /// 비었거나 없으면 "SSO".
    button_label: Option<String>,
}

async fn settings_response(
    state: &AppState,
    config: Option<&OidcConfig>,
) -> AppResult<OidcSettingsResponse> {
    let (linked, passwordless) = match config {
        Some(c) => users::queries::oidc_link_counts(&state.pool, &c.issuer_url).await?,
        None => (0, 0),
    };
    Ok(OidcSettingsResponse {
        config: config.map(OidcConfigView::from),
        callback_url: super::callback_url(&state.config.public_url),
        linked,
        passwordless,
    })
}

#[utoipa::path(
    get,
    path = "/api/oidc",
    responses(
        (status = 200, description = "OIDC 설정. client secret 은 설정 여부만", body = OidcSettingsResponse),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님", body = ErrorBody),
    )
)]
async fn get_oidc_settings(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<AxumJson<OidcSettingsResponse>> {
    current.require(Role::Owner)?;
    let config = settings::load(&state.pool).await?;
    Ok(AxumJson(settings_response(&state, config.as_ref()).await?))
}

#[utoipa::path(
    put,
    path = "/api/oidc",
    request_body = OidcSettingsRequest,
    responses(
        (status = 200, description = "discovery 확인 후 저장한 설정", body = OidcSettingsResponse),
        (status = 400, description = "본문 형식 오류, `oidc.clientIdRequired`, `oidc.discoveryFailed`(`message`)", body = ErrorBody),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
    )
)]
async fn update_oidc_settings(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(req): Json<OidcSettingsRequest>,
) -> AppResult<AxumJson<OidcSettingsResponse>> {
    current.require(Role::Owner)?;
    let client_id = req.client_id.trim();
    if client_id.is_empty() {
        return Err(AppError::bad_request("oidc.clientIdRequired"));
    }
    // 트랜잭션 밖에서 네트워크 조회. 실패하면 저장 안 함.
    let metadata =
        super::discover(req.issuer_url.trim())
            .await
            .map_err(|OidcError::Discovery(message)| AppError::BadRequest {
                key: "oidc.discoveryFailed",
                params: json!({ "message": message }),
            })?;
    let button_label = req
        .button_label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .unwrap_or(DEFAULT_LABEL);

    let mut tx = state.pool.begin().await?;
    let issuer_url = metadata.issuer().as_str().to_owned();
    let client_secret = match req.client_secret.as_deref().map(str::trim) {
        // 다른 IdP 나 client 로 바뀌면 옛 secret 이월 금지.
        None => settings::load(&mut *tx)
            .await?
            .filter(|c| super::same_issuer(&c.issuer_url, &issuer_url) && c.client_id == client_id)
            .and_then(|c| c.client_secret),
        Some("") => None,
        Some(secret) => Some(secret.to_owned()),
    };
    let config = OidcConfig {
        issuer_url,
        client_id: client_id.to_owned(),
        client_secret,
        button_label: button_label.to_owned(),
    };
    settings::save(&mut *tx, &config).await?;
    tx.commit().await?;
    Ok(AxumJson(settings_response(&state, Some(&config)).await?))
}

#[utoipa::path(
    delete,
    path = "/api/oidc",
    responses(
        (status = 204, description = "OIDC 설정 삭제. 외부 로그인 꺼짐, 사용자 연결 값은 유지"),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
    )
)]
async fn delete_oidc_settings(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<StatusCode> {
    current.require(Role::Owner)?;
    settings::remove(&state.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}
