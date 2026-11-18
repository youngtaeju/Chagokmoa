//! 백업 설정. `settings` 키 `backup`. 복구 키와 원격 인증값은 평문 저장(사용자 결정), 출력 금지.

use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

use anyhow::{Context, anyhow};
use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;

use super::{
    mask_location,
    restic::{Repo, Retention},
};
use crate::{config::Config, error::AppResult, settings::queries as instance};

const KEY: &str = "backup";

/// `Debug` 는 비밀값 없이(아래 수동 구현).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupSettings {
    /// 자동 백업. 수동 실행은 꺼져 있어도 가능.
    pub enabled: bool,
    /// 인스턴스 시간대 기준 매일 실행 시각.
    pub time: NaiveTime,
    pub retention: Retention,
    /// restic 비밀번호(복구 키). 로컬과 원격 공용. 없으면 미설정.
    pub password: Option<String>,
    pub remote: Option<Remote>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    /// restic 저장소 주소. 예: `s3:https://host/bucket`.
    pub repository: String,
    /// 원격 인증값 환경 변수.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            time: NaiveTime::from_hms_opt(3, 0, 0).unwrap_or_default(),
            retention: Retention::default(),
            password: None,
            remote: None,
        }
    }
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            daily: 7,
            weekly: 4,
            monthly: 12,
        }
    }
}

impl fmt::Debug for BackupSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackupSettings")
            .field("enabled", &self.enabled)
            .field("time", &self.time)
            .field("retention", &self.retention)
            .field("password_set", &self.password.is_some())
            .field("remote", &self.remote)
            .finish()
    }
}

impl fmt::Debug for Remote {
    /// 주소의 userinfo 비밀번호는 `***`, env 는 키만.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let env_keys: Vec<&str> = self.env.keys().map(String::as_str).collect();
        f.debug_struct("Remote")
            .field("repository", &mask_location(&self.repository))
            .field("env_keys", &env_keys)
            .finish()
    }
}

/// restic 캐시. 백업 대상 아님.
pub fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("cache/restic")
}

impl BackupSettings {
    /// 복구 키 없으면 None.
    pub fn local_repo(&self, config: &Config) -> Option<Repo> {
        let password = self.password.clone()?;
        Some(Repo {
            location: config.backup_dir.to_string_lossy().into_owned(),
            env: vec![("RESTIC_PASSWORD".to_owned(), password)],
            cache_dir: cache_dir(&config.data_dir),
        })
    }

    /// 복구 키나 원격 설정이 없으면 None. 복구 키는 인증값 뒤에 두어 덮어쓰기 방지.
    pub fn remote_repo(&self, config: &Config) -> Option<Repo> {
        let password = self.password.clone()?;
        let remote = self.remote.as_ref()?;
        let mut env: Vec<(String, String)> = remote
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        env.push(("RESTIC_PASSWORD".to_owned(), password));
        Some(Repo {
            location: remote.repository.clone(),
            env,
            cache_dir: cache_dir(&config.data_dir),
        })
    }
}

/// 미저장이면 기본값. 형식 오류는 값 노출 없이 오류(조용히 백업이 꺼지는 것 방지).
pub async fn load(exec: impl PgExecutor<'_>) -> AppResult<BackupSettings> {
    let value = sqlx::query_scalar!("SELECT value FROM settings WHERE key = 'backup'")
        .fetch_optional(exec)
        .await?;
    let Some(value) = value else {
        return Ok(BackupSettings::default());
    };
    // serde 오류 문구에 값이 실릴 수 있어 분류만.
    serde_json::from_value(value)
        .map_err(|e| anyhow!("backup settings unreadable ({:?})", e.classify()).into())
}

pub async fn save(exec: impl PgExecutor<'_>, settings: &BackupSettings) -> AppResult<()> {
    let value = serde_json::to_value(settings).context("backup settings serialize failed")?;
    instance::set(exec, KEY, value).await
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn defaults() {
        let s: BackupSettings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(s, BackupSettings::default());
        assert!(!s.enabled);
        assert_eq!(s.time, NaiveTime::from_hms_opt(3, 0, 0).unwrap());
        assert_eq!(
            s.retention,
            Retention {
                daily: 7,
                weekly: 4,
                monthly: 12
            }
        );
    }

    #[test]
    fn debug_hides_secrets() {
        let s = BackupSettings {
            password: Some("recovery-key-secret".into()),
            remote: Some(Remote {
                repository: "rest:https://u:pass-in-url@host/repo".into(),
                env: BTreeMap::from([("AWS_SECRET_ACCESS_KEY".into(), "aws-secret".into())]),
            }),
            ..BackupSettings::default()
        };
        let out = format!("{s:?}");
        for secret in ["recovery-key-secret", "pass-in-url", "aws-secret"] {
            assert!(!out.contains(secret), "{out}");
        }
        assert!(out.contains("AWS_SECRET_ACCESS_KEY"), "{out}");
    }
}
