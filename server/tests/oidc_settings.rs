//! OIDC 설정 API. discovery 는 테스트 안 가짜 IdP.

mod common;

use axum::http::StatusCode;
use axum_test::{TestResponse, TestServer};
use chagokmoa::{
    auth::password,
    users::{self, NewUser, Role},
};
use common::{
    PASSWORD, XRW, XRW_VALUE, assert_error, oidc::FakeIdp, server_as, test_server_without_cookies,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const SECRET: &str = "oidc-client-secret-4321";
const CALLBACK: &str = "http://localhost/api/auth/oidc/callback";

fn config_body(issuer: &str, secret: Option<&str>) -> Value {
    let mut body = json!({
        "issuer_url": issuer,
        "client_id": "chagokmoa",
    });
    if let Some(secret) = secret {
        body["client_secret"] = json!(secret);
    }
    body
}

async fn put_oidc(server: &TestServer, body: &Value) -> TestResponse {
    server
        .put("/api/oidc")
        .add_header(XRW, XRW_VALUE)
        .json(body)
        .await
}

async fn stored(pool: &PgPool) -> Option<Value> {
    sqlx::query_scalar!("SELECT value FROM settings WHERE key = 'oidc'")
        .fetch_optional(pool)
        .await
        .expect("oidc settings")
}

async fn status_oidc(pool: &PgPool) -> Value {
    let server = test_server_without_cookies(pool.clone()).await;
    let res = server.get("/api/auth/status").await;
    res.assert_status_ok();
    res.json::<Value>()["oidc"].clone()
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

/// 없는 주소. 묶었다 푼 포트라 접속 거부.
async fn closed_port_url() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

#[sqlx::test(migrations = "./migrations")]
async fn oidc_settings_owner_only(pool: PgPool) {
    let idp = FakeIdp::start().await;
    for (name, role) in [("dad", Role::Guardian), ("grandma", Role::Family)] {
        let (server, _) = server_as(&pool, name, role).await;
        assert_error(
            &server.get("/api/oidc").await,
            StatusCode::FORBIDDEN,
            "auth.forbidden",
        );
        assert_error(
            &put_oidc(&server, &config_body(&idp.issuer, None)).await,
            StatusCode::FORBIDDEN,
            "auth.forbidden",
        );
        assert_error(
            &server.delete("/api/oidc").add_header(XRW, XRW_VALUE).await,
            StatusCode::FORBIDDEN,
            "auth.forbidden",
        );
    }
    assert_eq!(stored(&pool).await, None);
}

#[sqlx::test(migrations = "./migrations")]
async fn get_hides_secret(pool: PgPool) {
    let idp = FakeIdp::start().await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;

    let res = server.get("/api/oidc").await;
    res.assert_status_ok();
    res.assert_json(&json!({
        "config": null,
        "callback_url": CALLBACK,
        "linked": 0,
        "passwordless": 0,
    }));

    let res = put_oidc(&server, &config_body(&idp.issuer, Some(SECRET))).await;
    res.assert_status_ok();
    assert!(!res.text().contains(SECRET), "{}", res.text());

    let res = server.get("/api/oidc").await;
    res.assert_status_ok();
    assert!(!res.text().contains(SECRET), "{}", res.text());
    res.assert_json(&json!({
        "config": {
            "issuer_url": idp.issuer,
            "client_id": "chagokmoa",
            "secret_set": true,
            "button_label": "SSO",
        },
        "callback_url": CALLBACK,
        "linked": 0,
        "passwordless": 0,
    }));
}

#[sqlx::test(migrations = "./migrations")]
async fn put_keeps_secret_when_omitted(pool: PgPool) {
    let idp = FakeIdp::start().await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    put_oidc(&server, &config_body(&idp.issuer, Some(SECRET)))
        .await
        .assert_status_ok();

    let mut body = config_body(&idp.issuer, None);
    body["client_id"] = json!(" chagokmoa ");
    body["button_label"] = json!("Authentik");
    let res = put_oidc(&server, &body).await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["config"]["secret_set"], true);
    let value = stored(&pool).await.expect("saved");
    assert_eq!(value["client_secret"], SECRET);
    assert_eq!(value["client_id"], "chagokmoa");
}

#[sqlx::test(migrations = "./migrations")]
async fn put_drops_secret_on_issuer_change(pool: PgPool) {
    let idp = FakeIdp::start().await;
    let other = FakeIdp::start().await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;

    // issuer 변경.
    put_oidc(&server, &config_body(&idp.issuer, Some(SECRET)))
        .await
        .assert_status_ok();
    let res = put_oidc(&server, &config_body(&other.issuer, None)).await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["config"]["secret_set"], false);
    let value = stored(&pool).await.expect("saved");
    assert!(value.get("client_secret").is_none(), "{value}");

    // client_id 변경.
    put_oidc(&server, &config_body(&idp.issuer, Some(SECRET)))
        .await
        .assert_status_ok();
    let mut body = config_body(&idp.issuer, None);
    body["client_id"] = json!("other-client");
    let res = put_oidc(&server, &body).await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["config"]["secret_set"], false);

    // 새 secret 을 함께 보내면 유지.
    let mut body = config_body(&other.issuer, Some("fresh-secret"));
    body["client_id"] = json!("third");
    put_oidc(&server, &body).await.assert_status_ok();
    assert_eq!(
        stored(&pool).await.expect("saved")["client_secret"],
        "fresh-secret"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn put_clears_secret_with_empty(pool: PgPool) {
    let idp = FakeIdp::start().await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    put_oidc(&server, &config_body(&idp.issuer, Some(SECRET)))
        .await
        .assert_status_ok();

    let res = put_oidc(&server, &config_body(&idp.issuer, Some(""))).await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["config"]["secret_set"], false);
    let value = stored(&pool).await.expect("saved");
    assert!(value.get("client_secret").is_none(), "{value}");
}

#[sqlx::test(migrations = "./migrations")]
async fn put_rejects_unreachable_issuer(pool: PgPool) {
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    let issuer = closed_port_url().await;
    let res = put_oidc(&server, &config_body(&issuer, Some(SECRET))).await;
    res.assert_status(StatusCode::BAD_REQUEST);
    let body = res.json::<Value>();
    assert_eq!(body["error"], "oidc.discoveryFailed");
    let message = body["params"]["message"].as_str().expect("message");
    assert!(!message.is_empty());
    assert!(!res.text().contains(SECRET), "{}", res.text());
    assert_eq!(stored(&pool).await, None);
}

#[sqlx::test(migrations = "./migrations")]
async fn put_rejects_issuer_mismatch(pool: PgPool) {
    let idp = FakeIdp::start().await;
    idp.set_discovery_issuer("https://other.example");
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    let res = put_oidc(&server, &config_body(&idp.issuer, None)).await;
    res.assert_status(StatusCode::BAD_REQUEST);
    let body = res.json::<Value>();
    assert_eq!(body["error"], "oidc.discoveryFailed");
    let message = body["params"]["message"].as_str().expect("message");
    assert!(message.contains("https://other.example"), "{message}");
    assert_eq!(stored(&pool).await, None);
}

#[sqlx::test(migrations = "./migrations")]
async fn put_stores_discovery_issuer(pool: PgPool) {
    // 입력과 discovery issuer 의 끝 슬래시 차이 허용. 저장은 discovery 값.
    let idp = FakeIdp::start().await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    let res = put_oidc(&server, &config_body(&format!(" {}/ ", idp.issuer), None)).await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["config"]["issuer_url"], idp.issuer);

    idp.set_discovery_issuer(&format!("{}/", idp.issuer));
    let res = put_oidc(&server, &config_body(&idp.issuer, None)).await;
    res.assert_status_ok();
    assert_eq!(
        res.json::<Value>()["config"]["issuer_url"],
        format!("{}/", idp.issuer)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn put_requires_client_id(pool: PgPool) {
    let idp = FakeIdp::start().await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    let mut body = config_body(&idp.issuer, None);
    body["client_id"] = json!("  ");
    assert_error(
        &put_oidc(&server, &body).await,
        StatusCode::BAD_REQUEST,
        "oidc.clientIdRequired",
    );
    assert_eq!(stored(&pool).await, None);
}

#[sqlx::test(migrations = "./migrations")]
async fn status_reports_oidc_label(pool: PgPool) {
    let idp = FakeIdp::start().await;
    assert_eq!(status_oidc(&pool).await, Value::Null);

    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    let mut body = config_body(&idp.issuer, None);
    body["button_label"] = json!("  ");
    put_oidc(&server, &body).await.assert_status_ok();
    assert_eq!(status_oidc(&pool).await, json!({ "label": "SSO" }));

    body["button_label"] = json!(" Authentik ");
    let res = put_oidc(&server, &body).await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["config"]["button_label"], "Authentik");
    assert_eq!(status_oidc(&pool).await, json!({ "label": "Authentik" }));
}

#[sqlx::test(migrations = "./migrations")]
async fn counts_linked_and_passwordless(pool: PgPool) {
    let idp = FakeIdp::start().await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    put_oidc(&server, &config_body(&idp.issuer, None))
        .await
        .assert_status_ok();

    let a = create_user(&pool, "aunt", true).await;
    let b = create_user(&pool, "uncle", false).await;
    let c = create_user(&pool, "cousin", false).await;
    link(&pool, a, &idp.issuer, "sub-a").await;
    link(&pool, b, &idp.issuer, "sub-b").await;
    link(&pool, c, "https://old-idp.example", "sub-c").await;

    let res = server.get("/api/oidc").await;
    res.assert_status_ok();
    let body = res.json::<Value>();
    assert_eq!(body["linked"], 2);
    assert_eq!(body["passwordless"], 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_keeps_user_links(pool: PgPool) {
    let idp = FakeIdp::start().await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    put_oidc(&server, &config_body(&idp.issuer, Some(SECRET)))
        .await
        .assert_status_ok();
    let a = create_user(&pool, "aunt", false).await;
    link(&pool, a, &idp.issuer, "sub-a").await;

    server
        .delete("/api/oidc")
        .add_header(XRW, XRW_VALUE)
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(stored(&pool).await, None);
    assert_eq!(status_oidc(&pool).await, Value::Null);
    let res = server.get("/api/oidc").await;
    res.assert_status_ok();
    res.assert_json(&json!({
        "config": null,
        "callback_url": CALLBACK,
        "linked": 0,
        "passwordless": 0,
    }));

    let row = sqlx::query!(
        "SELECT oidc_issuer, oidc_subject FROM users WHERE id = $1",
        a
    )
    .fetch_one(&pool)
    .await
    .expect("user");
    assert_eq!(row.oidc_issuer.as_deref(), Some(idp.issuer.as_str()));
    assert_eq!(row.oidc_subject.as_deref(), Some("sub-a"));

    // 같은 설정을 다시 넣으면 연결 복구.
    put_oidc(&server, &config_body(&idp.issuer, None))
        .await
        .assert_status_ok();
    let res = server.get("/api/oidc").await;
    assert_eq!(res.json::<Value>()["linked"], 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn login_method_reads_configured_issuer(pool: PgPool) {
    let idp = FakeIdp::start().await;
    let (server, mom) = server_as(&pool, "mom", Role::Owner).await;
    link(&pool, mom.id, &idp.issuer, "sub-mom").await;

    let res = server.get("/api/auth/login-method").await;
    res.assert_json(&json!({ "has_password": true, "oidc": "stale" }));

    put_oidc(&server, &config_body(&idp.issuer, None))
        .await
        .assert_status_ok();
    let res = server.get("/api/auth/login-method").await;
    res.assert_json(&json!({ "has_password": true, "oidc": "linked" }));
}

#[sqlx::test(migrations = "./migrations")]
async fn discovery_does_not_follow_redirect(pool: PgPool) {
    // 302 대상은 정상 discovery. 따라가면 issuer 불일치로 다른 문구.
    let idp = FakeIdp::start().await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    let res = put_oidc(
        &server,
        &config_body(&format!("{}/moved", idp.issuer), None),
    )
    .await;
    res.assert_status(StatusCode::BAD_REQUEST);
    let body = res.json::<Value>();
    assert_eq!(body["error"], "oidc.discoveryFailed");
    let message = body["params"]["message"].as_str().expect("message");
    assert!(message.contains("302"), "{message}");
    assert_eq!(stored(&pool).await, None);
}

#[sqlx::test(migrations = "./migrations")]
async fn discovery_rejects_oversized_body(pool: PgPool) {
    let idp = FakeIdp::start().await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    let res = put_oidc(&server, &config_body(&format!("{}/huge", idp.issuer), None)).await;
    res.assert_status(StatusCode::BAD_REQUEST);
    let body = res.json::<Value>();
    assert_eq!(body["error"], "oidc.discoveryFailed");
    let message = body["params"]["message"].as_str().expect("message");
    assert!(message.contains("too large"), "{message}");
    assert_eq!(stored(&pool).await, None);
}
