//! 테스트용 가짜 OIDC 제공자. 127.0.0.1 평문 HTTP, discovery, JWKS, token 엔드포인트만.
//! ID 토큰은 고정 테스트 키(`tests/fixtures/oidc_test_key.pem`, RS256)로 서명.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    Form, Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use openidconnect::{
    AccessToken, Audience, EmptyAdditionalClaims, IssuerUrl, JsonWebKeyId, Nonce,
    PrivateSigningKey, StandardClaims, SubjectIdentifier,
    core::{
        CoreIdToken, CoreIdTokenClaims, CoreJsonWebKeySet, CoreJwsSigningAlgorithm,
        CoreRsaPrivateSigningKey,
    },
};
use serde_json::{Value, json};
use tokio::{net::TcpListener, task::JoinHandle};
use uuid::Uuid;

const TEST_KEY: &str = include_str!("../fixtures/oidc_test_key.pem");
/// JWKS 에 없는 키. 서명 검증 실패 시험용.
const OTHER_KEY: &str = include_str!("../fixtures/oidc_other_key.pem");
/// 두 키 같은 kid. JWKS 에서 키를 찾은 뒤 서명 검증 단계에서 실패하게.
const KID: &str = "test-key";

/// code 하나로 발급할 ID 토큰 내용.
#[derive(Clone)]
struct Grant {
    sub: String,
    nonce: String,
    aud: String,
    other_key: bool,
}

/// token 엔드포인트가 받은 요청. 클라이언트 인증, PKCE 확인용.
#[derive(Clone, Debug)]
pub struct TokenRequest {
    pub form: HashMap<String, String>,
    pub authorization: Option<String>,
}

#[derive(Default)]
struct Shared {
    /// discovery 문서의 `issuer`. None 이면 실제 issuer.
    discovery_issuer: Option<String>,
    codes: HashMap<String, Grant>,
    token_requests: Vec<TokenRequest>,
}

#[derive(Clone)]
struct IdpState {
    issuer: String,
    shared: Arc<Mutex<Shared>>,
}

pub struct FakeIdp {
    /// `http://127.0.0.1:<port>`. 끝 슬래시 없음.
    pub issuer: String,
    shared: Arc<Mutex<Shared>>,
    task: JoinHandle<()>,
}

impl FakeIdp {
    pub async fn start() -> FakeIdp {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind idp");
        let port = listener.local_addr().expect("idp addr").port();
        let issuer = format!("http://127.0.0.1:{port}");
        let shared = Arc::new(Mutex::new(Shared::default()));
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/authorize", get(|| async { "fake authorize" }))
            .route("/token", post(token))
            // 리다이렉트 비추종, 본문 상한 시험용 discovery.
            .route("/moved/.well-known/openid-configuration", get(moved))
            .route("/huge/.well-known/openid-configuration", get(huge))
            .with_state(IdpState {
                issuer: issuer.clone(),
                shared: shared.clone(),
            });
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("idp serve");
        });
        FakeIdp {
            issuer,
            shared,
            task,
        }
    }

    /// discovery 문서가 실제 주소와 다른 `issuer` 를 알리게.
    pub fn set_discovery_issuer(&self, issuer: &str) {
        self.lock().discovery_issuer = Some(issuer.to_owned());
    }

    /// 다음 token 요청에서 ID 토큰으로 바꿀 code. 1회용.
    pub fn issue_code(&self, sub: &str, nonce: &str, aud: &str) -> String {
        self.register(sub, nonce, aud, false)
    }

    /// JWKS 에 없는 키로 서명한 ID 토큰을 주는 code.
    pub fn issue_code_other_key(&self, sub: &str, nonce: &str, aud: &str) -> String {
        self.register(sub, nonce, aud, true)
    }

    pub fn token_requests(&self) -> Vec<TokenRequest> {
        self.lock().token_requests.clone()
    }

    fn register(&self, sub: &str, nonce: &str, aud: &str, other_key: bool) -> String {
        let code = Uuid::now_v7().simple().to_string();
        let grant = Grant {
            sub: sub.to_owned(),
            nonce: nonce.to_owned(),
            aud: aud.to_owned(),
            other_key,
        };
        self.lock().codes.insert(code.clone(), grant);
        code
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().expect("idp state")
    }
}

impl Drop for FakeIdp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn signing_key(pem: &str) -> CoreRsaPrivateSigningKey {
    CoreRsaPrivateSigningKey::from_pem(pem, Some(JsonWebKeyId::new(KID.to_owned())))
        .expect("test rsa key")
}

/// discovery 와 ID 토큰 `iss` 공용. 덮어쓴 값이 있으면 그 값.
fn issuer(idp: &IdpState) -> String {
    idp.shared
        .lock()
        .expect("idp state")
        .discovery_issuer
        .clone()
        .unwrap_or_else(|| idp.issuer.clone())
}

async fn moved(State(idp): State<IdpState>) -> (StatusCode, [(header::HeaderName, String); 1]) {
    (
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("{}/.well-known/openid-configuration", idp.issuer),
        )],
    )
}

/// 상한(1 MiB) 넘는 본문.
async fn huge() -> String {
    format!("{{\"pad\":\"{}\"}}", "x".repeat(2 * 1024 * 1024))
}

async fn discovery(State(idp): State<IdpState>) -> Json<Value> {
    let base = &idp.issuer;
    let issuer = issuer(&idp);
    Json(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "jwks_uri": format!("{base}/jwks"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post", "none"],
        "code_challenge_methods_supported": ["S256"],
        "scopes_supported": ["openid", "profile", "email"],
    }))
}

async fn jwks() -> Json<CoreJsonWebKeySet> {
    Json(CoreJsonWebKeySet::new(vec![
        signing_key(TEST_KEY).as_verification_key(),
    ]))
}

async fn token(
    State(idp): State<IdpState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let grant = {
        let mut shared = idp.shared.lock().expect("idp state");
        shared.token_requests.push(TokenRequest {
            form: form.clone(),
            authorization: headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned),
        });
        form.get("code").and_then(|c| shared.codes.remove(c))
    };
    let Some(grant) = grant else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid_grant" })),
        ));
    };

    let access_token = AccessToken::new(Uuid::now_v7().simple().to_string());
    let now = Utc::now();
    let claims = CoreIdTokenClaims::new(
        IssuerUrl::new(issuer(&idp)).expect("issuer url"),
        vec![Audience::new(grant.aud)],
        now + Duration::minutes(5),
        now,
        StandardClaims::new(SubjectIdentifier::new(grant.sub)),
        EmptyAdditionalClaims {},
    )
    .set_nonce(Some(Nonce::new(grant.nonce)));
    let key = signing_key(if grant.other_key { OTHER_KEY } else { TEST_KEY });
    let id_token = CoreIdToken::new(
        claims,
        &key,
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        Some(&access_token),
        None,
    )
    .expect("sign id token");
    Ok(Json(json!({
        "access_token": access_token.secret(),
        "token_type": "Bearer",
        "expires_in": 300,
        "id_token": id_token.to_string(),
    })))
}
