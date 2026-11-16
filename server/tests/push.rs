mod common;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use chagokmoa::{push::vapid, users::Role};
use common::server_as;
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
