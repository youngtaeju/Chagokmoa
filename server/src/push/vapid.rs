//! VAPID 키 쌍. `settings` 키 `vapid` 에 저장.

use anyhow::Context;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use web_push_native::jwt_simple::algorithms::{ECDSAP256PublicKeyLike, ES256KeyPair};

use crate::error::AppResult;

pub struct Vapid {
    pub key_pair: ES256KeyPair,
    /// 비압축 65바이트 공개키의 base64url(패딩 없음). 브라우저 `applicationServerKey` 형식.
    pub public_key: String,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    public_key: String,
    private_key: String,
}

fn parse(value: serde_json::Value) -> anyhow::Result<Vapid> {
    let stored: Stored = serde_json::from_value(value).context("vapid 설정 형식 오류")?;
    let private = B64
        .decode(&stored.private_key)
        .context("vapid 개인키 디코드 실패")?;
    let key_pair = ES256KeyPair::from_bytes(&private).context("vapid 개인키 오류")?;
    Ok(Vapid {
        key_pair,
        public_key: stored.public_key,
    })
}

/// 없으면 생성. 동시 기동에도 먼저 저장된 한 쌍만 사용.
pub async fn load_or_create(pool: &PgPool) -> AppResult<Vapid> {
    let key_pair = ES256KeyPair::generate();
    let public = key_pair.public_key().public_key().to_bytes_uncompressed();
    let stored = Stored {
        public_key: B64.encode(public),
        private_key: B64.encode(key_pair.to_bytes()),
    };
    sqlx::query!(
        "INSERT INTO settings (key, value) VALUES ('vapid', $1) ON CONFLICT (key) DO NOTHING",
        serde_json::to_value(&stored).context("vapid 직렬화 실패")?,
    )
    .execute(pool)
    .await?;
    let value = sqlx::query_scalar!("SELECT value FROM settings WHERE key = 'vapid'")
        .fetch_one(pool)
        .await?;
    Ok(parse(value)?)
}
