//! OIDC 로그인과 계정 연결. 테스트 안 가짜 IdP 와 authorization code 흐름 전체.

mod common;

use axum::http::{StatusCode, header};
use axum_extra::extract::cookie::Cookie;
use axum_test::{TestResponse, TestServer};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chagokmoa::{
    auth::{password, session},
    oidc::{
        cookie::{self as progress, Progress},
        settings::{self, OidcConfig},
    },
    users::{self, NewUser, Role},
};
use chrono::Utc;
use common::{PASSWORD, XRW, XRW_VALUE, oidc::FakeIdp, test_config, test_server_without_cookies};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

const CLIENT_ID: &str = "chagokmoa";
const SECRET: &str = "oidc-client-secret-4321";
const CALLBACK: &str = "http://localhost/api/auth/oidc/callback";

async fn configure(pool: &PgPool, issuer: &str, secret: Option<&str>) {
    settings::save(
        pool,
        &OidcConfig {
            issuer_url: issuer.to_owned(),
            client_id: CLIENT_ID.to_owned(),
            client_secret: secret.map(str::to_owned),
            button_label: "SSO".to_owned(),
        },
    )
    .await
    .expect("save oidc settings");
}

async fn create_user(pool: &PgPool, username: &str, with_password: bool) -> Uuid {
    let password_hash = with_password.then(|| password::hash(PASSWORD).expect("hash"));
    users::queries::create(
        pool,
        NewUser {
            username: username.to_owned(),
            password_hash,
            display_name: username.to_owned(),
            role: Role::Family,
            email: None,
            locale: "ko".to_owned(),
        },
    )
    .await
    .expect("user")
    .id
}

async fn link(pool: &PgPool, id: Uuid, issuer: &str, subject: &str) {
    sqlx::query!(
        "UPDATE users SET oidc_issuer = $2, oidc_subject = $3 WHERE id = $1",
        id,
        issuer,
        subject,
    )
    .execute(pool)
    .await
    .expect("link user");
}

async fn stored_link(pool: &PgPool, id: Uuid) -> (Option<String>, Option<String>) {
    let row = sqlx::query!(
        "SELECT oidc_issuer, oidc_subject FROM users WHERE id = $1",
        id
    )
    .fetch_one(pool)
    .await
    .expect("user");
    (row.oidc_issuer, row.oidc_subject)
}

/// 비밀번호 로그인 세션 쿠키.
async fn password_login(server: &TestServer, username: &str) -> Cookie<'static> {
    let res = server
        .post("/api/auth/login")
        .json(&json!({ "username": username, "password": PASSWORD }))
        .await;
    res.assert_status_ok();
    res.cookie(session::COOKIE_NAME)
}

/// 비밀번호 없는 계정용. 세션 행 직접 생성.
async fn direct_session(pool: &PgPool, user_id: Uuid) -> Cookie<'static> {
    let (raw, _) = session::create(pool, user_id, None).await.expect("session");
    Cookie::new(session::COOKIE_NAME, raw)
}

fn location(res: &TestResponse) -> String {
    res.assert_status(StatusCode::FOUND);
    res.header(header::LOCATION)
        .to_str()
        .expect("location")
        .to_owned()
}

/// authorize 로 보낸 값과 진행 쿠키.
struct Started {
    state: String,
    nonce: String,
    challenge: String,
    progress: Cookie<'static>,
}

async fn start(
    server: &TestServer,
    idp: &FakeIdp,
    query: &str,
    session: Option<&Cookie<'static>>,
) -> Started {
    let mut req = server.get(&format!("/api/auth/oidc/start?{query}"));
    if let Some(s) = session {
        req = req.add_cookie(s.clone());
    }
    let res = req.await;
    let to = location(&res);
    let url = reqwest::Url::parse(&to).expect("authorize url");
    assert_eq!(
        format!("{}{}", url.origin().ascii_serialization(), url.path()),
        format!("{}/authorize", idp.issuer),
        "{to}"
    );
    let params: std::collections::HashMap<String, String> =
        url.query_pairs().into_owned().collect();
    assert_eq!(params["response_type"], "code");
    assert_eq!(params["client_id"], CLIENT_ID);
    assert_eq!(params["redirect_uri"], CALLBACK);
    assert_eq!(params["code_challenge_method"], "S256");
    assert!(
        params["scope"].split(' ').any(|s| s == "openid"),
        "{}",
        params["scope"]
    );
    let progress = res.cookie(progress::COOKIE_NAME);
    assert_eq!(progress.http_only(), Some(true));
    assert_eq!(progress.path(), Some("/api/auth/oidc"));
    assert_eq!(
        progress.same_site(),
        Some(axum_extra::extract::cookie::SameSite::Lax)
    );
    assert_eq!(progress.max_age().map(|d| d.whole_seconds()), Some(600));
    Started {
        state: params["state"].clone(),
        nonce: params["nonce"].clone(),
        challenge: params["code_challenge"].clone(),
        progress,
    }
}

async fn callback(
    server: &TestServer,
    progress: Option<&Cookie<'static>>,
    session: Option<&Cookie<'static>>,
    query: &[(&str, &str)],
) -> TestResponse {
    let mut req = server.get("/api/auth/oidc/callback");
    for (k, v) in query {
        req = req.add_query_param(k, v);
    }
    if let Some(p) = progress {
        req = req.add_cookie(p.clone());
    }
    if let Some(s) = session {
        req = req.add_cookie(s.clone());
    }
    let res = req.await;
    // 성공, 실패 모두 진행 쿠키 삭제.
    let cleared = res.cookie(progress::COOKIE_NAME);
    assert_eq!(cleared.value(), "");
    assert_eq!(cleared.path(), Some("/api/auth/oidc"));
    assert_eq!(cleared.max_age().map(|d| d.whole_seconds()), Some(0));
    res
}

/// start 부터 callback 까지. IdP 는 `sub` 로 code 발급.
async fn login_flow(server: &TestServer, idp: &FakeIdp, query: &str, sub: &str) -> TestResponse {
    let s = start(server, idp, query, None).await;
    let code = idp.issue_code(sub, &s.nonce, CLIENT_ID);
    callback(
        server,
        Some(&s.progress),
        None,
        &[("code", &code), ("state", &s.state)],
    )
    .await
}

fn assert_no_session(res: &TestResponse) {
    assert!(
        res.maybe_cookie(session::COOKIE_NAME).is_none(),
        "unexpected session cookie"
    );
}

async fn me(server: &TestServer, session: &Cookie<'static>) -> Value {
    let res = server.get("/api/auth/me").add_cookie(session.clone()).await;
    res.assert_status_ok();
    res.json::<Value>()
}

async fn login_method(server: &TestServer, session: &Cookie<'static>) -> Value {
    let res = server
        .get("/api/auth/login-method")
        .add_cookie(session.clone())
        .await;
    res.assert_status_ok();
    res.json::<Value>()
}

#[sqlx::test(migrations = "./migrations")]
async fn oidc_login_success(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, Some(SECRET)).await;
    let mom = create_user(&pool, "mom", true).await;
    link(&pool, mom, &idp.issuer, "sub-mom").await;
    let server = test_server_without_cookies(pool.clone()).await;

    let s = start(
        &server,
        &idp,
        "purpose=login&redirect=%2Fdays%2F2026-11-21%3Fbaby%3Dx",
        None,
    )
    .await;
    let code = idp.issue_code("sub-mom", &s.nonce, CLIENT_ID);
    let res = callback(
        &server,
        Some(&s.progress),
        None,
        &[("code", &code), ("state", &s.state)],
    )
    .await;
    assert_eq!(location(&res), "/days/2026-11-21?baby=x");
    let session_cookie = res.cookie(session::COOKIE_NAME);
    assert_eq!(me(&server, &session_cookie).await["user"]["id"], json!(mom));

    // PKCE 검증값은 S256 challenge 의 원문. confidential client 는 Basic 인증.
    let requests = idp.token_requests();
    assert_eq!(requests.len(), 1);
    let form = &requests[0].form;
    let verifier = form.get("code_verifier").expect("code_verifier");
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        s.challenge
    );
    assert_eq!(form["redirect_uri"], CALLBACK);
    assert_eq!(form["grant_type"], "authorization_code");
    assert!(!form.contains_key("client_secret"));
    let basic = requests[0].authorization.as_deref().expect("basic auth");
    assert!(basic.starts_with("Basic "), "{basic}");

    // redirect 없으면 홈.
    let res = login_flow(&server, &idp, "purpose=login", "sub-mom").await;
    assert_eq!(location(&res), "/");
    assert!(res.maybe_cookie(session::COOKIE_NAME).is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn oidc_login_uses_token_issuer(pool: PgPool) {
    // discovery issuer 가 끝 슬래시 포함. 연결 issuer 는 ID 토큰 iss 그대로.
    let idp = FakeIdp::start().await;
    let issuer = format!("{}/", idp.issuer);
    idp.set_discovery_issuer(&issuer);
    configure(&pool, &issuer, None).await;
    let mom = create_user(&pool, "mom", true).await;
    link(&pool, mom, &issuer, "sub-mom").await;
    let server = test_server_without_cookies(pool.clone()).await;

    let res = login_flow(&server, &idp, "purpose=login", "sub-mom").await;
    assert_eq!(location(&res), "/");
    let session_cookie = res.cookie(session::COOKIE_NAME);
    assert_eq!(me(&server, &session_cookie).await["user"]["id"], json!(mom));
}

#[sqlx::test(migrations = "./migrations")]
async fn oidc_login_not_linked(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    create_user(&pool, "mom", true).await;
    let server = test_server_without_cookies(pool.clone()).await;

    let res = login_flow(&server, &idp, "purpose=login", "sub-unknown").await;
    assert_eq!(location(&res), "/login?oidc_error=oidc.notLinked");
    assert_no_session(&res);
}

#[sqlx::test(migrations = "./migrations")]
async fn oidc_login_disabled_user(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let mom = create_user(&pool, "mom", true).await;
    link(&pool, mom, &idp.issuer, "sub-mom").await;
    users::queries::set_disabled(&pool, mom, true)
        .await
        .expect("disable");
    let server = test_server_without_cookies(pool.clone()).await;

    let res = login_flow(&server, &idp, "purpose=login", "sub-mom").await;
    assert_eq!(location(&res), "/login?oidc_error=auth.disabled");
    assert_no_session(&res);
}

#[sqlx::test(migrations = "./migrations")]
async fn callback_rejects_bad_progress_cookie(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let mom = create_user(&pool, "mom", true).await;
    link(&pool, mom, &idp.issuer, "sub-mom").await;
    let server = test_server_without_cookies(pool.clone()).await;
    let secret = test_config().secret_key;

    // 다른 브라우저: 쿠키 없음.
    let s = start(&server, &idp, "purpose=login", None).await;
    let code = idp.issue_code("sub-mom", &s.nonce, CLIENT_ID);
    let res = callback(&server, None, None, &[("code", &code), ("state", &s.state)]).await;
    assert_eq!(location(&res), "/login?oidc_error=oidc.failed");
    assert_no_session(&res);

    // 서명 변조: 내용 바꾸고 옛 서명 유지.
    let s = start(&server, &idp, "purpose=login", None).await;
    let code = idp.issue_code("sub-mom", &s.nonce, CLIENT_ID);
    let (payload, sig) = s.progress.value().split_once('.').expect("signed cookie");
    let mut json: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
    json["redirect"] = json!("/forged");
    let forged = format!(
        "{}.{sig}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json).unwrap())
    );
    let res = callback(
        &server,
        Some(&Cookie::new(progress::COOKIE_NAME, forged)),
        None,
        &[("code", &code), ("state", &s.state)],
    )
    .await;
    assert_eq!(location(&res), "/login?oidc_error=oidc.failed");
    assert_no_session(&res);

    // 다른 키 서명.
    let s = start(&server, &idp, "purpose=login", None).await;
    let code = idp.issue_code("sub-mom", &s.nonce, CLIENT_ID);
    let decoded = progress::decode(&secret, s.progress.value(), Utc::now()).expect("decode");
    let other_key = progress::encode(b"another-secret-key", &decoded);
    let res = callback(
        &server,
        Some(&Cookie::new(progress::COOKIE_NAME, other_key)),
        None,
        &[("code", &code), ("state", &s.state)],
    )
    .await;
    assert_eq!(location(&res), "/login?oidc_error=oidc.failed");
    assert_no_session(&res);

    // 만료: 같은 키로 exp 만 과거.
    let s = start(&server, &idp, "purpose=login", None).await;
    let code = idp.issue_code("sub-mom", &s.nonce, CLIENT_ID);
    let decoded: Progress =
        progress::decode(&secret, s.progress.value(), Utc::now()).expect("decode");
    let expired = progress::encode(
        &secret,
        &Progress {
            exp: Utc::now().timestamp() - 1,
            ..decoded
        },
    );
    let res = callback(
        &server,
        Some(&Cookie::new(progress::COOKIE_NAME, expired)),
        None,
        &[("code", &code), ("state", &s.state)],
    )
    .await;
    assert_eq!(location(&res), "/login?oidc_error=oidc.failed");
    assert_no_session(&res);

    // state 불일치.
    let s = start(&server, &idp, "purpose=login", None).await;
    let code = idp.issue_code("sub-mom", &s.nonce, CLIENT_ID);
    let res = callback(
        &server,
        Some(&s.progress),
        None,
        &[("code", &code), ("state", "not-the-state")],
    )
    .await;
    assert_eq!(location(&res), "/login?oidc_error=oidc.failed");
    assert_no_session(&res);

    // 모두 code 교환 전에 거부.
    assert!(idp.token_requests().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn callback_rejects_invalid_id_token(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, Some(SECRET)).await;
    let mom = create_user(&pool, "mom", true).await;
    link(&pool, mom, &idp.issuer, "sub-mom").await;
    let server = test_server_without_cookies(pool.clone()).await;

    for case in ["aud", "nonce", "key"] {
        let s = start(&server, &idp, "purpose=login", None).await;
        let code = match case {
            "aud" => idp.issue_code("sub-mom", &s.nonce, "other-client"),
            "nonce" => idp.issue_code("sub-mom", "other-nonce", CLIENT_ID),
            _ => idp.issue_code_other_key("sub-mom", &s.nonce, CLIENT_ID),
        };
        let res = callback(
            &server,
            Some(&s.progress),
            None,
            &[("code", &code), ("state", &s.state)],
        )
        .await;
        assert_eq!(location(&res), "/login?oidc_error=oidc.failed", "{case}");
        assert_no_session(&res);
    }
    assert_eq!(idp.token_requests().len(), 3);
}

#[sqlx::test(migrations = "./migrations")]
async fn callback_idp_error_is_denied(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let server = test_server_without_cookies(pool.clone()).await;

    let s = start(&server, &idp, "purpose=login", None).await;
    let res = callback(
        &server,
        Some(&s.progress),
        None,
        &[("error", "access_denied"), ("state", &s.state)],
    )
    .await;
    assert_eq!(location(&res), "/login?oidc_error=oidc.denied");
    assert_no_session(&res);
    assert!(idp.token_requests().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn start_when_disabled(pool: PgPool) {
    let idp = FakeIdp::start().await;
    create_user(&pool, "mom", true).await;
    let server = test_server_without_cookies(pool.clone()).await;
    let session_cookie = password_login(&server, "mom").await;

    let res = server.get("/api/auth/oidc/start?purpose=login").await;
    assert_eq!(location(&res), "/login?oidc_error=oidc.disabled");
    assert!(res.maybe_cookie(progress::COOKIE_NAME).is_none());

    let res = server
        .get("/api/auth/oidc/start?purpose=link")
        .add_cookie(session_cookie.clone())
        .await;
    assert_eq!(
        location(&res),
        "/settings/login-method?oidc_error=oidc.disabled"
    );
    assert!(res.maybe_cookie(progress::COOKIE_NAME).is_none());

    // 시작 뒤 설정 삭제.
    configure(&pool, &idp.issuer, None).await;
    let s = start(&server, &idp, "purpose=login", None).await;
    settings::remove(&pool).await.expect("remove");
    let code = idp.issue_code("sub-mom", &s.nonce, CLIENT_ID);
    let res = callback(
        &server,
        Some(&s.progress),
        None,
        &[("code", &code), ("state", &s.state)],
    )
    .await;
    assert_eq!(location(&res), "/login?oidc_error=oidc.disabled");
    assert_no_session(&res);
}

#[sqlx::test(migrations = "./migrations")]
async fn link_success(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let mom = create_user(&pool, "mom", true).await;
    let server = test_server_without_cookies(pool.clone()).await;
    let session_cookie = password_login(&server, "mom").await;
    assert_eq!(login_method(&server, &session_cookie).await["oidc"], "none");

    let s = start(&server, &idp, "purpose=link", Some(&session_cookie)).await;
    let code = idp.issue_code("sub-mom", &s.nonce, CLIENT_ID);
    let res = callback(
        &server,
        Some(&s.progress),
        Some(&session_cookie),
        &[("code", &code), ("state", &s.state)],
    )
    .await;
    assert_eq!(location(&res), "/settings/login-method");
    assert_no_session(&res);
    assert_eq!(
        stored_link(&pool, mom).await,
        (Some(idp.issuer.clone()), Some("sub-mom".to_owned()))
    );
    assert_eq!(
        login_method(&server, &session_cookie).await["oidc"],
        "linked"
    );

    // public client: Basic 없이 본문 client_id.
    let requests = idp.token_requests();
    assert_eq!(requests[0].authorization, None);
    assert_eq!(requests[0].form["client_id"], CLIENT_ID);

    // 연결 뒤 외부 로그인.
    let res = login_flow(&server, &idp, "purpose=login", "sub-mom").await;
    assert_eq!(location(&res), "/");
    let new_session = res.cookie(session::COOKIE_NAME);
    assert_eq!(me(&server, &new_session).await["user"]["id"], json!(mom));
}

#[sqlx::test(migrations = "./migrations")]
async fn link_rejects_already_linked_sub(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let mom = create_user(&pool, "mom", true).await;
    let aunt = create_user(&pool, "aunt", true).await;
    link(&pool, aunt, &idp.issuer, "sub-shared").await;
    let server = test_server_without_cookies(pool.clone()).await;
    let session_cookie = password_login(&server, "mom").await;

    let s = start(&server, &idp, "purpose=link", Some(&session_cookie)).await;
    let code = idp.issue_code("sub-shared", &s.nonce, CLIENT_ID);
    let res = callback(
        &server,
        Some(&s.progress),
        Some(&session_cookie),
        &[("code", &code), ("state", &s.state)],
    )
    .await;
    assert_eq!(
        location(&res),
        "/settings/login-method?oidc_error=oidc.alreadyLinked"
    );
    assert_eq!(stored_link(&pool, mom).await, (None, None));
    assert_eq!(
        stored_link(&pool, aunt).await,
        (Some(idp.issuer.clone()), Some("sub-shared".to_owned()))
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn link_rejects_session_change(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let mom = create_user(&pool, "mom", true).await;
    let dad = create_user(&pool, "dad", true).await;
    let server = test_server_without_cookies(pool.clone()).await;
    let mom_session = password_login(&server, "mom").await;
    let dad_session = password_login(&server, "dad").await;

    // IdP 에 다녀오는 사이 다른 사용자로 로그인.
    let s = start(&server, &idp, "purpose=link", Some(&mom_session)).await;
    let code = idp.issue_code("sub-x", &s.nonce, CLIENT_ID);
    let res = callback(
        &server,
        Some(&s.progress),
        Some(&dad_session),
        &[("code", &code), ("state", &s.state)],
    )
    .await;
    assert_eq!(
        location(&res),
        "/settings/login-method?oidc_error=oidc.sessionChanged"
    );

    // 그사이 로그아웃.
    let s = start(&server, &idp, "purpose=link", Some(&mom_session)).await;
    let code = idp.issue_code("sub-x", &s.nonce, CLIENT_ID);
    let res = callback(
        &server,
        Some(&s.progress),
        None,
        &[("code", &code), ("state", &s.state)],
    )
    .await;
    assert_eq!(
        location(&res),
        "/settings/login-method?oidc_error=oidc.sessionChanged"
    );

    assert_eq!(stored_link(&pool, mom).await, (None, None));
    assert_eq!(stored_link(&pool, dad).await, (None, None));
}

#[sqlx::test(migrations = "./migrations")]
async fn link_requires_session(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let server = test_server_without_cookies(pool.clone()).await;

    let res = server.get("/api/auth/oidc/start?purpose=link").await;
    assert_eq!(location(&res), "/login");
    assert!(res.maybe_cookie(progress::COOKIE_NAME).is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn stale_issuer_link_ignored(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let mom = create_user(&pool, "mom", true).await;
    link(&pool, mom, "https://old-idp.example", "sub-mom").await;
    let server = test_server_without_cookies(pool.clone()).await;

    let res = login_flow(&server, &idp, "purpose=login", "sub-mom").await;
    assert_eq!(location(&res), "/login?oidc_error=oidc.notLinked");
    assert_no_session(&res);

    let session_cookie = password_login(&server, "mom").await;
    assert_eq!(
        login_method(&server, &session_cookie).await["oidc"],
        "stale"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn unlink_success(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let mom = create_user(&pool, "mom", true).await;
    link(&pool, mom, &idp.issuer, "sub-mom").await;
    let server = test_server_without_cookies(pool.clone()).await;
    let session_cookie = password_login(&server, "mom").await;

    // CSRF 헤더 필요.
    let res = server
        .delete("/api/auth/oidc/link")
        .add_cookie(session_cookie.clone())
        .await;
    res.assert_status(StatusCode::FORBIDDEN);
    assert_eq!(res.json::<Value>()["error"], "auth.csrf");

    server
        .delete("/api/auth/oidc/link")
        .add_cookie(session_cookie.clone())
        .add_header(XRW, XRW_VALUE)
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(stored_link(&pool, mom).await, (None, None));
    assert_eq!(login_method(&server, &session_cookie).await["oidc"], "none");

    // 비로그인.
    let res = server.delete("/api/auth/oidc/link").await;
    res.assert_status(StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn unlink_passwordless_rejected(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let kid = create_user(&pool, "aunt", false).await;
    link(&pool, kid, &idp.issuer, "sub-aunt").await;
    let server = test_server_without_cookies(pool.clone()).await;
    let session_cookie = direct_session(&pool, kid).await;

    let res = server
        .delete("/api/auth/oidc/link")
        .add_cookie(session_cookie)
        .add_header(XRW, XRW_VALUE)
        .await;
    res.assert_status(StatusCode::CONFLICT);
    res.assert_json(&json!({ "error": "oidc.needPassword", "params": {} }));
    assert_eq!(
        stored_link(&pool, kid).await,
        (Some(idp.issuer.clone()), Some("sub-aunt".to_owned()))
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn start_redirect_rejects_external(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let mom = create_user(&pool, "mom", true).await;
    link(&pool, mom, &idp.issuer, "sub-mom").await;
    let server = test_server_without_cookies(pool.clone()).await;

    for redirect in [
        "https%3A%2F%2Fevil.example",
        "%2F%2Fevil.example",
        "%2F%5Cevil.example",
        "%2F%09%2Fevil.example",
        // 서버가 받는 값 `/%2F%2Fevil.example`. 한 번 더 풀면 `///evil.example`.
        "%2F%252F%252Fevil.example",
        "relative",
    ] {
        let res = login_flow(
            &server,
            &idp,
            &format!("purpose=login&redirect={redirect}"),
            "sub-mom",
        )
        .await;
        assert_eq!(location(&res), "/", "{redirect}");
        assert!(res.maybe_cookie(session::COOKIE_NAME).is_some());
    }
}
