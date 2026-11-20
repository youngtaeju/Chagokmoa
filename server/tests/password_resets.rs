mod common;

use axum::http::StatusCode;
use axum_test::{TestResponse, TestServer};
use chagokmoa::{auth::session, users::Role};
use chrono::{DateTime, Duration, Utc};
use common::{
    PASSWORD, XRW, XRW_VALUE, assert_error, server_as,
    smtp::{self, Mode, Receiver},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn create_link(server: &TestServer, user_id: Uuid) -> TestResponse {
    create_link_with(server, user_id, json!({})).await
}

async fn create_link_with(server: &TestServer, user_id: Uuid, body: Value) -> TestResponse {
    server
        .post(&format!("/api/users/{user_id}/password-reset"))
        .add_header(XRW, XRW_VALUE)
        .json(&body)
        .await
}

/// 링크 생성. 원문 토큰 반환.
async fn link(server: &TestServer, user_id: Uuid) -> String {
    let res = create_link(server, user_id).await;
    res.assert_status(StatusCode::CREATED);
    res.json::<Value>()["url"]
        .as_str()
        .expect("url")
        .strip_prefix("http://localhost/reset/")
        .expect("url prefix")
        .to_string()
}

async fn preview(server: &TestServer, raw: &str) -> TestResponse {
    server
        .get(&format!("/api/password-resets/{raw}/preview"))
        .await
}

async fn accept(server: &TestServer, raw: &str, password: &str) -> TestResponse {
    server
        .post(&format!("/api/password-resets/{raw}/accept"))
        .add_header(XRW, XRW_VALUE)
        .json(&json!({ "password": password }))
        .await
}

async fn login(server: &TestServer, username: &str, password: &str) -> TestResponse {
    server
        .post("/api/auth/login")
        .json(&json!({ "username": username, "password": password }))
        .await
}

/// owner `mom`, family `dad` (쿠키 저장 서버 포함).
async fn setup(pool: &PgPool) -> (TestServer, TestServer, Uuid) {
    let (owner, _) = server_as(pool, "mom", Role::Owner).await;
    let (dad_server, dad) = server_as(pool, "dad", Role::Family).await;
    (owner, dad_server, dad.id)
}

#[sqlx::test(migrations = "./migrations")]
async fn create_owner_only(pool: PgPool) {
    let (_, _, dad_id) = setup(&pool).await;
    let (guardian, _) = server_as(&pool, "aunt", Role::Guardian).await;
    let (family, _) = server_as(&pool, "uncle", Role::Family).await;
    for server in [&guardian, &family] {
        assert_error(
            &create_link(server, dad_id).await,
            StatusCode::FORBIDDEN,
            "auth.forbidden",
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn create_self_rejected(pool: PgPool) {
    let (owner, mom) = server_as(&pool, "mom", Role::Owner).await;
    assert_error(
        &create_link(&owner, mom.id).await,
        StatusCode::BAD_REQUEST,
        "passwordReset.self",
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn create_disabled_rejected(pool: PgPool) {
    let (owner, _, dad_id) = setup(&pool).await;
    owner
        .patch(&format!("/api/users/{dad_id}"))
        .add_header(XRW, XRW_VALUE)
        .json(&json!({ "disabled": true }))
        .await
        .assert_status_ok();
    assert_error(
        &create_link(&owner, dad_id).await,
        StatusCode::CONFLICT,
        "passwordReset.userDisabled",
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn create_unknown_user(pool: PgPool) {
    let (owner, _, _) = setup(&pool).await;
    assert_error(
        &create_link(&owner, Uuid::now_v7()).await,
        StatusCode::NOT_FOUND,
        "users.notFound",
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn create_returns_reset_url(pool: PgPool) {
    let (owner, _, dad_id) = setup(&pool).await;
    let res = create_link(&owner, dad_id).await;
    res.assert_status(StatusCode::CREATED);
    let body: Value = res.json();
    assert!(
        body["url"]
            .as_str()
            .expect("url")
            .starts_with("http://localhost/reset/")
    );
    assert!(body["mail_sent"].is_null());
    let expires: DateTime<Utc> = serde_json::from_value(body["expires_at"].clone()).expect("ts");
    let diff = (expires - (Utc::now() + Duration::hours(24)))
        .num_seconds()
        .abs();
    assert!(diff < 60, "{expires}");
}

#[sqlx::test(migrations = "./migrations")]
async fn preview_shows_target(pool: PgPool) {
    let (owner, _, dad_id) = setup(&pool).await;
    let raw = link(&owner, dad_id).await;
    let guest = common::test_server(pool.clone()).await;
    let res = preview(&guest, &raw).await;
    res.assert_status_ok();
    let body: Value = res.json();
    assert_eq!(body["display_name"], "dad");
    assert_eq!(body["username"], "dad");
    assert!(body["expires_at"].is_string());

    assert_error(
        &preview(&guest, "nope").await,
        StatusCode::NOT_FOUND,
        "passwordReset.notFound",
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn reset_link_does_not_change_until_used(pool: PgPool) {
    let (owner, dad_server, dad_id) = setup(&pool).await;
    link(&owner, dad_id).await;
    dad_server.get("/api/auth/me").await.assert_status_ok();
    let fresh = common::test_server(pool.clone()).await;
    login(&fresh, "dad", PASSWORD).await.assert_status_ok();
}

#[sqlx::test(migrations = "./migrations")]
async fn accept_changes_password_and_logs_out_others(pool: PgPool) {
    let (owner, dad_server, dad_id) = setup(&pool).await;
    let raw = link(&owner, dad_id).await;

    let guest = common::test_server(pool.clone()).await;
    let res = accept(&guest, &raw, "brand-new-pass").await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["user"]["username"], "dad");
    let cookie = res.cookie(session::COOKIE_NAME);

    let other = common::test_server(pool.clone()).await;
    login(&other, "dad", "brand-new-pass")
        .await
        .assert_status_ok();
    login(&other, "dad", PASSWORD)
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    dad_server
        .get("/api/auth/me")
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    // 응답 쿠키 세션 유효.
    let me = guest.get("/api/auth/me").add_cookie(cookie).await;
    me.assert_status_ok();
    assert_eq!(me.json::<Value>()["user"]["username"], "dad");
}

#[sqlx::test(migrations = "./migrations")]
async fn reset_link_single_use(pool: PgPool) {
    let (owner, _, dad_id) = setup(&pool).await;
    let raw = link(&owner, dad_id).await;
    let guest = common::test_server(pool.clone()).await;
    accept(&guest, &raw, "brand-new-pass")
        .await
        .assert_status_ok();
    assert_error(
        &accept(&guest, &raw, "another-pass-1").await,
        StatusCode::GONE,
        "passwordReset.used",
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn new_link_revokes_previous(pool: PgPool) {
    let (owner, _, dad_id) = setup(&pool).await;
    let first = link(&owner, dad_id).await;
    let second = link(&owner, dad_id).await;
    let guest = common::test_server(pool.clone()).await;
    assert_error(
        &preview(&guest, &first).await,
        StatusCode::GONE,
        "passwordReset.revoked",
    );
    preview(&guest, &second).await.assert_status_ok();
}

#[sqlx::test(migrations = "./migrations")]
async fn expired_link_rejected(pool: PgPool) {
    let (owner, _, dad_id) = setup(&pool).await;
    let raw = link(&owner, dad_id).await;
    sqlx::query("UPDATE password_resets SET expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .expect("expire");
    let guest = common::test_server(pool.clone()).await;
    assert_error(
        &preview(&guest, &raw).await,
        StatusCode::GONE,
        "passwordReset.expired",
    );
    assert_error(
        &accept(&guest, &raw, "brand-new-pass").await,
        StatusCode::GONE,
        "passwordReset.expired",
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn accept_rejected_if_suspended_meanwhile(pool: PgPool) {
    let (owner, _, dad_id) = setup(&pool).await;
    let raw = link(&owner, dad_id).await;
    owner
        .patch(&format!("/api/users/{dad_id}"))
        .add_header(XRW, XRW_VALUE)
        .json(&json!({ "disabled": true }))
        .await
        .assert_status_ok();
    let guest = common::test_server(pool.clone()).await;
    assert_error(
        &preview(&guest, &raw).await,
        StatusCode::GONE,
        "passwordReset.revoked",
    );
    assert_error(
        &accept(&guest, &raw, "brand-new-pass").await,
        StatusCode::GONE,
        "passwordReset.revoked",
    );
    // 정지 해제 후에도 회수 상태 유지.
    owner
        .patch(&format!("/api/users/{dad_id}"))
        .add_header(XRW, XRW_VALUE)
        .json(&json!({ "disabled": false }))
        .await
        .assert_status_ok();
    assert_error(
        &preview(&guest, &raw).await,
        StatusCode::GONE,
        "passwordReset.revoked",
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn own_password_change_revokes_link(pool: PgPool) {
    let (owner, dad_server, dad_id) = setup(&pool).await;
    let raw = link(&owner, dad_id).await;
    dad_server
        .post("/api/users/me/password")
        .add_header(XRW, XRW_VALUE)
        .json(&json!({ "current_password": PASSWORD, "new_password": "changed-pass-1" }))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let guest = common::test_server(pool.clone()).await;
    assert_error(
        &preview(&guest, &raw).await,
        StatusCode::GONE,
        "passwordReset.revoked",
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn accept_short_password(pool: PgPool) {
    let (owner, _, dad_id) = setup(&pool).await;
    let raw = link(&owner, dad_id).await;
    let guest = common::test_server(pool.clone()).await;
    assert_error(
        &accept(&guest, &raw, "short").await,
        StatusCode::BAD_REQUEST,
        "auth.passwordTooShort",
    );
    accept(&guest, &raw, "brand-new-pass")
        .await
        .assert_status_ok();
}

#[sqlx::test(migrations = "./migrations")]
async fn reset_mail_sent_in_target_locale(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (owner, _, dad_id) = setup(&pool).await;
    smtp::configure(&pool, receiver.port).await;
    sqlx::query("UPDATE users SET email = $1, locale = 'en' WHERE id = $2")
        .bind("dad@example.com")
        .bind(dad_id)
        .execute(&pool)
        .await
        .expect("set email");

    let res = create_link_with(&owner, dad_id, json!({ "send_email": true })).await;
    res.assert_status(StatusCode::CREATED);
    let body: Value = res.json();
    assert_eq!(body["mail_sent"], true);
    let mails = receiver.messages();
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to, vec!["dad@example.com".to_owned()]);
    let text = mails[0].body();
    assert!(text.contains(body["url"].as_str().expect("url")), "{text}");
    assert!(text.contains("dad"), "{text}");
    assert!(text.contains("24 hours"), "{text}");
    assert!(text.contains("password"), "{text}");
}

#[sqlx::test(migrations = "./migrations")]
async fn reset_mail_no_target_email(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (owner, _, dad_id) = setup(&pool).await;
    smtp::configure(&pool, receiver.port).await;

    let res = create_link_with(&owner, dad_id, json!({ "send_email": true })).await;
    res.assert_status(StatusCode::CREATED);
    assert_eq!(res.json::<Value>()["mail_sent"], false);
    assert!(receiver.messages().is_empty());
}
