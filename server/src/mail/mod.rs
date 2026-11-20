//! SMTP 메일 발송. 설정은 `settings` 키 `mail`, 미설정이면 메일 기능 없음.

pub mod reset_job;
pub mod routes;
pub mod settings;
pub mod text;

use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use lettre::{
    Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use sqlx::PgExecutor;
use uuid::Uuid;

pub use settings::{MailConfig, MailSecurity};

use crate::{
    error::{AppError, AppResult},
    settings::queries as instance,
    state::AppState,
};

/// `AppState::mail_timeout` 기본값. 접속부터 전송 완료까지.
pub const MAIL_TIMEOUT: Duration = Duration::from_secs(15);
/// RFC 5321 경로 길이 상한.
const ADDRESS_MAX: usize = 254;

/// 일반 텍스트 메일 한 통.
pub struct Outgoing {
    pub to: String,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    /// 연결, 인증, 거부, 시간 초과 등. 문구는 비밀번호 제거 후.
    #[error("{0}")]
    Send(String),
    /// 발송 전 서버 내부 오류(인스턴스 설정 조회 등).
    #[error(transparent)]
    App(#[from] AppError),
}

/// 메일 주소 형식. 길이 상한과 `local@domain` 파싱.
pub fn valid_address(address: &str) -> bool {
    address.len() <= ADDRESS_MAX && address.parse::<Address>().is_ok()
}

/// 설정 키 존재 여부. 저장 검증이 host, from_address 보장.
pub async fn is_configured(exec: impl PgExecutor<'_>) -> AppResult<bool> {
    let exists = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM settings WHERE key = 'mail') AS "exists!""#
    )
    .fetch_one(exec)
    .await?;
    Ok(exists)
}

/// 보내는 이름은 `instance_name`. 전체 과정 `state.mail_timeout` 제한.
pub async fn send(state: &AppState, cfg: &MailConfig, msg: Outgoing) -> Result<(), MailError> {
    let instance_name = instance::load(&state.pool).await?.instance_name;
    let fail = |e: &dyn std::fmt::Display| MailError::Send(redact(cfg, &e.to_string()));

    let from = cfg.from_address.parse::<Address>().map_err(|e| fail(&e))?;
    let to = msg.to.parse::<Address>().map_err(|e| fail(&e))?;
    // Gmail 등 수신 측이 Message-ID 없는 메일 거부. 도메인은 보내는 주소 기준.
    let message_id = format!("<{}@{}>", Uuid::now_v7(), from.domain());
    let message = Message::builder()
        .message_id(Some(message_id))
        .from(Mailbox::new(Some(instance_name), from))
        .to(Mailbox::new(None, to))
        .subject(msg.subject)
        .header(ContentType::TEXT_PLAIN)
        .body(msg.body)
        .map_err(|e| fail(&e))?;

    let builder = match cfg.security {
        MailSecurity::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host),
        MailSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host),
        MailSecurity::None => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
            &cfg.host,
        )),
    }
    .map_err(|e| fail(&e))?;
    let mut builder = builder.port(cfg.port);
    if let Some(username) = &cfg.username {
        builder = builder.credentials(Credentials::new(
            username.clone(),
            cfg.password.clone().unwrap_or_default(),
        ));
    }
    let transport = builder.build();

    match tokio::time::timeout(state.mail_timeout, transport.send(message)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(fail(&e)),
        Err(_) => Err(MailError::Send("timed out".to_owned())),
    }
}

/// 요청 안 즉시 발송 결과. 실패해도 원래 작업은 성공 처리.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailOutcome {
    Sent,
    Failed,
}

/// 미설정, 발송 실패, 내부 오류 모두 `Failed`. 경고 문구는 비밀번호 제거된 오류만.
pub async fn send_now(
    state: &AppState,
    to: &str,
    (subject, body): (String, String),
) -> MailOutcome {
    let config = match settings::load(&state.pool).await {
        Ok(Some(config)) => config,
        Ok(None) => return MailOutcome::Failed,
        Err(e) => {
            tracing::warn!(error = %e, "mail settings load failed");
            return MailOutcome::Failed;
        }
    };
    let msg = Outgoing {
        to: to.to_owned(),
        subject,
        body,
    };
    match send(state, &config, msg).await {
        Ok(()) => MailOutcome::Sent,
        Err(e) => {
            tracing::warn!(error = %e, "mail send failed");
            MailOutcome::Failed
        }
    }
}

/// 비밀번호 원문과 AUTH PLAIN, LOGIN 의 base64 형태를 `***` 로.
fn redact(cfg: &MailConfig, text: &str) -> String {
    let Some(password) = cfg.password.as_deref().filter(|p| !p.is_empty()) else {
        return text.to_owned();
    };
    let username = cfg.username.as_deref().unwrap_or_default();
    [
        STANDARD.encode(format!("\0{username}\0{password}")),
        STANDARD.encode(password),
        password.to_owned(),
    ]
    .iter()
    .fold(text.to_owned(), |out, secret| out.replace(secret, "***"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(password: Option<&str>) -> MailConfig {
        MailConfig {
            host: "smtp.example.com".into(),
            port: 587,
            security: MailSecurity::Starttls,
            username: Some("mom".into()),
            password: password.map(str::to_owned),
            from_address: "mom@example.com".into(),
        }
    }

    #[test]
    fn redacts_password_forms() {
        let c = cfg(Some("pw-secret"));
        let plain = STANDARD.encode("\0mom\0pw-secret");
        let login = STANDARD.encode("pw-secret");
        let out = redact(&c, &format!("535 bad pw-secret {plain} {login}"));
        assert_eq!(out, "535 bad *** *** ***");
        assert_eq!(redact(&cfg(None), "535 bad"), "535 bad");
    }

    #[test]
    fn address_format() {
        assert!(valid_address("mom@example.com"));
        assert!(!valid_address("nope"));
        assert!(!valid_address("a b@example.com"));
        assert!(!valid_address(&format!("{}@example.com", "a".repeat(250))));
    }
}
