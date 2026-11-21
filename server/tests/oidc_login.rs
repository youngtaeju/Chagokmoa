//! OIDC 로그인과 계정 연결. 테스트 안 가짜 IdP 와 authorization code 흐름 전체.

mod common;

use axum::http::{StatusCode, header};
use axum_extra::extract::cookie::Cookie;
use axum_test::{TestResponse, TestServer};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chagokmoa::{
    auth::{password, session},
    invites::{self, InviteStatus, NewInvite},
    oidc::{
        cookie::{self as progress, Progress},
        settings::{self, OidcConfig},
    },
    users::{self, NewUser, Role},
};
use chrono::{Duration, Utc};
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
    url: String,
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
        url: to,
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

async fn new_invite(pool: &PgPool, created_by: Uuid, role: Role) -> String {
    let (raw, _) = invites::queries::create(
        pool,
        NewInvite {
            role,
            created_by,
            expires_in: Duration::hours(72),
        },
    )
    .await
    .expect("invite");
    raw
}

async fn invite_row(pool: &PgPool, raw: &str) -> invites::Invite {
    invites::queries::find_by_raw(pool, raw)
        .await
        .expect("find invite")
        .expect("invite exists")
}

async fn user_count(pool: &PgPool) -> i64 {
    users::queries::count(pool).await.expect("count")
}

fn invite_query(raw: &str, username: &str, display_name: &str) -> String {
    format!("purpose=invite&invite={raw}&username={username}&display_name={display_name}")
}

#[sqlx::test(migrations = "./migrations")]
async fn invite_oidc_signup_uses_locale(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let owner = create_user(&pool, "owner", true).await;
    let server = test_server_without_cookies(pool.clone()).await;

    // 허용 언어는 사용자 locale 로, 잘못된 값은 인스턴스 기본 언어(ko)로.
    for (locale, expected) in [("en", "en"), ("xx", "ko")] {
        let raw = new_invite(&pool, owner, Role::Guardian).await;
        let query = format!(
            "{}&locale={locale}",
            invite_query(&raw, &format!("aunt_{locale}"), "Aunt")
        );
        let s = start(&server, &idp, &query, None).await;
        let code = idp.issue_code(&format!("sub-{locale}"), &s.nonce, CLIENT_ID);
        let res = callback(
            &server,
            Some(&s.progress),
            None,
            &[("code", &code), ("state", &s.state)],
        )
        .await;
        assert_eq!(location(&res), "/");
        let session_cookie = res.cookie(session::COOKIE_NAME);
        let user = me(&server, &session_cookie).await["user"].clone();
        assert_eq!(user["locale"], expected, "{locale}");
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn invite_oidc_signup_success(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let owner = create_user(&pool, "owner", true).await;
    let raw = new_invite(&pool, owner, Role::Guardian).await;
    let server = test_server_without_cookies(pool.clone()).await;

    // 아이디 소문자 정규화, 표시 이름 앞뒤 공백 제거. " 이모 " 인코딩.
    let s = start(
        &server,
        &idp,
        &invite_query(&raw, "Aunt", "%20%EC%9D%B4%EB%AA%A8%20"),
        None,
    )
    .await;
    // 초대 원문과 입력값은 IdP 주소에 없음. 진행 쿠키에만.
    assert!(!s.url.contains(&raw), "{}", s.url);
    assert!(!s.url.to_lowercase().contains("aunt"), "{}", s.url);
    let decoded = progress::decode(&test_config().secret_key, s.progress.value(), Utc::now())
        .expect("decode");
    assert_eq!(decoded.invite.as_deref(), Some(raw.as_str()));
    assert_eq!(decoded.username.as_deref(), Some("aunt"));
    assert_eq!(decoded.display_name.as_deref(), Some("\u{c774}\u{baa8}"));

    let code = idp.issue_code("sub-aunt", &s.nonce, CLIENT_ID);
    let res = callback(
        &server,
        Some(&s.progress),
        None,
        &[("code", &code), ("state", &s.state)],
    )
    .await;
    assert_eq!(location(&res), "/");
    let session_cookie = res.cookie(session::COOKIE_NAME);
    let user = me(&server, &session_cookie).await["user"].clone();
    assert_eq!(user["username"], "aunt");
    assert_eq!(user["display_name"], "\u{c774}\u{baa8}");
    assert_eq!(user["role"], "guardian");
    assert_eq!(user["locale"], "ko");
    let id: Uuid = serde_json::from_value(user["id"].clone()).expect("user id");

    let hash = sqlx::query_scalar!("SELECT password_hash FROM users WHERE id = $1", id)
        .fetch_one(&pool)
        .await
        .expect("user row");
    assert_eq!(hash, None);
    assert_eq!(
        stored_link(&pool, id).await,
        (Some(idp.issuer.clone()), Some("sub-aunt".to_owned()))
    );
    let invite = invite_row(&pool, &raw).await;
    assert_eq!(invite.status, InviteStatus::Used);
    assert_eq!(invite.used_by, Some(id));
    let method = login_method(&server, &session_cookie).await;
    assert_eq!(method["has_password"], false);
    assert_eq!(method["oidc"], "linked");

    // 다음부터 외부 로그인.
    let res = login_flow(&server, &idp, "purpose=login", "sub-aunt").await;
    assert_eq!(location(&res), "/");
    let new_session = res.cookie(session::COOKIE_NAME);
    assert_eq!(me(&server, &new_session).await["user"]["id"], json!(id));
}

#[sqlx::test(migrations = "./migrations")]
async fn invite_start_validates(pool: PgPool) {
    let idp = FakeIdp::start().await;
    let owner = create_user(&pool, "owner", true).await;
    create_user(&pool, "taken", true).await;
    let raw = new_invite(&pool, owner, Role::Family).await;
    let server = test_server_without_cookies(pool.clone()).await;

    let expect = |query: String, to: String| {
        let server = &server;
        async move {
            let res = server.get(&format!("/api/auth/oidc/start?{query}")).await;
            assert_eq!(location(&res), to, "{query}");
            assert!(res.maybe_cookie(progress::COOKIE_NAME).is_none(), "{query}");
        }
    };

    // OIDC 꺼짐.
    expect(
        invite_query(&raw, "aunt", "aunt"),
        format!("/invite/{raw}?oidc_error=oidc.disabled"),
    )
    .await;

    configure(&pool, &idp.issuer, None).await;
    for (username, display_name, key) in [
        ("TAKEN", "aunt", "auth.usernameTaken"),
        ("a!", "aunt", "auth.usernameInvalid"),
        ("", "aunt", "auth.usernameInvalid"),
        ("aunt", "%20%20", "auth.displayNameInvalid"),
    ] {
        expect(
            invite_query(&raw, username, display_name),
            format!("/invite/{raw}?oidc_error={key}"),
        )
        .await;
    }

    // 사용, 회수, 만료, 없는 초대.
    let used = new_invite(&pool, owner, Role::Family).await;
    server
        .post(&format!("/api/invites/{used}/accept"))
        .json(&json!({ "username": "uncle", "password": PASSWORD, "display_name": "uncle" }))
        .await
        .assert_status(StatusCode::CREATED);
    expect(
        invite_query(&used, "aunt", "aunt"),
        format!("/invite/{used}?oidc_error=invite.used"),
    )
    .await;
    let revoked = new_invite(&pool, owner, Role::Family).await;
    invites::queries::revoke(&pool, invite_row(&pool, &revoked).await.id)
        .await
        .expect("revoke");
    expect(
        invite_query(&revoked, "aunt", "aunt"),
        format!("/invite/{revoked}?oidc_error=invite.revoked"),
    )
    .await;
    let expired = new_invite(&pool, owner, Role::Family).await;
    expire(&pool, &expired).await;
    expect(
        invite_query(&expired, "aunt", "aunt"),
        format!("/invite/{expired}?oidc_error=invite.expired"),
    )
    .await;
    // 경로 조각으로 인코딩. 다른 경로나 쿼리로 새지 않음.
    expect(
        "purpose=invite&invite=..%2Fx%3Fa%23b&username=aunt&display_name=aunt".to_owned(),
        "/invite/..%2Fx%3Fa%23b?oidc_error=invite.notFound".to_owned(),
    )
    .await;

    // 초대 원문 없으면 돌아갈 초대 화면이 없음.
    let res = server
        .get("/api/auth/oidc/start?purpose=invite&username=aunt&display_name=aunt")
        .await;
    res.assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(res.json::<Value>()["error"], "request.invalidQuery");

    assert!(idp.token_requests().is_empty());
    assert_eq!(user_count(&pool).await, 3);
    assert_eq!(invite_row(&pool, &raw).await.status, InviteStatus::Active);
}

async fn expire(pool: &PgPool, raw: &str) {
    let id = invite_row(pool, raw).await.id;
    sqlx::query!(
        "UPDATE invites SET expires_at = now() - interval '1 second' WHERE id = $1",
        id
    )
    .execute(pool)
    .await
    .expect("expire invite");
}

#[sqlx::test(migrations = "./migrations")]
async fn invite_callback_rechecks(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let owner = create_user(&pool, "owner", true).await;
    let server = test_server_without_cookies(pool.clone()).await;

    // 아이디 선점.
    let raw = new_invite(&pool, owner, Role::Family).await;
    let s = start(&server, &idp, &invite_query(&raw, "aunt", "aunt"), None).await;
    create_user(&pool, "Aunt", true).await;
    let code = idp.issue_code("sub-aunt", &s.nonce, CLIENT_ID);
    let res = callback(
        &server,
        Some(&s.progress),
        None,
        &[("code", &code), ("state", &s.state)],
    )
    .await;
    assert_eq!(
        location(&res),
        format!("/invite/{raw}?oidc_error=auth.usernameTaken")
    );
    assert_no_session(&res);
    assert_eq!(invite_row(&pool, &raw).await.status, InviteStatus::Active);

    // 회수, 만료, 다른 사람이 먼저 수락.
    let revoked = new_invite(&pool, owner, Role::Family).await;
    let expired = new_invite(&pool, owner, Role::Family).await;
    let used = new_invite(&pool, owner, Role::Family).await;
    for (raw, key) in [
        (&revoked, "invite.revoked"),
        (&expired, "invite.expired"),
        (&used, "invite.used"),
    ] {
        let s = start(&server, &idp, &invite_query(raw, "uncle", "uncle"), None).await;
        match key {
            "invite.revoked" => invites::queries::revoke(&pool, invite_row(&pool, raw).await.id)
                .await
                .expect("revoke"),
            "invite.expired" => expire(&pool, raw).await,
            _ => server
                .post(&format!("/api/invites/{raw}/accept"))
                .json(&json!({ "username": "cousin", "password": PASSWORD, "display_name": "cousin" }))
                .await
                .assert_status(StatusCode::CREATED),
        }
        let code = idp.issue_code("sub-uncle", &s.nonce, CLIENT_ID);
        let res = callback(
            &server,
            Some(&s.progress),
            None,
            &[("code", &code), ("state", &s.state)],
        )
        .await;
        assert_eq!(location(&res), format!("/invite/{raw}?oidc_error={key}"));
        assert_no_session(&res);
    }

    // owner, 선점한 Aunt, 먼저 수락한 cousin 뿐.
    assert_eq!(user_count(&pool).await, 3);
    let linked = users::queries::find_by_oidc(&pool, &idp.issuer, "sub-aunt")
        .await
        .expect("find");
    assert!(linked.is_none());
    let linked = users::queries::find_by_oidc(&pool, &idp.issuer, "sub-uncle")
        .await
        .expect("find");
    assert!(linked.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn invite_rejects_already_linked_sub(pool: PgPool) {
    let idp = FakeIdp::start().await;
    configure(&pool, &idp.issuer, None).await;
    let owner = create_user(&pool, "owner", true).await;
    let mom = create_user(&pool, "mom", true).await;
    link(&pool, mom, &idp.issuer, "sub-mom").await;
    let raw = new_invite(&pool, owner, Role::Family).await;
    let server = test_server_without_cookies(pool.clone()).await;

    let s = start(&server, &idp, &invite_query(&raw, "aunt", "aunt"), None).await;
    let code = idp.issue_code("sub-mom", &s.nonce, CLIENT_ID);
    let res = callback(
        &server,
        Some(&s.progress),
        None,
        &[("code", &code), ("state", &s.state)],
    )
    .await;
    assert_eq!(
        location(&res),
        format!("/invite/{raw}?oidc_error=oidc.alreadyLinked")
    );
    assert_no_session(&res);
    assert_eq!(user_count(&pool).await, 2);
    assert_eq!(invite_row(&pool, &raw).await.status, InviteStatus::Active);
    assert_eq!(
        stored_link(&pool, mom).await,
        (Some(idp.issuer.clone()), Some("sub-mom".to_owned()))
    );
}
