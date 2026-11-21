//! 진행 쿠키. start 에서 callback 까지의 state, nonce, PKCE 검증값과 용도별 값.
//! `SECRET_KEY` HMAC 서명(테이블 없음), 10분.

use axum_extra::extract::cookie::{Cookie, SameSite};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{auth::token, config::Config};

pub const COOKIE_NAME: &str = "cgm_oidc";
/// start, callback 에만 전송.
pub const COOKIE_PATH: &str = "/api/auth/oidc";
pub const TTL_SECS: u64 = 600;
/// 서명 용도 구분.
const CONTEXT: &str = "oidc-progress";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Purpose {
    /// 연결된 계정으로 로그인.
    Login,
    /// 로그인한 본인 계정에 IdP 계정 연결.
    Link,
    /// 초대 링크로 비밀번호 없는 계정 가입.
    Invite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub purpose: Purpose,
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
    /// unix 초. 이 시각부터 무효.
    pub exp: i64,
    /// login 용도. 검사 통과한 앱 경로.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect: Option<String>,
    /// link 용도. 시작한 세션의 사용자.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Uuid>,
    /// invite 용도. 초대 원문 토큰.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite: Option<String>,
    /// invite 용도. 검사 통과한 정규화 아이디.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// invite 용도. 검사 통과한 표시 이름.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// invite 용도. 검사 통과한 계정 언어.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

pub fn encode(secret: &[u8], progress: &Progress) -> String {
    // 문자열, 정수, uuid 만이라 직렬화 실패 불가.
    let json = serde_json::to_vec(progress).unwrap_or_else(|_| unreachable!());
    token::sign_opaque(secret, CONTEXT, &json)
}

/// 서명 불일치, 형식 오류, 만료(`now >= exp`)는 `None`.
pub fn decode(secret: &[u8], value: &str, now: DateTime<Utc>) -> Option<Progress> {
    let json = token::verify_opaque(secret, CONTEXT, value)?;
    let progress: Progress = serde_json::from_slice(&json).ok()?;
    (now.timestamp() < progress.exp).then_some(progress)
}

pub fn cookie(config: &Config, value: String) -> Cookie<'static> {
    // 10분은 time::Duration 범위 내. 변환 실패 불가.
    let max_age = std::time::Duration::from_secs(TTL_SECS)
        .try_into()
        .unwrap_or_else(|_| unreachable!());
    Cookie::build((COOKIE_NAME, value))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path(COOKIE_PATH)
        .secure(config.public_url.starts_with("https://"))
        .max_age(max_age)
        .build()
}

/// 발급 쿠키와 같은 이름, 경로, 속성. 빈 값, Max-Age 0.
pub fn clear_cookie(config: &Config) -> Cookie<'static> {
    let mut cookie = cookie(config, String::new());
    cookie.make_removal();
    cookie
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn sample(exp: i64) -> Progress {
        Progress {
            purpose: Purpose::Link,
            state: "state".into(),
            nonce: "nonce".into(),
            pkce_verifier: "verifier".into(),
            exp,
            redirect: None,
            user_id: Some(Uuid::now_v7()),
            invite: None,
            username: None,
            display_name: None,
            locale: None,
        }
    }

    #[test]
    fn roundtrip_and_expiry() {
        let secret = b"secret";
        let now = Utc::now();
        let exp = (now + Duration::seconds(60)).timestamp();
        let p = sample(exp);
        let value = encode(secret, &p);
        assert_eq!(decode(secret, &value, now), Some(p.clone()));
        assert_eq!(
            decode(secret, &value, DateTime::from_timestamp(exp, 0).unwrap()),
            None
        );
        assert_eq!(decode(b"other", &value, now), None);
        // 다른 용도 서명 값은 무효.
        let json = serde_json::to_vec(&p).unwrap();
        assert_eq!(
            decode(secret, &token::sign_opaque(secret, "other", &json), now),
            None
        );
    }

    #[test]
    fn payload_shape() {
        let p = Progress {
            purpose: Purpose::Login,
            redirect: Some("/days".into()),
            user_id: None,
            ..sample(1)
        };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            serde_json::json!({
                "purpose": "login",
                "state": "state",
                "nonce": "nonce",
                "pkce_verifier": "verifier",
                "exp": 1,
                "redirect": "/days",
            })
        );
    }
}
