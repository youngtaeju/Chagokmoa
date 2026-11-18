//! Postgres 작업 큐.

use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

pub mod worker;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    ProcessAsset {
        asset_id: Uuid,
    },
    /// 파생물만 다시 생성. 상태와 메타데이터 불변.
    RegenerateDerivatives {
        asset_id: Uuid,
    },
    SendPush {
        notification_id: Uuid,
    },
}

#[derive(Deserialize)]
struct AssetPayload {
    asset_id: Uuid,
}

#[derive(Deserialize)]
struct NotificationPayload {
    notification_id: Uuid,
}

impl Job {
    /// `jobs.kind` 값.
    pub fn kind(&self) -> &'static str {
        match self {
            Job::ProcessAsset { .. } => "process_asset",
            Job::RegenerateDerivatives { .. } => "regenerate_derivatives",
            Job::SendPush { .. } => "send_push",
        }
    }

    /// `jobs.payload` 값.
    pub fn payload(&self) -> Value {
        match self {
            Job::ProcessAsset { asset_id } | Job::RegenerateDerivatives { asset_id } => {
                json!({ "asset_id": asset_id })
            }
            Job::SendPush { notification_id } => json!({ "notification_id": notification_id }),
        }
    }

    pub fn parse(kind: &str, payload: Value) -> anyhow::Result<Job> {
        match kind {
            "process_asset" => {
                let p: AssetPayload = serde_json::from_value(payload)?;
                Ok(Job::ProcessAsset {
                    asset_id: p.asset_id,
                })
            }
            "regenerate_derivatives" => {
                let p: AssetPayload = serde_json::from_value(payload)?;
                Ok(Job::RegenerateDerivatives {
                    asset_id: p.asset_id,
                })
            }
            "send_push" => {
                let p: NotificationPayload = serde_json::from_value(payload)?;
                Ok(Job::SendPush {
                    notification_id: p.notification_id,
                })
            }
            other => anyhow::bail!("unknown job kind: {other}"),
        }
    }
}

/// `queued` 로 즉시 실행 대상. 호출 측이 커밋 후 `AppState::jobs_wake.notify_one()`.
pub async fn enqueue(exec: impl PgExecutor<'_>, job: &Job) -> AppResult<Uuid> {
    let id = sqlx::query_scalar!(
        "INSERT INTO jobs (id, kind, payload) VALUES ($1, $2, $3) RETURNING id",
        Uuid::now_v7(),
        job.kind(),
        job.payload(),
    )
    .fetch_one(exec)
    .await?;
    Ok(id)
}

/// 자산 삭제 시 함께 지울 작업 행 (상태 무관).
pub async fn delete_for_asset(exec: impl PgExecutor<'_>, asset_id: Uuid) -> AppResult<()> {
    sqlx::query!(
        "DELETE FROM jobs WHERE payload ->> 'asset_id' = $1",
        asset_id.to_string()
    )
    .execute(exec)
    .await?;
    Ok(())
}

/// `running` 으로 집은 작업.
#[derive(Debug)]
pub struct ClaimedJob {
    pub id: Uuid,
    pub job: Job,
    /// 이번 실행 포함 횟수.
    pub attempts: i32,
}

/// 실행 시각이 된 `kinds` 종류의 `queued` 작업 1개를 `running` 으로. 동시 워커끼리 행 잠금 건너뜀.
/// 재생성 작업은 다른 종류보다 뒤(대량 재생성 중에도 새 업로드 먼저).
/// 해석 불가 작업은 `failed` 로 돌리고 에러.
pub async fn claim(pool: &PgPool, kinds: &[&str]) -> AppResult<Option<ClaimedJob>> {
    let Some(row) = sqlx::query!(
        "UPDATE jobs SET status = 'running', locked_at = now(), attempts = attempts + 1
         WHERE id = (SELECT id FROM jobs
                     WHERE status = 'queued' AND run_after <= now() AND kind = ANY($1)
                     ORDER BY (kind = 'regenerate_derivatives'), run_after LIMIT 1 FOR UPDATE SKIP LOCKED)
         RETURNING id, kind, payload, attempts",
        kinds as &[&str]
    )
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    match Job::parse(&row.kind, row.payload) {
        Ok(job) => Ok(Some(ClaimedJob {
            id: row.id,
            job,
            attempts: row.attempts,
        })),
        Err(err) => {
            fail(pool, row.id, &format!("{err:#}")).await?;
            Err(AppError::Internal(err.context(format!("job {}", row.id))))
        }
    }
}

/// 성공 작업 행 삭제. 이미 없어도 성공.
pub async fn finish(exec: impl PgExecutor<'_>, id: Uuid) -> AppResult<()> {
    sqlx::query!("DELETE FROM jobs WHERE id = $1", id)
        .execute(exec)
        .await?;
    Ok(())
}

/// 자동 재시도 없음. 이미 없어도 성공.
pub async fn fail(exec: impl PgExecutor<'_>, id: Uuid, error: &str) -> AppResult<()> {
    sqlx::query!(
        "UPDATE jobs SET status = 'failed', last_error = $2 WHERE id = $1",
        id,
        error
    )
    .execute(exec)
    .await?;
    Ok(())
}

/// 이전 프로세스가 실행 중 종료한 작업을 큐로 복귀. 워커 기동 전 1회.
pub async fn requeue_running(exec: impl PgExecutor<'_>) -> AppResult<u64> {
    let res = sqlx::query!(
        "UPDATE jobs SET status = 'queued', locked_at = NULL WHERE status = 'running'"
    )
    .execute(exec)
    .await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_round_trip() {
        let job = Job::ProcessAsset {
            asset_id: Uuid::now_v7(),
        };
        assert_eq!(Job::parse(job.kind(), job.payload()).unwrap(), job);
        assert!(Job::parse("unknown", job.payload()).is_err());
        assert!(Job::parse("process_asset", json!({})).is_err());

        let regen = Job::RegenerateDerivatives {
            asset_id: Uuid::now_v7(),
        };
        assert_eq!(regen.kind(), "regenerate_derivatives");
        assert_eq!(Job::parse(regen.kind(), regen.payload()).unwrap(), regen);

        let push = Job::SendPush {
            notification_id: Uuid::now_v7(),
        };
        assert_eq!(push.kind(), "send_push");
        assert_eq!(Job::parse(push.kind(), push.payload()).unwrap(), push);
        assert!(Job::parse("send_push", json!({})).is_err());
    }
}
