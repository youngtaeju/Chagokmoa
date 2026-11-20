//! 메일 설정. `settings` 키 `mail`. SMTP 비밀번호는 평문 저장(사용자 결정), 출력 금지.

use std::fmt;

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;
use utoipa::ToSchema;

use crate::{error::AppResult, settings::queries as instance};

const KEY: &str = "mail";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MailSecurity {
    /// 평문 접속 후 STARTTLS 필수. 보통 587.
    Starttls,
    /// 처음부터 TLS. 보통 465.
    Tls,
    /// 암호화 없음. 같은 네트워크 릴레이용.
    None,
}

/// `Debug` 는 비밀번호 없이(아래 수동 구현).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailConfig {
    pub host: String,
    pub port: u16,
    pub security: MailSecurity,
    /// 없으면 인증 없음.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    pub from_address: String,
}

impl fmt::Debug for MailConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &self.username)
            .field("password_set", &self.password.is_some())
            .field("from_address", &self.from_address)
            .finish()
    }
}

/// 미설정이면 None. 형식 오류는 값 노출 없이 오류.
pub async fn load(exec: impl PgExecutor<'_>) -> AppResult<Option<MailConfig>> {
    let value = sqlx::query_scalar!("SELECT value FROM settings WHERE key = 'mail'")
        .fetch_optional(exec)
        .await?;
    let Some(value) = value else {
        return Ok(None);
    };
    // serde 오류 문구에 값이 실릴 수 있어 분류만.
    serde_json::from_value(value)
        .map(Some)
        .map_err(|e| anyhow!("mail settings unreadable ({:?})", e.classify()).into())
}

pub async fn save(exec: impl PgExecutor<'_>, config: &MailConfig) -> AppResult<()> {
    let value = serde_json::to_value(config).context("mail settings serialize failed")?;
    instance::set(exec, KEY, value).await
}

pub async fn remove(exec: impl PgExecutor<'_>) -> AppResult<()> {
    instance::remove(exec, KEY).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_password() {
        let c = MailConfig {
            host: "smtp.example.com".into(),
            port: 587,
            security: MailSecurity::Starttls,
            username: Some("mom".into()),
            password: Some("smtp-secret".into()),
            from_address: "mom@example.com".into(),
        };
        let out = format!("{c:?}");
        assert!(!out.contains("smtp-secret"), "{out}");
        assert!(out.contains("password_set: true"), "{out}");
    }

    #[test]
    fn stored_shape() {
        let c = MailConfig {
            host: "relay.lan".into(),
            port: 25,
            security: MailSecurity::None,
            username: None,
            password: None,
            from_address: "home@example.com".into(),
        };
        let value = serde_json::to_value(&c).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "host": "relay.lan",
                "port": 25,
                "security": "none",
                "from_address": "home@example.com",
            })
        );
        assert_eq!(serde_json::from_value::<MailConfig>(value).unwrap(), c);
    }
}
