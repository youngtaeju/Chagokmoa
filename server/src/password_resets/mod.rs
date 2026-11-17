use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

pub mod queries;
pub mod routes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResetStatus {
    Active,
    Expired,
    Used,
    Revoked,
}

impl ResetStatus {
    /// 우선순위 revoked > used > expired > active. 초대와 동일.
    fn at(
        expires_at: DateTime<Utc>,
        used_at: Option<DateTime<Utc>>,
        revoked_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Self {
        if revoked_at.is_some() {
            ResetStatus::Revoked
        } else if used_at.is_some() {
            ResetStatus::Used
        } else if expires_at <= now {
            ResetStatus::Expired
        } else {
            ResetStatus::Active
        }
    }
}

/// 토큰 해시 미포함.
#[derive(Debug, Clone)]
pub struct PasswordReset {
    pub id: Uuid,
    pub user_id: Uuid,
    pub expires_at: DateTime<Utc>,
    status: ResetStatus,
}

impl PasswordReset {
    /// 미리보기, 사용 가능 여부. 불가 시 410.
    pub fn ensure_active(&self) -> AppResult<()> {
        match self.status {
            ResetStatus::Active => Ok(()),
            ResetStatus::Expired => Err(AppError::Gone("passwordReset.expired")),
            ResetStatus::Used => Err(AppError::Gone("passwordReset.used")),
            ResetStatus::Revoked => Err(AppError::Gone("passwordReset.revoked")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn status_precedence() {
        let now = Utc::now();
        let past = now - Duration::hours(1);
        let future = now + Duration::hours(1);
        let at = |e, u, r| ResetStatus::at(e, u, r, now);

        assert_eq!(at(future, None, None), ResetStatus::Active);
        assert_eq!(at(now, None, None), ResetStatus::Expired);
        assert_eq!(at(past, Some(past), None), ResetStatus::Used);
        assert_eq!(at(future, Some(past), Some(past)), ResetStatus::Revoked);
    }
}
