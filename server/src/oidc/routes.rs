//! OIDC 설정 API(owner, 응답에 client secret 없음)와 로그인, 계정 연결 흐름.

use axum::{
    Json as AxumJson,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::{
    OidcError,
    cookie::{self, Progress, Purpose},
    flow,
    settings::{self, DEFAULT_LABEL, OidcConfig},
};
use crate::{
    auth::{extract::CurrentUser, session},
    error::{AppError, AppResult, ErrorBody},
    request::{Json, Query},
    state::AppState,
    users::{self, Role},
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            get_oidc_settings,
            update_oidc_settings,
            delete_oidc_settings
        ))
        .routes(routes!(start))
        .routes(routes!(callback))
        .routes(routes!(unlink))
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
    let metadata = super::discover(req.issuer_url.trim())
        .await
        .map_err(|e: OidcError| AppError::BadRequest {
            key: "oidc.discoveryFailed",
            params: json!({ "message": e.to_string() }),
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

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StartQuery {
    /// `login`: 연결된 계정으로 로그인. `link`: 로그인한 본인 계정에 연결(세션 필요).
    #[param(inline)]
    purpose: Purpose,
    /// `login` 성공 뒤 이동할 앱 경로. `/` 로 시작하고 `//` 아님, 역슬래시와 제어 문자 없음. 아니면 무시하고 `/`.
    redirect: Option<String>,
}

/// IdP 가 붙여 보내는 값. 그 밖의 쿼리 무시.
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    /// IdP 오류(사용자 거부 등). 있으면 `oidc.denied`.
    error: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/auth/oidc/start",
    params(StartQuery),
    responses(
        (status = 302, description = "진행 쿠키(`cgm_oidc`) 설정 후 IdP authorize 로 이동. OIDC 꺼짐이나 실패는 용도별 화면(`/login`, `/settings/login-method`)에 `oidc_error`, `link` 인데 세션 없으면 `/login`"),
        (status = 400, description = "쿼리 오류 (`request.invalidQuery`)", body = ErrorBody),
    )
)]
async fn start(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(q): Query<StartQuery>,
) -> Response {
    let purpose = q.purpose;
    let config = match load_config(&state).await {
        Ok(Some(c)) => c,
        Ok(None) => return fail(jar, purpose, "oidc.disabled"),
        Err(key) => return fail(jar, purpose, key),
    };
    let user_id = match purpose {
        Purpose::Login => None,
        Purpose::Link => match session_user(&state, &jar).await {
            Ok(Some(id)) => Some(id),
            Ok(None) => return found(jar, "/login"),
            Err(key) => return fail(jar, purpose, key),
        },
    };
    let authorize = match flow::authorize(&config, &state.config.public_url).await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "oidc start failed");
            return fail(jar, purpose, "oidc.failed");
        }
    };
    let progress = Progress {
        purpose,
        state: authorize.state,
        nonce: authorize.nonce,
        pkce_verifier: authorize.pkce_verifier,
        exp: (Utc::now() + Duration::seconds(cookie::TTL_SECS as i64)).timestamp(),
        redirect: match purpose {
            Purpose::Login => q
                .redirect
                .as_deref()
                .and_then(safe_redirect)
                .map(str::to_owned),
            Purpose::Link => None,
        },
        user_id,
    };
    let value = cookie::encode(&state.config.secret_key, &progress);
    found(
        jar.add(cookie::cookie(&state.config, value)),
        authorize.url.as_str(),
    )
}

#[utoipa::path(
    get,
    path = "/api/auth/oidc/callback",
    params(CallbackQuery),
    responses(
        (status = 302, description = "진행 쿠키 삭제. `login` 성공은 세션 쿠키 발급 후 시작 때 `redirect` 또는 `/`, `link` 성공은 `/settings/login-method`. 실패는 용도별 화면에 `oidc_error`(`oidc.failed`, `oidc.denied`, `oidc.disabled`, `oidc.notLinked`, `oidc.alreadyLinked`, `oidc.sessionChanged`, `auth.disabled`)"),
    )
)]
async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let value = jar.get(cookie::COOKIE_NAME).map(|c| c.value().to_owned());
    let session_raw = jar.get(session::COOKIE_NAME).map(|c| c.value().to_owned());
    // 성공, 실패 모두 1회용.
    let jar = jar.add(cookie::clear_cookie(&state.config));
    let Some(progress) = value
        .as_deref()
        .and_then(|v| cookie::decode(&state.config.secret_key, v, Utc::now()))
    else {
        tracing::warn!(
            present = value.is_some(),
            "oidc callback without valid progress cookie"
        );
        return fail(jar, Purpose::Login, "oidc.failed");
    };
    let purpose = progress.purpose;
    match complete(&state, &headers, session_raw.as_deref(), progress, q).await {
        Ok(Done::LoggedIn { session, redirect }) => {
            found(jar.add(session::cookie(&state.config, session)), &redirect)
        }
        Ok(Done::Linked) => found(jar, LINK_SCREEN),
        Err(key) => fail(jar, purpose, key),
    }
}

#[utoipa::path(
    delete,
    path = "/api/auth/oidc/link",
    responses(
        (status = 204, description = "본인 계정의 IdP 연결 해제(연결 없어도 성공)"),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "CSRF 헤더 누락", body = ErrorBody),
        (status = 409, description = "비밀번호 없는 계정 (`oidc.needPassword`)", body = ErrorBody),
    )
)]
async fn unlink(State(state): State<AppState>, current: CurrentUser) -> AppResult<StatusCode> {
    if users::queries::clear_oidc_link(&state.pool, current.user.id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::conflict("oidc.needPassword"))
    }
}

const LOGIN_SCREEN: &str = "/login";
const LINK_SCREEN: &str = "/settings/login-method";

enum Done {
    LoggedIn { session: String, redirect: String },
    Linked,
}

/// 실패는 i18n 키. 원인은 여기서 기록(code, 토큰, 쿠키 값 제외).
async fn complete(
    state: &AppState,
    headers: &HeaderMap,
    session_raw: Option<&str>,
    progress: Progress,
    q: CallbackQuery,
) -> Result<Done, &'static str> {
    if let Some(error) = q.error {
        tracing::info!(error = %error, "oidc idp returned error");
        return Err("oidc.denied");
    }
    if !q
        .state
        .as_deref()
        .is_some_and(|s| same_secret(s, &progress.state))
    {
        tracing::warn!("oidc callback state mismatch");
        return Err("oidc.failed");
    }
    let Some(code) = q.code else {
        tracing::warn!("oidc callback without code");
        return Err("oidc.failed");
    };
    let config = load_config(state).await?.ok_or("oidc.disabled")?;
    let identity = flow::identify(
        &config,
        &state.config.public_url,
        code,
        progress.pkce_verifier,
        progress.nonce,
    )
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "oidc callback failed");
        "oidc.failed"
    })?;

    match progress.purpose {
        Purpose::Login => {
            let user =
                users::queries::find_by_oidc(&state.pool, &identity.issuer, &identity.subject)
                    .await
                    .map_err(internal)?
                    .ok_or("oidc.notLinked")?;
            if user.disabled_at.is_some() {
                return Err("auth.disabled");
            }
            let (session, _) = session::create(&state.pool, user.id, session::user_agent(headers))
                .await
                .map_err(internal)?;
            Ok(Done::LoggedIn {
                session,
                redirect: progress.redirect.unwrap_or_else(|| "/".to_owned()),
            })
        }
        Purpose::Link => {
            let current = match session_raw {
                Some(raw) => active_user(state, raw).await?,
                None => None,
            };
            let Some(user_id) = current.filter(|id| Some(*id) == progress.user_id) else {
                return Err("oidc.sessionChanged");
            };
            users::queries::set_oidc_link(
                &state.pool,
                user_id,
                &identity.issuer,
                &identity.subject,
            )
            .await
            .map_err(|e| match e {
                AppError::Conflict { key, .. } => key,
                e => internal(e),
            })?;
            Ok(Done::Linked)
        }
    }
}

/// 설정 손상은 기록 후 실패.
async fn load_config(state: &AppState) -> Result<Option<OidcConfig>, &'static str> {
    settings::load(&state.pool).await.map_err(internal)
}

/// 세션 쿠키의 정지되지 않은 사용자.
async fn session_user(state: &AppState, jar: &CookieJar) -> Result<Option<Uuid>, &'static str> {
    match jar.get(session::COOKIE_NAME) {
        Some(c) => active_user(state, c.value()).await,
        None => Ok(None),
    }
}

async fn active_user(state: &AppState, raw: &str) -> Result<Option<Uuid>, &'static str> {
    let found = session::find_by_raw(&state.pool, raw)
        .await
        .map_err(internal)?;
    Ok(found
        .filter(|f| f.user.disabled_at.is_none())
        .map(|f| f.user.id))
}

fn internal(e: AppError) -> &'static str {
    tracing::error!(error = %e, "oidc flow internal error");
    "oidc.failed"
}

/// 길이, 내용에 따른 시간 차 없이 비교(해시 비교).
fn same_secret(a: &str, b: &str) -> bool {
    Sha256::digest(a.as_bytes()) == Sha256::digest(b.as_bytes())
}

fn screen(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::Login => LOGIN_SCREEN,
        Purpose::Link => LINK_SCREEN,
    }
}

fn fail(jar: CookieJar, purpose: Purpose, key: &str) -> Response {
    found(jar, &format!("{}?oidc_error={key}", screen(purpose)))
}

/// 302. 헤더로 못 쓰는 주소(검사 통과한 비 ASCII 등)는 `/`.
fn found(jar: CookieJar, location: &str) -> Response {
    let location = HeaderValue::from_str(location).unwrap_or(HeaderValue::from_static("/"));
    (StatusCode::FOUND, jar, [(header::LOCATION, location)]).into_response()
}

/// 웹 로그인 화면과 같은 규칙. 퍼센트 인코딩을 한 번 푼 값도 검사.
fn safe_redirect(v: &str) -> Option<&str> {
    (is_app_path(v) && percent_decode(v).is_some_and(|d| is_app_path(&d))).then_some(v)
}

/// 브라우저가 제어 문자, 역슬래시를 지우거나 `/` 로 바꿔 `//host` 가 되는 경우 차단.
fn is_app_path(v: &str) -> bool {
    v.starts_with('/')
        && !v.starts_with("//")
        && !v.contains('\\')
        && !v.chars().any(|c| c.is_ascii_control())
}

/// `decodeURIComponent` 대응. 잘못된 `%` 시퀀스, UTF-8 아님은 `None`.
fn percent_decode(v: &str) -> Option<String> {
    let bytes = v.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_rule_matches_web() {
        for ok in ["/", "/days/2026-11-21?baby=x", "/a%20b", "/a#frag"] {
            assert_eq!(safe_redirect(ok), Some(ok), "{ok}");
        }
        for bad in [
            "",
            "days",
            "https://evil.example",
            "//evil.example",
            "/\\evil.example",
            "/\t/evil.example",
            "/\u{7f}",
            "/%2F%2Fevil.example",
            "/%5Cevil.example",
            "%2F%2Fevil.example",
            "/%0a",
            "/%zz",
            "/%2",
            "/%ff",
        ] {
            assert_eq!(safe_redirect(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn percent_decode_like_decode_uri_component() {
        assert_eq!(percent_decode("/a%2Fb%20c").as_deref(), Some("/a/b c"));
        assert_eq!(percent_decode("%ED%95%9C").as_deref(), Some("\u{d55c}"));
        assert_eq!(percent_decode("+%41").as_deref(), Some("+A"));
        assert_eq!(percent_decode("%+1"), None);
    }
}
