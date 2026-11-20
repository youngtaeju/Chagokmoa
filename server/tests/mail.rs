//! 메일 설정과 테스트 발송 API. 실제 발송은 테스트 안 SMTP 수신기.

mod common;

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use axum_test::{TestResponse, TestServer};
use chagokmoa::{settings::queries as instance, users::Role};
use common::{
    XRW, XRW_VALUE, assert_error, media_app, server_as,
    smtp::{Mode, Receiver},
    test_server_without_cookies,
};
use serde_json::{Value, json};
use sqlx::PgPool;

const SECRET: &str = "smtp-secret-pw-9876";
const FROM: &str = "noreply@example.com";

fn config_body(port: u16, username: Option<&str>, password: Option<&str>) -> Value {
    json!({
        "host": "127.0.0.1",
        "port": port,
        "security": "none",
        "username": username,
        "password": password,
        "from_address": FROM,
    })
}

async fn put_mail(server: &TestServer, body: &Value) -> TestResponse {
    server
        .put("/api/mail")
        .add_header(XRW, XRW_VALUE)
        .json(body)
        .await
}

async fn send_test(server: &TestServer, to: &str) -> TestResponse {
    server
        .post("/api/mail/test")
        .add_header(XRW, XRW_VALUE)
        .json(&json!({ "to": to }))
        .await
}

async fn mail_enabled(pool: &PgPool) -> bool {
    let server = test_server_without_cookies(pool.clone()).await;
    let res = server.get("/api/auth/status").await;
    res.assert_status_ok();
    res.json::<Value>()["mail_enabled"]
        .as_bool()
        .expect("mail_enabled")
}

#[sqlx::test(migrations = "./migrations")]
async fn mail_settings_owner_only(pool: PgPool) {
    for (name, role) in [("dad", Role::Guardian), ("grandma", Role::Family)] {
        let (server, _) = server_as(&pool, name, role).await;
        assert_error(
            &server.get("/api/mail").await,
            StatusCode::FORBIDDEN,
            "auth.forbidden",
        );
        assert_error(
            &put_mail(&server, &config_body(25, None, None)).await,
            StatusCode::FORBIDDEN,
            "auth.forbidden",
        );
        assert_error(
            &server.delete("/api/mail").add_header(XRW, XRW_VALUE).await,
            StatusCode::FORBIDDEN,
            "auth.forbidden",
        );
        assert_error(
            &send_test(&server, "a@example.com").await,
            StatusCode::FORBIDDEN,
            "auth.forbidden",
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn get_hides_password(pool: PgPool) {
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    let res = put_mail(&server, &config_body(2525, Some("mom"), Some(SECRET))).await;
    res.assert_status_ok();
    assert!(!res.text().contains(SECRET), "{}", res.text());

    let res = server.get("/api/mail").await;
    res.assert_status_ok();
    assert!(!res.text().contains(SECRET), "{}", res.text());
    res.assert_json(&json!({
        "config": {
            "host": "127.0.0.1",
            "port": 2525,
            "security": "none",
            "username": "mom",
            "password_set": true,
            "from_address": FROM,
        }
    }));
}

#[sqlx::test(migrations = "./migrations")]
async fn put_keeps_password_when_omitted(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    put_mail(
        &server,
        &config_body(receiver.port, Some("mom"), Some(SECRET)),
    )
    .await
    .assert_status_ok();

    let res = put_mail(&server, &config_body(receiver.port, Some(" mom "), None)).await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["config"]["password_set"], true);

    send_test(&server, "a@example.com")
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let mails = receiver.messages();
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].auth, Some(("mom".to_owned(), SECRET.to_owned())));
}

#[sqlx::test(migrations = "./migrations")]
async fn put_clears_password_without_username(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    put_mail(
        &server,
        &config_body(receiver.port, Some("mom"), Some(SECRET)),
    )
    .await
    .assert_status_ok();

    let res = put_mail(&server, &config_body(receiver.port, Some(""), None)).await;
    res.assert_status_ok();
    let config = &res.json::<Value>()["config"];
    assert_eq!(config["password_set"], false);
    assert_eq!(config["username"], Value::Null);

    send_test(&server, "a@example.com")
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let mails = receiver.messages();
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].auth, None);
}

#[sqlx::test(migrations = "./migrations")]
async fn put_validates(pool: PgPool) {
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    let mut body = config_body(25, None, None);
    body["host"] = json!("  ");
    assert_error(
        &put_mail(&server, &body).await,
        StatusCode::BAD_REQUEST,
        "mail.hostRequired",
    );

    assert_error(
        &put_mail(&server, &config_body(0, None, None)).await,
        StatusCode::BAD_REQUEST,
        "mail.invalidPort",
    );

    let mut body = config_body(25, None, None);
    body["from_address"] = json!("nope");
    assert_error(
        &put_mail(&server, &body).await,
        StatusCode::BAD_REQUEST,
        "mail.invalidAddress",
    );

    let res = server.get("/api/mail").await;
    res.assert_json(&json!({ "config": null }));
}

#[sqlx::test(migrations = "./migrations")]
async fn test_mail_delivered(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    instance::set(&pool, "instance_name", json!("Little Home"))
        .await
        .unwrap();
    put_mail(&server, &config_body(receiver.port, None, None))
        .await
        .assert_status_ok();

    send_test(&server, " dad@example.com ")
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let mails = receiver.messages();
    assert_eq!(mails.len(), 1);
    let mail = &mails[0];
    assert_eq!(mail.from, FROM);
    assert_eq!(mail.to, vec!["dad@example.com".to_owned()]);
    let from_header = mail
        .data
        .lines()
        .find(|l| l.starts_with("From:"))
        .expect("From header");
    assert!(from_header.contains("Little Home"), "{from_header}");
    assert!(from_header.contains(&format!("<{FROM}>")), "{from_header}");
    let message_id = mail
        .data
        .lines()
        .find(|l| l.starts_with("Message-ID:"))
        .expect("Message-ID header");
    assert!(
        message_id.contains('<') && message_id.ends_with("@example.com>"),
        "{message_id}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_mail_timeout(pool: PgPool) {
    let receiver = Receiver::start(Mode::Hang).await;
    let mut app = media_app(&pool, Role::Owner).await;
    app.state.mail_timeout = Duration::from_secs(1);
    let (server, _) = app.login_as("dad", Role::Owner).await;
    put_mail(&server, &config_body(receiver.port, None, None))
        .await
        .assert_status_ok();

    let started = Instant::now();
    let res = send_test(&server, "a@example.com").await;
    res.assert_status(StatusCode::CONFLICT);
    assert_eq!(res.json::<Value>()["error"], "mail.sendFailed");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[sqlx::test(migrations = "./migrations")]
async fn test_mail_not_configured(pool: PgPool) {
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    assert_error(
        &send_test(&server, "a@example.com").await,
        StatusCode::CONFLICT,
        "mail.notConfigured",
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_mail_invalid_address(pool: PgPool) {
    let receiver = Receiver::start(Mode::Accept).await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    put_mail(&server, &config_body(receiver.port, None, None))
        .await
        .assert_status_ok();
    assert_error(
        &send_test(&server, "nope").await,
        StatusCode::BAD_REQUEST,
        "mail.invalidAddress",
    );
    assert!(receiver.messages().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn send_failure_message_redacts_password(pool: PgPool) {
    let receiver =
        Receiver::start(Mode::Reject(535, format!("bad credentials for {SECRET}"))).await;
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    put_mail(
        &server,
        &config_body(receiver.port, Some("mom"), Some(SECRET)),
    )
    .await
    .assert_status_ok();

    let res = send_test(&server, "a@example.com").await;
    res.assert_status(StatusCode::CONFLICT);
    let body: Value = res.json();
    assert_eq!(body["error"], "mail.sendFailed");
    let message = body["params"]["message"].as_str().expect("message");
    assert!(!message.contains(SECRET), "{message}");
    assert!(message.contains("bad credentials"), "{message}");
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_removes_settings(pool: PgPool) {
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    put_mail(&server, &config_body(25, Some("mom"), Some(SECRET)))
        .await
        .assert_status_ok();

    server
        .delete("/api/mail")
        .add_header(XRW, XRW_VALUE)
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .get("/api/mail")
        .await
        .assert_json(&json!({ "config": null }));
    assert!(!mail_enabled(&pool).await);
}

#[sqlx::test(migrations = "./migrations")]
async fn status_reports_mail_enabled(pool: PgPool) {
    assert!(!mail_enabled(&pool).await);
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    put_mail(&server, &config_body(25, None, None))
        .await
        .assert_status_ok();
    assert!(mail_enabled(&pool).await);
}
