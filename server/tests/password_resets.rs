mod common;

use std::time::Duration as StdDuration;

use axum::http::StatusCode;
use axum_test::{TestResponse, TestServer};
use chagokmoa::{app::build_router, auth::session, push::send, state::AppState, users::Role};
use chrono::{DateTime, Duration, Utc};
use common::{
    PASSWORD, XRW, XRW_VALUE, assert_error, server_as,
    smtp::{self, Mode, ReceivedMail, Receiver},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::watch;
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

#[sqlx::test(migrations = "./migrations")]
async fn create_reset_without_body(pool: PgPool) {
    let (owner, _, dad_id) = setup(&pool).await;
    let res = owner
        .post(&format!("/api/users/{dad_id}/password-reset"))
        .add_header(XRW, XRW_VALUE)
        .await;
    res.assert_status(StatusCode::CREATED);
    assert!(res.json::<Value>()["mail_sent"].is_null());
}

/// 비로그인 서버와 같은 상태. 요청의 `push_wake` 를 워커가 받음.
struct Guest {
    state: AppState,
    server: TestServer,
}

fn guest(pool: &PgPool) -> Guest {
    let state = AppState::new(pool.clone(), common::test_config());
    let server = TestServer::new(build_router(state.clone())).expect("test server");
    Guest { state, server }
}

async fn forgot(server: &TestServer, login: &str) -> TestResponse {
    server
        .post("/api/auth/forgot-password")
        .json(&json!({ "login": login }))
        .await
}

/// 204, 본문 없음.
async fn forgot_ok(server: &TestServer, login: &str) {
    let res = forgot(server, login).await;
    res.assert_status(StatusCode::NO_CONTENT);
    assert!(res.text().is_empty(), "{}", res.text());
}

/// 알림 발송 워커를 대기, 실행 중 작업이 없어질 때까지 실행.
async fn drain(state: &AppState) {
    let (stop, shutdown) = watch::channel(false);
    let worker = send::spawn_worker(state.clone(), shutdown);
    state.push_wake.notify_one();
    for _ in 0..200 {
        let pending: i64 =
            sqlx::query_scalar("SELECT count(*) FROM jobs WHERE status IN ('queued', 'running')")
                .fetch_one(&state.pool)
                .await
                .expect("pending jobs");
        if pending == 0 {
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(25)).await;
    }
    stop.send(true).expect("stop worker");
    worker.await.expect("worker");
}

async fn set_email(pool: &PgPool, user_id: Uuid, email: Option<&str>) {
    sqlx::query("UPDATE users SET email = $1 WHERE id = $2")
        .bind(email)
        .bind(user_id)
        .execute(pool)
        .await
        .expect("set email");
}

async fn disable(pool: &PgPool, user_id: Uuid) {
    sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .expect("disable");
}

/// 본문의 재설정 링크 원문 토큰.
fn raw_in(mail: &ReceivedMail) -> String {
    const PREFIX: &str = "http://localhost/reset/";
    let body = mail.body();
    let start = body.find(PREFIX).expect("reset link") + PREFIX.len();
    body[start..]
        .split_whitespace()
        .next()
        .expect("raw token")
        .to_owned()
}

async fn self_rows(pool: &PgPool, user_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM password_resets WHERE created_by = $1 AND user_id = $1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .expect("self rows")
}

async fn uncle(pool: &PgPool) -> Uuid {
    let (_, user) = server_as(pool, "uncle", Role::Family).await;
    user.id
}

#[sqlx::test(migrations = "./migrations")]
async fn forgot_sends_mail_by_username(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (_, _, dad_id) = setup(&pool).await;
    smtp::configure(&pool, receiver.port).await;
    set_email(&pool, dad_id, Some("dad@example.com")).await;
    let g = guest(&pool);

    forgot_ok(&g.server, "dad").await;
    drain(&g.state).await;

    let mails = receiver.messages();
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to, vec!["dad@example.com".to_owned()]);
    let text = mails[0].body();
    assert!(text.contains("dad") && text.contains("1시간"), "{text}");
    let res = preview(&g.server, &raw_in(&mails[0])).await;
    res.assert_status_ok();
    let body: Value = res.json();
    assert_eq!(body["username"], "dad");
    let expires: DateTime<Utc> = serde_json::from_value(body["expires_at"].clone()).expect("ts");
    let diff = (expires - (Utc::now() + Duration::hours(1)))
        .num_seconds()
        .abs();
    assert!(diff < 60, "{expires}");
}

#[sqlx::test(migrations = "./migrations")]
async fn forgot_email_case_insensitive(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (_, _, dad_id) = setup(&pool).await;
    smtp::configure(&pool, receiver.port).await;
    set_email(&pool, dad_id, Some("Kim@Example.com")).await;
    let g = guest(&pool);

    forgot_ok(&g.server, "  kim@example.COM ").await;
    drain(&g.state).await;

    let mails = receiver.messages();
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to, vec!["Kim@Example.com".to_owned()]);
}

#[sqlx::test(migrations = "./migrations")]
async fn forgot_same_email_multiple_accounts(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (_, _, dad_id) = setup(&pool).await;
    let uncle_id = uncle(&pool).await;
    smtp::configure(&pool, receiver.port).await;
    set_email(&pool, dad_id, Some("home@example.com")).await;
    set_email(&pool, uncle_id, Some("home@example.com")).await;
    let g = guest(&pool);

    forgot_ok(&g.server, "home@example.com").await;
    drain(&g.state).await;

    let mails = receiver.messages();
    assert_eq!(mails.len(), 2);
    let mut usernames = Vec::new();
    for mail in &mails {
        let res = preview(&g.server, &raw_in(mail)).await;
        res.assert_status_ok();
        let username = res.json::<Value>()["username"]
            .as_str()
            .expect("username")
            .to_owned();
        assert!(mail.body().contains(&username), "{}", mail.body());
        usernames.push(username);
    }
    usernames.sort();
    assert_eq!(usernames, ["dad", "uncle"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn forgot_same_response_for_all(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (_, _, dad_id) = setup(&pool).await;
    let uncle_id = uncle(&pool).await;
    smtp::configure(&pool, receiver.port).await;
    // dad: 정지, uncle: 이메일 없음.
    set_email(&pool, dad_id, Some("dad@example.com")).await;
    disable(&pool, dad_id).await;
    let g = guest(&pool);

    for login in [
        "nobody",
        "nobody@example.com",
        "dad",
        "dad@example.com",
        "uncle",
        "",
        "   ",
    ] {
        forgot_ok(&g.server, login).await;
    }
    // SMTP 미설정.
    set_email(&pool, uncle_id, Some("uncle@example.com")).await;
    sqlx::query("DELETE FROM settings WHERE key = 'mail'")
        .execute(&pool)
        .await
        .expect("remove mail settings");
    forgot_ok(&g.server, "uncle").await;
    drain(&g.state).await;

    assert!(receiver.messages().is_empty());
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs")
        .fetch_one(&pool)
        .await
        .expect("jobs");
    assert_eq!(jobs, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn forgot_cooldown(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (_, _, dad_id) = setup(&pool).await;
    smtp::configure(&pool, receiver.port).await;
    set_email(&pool, dad_id, Some("dad@example.com")).await;
    let g = guest(&pool);

    // 처리 전 두 번째 요청은 진행 중 작업 확인으로 건너뜀.
    forgot_ok(&g.server, "dad").await;
    forgot_ok(&g.server, "dad").await;
    drain(&g.state).await;
    assert_eq!(receiver.messages().len(), 1);
    // 처리 후 요청은 등록 단계에서 건너뜀.
    forgot_ok(&g.server, "dad").await;
    drain(&g.state).await;
    assert_eq!(receiver.messages().len(), 1);

    sqlx::query(
        "UPDATE password_resets SET created_at = created_at - interval '16 minutes'
         WHERE created_by = $1",
    )
    .bind(dad_id)
    .execute(&pool)
    .await
    .expect("age self rows");
    forgot_ok(&g.server, "dad").await;
    drain(&g.state).await;
    let mails = receiver.messages();
    assert_eq!(mails.len(), 2);
    // 새 셀프 링크가 이전 셀프 링크 회수.
    assert_error(
        &preview(&g.server, &raw_in(&mails[0])).await,
        StatusCode::GONE,
        "passwordReset.revoked",
    );
    preview(&g.server, &raw_in(&mails[1]))
        .await
        .assert_status_ok();
}

#[sqlx::test(migrations = "./migrations")]
async fn forgot_send_failure_allows_retry(pool: PgPool) {
    let rejecting = Receiver::start(Mode::Reject(550, "mailbox unavailable".into())).await;
    let (_, _, dad_id) = setup(&pool).await;
    smtp::configure(&pool, rejecting.port).await;
    set_email(&pool, dad_id, Some("dad@example.com")).await;
    let g = guest(&pool);

    forgot_ok(&g.server, "dad").await;
    drain(&g.state).await;
    assert_eq!(self_rows(&pool, dad_id).await, 0);
    let statuses: Vec<(String, String)> = sqlx::query_as("SELECT kind, status::text FROM jobs")
        .fetch_all(&pool)
        .await
        .expect("jobs");
    assert_eq!(
        statuses,
        [("send_reset_mail".to_owned(), "failed".to_owned())]
    );

    let receiver = Receiver::start(Mode::Accept).await;
    smtp::configure(&pool, receiver.port).await;
    sqlx::query(
        "UPDATE jobs SET locked_at = now() - interval '61 seconds'
         WHERE kind = 'send_reset_mail' AND status = 'failed'",
    )
    .execute(&pool)
    .await
    .expect("age failed jobs");
    forgot_ok(&g.server, "dad").await;
    drain(&g.state).await;
    assert_eq!(receiver.messages().len(), 1);
    assert_eq!(self_rows(&pool, dad_id).await, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn self_link_keeps_admin_link(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (owner, _, dad_id) = setup(&pool).await;
    smtp::configure(&pool, receiver.port).await;
    set_email(&pool, dad_id, Some("dad@example.com")).await;
    let g = guest(&pool);

    let admin = link(&owner, dad_id).await;
    forgot_ok(&g.server, "dad").await;
    drain(&g.state).await;

    let mails = receiver.messages();
    assert_eq!(mails.len(), 1);
    preview(&g.server, &admin).await.assert_status_ok();
    preview(&g.server, &raw_in(&mails[0]))
        .await
        .assert_status_ok();
}

#[sqlx::test(migrations = "./migrations")]
async fn admin_link_revokes_self_link(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (owner, _, dad_id) = setup(&pool).await;
    smtp::configure(&pool, receiver.port).await;
    set_email(&pool, dad_id, Some("dad@example.com")).await;
    let g = guest(&pool);

    forgot_ok(&g.server, "dad").await;
    drain(&g.state).await;
    let own = raw_in(&receiver.messages()[0]);
    let admin = link(&owner, dad_id).await;

    assert_error(
        &preview(&g.server, &own).await,
        StatusCode::GONE,
        "passwordReset.revoked",
    );
    preview(&g.server, &admin).await.assert_status_ok();
}

#[sqlx::test(migrations = "./migrations")]
async fn accept_revokes_remaining_links(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (owner, _, dad_id) = setup(&pool).await;
    smtp::configure(&pool, receiver.port).await;
    set_email(&pool, dad_id, Some("dad@example.com")).await;
    let g = guest(&pool);

    let admin = link(&owner, dad_id).await;
    forgot_ok(&g.server, "dad").await;
    drain(&g.state).await;
    let own = raw_in(&receiver.messages()[0]);
    preview(&g.server, &admin).await.assert_status_ok();

    accept(&g.server, &own, "brand-new-pass")
        .await
        .assert_status_ok();
    assert_error(
        &preview(&g.server, &admin).await,
        StatusCode::GONE,
        "passwordReset.revoked",
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn reset_job_rechecks_target(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (_, _, dad_id) = setup(&pool).await;
    let uncle_id = uncle(&pool).await;
    smtp::configure(&pool, receiver.port).await;
    set_email(&pool, dad_id, Some("dad@example.com")).await;
    set_email(&pool, uncle_id, Some("uncle@example.com")).await;
    let g = guest(&pool);

    forgot_ok(&g.server, "dad").await;
    forgot_ok(&g.server, "uncle").await;
    let queued: i64 =
        sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind = 'send_reset_mail'")
            .fetch_one(&pool)
            .await
            .expect("jobs");
    assert_eq!(queued, 2);
    // 처리 전 정지, 이메일 삭제.
    disable(&pool, dad_id).await;
    set_email(&pool, uncle_id, None).await;
    drain(&g.state).await;

    assert!(receiver.messages().is_empty());
    assert_eq!(self_rows(&pool, dad_id).await, 0);
    assert_eq!(self_rows(&pool, uncle_id).await, 0);
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs")
        .fetch_one(&pool)
        .await
        .expect("jobs");
    assert_eq!(jobs, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn forgot_one_pending_job_per_user(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (_, _, dad_id) = setup(&pool).await;
    smtp::configure(&pool, receiver.port).await;
    set_email(&pool, dad_id, Some("dad@example.com")).await;
    let g = guest(&pool);

    forgot_ok(&g.server, "dad").await;
    forgot_ok(&g.server, "dad@example.com").await;
    let queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind = 'send_reset_mail' AND status = 'queued'",
    )
    .fetch_one(&pool)
    .await
    .expect("jobs");
    assert_eq!(queued, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn forgot_failed_jobs_bounded(pool: PgPool) {
    let rejecting = Receiver::start(Mode::Reject(550, "mailbox unavailable".into())).await;
    let (_, _, dad_id) = setup(&pool).await;
    smtp::configure(&pool, rejecting.port).await;
    set_email(&pool, dad_id, Some("dad@example.com")).await;
    let g = guest(&pool);
    let failed = || async {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM jobs WHERE kind = 'send_reset_mail' AND status = 'failed'",
        )
        .fetch_all(&pool)
        .await
        .expect("failed jobs")
    };

    forgot_ok(&g.server, "dad").await;
    drain(&g.state).await;
    let first = failed().await;
    assert_eq!(first.len(), 1);

    sqlx::query(
        "UPDATE jobs SET locked_at = now() - interval '61 seconds'
         WHERE kind = 'send_reset_mail' AND status = 'failed'",
    )
    .execute(&pool)
    .await
    .expect("age failed jobs");
    forgot_ok(&g.server, "dad").await;
    drain(&g.state).await;
    let second = failed().await;
    // 이전 실패 행 정리, 두 번째 요청도 발송 시도(새 작업이 실패).
    assert_eq!(second.len(), 1);
    assert_ne!(second, first);
}

#[sqlx::test(migrations = "./migrations")]
async fn forgot_failed_retry_waits_a_minute(pool: PgPool) {
    let rejecting = Receiver::start(Mode::Reject(550, "mailbox unavailable".into())).await;
    let (_, _, dad_id) = setup(&pool).await;
    smtp::configure(&pool, rejecting.port).await;
    set_email(&pool, dad_id, Some("dad@example.com")).await;
    let g = guest(&pool);
    let count = |status: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM jobs WHERE kind = 'send_reset_mail' AND status = $1::job_status",
            )
            .bind(status)
            .fetch_one(&pool)
            .await
            .expect("count")
        }
    };

    forgot_ok(&g.server, "dad").await;
    drain(&g.state).await;
    assert_eq!(count("failed").await, 1);

    forgot_ok(&g.server, "dad").await;
    assert_eq!(count("queued").await, 0);
    assert_eq!(count("failed").await, 1);

    sqlx::query(
        "UPDATE jobs SET locked_at = now() - interval '61 seconds'
         WHERE kind = 'send_reset_mail' AND status = 'failed'",
    )
    .execute(&pool)
    .await
    .expect("age failed jobs");
    forgot_ok(&g.server, "dad").await;
    assert_eq!(count("queued").await, 1);
}
