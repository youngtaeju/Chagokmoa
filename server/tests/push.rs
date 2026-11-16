mod common;

use axum::http::StatusCode;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use chagokmoa::{push::vapid, users::Role};
use common::{XRW, XRW_VALUE, assert_error, server_as};
use serde_json::Value;
use sqlx::PgPool;
use web_push_native::{Auth, decrypt, encrypt, p256};

#[sqlx::test(migrations = "./migrations")]
async fn vapid_created_once(pool: PgPool) {
    let first = vapid::load_or_create(&pool).await.unwrap();
    let second = vapid::load_or_create(&pool).await.unwrap();
    assert_eq!(first.public_key, second.public_key);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM settings WHERE key = 'vapid'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    let public = B64.decode(&first.public_key).unwrap();
    assert_eq!(public.len(), 65);
    assert_eq!(public[0], 0x04);
}

#[sqlx::test(migrations = "./migrations")]
async fn vapid_not_in_settings_api(pool: PgPool) {
    vapid::load_or_create(&pool).await.unwrap();
    let (server, _) = server_as(&pool, "owner", Role::Owner).await;
    let res = server.get("/api/settings").await;
    res.assert_status_ok();
    let body: Value = res.json();
    assert!(body.get("vapid").is_none());
    assert!(!res.text().contains("private_key"));
}

#[test]
fn rfc8291_example_decrypts() {
    // RFC 8291 부록 A 원문. 헤더 86바이트 + 암호문.
    let header = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8";
    let cipher = "8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ";
    let mut message = B64.decode(header).unwrap();
    assert_eq!(message.len(), 86);
    message.extend(B64.decode(cipher).unwrap());
    let ua_private = B64
        .decode("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94")
        .unwrap();
    let secret = p256::SecretKey::from_slice(&ua_private).unwrap();
    let auth = B64.decode("BTBZMqHH6r4Tts7J_aSIgg").unwrap();
    let plain = decrypt(message, &secret, &Auth::clone_from_slice(&auth)).unwrap();
    assert_eq!(plain, b"When I grow up, I want to be a watermelon");
}

#[test]
fn encrypt_round_trip() {
    let secret = p256::SecretKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
    let auth = Auth::clone_from_slice(&[7u8; 16]);
    let encrypted = encrypt(b"hello push".to_vec(), &secret.public_key(), &auth).unwrap();
    assert_eq!(decrypt(encrypted, &secret, &auth).unwrap(), b"hello push");
}

fn sub_body(endpoint: &str) -> Value {
    let secret = p256::SecretKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
    let point = secret.public_key().to_sec1_bytes();
    serde_json::json!({
        "endpoint": endpoint,
        "p256dh": B64.encode(point),
        "auth": B64.encode([7u8; 16]),
    })
}

const FCM: &str = "https://fcm.googleapis.com/fcm/send/x";

async fn put_sub(server: &axum_test::TestServer, body: &Value) -> axum_test::TestResponse {
    server
        .put("/api/push/subscription")
        .add_header(XRW, XRW_VALUE)
        .json(body)
        .await
}

async fn sub_rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM push_subscriptions")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn key_requires_login(pool: PgPool) {
    let server = common::test_server(pool).await;
    assert_error(
        &server.get("/api/push/key").await,
        StatusCode::UNAUTHORIZED,
        "auth.required",
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn key_returns_public_key(pool: PgPool) {
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    let body: Value = server.get("/api/push/key").await.json();
    let expected = vapid::load_or_create(&pool).await.unwrap().public_key;
    assert_eq!(body["public_key"], expected);
}

#[sqlx::test(migrations = "./migrations")]
async fn put_then_get_subscription(pool: PgPool) {
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    let none: Value = server.get("/api/push/subscription").await.json();
    assert!(none["endpoint"].is_null());
    put_sub(&server, &sub_body(FCM))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let got: Value = server.get("/api/push/subscription").await.json();
    assert_eq!(got["endpoint"], FCM);
    let second = "https://web.push.apple.com/y";
    put_sub(&server, &sub_body(second))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let got: Value = server.get("/api/push/subscription").await.json();
    assert_eq!(got["endpoint"], second);
    assert_eq!(sub_rows(&pool).await, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn same_endpoint_moves_to_new_session(pool: PgPool) {
    let (a, _) = server_as(&pool, "mom", Role::Owner).await;
    let (b, _) = server_as(&pool, "dad", Role::Guardian).await;
    put_sub(&a, &sub_body(FCM))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    put_sub(&b, &sub_body(FCM))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let got: Value = a.get("/api/push/subscription").await.json();
    assert!(got["endpoint"].is_null());
    let got: Value = b.get("/api/push/subscription").await.json();
    assert_eq!(got["endpoint"], FCM);
    assert_eq!(sub_rows(&pool).await, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_subscription_idempotent(pool: PgPool) {
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    put_sub(&server, &sub_body(FCM))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    for _ in 0..2 {
        server
            .delete("/api/push/subscription")
            .add_header(XRW, XRW_VALUE)
            .await
            .assert_status(StatusCode::NO_CONTENT);
    }
    assert_eq!(sub_rows(&pool).await, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn logout_cascades_subscription(pool: PgPool) {
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    put_sub(&server, &sub_body(FCM))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .post("/api/auth/logout")
        .add_header(XRW, XRW_VALUE)
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(sub_rows(&pool).await, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn endpoint_not_allowed(pool: PgPool) {
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    for url in [
        "http://fcm.googleapis.com/fcm/send/x",
        "https://evil.example/x",
        "https://fcm.googleapis.com.evil.example/x",
        "https://evilfcm.googleapis.com/x",
        "https://evil.example/?h=fcm.googleapis.com",
        "https://fcm.googleapis.com:22/x",
        "https://user@evil.example/x",
        "not a url",
    ] {
        assert_error(
            &put_sub(&server, &sub_body(url)).await,
            StatusCode::BAD_REQUEST,
            "push.endpointNotAllowed",
        );
    }
    for url in [
        FCM,
        "https://fcm.googleapis.com:443/fcm/send/x",
        "https://updates.push.services.mozilla.com/wpush/v2/x",
        "https://web.push.apple.com/x",
        "https://wns2-par02p.notify.windows.com/w/?token=x",
    ] {
        put_sub(&server, &sub_body(url))
            .await
            .assert_status(StatusCode::NO_CONTENT);
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn endpoint_too_long_rejected(pool: PgPool) {
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    let long = format!("https://fcm.googleapis.com/fcm/send/{}", "a".repeat(2048));
    assert_error(
        &put_sub(&server, &sub_body(&long)).await,
        StatusCode::BAD_REQUEST,
        "push.invalidSubscription",
    );
    assert_eq!(sub_rows(&pool).await, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_puts_do_not_conflict(pool: PgPool) {
    let (a, _) = server_as(&pool, "mom", Role::Owner).await;
    let (b, _) = server_as(&pool, "dad", Role::Guardian).await;
    let (b1, b2, a1, a2) = (
        sub_body(FCM),
        sub_body(FCM),
        sub_body("https://web.push.apple.com/a"),
        sub_body("https://web.push.apple.com/b"),
    );
    // 같은 주소를 다른 세션이, 같은 세션이 다른 주소를 동시에 등록.
    let (r1, r2, r3, r4) = tokio::join!(
        put_sub(&a, &b1),
        put_sub(&b, &b2),
        put_sub(&a, &a1),
        put_sub(&a, &a2),
    );
    for r in [r1, r2, r3, r4] {
        r.assert_status(StatusCode::NO_CONTENT);
    }
    let dup: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT endpoint FROM push_subscriptions GROUP BY endpoint HAVING count(*) > 1) d",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(dup, 0);
    assert!(sub_rows(&pool).await <= 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn invalid_keys_rejected(pool: PgPool) {
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    let mut bad_point = sub_body(FCM);
    bad_point["p256dh"] = B64.encode([4u8; 65]).into();
    let mut short_point = sub_body(FCM);
    short_point["p256dh"] = B64.encode([4u8; 10]).into();
    let mut bad_auth = sub_body(FCM);
    bad_auth["auth"] = B64.encode([7u8; 15]).into();
    let mut not_b64 = sub_body(FCM);
    not_b64["auth"] = "!!!".into();
    for body in [bad_point, short_point, bad_auth, not_b64] {
        assert_error(
            &put_sub(&server, &body).await,
            StatusCode::BAD_REQUEST,
            "push.invalidSubscription",
        );
    }
    assert_eq!(sub_rows(&pool).await, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn prefs_default_on_and_patch(pool: PgPool) {
    let (mom, _) = server_as(&pool, "mom", Role::Owner).await;
    let (dad, _) = server_as(&pool, "dad", Role::Guardian).await;
    let all = serde_json::json!({
        "upload": true, "comment": true, "mention": true, "story": true, "milestone": true
    });
    assert_eq!(mom.get("/api/push/prefs").await.json::<Value>(), all);
    let res = mom
        .patch("/api/push/prefs")
        .add_header(XRW, XRW_VALUE)
        .json(&serde_json::json!({ "mention": false }))
        .await;
    res.assert_status_ok();
    let expected = serde_json::json!({
        "upload": true, "comment": true, "mention": false, "story": true, "milestone": true
    });
    assert_eq!(res.json::<Value>(), expected);
    assert_eq!(mom.get("/api/push/prefs").await.json::<Value>(), expected);
    assert_eq!(dad.get("/api/push/prefs").await.json::<Value>(), all);
    // 다시 켜면 행 삭제
    mom.patch("/api/push/prefs")
        .add_header(XRW, XRW_VALUE)
        .json(&serde_json::json!({ "mention": true }))
        .await
        .assert_status_ok();
    assert_eq!(mom.get("/api/push/prefs").await.json::<Value>(), all);
}

#[sqlx::test(migrations = "./migrations")]
async fn mutations_require_csrf_header(pool: PgPool) {
    let (server, _) = server_as(&pool, "mom", Role::Owner).await;
    server
        .put("/api/push/subscription")
        .json(&sub_body(FCM))
        .await
        .assert_status(StatusCode::FORBIDDEN);
    server
        .delete("/api/push/subscription")
        .await
        .assert_status(StatusCode::FORBIDDEN);
    server
        .patch("/api/push/prefs")
        .json(&serde_json::json!({}))
        .await
        .assert_status(StatusCode::FORBIDDEN);
}
