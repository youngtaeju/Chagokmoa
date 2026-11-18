//! 백업 실행과 일정. 한 번에 하나(Postgres advisory lock, 서버와 CLI 공통).
//! 순서: 서버 버전 확인 → 덤프 → 로컬(빈 폴더면 init, unlock, backup) → 원격(unlock, backup) → 보관 정책(주 1회 prune, check).

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context;
use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, pool::PoolConnection};
use tokio::{sync::watch, task::JoinHandle};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    dump, queries,
    restic::{self, BackupSummary, Repo, RepoState, Retention},
    routes,
    settings::{self, BackupSettings},
};
use crate::{
    error::{AppError, AppResult},
    notifications::record,
    settings::queries as instance,
    state::AppState,
};

/// 실행 잠금 키. 다른 advisory lock(해시 기반)과 겹칠 일 없는 고정값.
pub const LOCK_KEY: i64 = 0x6367_6d62_6163_6b75;
/// 덤프 형식과 복구 도구(`pg_dump` 17) 기준.
const SUPPORTED_POSTGRES: u32 = 17;
/// 실행 기록 보관 개수.
const KEEP_RUNS: i64 = 100;
/// 일정 확인 주기.
const CHECK_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, ToSchema)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum Trigger {
    Scheduled,
    Manual,
}

impl Trigger {
    /// 스냅숏 태그.
    fn tag(self) -> &'static str {
        match self {
            Trigger::Scheduled => "scheduled",
            Trigger::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, ToSchema)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Success,
    /// 성공. originals 의 사라진 파일 등 경고 포함.
    Warning,
    Failed,
}

/// 진행 중 단계. `forget` 은 보관 정책과 prune, check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, ToSchema)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Dump,
    Local,
    Remote,
    Forget,
}

pub type ServerMajorFn = for<'a> fn(&'a PgPool) -> BoxFuture<'a, AppResult<u32>>;
pub type DumpFn = for<'a> fn(&'a str, &'a Path) -> BoxFuture<'a, AppResult<()>>;

/// DB 단계. 테스트에서 교체(러너의 pg_dump 16 으로 17 서버 덤프 불가).
#[derive(Clone, Copy)]
pub struct DbSteps {
    pub server_major: ServerMajorFn,
    pub dump: DumpFn,
}

fn real_server_major(pool: &PgPool) -> BoxFuture<'_, AppResult<u32>> {
    Box::pin(dump::server_major(pool))
}

fn real_dump<'a>(database_url: &'a str, path: &'a Path) -> BoxFuture<'a, AppResult<()>> {
    Box::pin(dump::to_file(database_url, path))
}

impl Default for DbSteps {
    fn default() -> Self {
        Self {
            server_major: real_server_major,
            dump: real_dump,
        }
    }
}

/// 저장소별 결과. `backup_runs.local`, `remote` 에 JSON 으로.
#[derive(Debug, Default, Clone, Serialize, Deserialize, ToSchema)]
pub struct RepoReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files_new: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_added: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
    /// 비밀값 제거 후.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl RepoReport {
    fn from_summary(s: BackupSummary) -> Self {
        Self {
            snapshot_id: Some(s.snapshot_id),
            files_new: Some(s.files_new),
            data_added: Some(s.data_added),
            duration_ms: Some(u64::try_from(s.duration.as_millis()).unwrap_or(u64::MAX)),
            warning: s.warning,
            error: None,
        }
    }

    fn failed(repo: &Repo, err: &AppError) -> Self {
        Self {
            error: Some(repo.redact(&describe(err))),
            ..Self::default()
        }
    }

    fn ok(&self) -> bool {
        self.error.is_none()
    }
}

/// 오류 문구. anyhow 는 원인 사슬까지.
pub(super) fn describe(err: &AppError) -> String {
    match err {
        AppError::Internal(e) => format!("{e:#}"),
        other => other.to_string(),
    }
}

/// `{DATA_DIR}/cache/backup/db.dump`. 실행 끝에 삭제.
pub fn dump_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cache/backup/db.dump")
}

pub fn ensure_supported(major: u32) -> AppResult<()> {
    if major == SUPPORTED_POSTGRES {
        return Ok(());
    }
    Err(AppError::Conflict {
        key: "backup.unsupportedPostgres",
        params: json!({ "major": major, "required": SUPPORTED_POSTGRES }),
    })
}

/// 잠금을 잡고 기록을 연 실행. 드롭 시 잠금 연결을 닫아 잠금 해제.
pub struct Started {
    id: Uuid,
    trigger: Trigger,
    retention: Retention,
    local: Repo,
    remote: Option<Repo>,
    _lock: PoolConnection<Postgres>,
}

/// 실행 잠금. 백업 실행과 저장소 만들기(설정 화면) 공용. 이미 잡혀 있으면 409 `backup.running`.
/// 드롭 시 연결째 닫아 해제.
pub(super) async fn try_lock(pool: &PgPool) -> AppResult<PoolConnection<Postgres>> {
    let mut lock = pool.acquire().await?;
    // 세션 잠금이라 풀로 돌아가면 남음. 드롭(취소, 패닉 포함) 때 연결째 닫음.
    // 질의 전에 설정. 잠금 획득 직후 취소돼도 연결이 풀로 돌아가지 않게.
    lock.close_on_drop();
    let locked = sqlx::query_scalar!(r#"SELECT pg_try_advisory_lock($1) AS "locked!""#, LOCK_KEY)
        .fetch_one(&mut *lock)
        .await?;
    if !locked {
        return Err(AppError::conflict("backup.running"));
    }
    Ok(lock)
}

/// 설정 확인, 잠금, 기록 시작, 서버 버전 확인까지. 버전 불일치는 실패 기록과 알림 뒤 오류.
pub async fn start(state: &AppState, trigger: Trigger) -> AppResult<Started> {
    let settings: BackupSettings = settings::load(&state.pool).await?;
    let Some(local) = settings.local_repo(&state.config) else {
        return Err(AppError::conflict("backup.notConfigured"));
    };
    let remote = settings.remote_repo(&state.config);

    let lock = try_lock(&state.pool).await?;

    let interrupted = queries::mark_interrupted(&state.pool).await?;
    if interrupted > 0 {
        tracing::warn!(count = interrupted, "previous backup run was interrupted");
    }
    // 중단된 이전 실행의 덤프. 복구 키와 인증값 포함이라 남기지 않음.
    remove_dump(&dump_path(&state.config.data_dir));
    let id = Uuid::now_v7();
    queries::insert(&state.pool, id, trigger).await?;
    let started = Started {
        id,
        trigger,
        retention: settings.retention,
        local,
        remote,
        _lock: lock,
    };

    let gate = match (state.backup_db.server_major)(&state.pool).await {
        Ok(major) => ensure_supported(major).map_err(|e| {
            (
                e,
                format!("postgres {major} unsupported, {SUPPORTED_POSTGRES} required"),
            )
        }),
        Err(e) => {
            let message = describe(&e);
            Err((e, message))
        }
    };
    if let Err((err, message)) = gate {
        let finish = queries::Finish {
            status: RunStatus::Failed,
            local: None,
            remote: None,
            error: Some(&message),
            pruned: false,
        };
        close(state, id, finish).await?;
        return Err(err);
    }
    Ok(started)
}

impl Started {
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// 덤프부터 끝까지. 저장소 실패와 도중 DB 오류 모두 기록과 알림으로 처리하고 `Ok`.
    /// `Err` 는 기록 마감 자체의 DB 오류.
    pub async fn finish(self, state: &AppState) -> AppResult<()> {
        let id = self.id;
        let dump = DumpFile(dump_path(&state.config.data_dir));
        let mut out = Outcome {
            remote: self.remote.as_ref().map(|_| RepoReport::default()),
            ..Outcome::default()
        };
        if let Err(e) = self.work(state, &dump.0, &mut out).await {
            out.error
                .get_or_insert_with(|| format!("internal: {}", self.redact(&describe(&e))));
        }
        drop(dump);

        let Outcome {
            local,
            remote,
            pruned,
            error,
        } = out;
        let error = error
            .or_else(|| local.error.as_ref().map(|e| format!("local: {e}")))
            .or_else(|| {
                remote
                    .as_ref()
                    .and_then(|r| r.error.as_ref())
                    .map(|e| format!("remote: {e}"))
            });
        let warned =
            local.warning.is_some() || remote.as_ref().is_some_and(|r| r.warning.is_some());
        let status = if error.is_some() {
            RunStatus::Failed
        } else if warned {
            RunStatus::Warning
        } else {
            RunStatus::Success
        };
        let finish = queries::Finish {
            status,
            local: Some(to_json(&local)?),
            remote: remote.as_ref().map(to_json).transpose()?,
            error: error.as_deref(),
            pruned: pruned && error.is_none(),
        };
        close(state, id, finish).await?;
        match &error {
            Some(error) => tracing::error!(%id, error, "backup failed"),
            None => tracing::info!(%id, ?status, "backup finished"),
        }
        Ok(())
    }

    /// 덤프, 저장소별 스냅숏, 보관 정책. 결과는 `out` 에 누적, `Err` 는 도중 DB 오류.
    async fn work(&self, state: &AppState, dump_file: &Path, out: &mut Outcome) -> AppResult<()> {
        let id = self.id;
        queries::set_phase(&state.pool, id, Phase::Dump).await?;
        if let Err(e) = dump_db(state, dump_file).await {
            out.error = Some(format!("dump: {}", self.redact(&describe(&e))));
            return Ok(());
        }
        let paths = [
            dump_file.to_path_buf(),
            state.config.data_dir.join("originals"),
        ];
        let tag = self.trigger.tag();
        queries::set_phase(&state.pool, id, Phase::Local).await?;
        out.local = backup_local(&self.local, &state.config.backup_dir, &paths, tag).await;
        if let (Some(repo), Some(report)) = (&self.remote, &mut out.remote) {
            queries::set_phase(&state.pool, id, Phase::Remote).await?;
            *report = backup_repo(repo, &paths, tag).await;
        }
        queries::set_phase(&state.pool, id, Phase::Forget).await?;
        let prune = queries::prune_due(&state.pool).await?;
        out.pruned = prune;
        let mut targets = vec![(&self.local, &mut out.local)];
        if let (Some(repo), Some(report)) = (&self.remote, &mut out.remote) {
            targets.push((repo, report));
        }
        for (repo, report) in targets {
            // 실패한 저장소는 지우는 작업 생략
            if !report.ok() {
                out.pruned = false;
                continue;
            }
            if let Err(e) = maintain(repo, self.retention, prune).await {
                report.error = Some(repo.redact(&describe(&e)));
                out.pruned = false;
            }
        }
        Ok(())
    }

    /// 로컬과 원격 비밀값 모두 가림.
    fn redact(&self, text: &str) -> String {
        let text = self.local.redact(text);
        match &self.remote {
            Some(remote) => remote.redact(&text),
            None => text,
        }
    }
}

/// 한 번의 실행 결과.
#[derive(Default)]
struct Outcome {
    local: RepoReport,
    remote: Option<RepoReport>,
    pruned: bool,
    /// 저장소 밖 오류(덤프, 도중 DB 오류). 비밀값 제거 후.
    error: Option<String>,
}

/// 덤프 파일. 드롭(정상 종료, 오류, 취소, 패닉) 때 삭제.
struct DumpFile(PathBuf);

impl Drop for DumpFile {
    fn drop(&mut self) {
        remove_dump(&self.0);
    }
}

fn remove_dump(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(error = %e, "backup dump cleanup failed");
    }
}

fn to_json(report: &RepoReport) -> AppResult<Value> {
    Ok(serde_json::to_value(report).context("backup report serialize failed")?)
}

async fn dump_db(state: &AppState, path: &Path) -> AppResult<()> {
    prepare_private(path)
        .await
        .context("backup dump file prepare failed")?;
    (state.backup_db.dump)(&state.config.database_url, path).await
}

/// 덤프 자리를 디렉터리 0700, 빈 파일 0600 으로 준비. pg_dump 는 기존 파일 권한 유지.
#[cfg(unix)]
async fn prepare_private(path: &Path) -> std::io::Result<()> {
    use std::{fs::Permissions, os::unix::fs::PermissionsExt};

    if let Some(dir) = path.parent() {
        tokio::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .await?;
        // 이전 버전이 만든 폴더 권한도 좁힘.
        tokio::fs::set_permissions(dir, Permissions::from_mode(0o700)).await?;
    }
    remove_dump(path);
    tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .await?;
    Ok(())
}

#[cfg(not(unix))]
async fn prepare_private(path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    Ok(())
}

/// 이전 실행이 남긴 잠금 해제 후 스냅숏. restic 은 죽은 프로세스의 잠금만 지움.
async fn backup_repo(repo: &Repo, paths: &[PathBuf], tag: &str) -> RepoReport {
    let result = async {
        repo.unlock().await?;
        repo.backup(paths, tag).await
    }
    .await;
    match result {
        Ok(summary) => RepoReport::from_summary(summary),
        Err(e) => RepoReport::failed(repo, &e),
    }
}

/// 로컬 저장소 경고. 비밀값 없는 고정 문구.
const LOCAL_CREATED: &str = "local repository created";

/// 로컬 스냅숏. 연결된 백업 폴더가 비어 있으면 저장된 키로 저장소 생성 후 경고.
/// 원격에서 새 서버로 복구, 디스크 교체 대비. 폴더 없음이나 쓰기 불가면 생성 없이 실패(컨테이너 레이어에 저장소 방지).
async fn backup_local(repo: &Repo, dir: &Path, paths: &[PathBuf], tag: &str) -> RepoReport {
    let created = match init_if_empty(repo, dir).await {
        Ok(created) => created,
        Err(e) => return RepoReport::failed(repo, &e),
    };
    let mut report = backup_repo(repo, paths, tag).await;
    if created {
        report.warning = Some(match report.warning.take() {
            Some(w) => format!("{LOCAL_CREATED}; {w}"),
            None => LOCAL_CREATED.to_owned(),
        });
    }
    report
}

/// 쓰기 가능하고 빈 폴더면 init, 만들었으면 true. 실행 잠금 안이라 설정 화면 init 과 경합 없음.
async fn init_if_empty(repo: &Repo, dir: &Path) -> AppResult<bool> {
    if !routes::writable(dir).await || restic::local_state(dir).await? != RepoState::Empty {
        return Ok(false);
    }
    repo.init().await?;
    tracing::warn!("local backup repository was empty, created with stored key");
    Ok(true)
}

/// 보관 정책. `prune` 이면 공간 회수와 무결성 확인까지.
async fn maintain(repo: &Repo, retention: Retention, prune: bool) -> AppResult<()> {
    repo.forget(retention).await?;
    if prune {
        repo.prune().await?;
        repo.check().await?;
    }
    Ok(())
}

/// 기록 마감, 오래된 기록 정리, 실패면 소유자 알림.
async fn close(state: &AppState, id: Uuid, finish: queries::Finish<'_>) -> AppResult<()> {
    let failed = finish.status == RunStatus::Failed;
    let mut tx = state.pool.begin().await?;
    queries::finish(&mut *tx, id, finish).await?;
    queries::trim(&mut *tx, KEEP_RUNS).await?;
    if failed {
        let owners = queries::owner_ids(&mut *tx).await?;
        record::record_backup_failed(&mut tx, &owners).await?;
    }
    tx.commit().await?;
    if failed {
        state.push_wake.notify_one();
    }
    Ok(())
}

/// 지금 실행. 끝날 때까지 대기. 저장소 실패도 기록 후 `Ok(id)`.
pub async fn run_once(state: &AppState, trigger: Trigger) -> AppResult<Uuid> {
    let started = start(state, trigger).await?;
    let id = started.id();
    started.finish(state).await?;
    Ok(id)
}

/// 자동 백업이 켜져 있고 오늘 예정 시각이 지났는데 그 뒤 실행이 없으면 true.
pub async fn due(state: &AppState, now: DateTime<Utc>) -> AppResult<bool> {
    let settings = settings::load(&state.pool).await?;
    if !settings.enabled || settings.password.is_none() {
        return Ok(false);
    }
    let timezone = instance::load(&state.pool).await?.timezone;
    queries::due(&state.pool, now, &timezone, settings.time).await
}

/// 일정 확인 1회. 실행했으면 그 id.
pub async fn tick(state: &AppState, now: DateTime<Utc>) -> AppResult<Option<Uuid>> {
    if !due(state, now).await? {
        return Ok(None);
    }
    match run_once(state, Trigger::Scheduled).await {
        Ok(id) => Ok(Some(id)),
        // 수동 실행 중. 끝난 뒤 다음 확인에서.
        Err(AppError::Conflict {
            key: "backup.running",
            ..
        }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 1분마다 일정 확인. 종료 신호 시 진행 중 실행도 중단(restic 종료, 덤프 삭제, 잠금 해제, 다음 실행 때 interrupted 처리).
pub fn spawn(state: AppState, mut shutdown: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                res = tick(&state, Utc::now()) => {
                    if let Err(err) = res {
                        tracing::error!(error = ?err, "scheduled backup failed");
                    }
                }
                _ = shutdown.wait_for(|stop| *stop) => break,
            }
            tokio::select! {
                _ = tokio::time::sleep(CHECK_INTERVAL) => {}
                _ = shutdown.wait_for(|stop| *stop) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_17_supported() {
        assert!(ensure_supported(17).is_ok());
        for major in [16, 18] {
            assert!(matches!(
                ensure_supported(major),
                Err(AppError::Conflict {
                    key: "backup.unsupportedPostgres",
                    ..
                })
            ));
        }
    }

    #[test]
    fn report_json_omits_empty() {
        let report = RepoReport {
            error: Some("x".into()),
            ..RepoReport::default()
        };
        assert_eq!(to_json(&report).unwrap(), json!({ "error": "x" }));
    }
}
