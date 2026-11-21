//! 외부 IdP(OpenID Connect) 로그인. 설정은 `settings` 키 `oidc`, 키 있으면 켜짐.

pub mod http;
pub mod routes;
pub mod settings;

use std::error::Error;

use openidconnect::{
    AsyncHttpClient, IssuerUrl, JsonWebKeySet,
    core::CoreProviderMetadata,
    http::{Method, StatusCode, header::ACCEPT},
};
use serde::Serialize;
use utoipa::ToSchema;

pub use settings::OidcConfig;

const DISCOVERY_PATH: &str = ".well-known/openid-configuration";

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    /// discovery 문서, JWKS 조회와 검사 실패. 문구에 비밀값 없음.
    #[error("{0}")]
    Discovery(String),
}

/// 외부 IdP 계정 연결 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum OidcLinkState {
    None,
    Linked,
    /// 연결 값은 있으나 현재 설정 issuer 와 다름(설정 없음 포함). 로그인에 안 쓰임.
    Stale,
}

/// 설정 issuer 와 사용자에 저장된 issuer 비교.
pub fn oidc_link_state(configured: Option<&str>, stored: Option<&str>) -> OidcLinkState {
    match (configured, stored) {
        (_, None) => OidcLinkState::None,
        (Some(c), Some(s)) if c == s => OidcLinkState::Linked,
        _ => OidcLinkState::Stale,
    }
}

/// `{PUBLIC_URL}/api/auth/oidc/callback`. IdP 에 등록할 주소.
pub fn callback_url(public_url: &str) -> String {
    format!("{public_url}/api/auth/oidc/callback")
}

/// discovery 문서와 JWKS 조회. 문서 `issuer` 는 입력과 끝 슬래시 하나 차이까지 허용.
/// 반환 메타데이터의 `issuer()` 가 IdP 기준 값(ID 토큰 `iss` 와 같은 문자열).
pub async fn discover(issuer_url: &str) -> Result<CoreProviderMetadata, OidcError> {
    let client = http::Client::shared().map_err(fail)?;
    let issuer = IssuerUrl::new(issuer_url.to_owned()).map_err(fail)?;
    let url = issuer.join(DISCOVERY_PATH).map_err(fail)?;
    let request = openidconnect::http::Request::builder()
        .method(Method::GET)
        .uri(url.as_str())
        .header(ACCEPT, "application/json")
        .body(Vec::new())
        .map_err(fail)?;
    let response = client.call(request).await.map_err(fail)?;
    if response.status() != StatusCode::OK {
        return Err(OidcError::Discovery(format!(
            "HTTP status {} at {url}",
            response.status()
        )));
    }
    let metadata: CoreProviderMetadata = serde_json::from_slice(response.body()).map_err(fail)?;
    if !same_issuer(metadata.issuer(), issuer_url) {
        return Err(OidcError::Discovery(format!(
            "issuer mismatch: discovery document says `{}`",
            metadata.issuer().as_str()
        )));
    }
    if metadata.token_endpoint().is_none() {
        return Err(OidcError::Discovery(
            "discovery document has no token_endpoint".to_owned(),
        ));
    }
    let jwks = JsonWebKeySet::fetch_async(metadata.jwks_uri(), client)
        .await
        .map_err(fail)?;
    Ok(metadata.set_jwks(jwks))
}

/// 끝 슬래시 하나 차이는 같은 issuer.
pub(crate) fn same_issuer(a: &str, b: &str) -> bool {
    a.strip_suffix('/').unwrap_or(a) == b.strip_suffix('/').unwrap_or(b)
}

/// 원인 사슬까지 한 줄로. 라이브러리 최상위 문구만으로는 원인 불명.
fn fail(e: impl Error) -> OidcError {
    let mut message = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        message.push_str(": ");
        message.push_str(&s.to_string());
        source = s.source();
    }
    OidcError::Discovery(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_state_compares_issuer() {
        let a = "https://idp.test/a";
        assert_eq!(oidc_link_state(None, None), OidcLinkState::None);
        assert_eq!(oidc_link_state(Some(a), None), OidcLinkState::None);
        assert_eq!(oidc_link_state(Some(a), Some(a)), OidcLinkState::Linked);
        assert_eq!(
            oidc_link_state(Some(a), Some("https://idp.test/b")),
            OidcLinkState::Stale
        );
        assert_eq!(oidc_link_state(None, Some(a)), OidcLinkState::Stale);
    }

    #[test]
    fn issuer_trailing_slash_only() {
        assert!(same_issuer("https://a/x", "https://a/x/"));
        assert!(same_issuer("https://a/x/", "https://a/x"));
        assert!(same_issuer("https://a/x", "https://a/x"));
        assert!(!same_issuer("https://a/x", "https://a/x//"));
        assert!(!same_issuer("https://a/x", "https://a/y"));
        assert!(!same_issuer("https://a/x", "http://a/x"));
    }
}
