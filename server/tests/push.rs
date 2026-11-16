mod common;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
};
use axum_test::TestServer;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use chagokmoa::{
    auth::session,
    jobs::{self, Job},
    notifications::record::record_upload,
    push::{queries as push_queries, send, vapid},
    state::AppState,
    users::{Role, User},
};
use common::{AssetSeed, XRW, XRW_VALUE, assert_error, insert_asset, server_as};
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::watch;
use uuid::Uuid;
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

// ---- 발송 ----

/// 가짜 푸시 서버가 받은 요청.
#[derive(Clone, Debug)]
struct Received {
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

/// 경로별 응답 코드(기본 201), 받은 요청 기록. `bump` 는 첫 요청 응답 전 그 알림 줄 count 1 증가.
#[derive(Clone, Default)]
struct Fake {
    received: Arc<Mutex<Vec<Received>>>,
    statuses: Arc<HashMap<&'static str, u16>>,
    bump: Arc<Mutex<Option<(PgPool, Uuid)>>>,
}

async fn fake_handler(
    State(fake): State<Fake>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let bump = fake.bump.lock().unwrap().take();
    if let Some((pool, id)) = bump {
        sqlx::query("UPDATE notifications SET count = count + 1 WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
    let path = uri.path().to_string();
    let status = fake.statuses.get(path.as_str()).copied().unwrap_or(201);
    fake.received.lock().unwrap().push(Received {
        path,
        headers,
        body: body.to_vec(),
    });
    StatusCode::from_u16(status).unwrap()
}

/// 임의 포트에 띄운 가짜 푸시 서버의 기준 주소.
async fn spawn_fake(fake: &Fake) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .fallback(fake_handler)
        .with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

impl Fake {
    fn received(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }
}

/// 브라우저 쪽 키. 받은 본문 복호화용.
struct Device {
    secret: p256::SecretKey,
    auth: Auth,
}

impl Device {
    fn open(&self, r: &Received) -> Value {
        let plain = decrypt(r.body.clone(), &self.secret, &self.auth).unwrap();
        serde_json::from_slice(&plain).unwrap()
    }
}

/// 새 세션에 `endpoint` 구독 저장.
async fn subscribe(pool: &PgPool, user: &User, endpoint: &str) -> Device {
    let secret = p256::SecretKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
    let auth: [u8; 16] = rand::random();
    let (_, session_id) = session::create(pool, user.id, None).await.unwrap();
    push_queries::replace(
        pool,
        session_id,
        endpoint,
        &B64.encode(secret.public_key().to_sec1_bytes()),
        &B64.encode(auth),
    )
    .await
    .unwrap();
    Device {
        secret,
        auth: Auth::clone_from_slice(&auth),
    }
}

/// owner 엄마(행동자)와 받는 사람 아빠. 인스턴스 이름 "우리집".
struct Home {
    state: AppState,
    mom: (TestServer, User),
    dad: User,
}

async fn home(pool: &PgPool) -> Home {
    let mom = server_as(pool, "mom", Role::Owner).await;
    let (_, dad) = server_as(pool, "dad", Role::Guardian).await;
    sqlx::query("UPDATE users SET display_name = '엄마' WHERE id = $1")
        .bind(mom.1.id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO settings (key, value) VALUES ('instance_name', '\"우리집\"')")
        .execute(pool)
        .await
        .unwrap();
    let mut state = AppState::new(pool.clone(), common::test_config());
    state.push_endpoint_check = false;
    Home { state, mom, dad }
}

/// 엄마 이야기 → 아빠의 story 줄. (이야기 id, 알림 id)
async fn story_note(pool: &PgPool, h: &Home, body: &str) -> (Uuid, Uuid) {
    let res = h
        .mom
        .0
        .post("/api/stories")
        .add_header(XRW, XRW_VALUE)
        .json(&serde_json::json!({
            "entry_date": "2024-06-01", "body": body, "baby_ids": [], "asset_ids": []
        }))
        .await;
    res.assert_status(StatusCode::CREATED);
    let story: Uuid = res.json::<Value>()["id"].as_str().unwrap().parse().unwrap();
    (story, note_of(pool, &h.dad).await)
}

/// 엄마 사진 올림 → 아빠의 upload 줄, count 지정. (사진 id, 알림 id)
async fn upload_note(pool: &PgPool, h: &Home, count: i32) -> (Uuid, Uuid) {
    let mut seed = AssetSeed::photo(h.mom.1.id);
    // 서울 2024-06-01 12:00
    seed.taken_at = "2024-06-01T03:00:00Z".parse().unwrap();
    let asset = insert_asset(pool, seed).await;
    let mut conn = pool.acquire().await.unwrap();
    record_upload(&mut conn, h.mom.1.id, asset).await.unwrap();
    let id = note_of(pool, &h.dad).await;
    set_count(pool, id, count).await;
    (asset, id)
}

async fn note_of(pool: &PgPool, user: &User) -> Uuid {
    sqlx::query_scalar("SELECT id FROM notifications WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn set_count(pool: &PgPool, id: Uuid, count: i32) {
    sqlx::query("UPDATE notifications SET count = $2 WHERE id = $1")
        .bind(id)
        .bind(count)
        .execute(pool)
        .await
        .unwrap();
}

/// (count, pushed_count)
async fn counts(pool: &PgPool, id: Uuid) -> (i32, i32) {
    sqlx::query_as("SELECT count, pushed_count FROM notifications WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn sends_encrypted_payload(pool: PgPool) {
    let h = home(&pool).await;
    let fake = Fake::default();
    let base = spawn_fake(&fake).await;
    let device = subscribe(&pool, &h.dad, &format!("{base}/push/dad")).await;
    let (story, note) = story_note(&pool, &h, "오늘 처음 웃었어").await;

    send::run(&h.state, note).await.unwrap();

    let got = fake.received();
    assert_eq!(got.len(), 1);
    let r = &got[0];
    assert_eq!(r.path, "/push/dad");
    assert_eq!(
        device.open(r),
        serde_json::json!({
            "title": "우리집",
            "body": "엄마님이 이야기를 남겼어요: \"오늘 처음 웃었어\"",
            "url": format!("/stories/{story}"),
        })
    );
    assert_eq!(r.headers["ttl"], "86400");
    assert_eq!(r.headers["urgency"], "normal");
    assert_eq!(r.headers["content-encoding"], "aes128gcm");
    let authz = r.headers["authorization"].to_str().unwrap();
    assert!(authz.starts_with("vapid t="), "{authz}");
    let jwt = authz["vapid t=".len()..].split(',').next().unwrap();
    let payload = B64.decode(jwt.split('.').nth(1).unwrap()).unwrap();
    let exp = serde_json::from_slice::<Value>(&payload).unwrap()["exp"]
        .as_i64()
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert!(exp > now && exp - now <= 43200 + 60, "exp {exp} now {now}");
    assert_eq!(counts(&pool, note).await, (1, 1));
}

#[sqlx::test(migrations = "./migrations")]
async fn recipient_locale_en(pool: PgPool) {
    let h = home(&pool).await;
    sqlx::query("UPDATE users SET locale = 'en' WHERE id = $1")
        .bind(h.dad.id)
        .execute(&pool)
        .await
        .unwrap();
    let fake = Fake::default();
    let base = spawn_fake(&fake).await;
    let device = subscribe(&pool, &h.dad, &format!("{base}/push/dad")).await;
    let (_, note) = story_note(&pool, &h, "first smile").await;

    send::run(&h.state, note).await.unwrap();

    let got = fake.received();
    assert_eq!(got.len(), 1);
    assert_eq!(
        device.open(&got[0])["body"],
        "엄마 shared a story: \"first smile\""
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn muted_kind_not_sent(pool: PgPool) {
    let h = home(&pool).await;
    sqlx::query("INSERT INTO push_mutes (user_id, kind) VALUES ($1, 'story')")
        .bind(h.dad.id)
        .execute(&pool)
        .await
        .unwrap();
    let fake = Fake::default();
    let base = spawn_fake(&fake).await;
    subscribe(&pool, &h.dad, &format!("{base}/push/dad")).await;
    let (_, note) = story_note(&pool, &h, "이야기").await;

    send::run(&h.state, note).await.unwrap();

    assert!(fake.received().is_empty());
    assert_eq!(counts(&pool, note).await, (1, 1));
}

#[sqlx::test(migrations = "./migrations")]
async fn disabled_user_not_sent(pool: PgPool) {
    let h = home(&pool).await;
    let fake = Fake::default();
    let base = spawn_fake(&fake).await;
    subscribe(&pool, &h.dad, &format!("{base}/push/dad")).await;
    let (_, note) = story_note(&pool, &h, "이야기").await;
    sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1")
        .bind(h.dad.id)
        .execute(&pool)
        .await
        .unwrap();

    send::run(&h.state, note).await.unwrap();

    assert!(fake.received().is_empty());
    assert_eq!(counts(&pool, note).await, (1, 1));
}

#[sqlx::test(migrations = "./migrations")]
async fn no_subscription_ok(pool: PgPool) {
    let h = home(&pool).await;
    let (_, note) = story_note(&pool, &h, "이야기").await;
    send::run(&h.state, note).await.unwrap();
    assert_eq!(counts(&pool, note).await, (1, 1));
    // 그사이 삭제된 줄
    send::run(&h.state, Uuid::now_v7()).await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn trashed_upload_not_pushed(pool: PgPool) {
    let h = home(&pool).await;
    let fake = Fake::default();
    let base = spawn_fake(&fake).await;
    subscribe(&pool, &h.dad, &format!("{base}/push/dad")).await;
    let (asset, note) = upload_note(&pool, &h, 4).await;
    sqlx::query("UPDATE assets SET deleted_at = now() WHERE id = $1")
        .bind(asset)
        .execute(&pool)
        .await
        .unwrap();

    send::run(&h.state, note).await.unwrap();

    assert!(fake.received().is_empty());
    assert_eq!(counts(&pool, note).await, (4, 4));
}

#[sqlx::test(migrations = "./migrations")]
async fn upload_delta_only(pool: PgPool) {
    let h = home(&pool).await;
    let fake = Fake::default();
    let base = spawn_fake(&fake).await;
    let device = subscribe(&pool, &h.dad, &format!("{base}/push/dad")).await;
    let (_, note) = upload_note(&pool, &h, 12).await;

    send::run(&h.state, note).await.unwrap();
    assert_eq!(counts(&pool, note).await, (12, 12));
    set_count(&pool, note, 15).await;
    send::run(&h.state, note).await.unwrap();
    // 늘어난 게 없으면 보내지 않음
    send::run(&h.state, note).await.unwrap();

    let got = fake.received();
    assert_eq!(got.len(), 2);
    let first = device.open(&got[0]);
    assert_eq!(first["body"], "엄마님이 사진 12장을 올렸어요");
    assert_eq!(first["url"], "/days/2024-06-01");
    let second = device.open(&got[1]);
    assert_eq!(second["body"], "엄마님이 사진 3장을 올렸어요");
    assert_eq!(second["url"], "/days/2024-06-01");
    assert_eq!(counts(&pool, note).await, (15, 15));
}

#[sqlx::test(migrations = "./migrations")]
async fn upload_during_send_pushes_rest_later(pool: PgPool) {
    let h = home(&pool).await;
    let fake = Fake::default();
    let base = spawn_fake(&fake).await;
    let device = subscribe(&pool, &h.dad, &format!("{base}/push/dad")).await;
    let (asset, note) = upload_note(&pool, &h, 12).await;
    *fake.bump.lock().unwrap() = Some((pool.clone(), note));

    send::run(&h.state, note).await.unwrap();
    assert_eq!(counts(&pool, note).await, (13, 12));

    send::run(&h.state, note).await.unwrap();
    let got = fake.received();
    assert_eq!(got.len(), 2);
    assert_eq!(
        device.open(&got[0])["body"],
        "엄마님이 사진 12장을 올렸어요"
    );
    let later = device.open(&got[1]);
    assert_eq!(later["body"], "엄마님이 사진을 올렸어요");
    assert_eq!(later["url"], format!("/photos/{asset}"));
    assert_eq!(counts(&pool, note).await, (13, 13));
}

#[sqlx::test(migrations = "./migrations")]
async fn gone_subscription_deleted(pool: PgPool) {
    let h = home(&pool).await;
    let fake = Fake {
        statuses: Arc::new(HashMap::from([("/gone", 410), ("/missing", 404)])),
        ..Fake::default()
    };
    let base = spawn_fake(&fake).await;
    subscribe(&pool, &h.dad, &format!("{base}/gone")).await;
    subscribe(&pool, &h.dad, &format!("{base}/missing")).await;
    subscribe(&pool, &h.dad, &format!("{base}/ok")).await;
    let (_, note) = story_note(&pool, &h, "이야기").await;

    send::run(&h.state, note).await.unwrap();

    assert_eq!(fake.received().len(), 3);
    let left: Vec<String> = sqlx::query_scalar("SELECT endpoint FROM push_subscriptions")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(left, [format!("{base}/ok")]);
}

#[sqlx::test(migrations = "./migrations")]
async fn failing_endpoint_does_not_block_others(pool: PgPool) {
    let h = home(&pool).await;
    let fake = Fake {
        statuses: Arc::new(HashMap::from([("/fail", 500)])),
        ..Fake::default()
    };
    let base = spawn_fake(&fake).await;
    subscribe(&pool, &h.dad, &format!("{base}/fail")).await;
    // 연결 거부
    subscribe(&pool, &h.dad, "http://127.0.0.1:1/refused").await;
    let device = subscribe(&pool, &h.dad, &format!("{base}/ok")).await;
    let (_, note) = story_note(&pool, &h, "이야기").await;

    send::run(&h.state, note).await.unwrap();

    let paths: Vec<String> = fake.received().into_iter().map(|r| r.path).collect();
    assert_eq!(paths, ["/fail", "/ok"]);
    assert_eq!(device.open(&fake.received()[1])["title"], "우리집");
    assert_eq!(sub_rows(&pool).await, 3);
}

#[sqlx::test(migrations = "./migrations")]
async fn claim_filters_kind(pool: PgPool) {
    let asset_job = Job::ProcessAsset {
        asset_id: Uuid::now_v7(),
    };
    let push_job = Job::SendPush {
        notification_id: Uuid::now_v7(),
    };
    jobs::enqueue(&pool, &asset_job).await.unwrap();
    jobs::enqueue(&pool, &push_job).await.unwrap();

    let claimed = jobs::claim(&pool, &["send_push"]).await.unwrap().unwrap();
    assert_eq!(claimed.job, push_job);
    assert!(jobs::claim(&pool, &["send_push"]).await.unwrap().is_none());
    let claimed = jobs::claim(&pool, &["process_asset"])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.job, asset_job);
}

#[sqlx::test(migrations = "./migrations")]
async fn worker_sends_queued_push(pool: PgPool) {
    let h = home(&pool).await;
    let fake = Fake::default();
    let base = spawn_fake(&fake).await;
    subscribe(&pool, &h.dad, &format!("{base}/push/dad")).await;
    let (_, note) = story_note(&pool, &h, "이야기").await;
    jobs::enqueue(
        &pool,
        &Job::SendPush {
            notification_id: note,
        },
    )
    .await
    .unwrap();

    let (stop, shutdown) = watch::channel(false);
    let worker = send::spawn_worker(h.state.clone(), shutdown);
    h.state.push_wake.notify_one();
    for _ in 0..100 {
        if !fake.received().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    stop.send(true).unwrap();
    worker.await.unwrap();

    assert_eq!(fake.received().len(), 1);
    let jobs_left: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(jobs_left, 0);
}
