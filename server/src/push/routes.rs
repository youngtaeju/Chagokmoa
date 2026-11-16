use axum::{Json as AxumJson, extract::State, http::StatusCode};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use web_push_native::p256;

use super::{PUSH_KINDS, queries};
use crate::{
    auth::extract::CurrentUser,
    error::{AppError, AppResult, ErrorBody},
    notifications::NotificationKind,
    request::Json,
    state::AppState,
};

/// 구독 주소로 허용하는 푸시 서비스 호스트 접미사. 점 경계 일치만.
const HOST_SUFFIXES: [&str; 3] = [
    ".push.services.mozilla.com",
    ".push.apple.com",
    ".notify.windows.com",
];
/// 구독 주소 최대 길이(바이트).
const MAX_ENDPOINT_BYTES: usize = 2048;
const HOST_EXACT: &str = "fcm.googleapis.com";

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_key))
        .routes(routes!(
            get_subscription,
            put_subscription,
            delete_subscription
        ))
        .routes(routes!(get_prefs, patch_prefs))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PushKey {
    /// base64url(패딩 없음) 65바이트 VAPID 공개키.
    pub public_key: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PushSubscriptionState {
    /// 이 세션의 구독 주소. 없으면 `null`.
    #[schema(required = true)]
    pub endpoint: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PushSubscriptionBody {
    pub endpoint: String,
    /// 브라우저 공개키(65바이트 P-256 점)의 base64url.
    pub p256dh: String,
    /// 16바이트 인증 비밀의 base64url.
    pub auth: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PushPrefs {
    pub upload: bool,
    pub comment: bool,
    pub mention: bool,
    pub story: bool,
    pub milestone: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PushPrefsPatch {
    pub upload: Option<bool>,
    pub comment: Option<bool>,
    pub mention: Option<bool>,
    pub story: Option<bool>,
    pub milestone: Option<bool>,
}

fn endpoint_allowed(endpoint: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return false;
    };
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    // 기본 포트(443)만 허용.
    if url.port().is_some() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    host == HOST_EXACT || HOST_SUFFIXES.iter().any(|s| host.ends_with(s))
}

/// base64url(패딩 허용) 디코드.
fn decode_key(value: &str) -> Option<Vec<u8>> {
    B64.decode(value.trim_end_matches('=')).ok()
}

fn validate(body: &PushSubscriptionBody, check_endpoint: bool) -> AppResult<()> {
    // 바이트 기준 상한.
    if body.endpoint.len() > MAX_ENDPOINT_BYTES {
        return Err(AppError::bad_request("push.invalidSubscription"));
    }
    if check_endpoint && !endpoint_allowed(&body.endpoint) {
        return Err(AppError::bad_request("push.endpointNotAllowed"));
    }
    let invalid = || AppError::bad_request("push.invalidSubscription");
    let point = decode_key(&body.p256dh).ok_or_else(invalid)?;
    p256::PublicKey::from_sec1_bytes(&point).map_err(|_| invalid())?;
    let auth = decode_key(&body.auth).ok_or_else(invalid)?;
    if auth.len() != 16 {
        return Err(invalid());
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/push/key",
    responses(
        (status = 200, description = "VAPID 공개키", body = PushKey),
        (status = 401, description = "인증 필요", body = ErrorBody),
    )
)]
async fn get_key(
    State(state): State<AppState>,
    _current: CurrentUser,
) -> AppResult<AxumJson<PushKey>> {
    let public_key = state.vapid().await?.public_key.clone();
    Ok(AxumJson(PushKey { public_key }))
}

#[utoipa::path(
    get,
    path = "/api/push/subscription",
    responses(
        (status = 200, description = "이 세션의 구독", body = PushSubscriptionState),
        (status = 401, description = "인증 필요", body = ErrorBody),
    )
)]
async fn get_subscription(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<AxumJson<PushSubscriptionState>> {
    let endpoint = queries::endpoint_of_session(&state.pool, current.session_id).await?;
    Ok(AxumJson(PushSubscriptionState { endpoint }))
}

#[utoipa::path(
    put,
    path = "/api/push/subscription",
    request_body = PushSubscriptionBody,
    responses(
        (status = 204, description = "이 세션의 구독으로 저장. 같은 주소의 다른 세션 구독은 이동"),
        (status = 400, description = "주소 미허용 (`push.endpointNotAllowed`) 또는 키 오류 (`push.invalidSubscription`)", body = ErrorBody),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "CSRF 헤더 누락", body = ErrorBody),
    )
)]
async fn put_subscription(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<PushSubscriptionBody>,
) -> AppResult<StatusCode> {
    validate(&body, state.push_endpoint_check)?;
    queries::replace(
        &state.pool,
        current.session_id,
        &body.endpoint,
        &body.p256dh,
        &body.auth,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/api/push/subscription",
    responses(
        (status = 204, description = "이 세션의 구독 삭제. 없어도 204"),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "CSRF 헤더 누락", body = ErrorBody),
    )
)]
async fn delete_subscription(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<StatusCode> {
    queries::delete_by_session(&state.pool, current.session_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn load_prefs(state: &AppState, user_id: uuid::Uuid) -> AppResult<PushPrefs> {
    let muted = queries::muted(&state.pool, user_id).await?;
    let on = |kind| !muted.contains(&kind);
    Ok(PushPrefs {
        upload: on(NotificationKind::Upload),
        comment: on(NotificationKind::Comment),
        mention: on(NotificationKind::Mention),
        story: on(NotificationKind::Story),
        milestone: on(NotificationKind::Milestone),
    })
}

#[utoipa::path(
    get,
    path = "/api/push/prefs",
    responses(
        (status = 200, description = "종류별 푸시 켜짐 여부. 기본 전부 켜짐", body = PushPrefs),
        (status = 401, description = "인증 필요", body = ErrorBody),
    )
)]
async fn get_prefs(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<AxumJson<PushPrefs>> {
    Ok(AxumJson(load_prefs(&state, current.user.id).await?))
}

#[utoipa::path(
    patch,
    path = "/api/push/prefs",
    request_body = PushPrefsPatch,
    responses(
        (status = 200, description = "갱신 후 전체 설정", body = PushPrefs),
        (status = 400, description = "본문 형식 오류", body = ErrorBody),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "CSRF 헤더 누락", body = ErrorBody),
    )
)]
async fn patch_prefs(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(patch): Json<PushPrefsPatch>,
) -> AppResult<AxumJson<PushPrefs>> {
    let wanted = |kind| match kind {
        NotificationKind::Upload => patch.upload,
        NotificationKind::Comment => patch.comment,
        NotificationKind::Mention => patch.mention,
        NotificationKind::Story => patch.story,
        NotificationKind::Milestone => patch.milestone,
    };
    let mut tx = state.pool.begin().await?;
    for kind in PUSH_KINDS {
        if let Some(on) = wanted(kind) {
            queries::set_muted(&mut tx, current.user.id, kind, !on).await?;
        }
    }
    tx.commit().await?;
    Ok(AxumJson(load_prefs(&state, current.user.id).await?))
}
