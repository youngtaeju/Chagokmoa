//! OIDC 설정. `settings` 키 `oidc`. client secret 은 평문 저장(메일과 같은 결정), 출력 금지.

use std::fmt;

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;

use crate::{error::AppResult, settings::queries as instance};

const KEY: &str = "oidc";
/// 버튼 이름을 비웠을 때.
pub const DEFAULT_LABEL: &str = "SSO";

/// `Debug` 는 secret 없이(아래 수동 구현).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OidcConfig {
    /// discovery 문서의 `issuer` 값. 사용자 연결의 issuer 와 같은 문자열.
    pub issuer_url: String,
    pub client_id: String,
    /// 없으면 public client(PKCE 만).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    pub button_label: String,
}

impl fmt::Debug for OidcConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OidcConfig")
            .field("issuer_url", &self.issuer_url)
            .field("client_id", &self.client_id)
            .field("secret_set", &self.client_secret.is_some())
            .field("button_label", &self.button_label)
            .finish()
    }
}

/// 미설정이면 None. 형식 오류는 값 노출 없이 오류.
pub async fn load(exec: impl PgExecutor<'_>) -> AppResult<Option<OidcConfig>> {
    let value = sqlx::query_scalar!("SELECT value FROM settings WHERE key = 'oidc'")
        .fetch_optional(exec)
        .await?;
    let Some(value) = value else {
        return Ok(None);
    };
    // serde 오류 문구에 값이 실릴 수 있어 분류만.
    serde_json::from_value(value)
        .map(Some)
        .map_err(|e| anyhow!("oidc settings unreadable ({:?})", e.classify()).into())
}

pub async fn save(exec: impl PgExecutor<'_>, config: &OidcConfig) -> AppResult<()> {
    let value = serde_json::to_value(config).context("oidc settings serialize failed")?;
    instance::set(exec, KEY, value).await
}

pub async fn remove(exec: impl PgExecutor<'_>) -> AppResult<()> {
    instance::remove(exec, KEY).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_secret() {
        let c = OidcConfig {
            issuer_url: "https://auth.example.com".into(),
            client_id: "chagokmoa".into(),
            client_secret: Some("oidc-secret".into()),
            button_label: "SSO".into(),
        };
        let out = format!("{c:?}");
        assert!(!out.contains("oidc-secret"), "{out}");
        assert!(out.contains("secret_set: true"), "{out}");
    }

    #[test]
    fn stored_shape() {
        let c = OidcConfig {
            issuer_url: "https://auth.example.com/app/".into(),
            client_id: "chagokmoa".into(),
            client_secret: None,
            button_label: "Authentik".into(),
        };
        let value = serde_json::to_value(&c).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "issuer_url": "https://auth.example.com/app/",
                "client_id": "chagokmoa",
                "button_label": "Authentik",
            })
        );
        assert_eq!(serde_json::from_value::<OidcConfig>(value).unwrap(), c);
    }
}
