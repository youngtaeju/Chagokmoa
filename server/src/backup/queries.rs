//! 실행 기록 `backup_runs` 와 일정 판정.

use chrono::{DateTime, NaiveTime, Utc};
use serde_json::Value;
use sqlx::PgExecutor;
use uuid::Uuid;

use super::run::{Phase, RunStatus, Trigger};
use crate::error::AppResult;

#[derive(Debug, Clone)]
pub struct BackupRun {
    pub id: Uuid,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub trigger: Trigger,
    pub status: RunStatus,
    /// 진행 중 단계. 끝나면 None.
    pub phase: Option<Phase>,
    /// 저장소별 결과(`run::RepoReport`).
    pub local: Option<Value>,
    pub remote: Option<Value>,
    pub error: Option<String>,
}

pub async fn get(exec: impl PgExecutor<'_>, id: Uuid) -> AppResult<Option<BackupRun>> {
    Ok(sqlx::query_as!(
        BackupRun,
        r#"SELECT id, started_at, finished_at, trigger AS "trigger: Trigger",
                  status AS "status: RunStatus", phase AS "phase: Phase", local, remote, error
           FROM backup_runs WHERE id = $1"#,
        id
    )
    .fetch_optional(exec)
    .await?)
}

/// 진행 중 실행. 진행 표시용.
pub async fn running(exec: impl PgExecutor<'_>) -> AppResult<Option<BackupRun>> {
    Ok(sqlx::query_as!(
        BackupRun,
        r#"SELECT id, started_at, finished_at, trigger AS "trigger: Trigger",
                  status AS "status: RunStatus", phase AS "phase: Phase", local, remote, error
           FROM backup_runs WHERE status = 'running'
           ORDER BY started_at DESC LIMIT 1"#
    )
    .fetch_optional(exec)
    .await?)
}

/// 잠금 보유 중에만 호출. 남은 running 은 중단된 이전 실행.
pub(super) async fn mark_interrupted(exec: impl PgExecutor<'_>) -> AppResult<u64> {
    let result = sqlx::query!(
        "UPDATE backup_runs
         SET status = 'failed', phase = NULL, finished_at = now(), error = 'interrupted'
         WHERE status = 'running'"
    )
    .execute(exec)
    .await?;
    Ok(result.rows_affected())
}

pub(super) async fn insert(exec: impl PgExecutor<'_>, id: Uuid, trigger: Trigger) -> AppResult<()> {
    sqlx::query!(
        "INSERT INTO backup_runs (id, trigger, status) VALUES ($1, $2, 'running')",
        id,
        trigger as Trigger
    )
    .execute(exec)
    .await?;
    Ok(())
}

pub(super) async fn set_phase(exec: impl PgExecutor<'_>, id: Uuid, phase: Phase) -> AppResult<()> {
    sqlx::query!(
        "UPDATE backup_runs SET phase = $2 WHERE id = $1",
        id,
        phase as Phase
    )
    .execute(exec)
    .await?;
    Ok(())
}

pub(super) struct Finish<'a> {
    pub status: RunStatus,
    pub local: Option<Value>,
    pub remote: Option<Value>,
    pub error: Option<&'a str>,
    pub pruned: bool,
}

pub(super) async fn finish(exec: impl PgExecutor<'_>, id: Uuid, f: Finish<'_>) -> AppResult<()> {
    sqlx::query!(
        "UPDATE backup_runs
         SET status = $2, phase = NULL, finished_at = now(), local = $3, remote = $4,
             error = $5, pruned = $6
         WHERE id = $1",
        id,
        f.status as RunStatus,
        f.local,
        f.remote,
        f.error,
        f.pruned
    )
    .execute(exec)
    .await?;
    Ok(())
}

/// 최근 `keep` 개만 남김.
pub(super) async fn trim(exec: impl PgExecutor<'_>, keep: i64) -> AppResult<()> {
    sqlx::query!(
        "DELETE FROM backup_runs WHERE id IN (
           SELECT id FROM backup_runs ORDER BY started_at DESC, id DESC OFFSET $1)",
        keep
    )
    .execute(exec)
    .await?;
    Ok(())
}

/// 최근 7일 안에 prune, check 를 마친 실행이 없으면 true.
pub(super) async fn prune_due(exec: impl PgExecutor<'_>) -> AppResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT NOT EXISTS (
             SELECT 1 FROM backup_runs WHERE pruned AND started_at > now() - interval '7 days'
           ) AS "due!""#
    )
    .fetch_one(exec)
    .await?)
}

/// 알림 받을 소유자. 정지 제외.
pub(super) async fn owner_ids(exec: impl PgExecutor<'_>) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar!(
        "SELECT id FROM users WHERE role = 'owner' AND disabled_at IS NULL ORDER BY id"
    )
    .fetch_all(exec)
    .await?)
}

/// `now` 가 그날(`timezone` 기준) 예정 시각 이후이고 그 시각 뒤 시작한 실행이 없으면 true.
/// 서버가 꺼져 있던 동안 지난 일정도 그날 안이면 한 번 실행.
pub(super) async fn due(
    exec: impl PgExecutor<'_>,
    now: DateTime<Utc>,
    timezone: &str,
    time: NaiveTime,
) -> AppResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"WITH s AS (
             SELECT ((($1::timestamptz AT TIME ZONE $2)::date + $3::time) AT TIME ZONE $2) AS at
           )
           SELECT $1 >= s.at AND NOT EXISTS (SELECT 1 FROM backup_runs WHERE started_at >= s.at)
             AS "due!"
           FROM s"#,
        now,
        timezone,
        time
    )
    .fetch_one(exec)
    .await?)
}
