//! Authorization Code + PKCE(S256) 흐름. 보안 검증(서명, iss, aud, nonce, 만료, PKCE)은 라이브러리.

use openidconnect::{
    AuthType, AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken,
    EndpointMaybeSet, EndpointNotSet, EndpointSet, Nonce, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, TokenResponse,
    core::{CoreClient, CoreClientAuthMethod, CoreProviderMetadata, CoreResponseType},
    url::Url,
};

use super::{OidcConfig, OidcError, callback_url, discover, http};

type Client = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

/// IdP authorize 주소와 진행 쿠키에 담을 값.
pub struct Authorize {
    pub url: Url,
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
}

/// 검증된 ID 토큰의 계정 식별자.
pub struct Identity {
    /// ID 토큰 `iss` 원문.
    pub issuer: String,
    pub subject: String,
}

/// discovery 후 클라이언트 구성. 문서 issuer 는 저장 설정과 정확히 같아야 함
/// (연결 issuer 와 설정 issuer 비교 일관성. 바뀌었으면 owner 가 설정을 다시 저장).
async fn client(config: &OidcConfig, public_url: &str) -> Result<Client, OidcError> {
    let metadata: CoreProviderMetadata = discover(&config.issuer_url).await?;
    if metadata.issuer().as_str() != config.issuer_url {
        return Err(OidcError::Flow(format!(
            "discovery issuer `{}` differs from saved issuer, save oidc settings again",
            metadata.issuer().as_str()
        )));
    }
    // client_secret_basic 기본. IdP 가 post 만 지원하면 본문으로.
    let post_only = metadata
        .token_endpoint_auth_methods_supported()
        .is_some_and(|m| {
            !m.contains(&CoreClientAuthMethod::ClientSecretBasic)
                && m.contains(&CoreClientAuthMethod::ClientSecretPost)
        });
    let redirect = RedirectUrl::new(callback_url(public_url))
        .map_err(|e| OidcError::Flow(format!("invalid callback url: {e}")))?;
    let client = CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(config.client_id.clone()),
        config.client_secret.clone().map(ClientSecret::new),
    )
    .set_redirect_uri(redirect);
    Ok(if post_only {
        client.set_auth_type(AuthType::RequestBody)
    } else {
        client
    })
}

/// scope 는 `openid` 만(라이브러리 기본). 계정은 issuer + sub 로만 찾음.
pub async fn authorize(config: &OidcConfig, public_url: &str) -> Result<Authorize, OidcError> {
    let client = client(config, public_url).await?;
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, state, nonce) = client
        .authorize_url(
            AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .set_pkce_challenge(challenge)
        .url();
    Ok(Authorize {
        url,
        state: state.secret().clone(),
        nonce: nonce.secret().clone(),
        pkce_verifier: verifier.secret().clone(),
    })
}

/// code 교환과 ID 토큰 검증. 오류 문구에 code, 토큰 없음(최상위 문구만).
pub async fn identify(
    config: &OidcConfig,
    public_url: &str,
    code: String,
    pkce_verifier: String,
    nonce: String,
) -> Result<Identity, OidcError> {
    let client = client(config, public_url).await?;
    let http = http::Client::shared()
        .map_err(|e| OidcError::Flow(format!("http client unavailable: {e}")))?;
    let response = client
        .exchange_code(AuthorizationCode::new(code))
        .map_err(|e| OidcError::Flow(e.to_string()))?
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier))
        .request_async(http)
        .await
        // 원인 사슬 제외. 파싱 오류 원인에 응답 값이 실릴 수 있음.
        .map_err(|e| OidcError::Flow(format!("token request: {e}")))?;
    let id_token = response
        .id_token()
        .ok_or_else(|| OidcError::Flow("token response has no id_token".to_owned()))?;
    let claims = id_token
        .claims(&client.id_token_verifier(), &Nonce::new(nonce))
        .map_err(|e| OidcError::Flow(format!("id token: {e}")))?;
    Ok(Identity {
        issuer: claims.issuer().as_str().to_owned(),
        subject: claims.subject().as_str().to_owned(),
    })
}
