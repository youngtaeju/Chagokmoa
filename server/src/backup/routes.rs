//! 백업 설정과 상태 API. 모두 owner.
//! 응답에 복구 키와 원격 인증값 없음(설정 여부만). 예외는 새 키 생성과 비밀번호 확인 뒤 다시 보기.

use std::{collections::BTreeMap, path::Path};

use axum::{Json as AxumJson, extract::State, http::StatusCode};
use chrono::{DateTime, NaiveTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::{
    mask_location, queries,
    restic::{self, Repo, RepoState, Retention},
    run::{self, Phase, RepoReport, RunStatus, Trigger},
    settings::{self, BackupSettings, Remote},
};
use crate::{
    auth::{extract::CurrentUser, password},
    error::{AppError, AppResult, ErrorBody},
    request::Json,
    settings::queries as instance,
    state::AppState,
    users::{self, Role},
};

/// 상태에 싣는 최근 실행 수.
const RUNS_SHOWN: i64 = 20;
/// 새 복구 키. 손으로 옮겨 적기 쉬운 Crockford base32(I, L, O, U 제외), 4자씩 8묶음. 160비트.
const KEY_ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const KEY_GROUPS: usize = 8;
const KEY_GROUP_LEN: usize = 4;
/// 새 저장소에 쓰는 키 최소 길이. 기존 저장소 키는 길이 제한 없음.
const NEW_KEY_MIN: usize = 32;
const KEY_MAX: usize = 1024;
const REPOSITORY_MAX: usize = 2048;
const ENV_VALUE_MAX: usize = 8192;
/// 보관 개수 상한. 일 1년, 주 10년, 월 20년.
const DAILY_MAX: u32 = 365;
const WEEKLY_MAX: u32 = 520;
const MONTHLY_MAX: u32 = 240;
/// 원격 접두어별 허용 인증값 환경 변수. 그 밖(sftp, rclone, swift, 로컬 경로)과 다른 변수는 거부.
/// `LD_PRELOAD`, `PATH`, `RESTIC_*` 등 실행 환경 주입 차단. `rest:` 는 주소 안 인증.
/// `gs:` 의 `GOOGLE_APPLICATION_CREDENTIALS` 는 컨테이너에 연결한 키 파일 경로.
const REMOTE_BACKENDS: [(&str, &[&str]); 5] = [
    (
        "s3:",
        &[
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_DEFAULT_REGION",
            "AWS_SESSION_TOKEN",
        ],
    ),
    ("b2:", &["B2_ACCOUNT_ID", "B2_ACCOUNT_KEY"]),
    ("rest:", &[]),
    (
        "azure:",
        &[
            "AZURE_ACCOUNT_NAME",
            "AZURE_ACCOUNT_KEY",
            "AZURE_ACCOUNT_SAS",
            "AZURE_ENDPOINT_SUFFIX",
        ],
    ),
    (
        "gs:",
        &["GOOGLE_PROJECT_ID", "GOOGLE_APPLICATION_CREDENTIALS"],
    ),
];

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_status, update_settings))
        .routes(routes!(new_key))
        .routes(routes!(enable))
        .routes(routes!(reveal_key))
        .routes(routes!(test_remote))
        .routes(routes!(save_remote, delete_remote))
        .routes(routes!(run_now))
}

#[derive(Serialize, ToSchema)]
pub struct BackupStatus {
    /// 로컬 백업 폴더(`/backup`)가 있고 쓰기 가능.
    mounted: bool,
    /// 로컬 폴더 내용(키 확인 없음). 연결 안 됨이면 null. `wrong_key` 는 나오지 않음.
    repo_state: Option<RepoState>,
    /// 자동 백업.
    enabled: bool,
    /// 매일 실행 시각 `HH:MM`. 인스턴스 시간대.
    time: String,
    retention: Retention,
    remote: Option<RemoteStatus>,
    /// 복구 키 저장됨(백업 설정 완료).
    key_set: bool,
    /// 끝난 실행 중 최신.
    last_run: Option<RunView>,
    /// 다음 자동 백업 예정. 꺼져 있으면 null. 지난 시각이면 곧 실행.
    next_run: Option<DateTime<Utc>>,
    /// 최근 실행. 최신 순. 진행 중이면 첫 항목이 `running`.
    runs: Vec<RunView>,
}

#[derive(Serialize, ToSchema)]
pub struct RemoteStatus {
    /// 저장소 주소. 주소 안 비밀번호는 `***`.
    repository: String,
    /// 인증값 환경 변수 저장됨.
    configured: bool,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct RunView {
    id: Uuid,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    trigger: Trigger,
    status: RunStatus,
    /// 진행 중 단계. 끝났으면 null.
    phase: Option<Phase>,
    local: Option<RepoReport>,
    remote: Option<RepoReport>,
    /// 실패 사유. 비밀값 제거 후.
    error: Option<String>,
}

impl RunView {
    /// `live` 가 아니면 `running` 행은 중단된 실행(크래시). 다음 실행 시작 때 기록될 값으로 표시.
    fn new(run: queries::BackupRun, live: bool) -> Self {
        let interrupted = run.status == RunStatus::Running && !live;
        let report = |v: Option<serde_json::Value>| {
            v.and_then(|v| serde_json::from_value::<RepoReport>(v).ok())
        };
        Self {
            id: run.id,
            started_at: run.started_at,
            finished_at: run.finished_at,
            trigger: run.trigger,
            status: if interrupted {
                RunStatus::Failed
            } else {
                run.status
            },
            phase: if interrupted { None } else { run.phase },
            local: report(run.local),
            remote: report(run.remote),
            error: if interrupted {
                Some("interrupted".to_owned())
            } else {
                run.error
            },
        }
    }
}

/// 생략 필드는 유지.
#[derive(Deserialize, ToSchema)]
pub struct UpdateBackupRequest {
    /// 켜기는 복구 키 저장과 백업 폴더 연결 필요.
    enabled: Option<bool>,
    /// `HH:MM`, 인스턴스 시간대.
    time: Option<String>,
    /// 일 0~365, 주 0~520, 월 0~240. 하나 이상 0 초과.
    retention: Option<Retention>,
}

#[derive(Serialize, ToSchema)]
pub struct KeyResponse {
    key: String,
}

#[derive(Deserialize, ToSchema)]
pub struct EnableRequest {
    /// 빈 폴더면 새 키(`POST /api/backup/key`), 기존 저장소면 그 저장소의 키.
    key: String,
    /// 빈 폴더일 때 필수. 복구 키를 따로 적어 둠 확인.
    #[serde(default)]
    confirmed: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct RevealKeyRequest {
    /// 현재 소유자 비밀번호.
    password: String,
}

#[derive(Deserialize, ToSchema)]
pub struct RemoteRequest {
    /// `s3:`, `b2:`, `rest:`, `azure:`, `gs:` 로 시작하는 restic 저장소 주소.
    repository: String,
    /// 인증값 환경 변수. 접두어별 허용 이름만.
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Serialize, ToSchema)]
pub struct RemoteTestResult {
    /// 이 설정으로 저장 가능(비어 있거나 같은 키로 열림).
    ok: bool,
    /// 접속 실패면 null.
    state: Option<RepoState>,
    /// 접속 실패 사유(restic 출력, 비밀값 제거 후).
    message: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct RunStarted {
    run_id: Uuid,
}

/// 백업 폴더 연결 확인. 폴더가 있고 파일을 만들 수 있으면 true. 시험 파일 이름은 `.` 시작(빈 폴더 판정에서 무시).
pub(super) async fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".chagokmoa-write-{}", Uuid::now_v7()));
    let created = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .await;
    match created {
        Ok(file) => {
            drop(file);
            if let Err(e) = tokio::fs::remove_file(&probe).await {
                tracing::warn!(error = %e, "backup dir probe cleanup failed");
            }
            true
        }
        Err(e) => {
            tracing::debug!(error = %e, dir = %dir.display(), "backup dir not writable");
            false
        }
    }
}

async fn ensure_mounted(state: &AppState) -> AppResult<()> {
    if writable(&state.config.backup_dir).await {
        Ok(())
    } else {
        Err(AppError::conflict("backup.notMounted"))
    }
}

async fn status(state: &AppState) -> AppResult<BackupStatus> {
    let s = settings::load(&state.pool).await?;
    let dir = &state.config.backup_dir;
    let mounted = writable(dir).await;
    let repo_state = if mounted {
        match restic::local_state(dir).await {
            Ok(st) => Some(st),
            Err(e) => {
                tracing::warn!(error = ?e, "backup dir unreadable");
                None
            }
        }
    } else {
        None
    };
    let live = queries::lock_held(&state.pool).await?;
    let runs: Vec<RunView> = queries::recent(&state.pool, RUNS_SHOWN)
        .await?
        .into_iter()
        .map(|r| RunView::new(r, live))
        .collect();
    let last_run = runs
        .iter()
        .find(|r| r.status != RunStatus::Running)
        .cloned();
    let next_run = if s.enabled && s.password.is_some() {
        let timezone = instance::load(&state.pool).await?.timezone;
        Some(queries::next_run(&state.pool, Utc::now(), &timezone, s.time).await?)
    } else {
        None
    };
    Ok(BackupStatus {
        mounted,
        repo_state,
        enabled: s.enabled,
        time: s.time.format("%H:%M").to_string(),
        retention: s.retention,
        remote: s.remote.as_ref().map(|r| RemoteStatus {
            repository: mask_location(&r.repository),
            configured: !r.env.is_empty(),
        }),
        key_set: s.password.is_some(),
        last_run,
        next_run,
        runs,
    })
}

/// 새 복구 키. 저장하지 않음.
pub fn generate_key() -> String {
    let mut rng = rand::rng();
    let groups: Vec<String> = (0..KEY_GROUPS)
        .map(|_| {
            (0..KEY_GROUP_LEN)
                .map(|_| char::from(KEY_ALPHABET[rng.random_range(0..KEY_ALPHABET.len())]))
                .collect()
        })
        .collect();
    groups.join("-")
}

/// 환경 변수로 넘길 수 있는 값. 비어 있지 않고 제어 문자(NUL 포함) 없음.
fn plain(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

/// 원격 주소와 인증값 검증. 주소와 값은 앞뒤 공백 제거.
pub fn validate_remote(repository: &str, env: &BTreeMap<String, String>) -> AppResult<Remote> {
    let repository = repository.trim();
    let Some((prefix, allowed)) = REMOTE_BACKENDS
        .iter()
        .find(|(prefix, _)| repository.starts_with(prefix))
    else {
        return Err(AppError::bad_request("backup.remoteUnsupported"));
    };
    if repository.len() == prefix.len() || !plain(repository, REPOSITORY_MAX) {
        return Err(AppError::bad_request("backup.remoteInvalid"));
    }
    let mut out = BTreeMap::new();
    for (name, value) in env {
        let value = value.trim();
        if !allowed.contains(&name.as_str()) || !plain(value, ENV_VALUE_MAX) {
            return Err(AppError::BadRequest {
                key: "backup.remoteEnvInvalid",
                params: json!({ "name": name }),
            });
        }
        out.insert(name.clone(), value.to_owned());
    }
    Ok(Remote {
        repository: repository.to_owned(),
        env: out,
    })
}

/// 원격 주소 접두어의 허용 인증값 이름. 지원하지 않는 주소면 None.
pub fn remote_env_names(repository: &str) -> Option<&'static [&'static str]> {
    REMOTE_BACKENDS
        .iter()
        .find(|(prefix, _)| repository.trim().starts_with(prefix))
        .map(|(_, names)| *names)
}

/// 복구 키가 있는 설정과 그 원격 저장소. 키 없으면 409.
async fn remote_with_key(state: &AppState, req: &RemoteRequest) -> AppResult<(Remote, Repo)> {
    let remote = validate_remote(&req.repository, &req.env)?;
    let s = settings::load(&state.pool).await?;
    if s.password.is_none() {
        return Err(AppError::conflict("backup.notConfigured"));
    }
    let repo = BackupSettings {
        remote: Some(remote.clone()),
        ..s
    }
    .remote_repo(&state.config)
    .ok_or(AppError::conflict("backup.notConfigured"))?;
    Ok((remote, repo))
}

/// 접속 실패 사유. 비밀값 제거 후.
fn failure_message(repo: &Repo, err: &AppError) -> String {
    repo.redact(&run::describe(err))
}

#[utoipa::path(
    get,
    path = "/api/backup",
    responses(
        (status = 200, description = "백업 설정과 상태. 비밀값 없음", body = BackupStatus),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님", body = ErrorBody),
    )
)]
async fn get_status(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<AxumJson<BackupStatus>> {
    current.require(Role::Owner)?;
    Ok(AxumJson(status(&state).await?))
}

#[utoipa::path(
    put,
    path = "/api/backup",
    request_body = UpdateBackupRequest,
    responses(
        (status = 200, description = "갱신 후 상태", body = BackupStatus),
        (status = 400, description = "본문 형식 오류, `backup.timeInvalid`, `backup.retentionInvalid`", body = ErrorBody),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
        (status = 409, description = "켜기 불가. `backup.notConfigured`, `backup.notMounted`", body = ErrorBody),
    )
)]
async fn update_settings(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(req): Json<UpdateBackupRequest>,
) -> AppResult<AxumJson<BackupStatus>> {
    current.require(Role::Owner)?;
    // 전 필드 검증 후 기록.
    let time = match &req.time {
        Some(raw) => Some(parse_time(raw)?),
        None => None,
    };
    if let Some(r) = req.retention {
        validate_retention(r)?;
    }
    if req.enabled == Some(true) {
        ensure_mounted(&state).await?;
    }

    let mut tx = state.pool.begin().await?;
    let mut s = settings::load(&mut *tx).await?;
    if req.enabled == Some(true) && s.password.is_none() {
        return Err(AppError::conflict("backup.notConfigured"));
    }
    if let Some(enabled) = req.enabled {
        s.enabled = enabled;
    }
    if let Some(time) = time {
        s.time = time;
    }
    if let Some(retention) = req.retention {
        s.retention = retention;
    }
    settings::save(&mut *tx, &s).await?;
    tx.commit().await?;
    Ok(AxumJson(status(&state).await?))
}

/// 정확히 `HH:MM`.
fn parse_time(raw: &str) -> AppResult<NaiveTime> {
    let digits = raw.bytes().enumerate().all(|(i, b)| {
        if i == 2 {
            b == b':'
        } else {
            b.is_ascii_digit()
        }
    });
    if raw.len() != 5 || !digits {
        return Err(AppError::bad_request("backup.timeInvalid"));
    }
    NaiveTime::parse_from_str(raw, "%H:%M").map_err(|_| AppError::bad_request("backup.timeInvalid"))
}

fn validate_retention(r: Retention) -> AppResult<()> {
    let in_range = r.daily <= DAILY_MAX && r.weekly <= WEEKLY_MAX && r.monthly <= MONTHLY_MAX;
    // 모두 0 이면 아무것도 안 지워져 저장소가 끝없이 커짐.
    let any = r.daily > 0 || r.weekly > 0 || r.monthly > 0;
    if in_range && any {
        Ok(())
    } else {
        Err(AppError::bad_request("backup.retentionInvalid"))
    }
}

#[utoipa::path(
    post,
    path = "/api/backup/key",
    responses(
        (status = 200, description = "새 복구 키. 저장 안 함(`POST /api/backup/enable` 에서 저장)", body = KeyResponse),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
        (status = 409, description = "`backup.alreadyConfigured`, `backup.notMounted`, `backup.repoNotEmpty`", body = ErrorBody),
    )
)]
async fn new_key(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<AxumJson<KeyResponse>> {
    current.require(Role::Owner)?;
    if settings::load(&state.pool).await?.password.is_some() {
        return Err(AppError::conflict("backup.alreadyConfigured"));
    }
    ensure_mounted(&state).await?;
    if restic::local_state(&state.config.backup_dir).await? != RepoState::Empty {
        return Err(AppError::conflict("backup.repoNotEmpty"));
    }
    Ok(AxumJson(KeyResponse {
        key: generate_key(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/backup/enable",
    request_body = EnableRequest,
    responses(
        (status = 200, description = "키 저장, 자동 백업 켬. 빈 폴더면 새 저장소 생성", body = BackupStatus),
        (status = 400, description = "본문 형식 오류, `backup.keyInvalid`, `backup.keyNotConfirmed`", body = ErrorBody),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
        (status = 409, description = "`backup.alreadyConfigured`, `backup.notMounted`, `backup.wrongKey`, `backup.foreignRepo`, `backup.running`", body = ErrorBody),
    )
)]
async fn enable(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(req): Json<EnableRequest>,
) -> AppResult<AxumJson<BackupStatus>> {
    current.require(Role::Owner)?;
    if settings::load(&state.pool).await?.password.is_some() {
        return Err(AppError::conflict("backup.alreadyConfigured"));
    }
    ensure_mounted(&state).await?;
    if !plain(&req.key, KEY_MAX) {
        return Err(AppError::bad_request("backup.keyInvalid"));
    }
    let candidate = BackupSettings {
        password: Some(req.key.clone()),
        ..BackupSettings::default()
    };
    let repo = candidate
        .local_repo(&state.config)
        .ok_or(AppError::bad_request("backup.keyInvalid"))?;
    // 확인부터 저장까지 잠금. 동시 init 두 번이면 키가 둘인 저장소가 생겨 열리지 않을 수 있음.
    let _lock = run::try_lock(&state.pool).await?;
    match repo.probe().await? {
        RepoState::Empty => {
            if req.key.chars().count() < NEW_KEY_MIN {
                return Err(AppError::bad_request("backup.keyInvalid"));
            }
            if !req.confirmed {
                return Err(AppError::bad_request("backup.keyNotConfirmed"));
            }
            repo.init().await?;
        }
        RepoState::Restic => {}
        RepoState::WrongKey => return Err(AppError::conflict("backup.wrongKey")),
        RepoState::Foreign => return Err(AppError::conflict("backup.foreignRepo")),
    }

    // restic 실행 동안 바뀌었을 수 있어 다시 읽고 저장.
    let mut tx = state.pool.begin().await?;
    let mut s = settings::load(&mut *tx).await?;
    if s.password.is_some() {
        return Err(AppError::conflict("backup.alreadyConfigured"));
    }
    s.password = Some(req.key);
    s.enabled = true;
    settings::save(&mut *tx, &s).await?;
    tx.commit().await?;
    Ok(AxumJson(status(&state).await?))
}

#[utoipa::path(
    post,
    path = "/api/backup/key/reveal",
    request_body = RevealKeyRequest,
    responses(
        (status = 200, description = "저장된 복구 키", body = KeyResponse),
        (status = 400, description = "본문 형식 오류", body = ErrorBody),
        (status = 401, description = "인증 필요 또는 비밀번호 불일치(`auth.invalidCredentials`)", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
        (status = 409, description = "`backup.notConfigured`", body = ErrorBody),
    )
)]
async fn reveal_key(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(req): Json<RevealKeyRequest>,
) -> AppResult<AxumJson<KeyResponse>> {
    current.require(Role::Owner)?;
    let stored = users::queries::password_hash(&state.pool, current.user.id)
        .await?
        .ok_or(AppError::Unauthorized("auth.required"))?;
    if !password::verify_async(req.password, stored).await? {
        return Err(AppError::Unauthorized("auth.invalidCredentials"));
    }
    let key = settings::load(&state.pool)
        .await?
        .password
        .ok_or(AppError::conflict("backup.notConfigured"))?;
    Ok(AxumJson(KeyResponse { key }))
}

#[utoipa::path(
    post,
    path = "/api/backup/remote/test",
    request_body = RemoteRequest,
    responses(
        (status = 200, description = "연결 시험 결과. 저장과 초기화 안 함", body = RemoteTestResult),
        (status = 400, description = "본문 형식 오류, `backup.remoteUnsupported`, `backup.remoteInvalid`, `backup.remoteEnvInvalid`(`name`)", body = ErrorBody),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
        (status = 409, description = "`backup.notConfigured`", body = ErrorBody),
    )
)]
async fn test_remote(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(req): Json<RemoteRequest>,
) -> AppResult<AxumJson<RemoteTestResult>> {
    current.require(Role::Owner)?;
    let (_, repo) = remote_with_key(&state, &req).await?;
    let result = match repo.probe().await {
        Ok(st) => RemoteTestResult {
            ok: matches!(st, RepoState::Empty | RepoState::Restic),
            state: Some(st),
            message: None,
        },
        Err(e) => RemoteTestResult {
            ok: false,
            state: None,
            message: Some(failure_message(&repo, &e)),
        },
    };
    Ok(AxumJson(result))
}

#[utoipa::path(
    put,
    path = "/api/backup/remote",
    request_body = RemoteRequest,
    responses(
        (status = 200, description = "시험 통과 후 저장. 비어 있으면 같은 키로 초기화", body = BackupStatus),
        (status = 400, description = "본문 형식 오류, `backup.remoteUnsupported`, `backup.remoteInvalid`, `backup.remoteEnvInvalid`(`name`)", body = ErrorBody),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
        (status = 409, description = "`backup.notConfigured`, `backup.wrongKey`, `backup.remoteFailed`(`message`), `backup.running`", body = ErrorBody),
    )
)]
async fn save_remote(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(req): Json<RemoteRequest>,
) -> AppResult<AxumJson<BackupStatus>> {
    current.require(Role::Owner)?;
    let (remote, repo) = remote_with_key(&state, &req).await?;
    // 확인부터 저장까지 잠금(동시 init 방지, 진행 중 백업과 겹침 방지).
    let _lock = run::try_lock(&state.pool).await?;
    let ready = async {
        match repo.probe().await? {
            RepoState::Empty => repo.init().await.map(|()| true),
            RepoState::Restic => Ok(true),
            RepoState::WrongKey | RepoState::Foreign => Ok(false),
        }
    }
    .await;
    match ready {
        Ok(true) => {}
        Ok(false) => return Err(AppError::conflict("backup.wrongKey")),
        Err(e) => {
            return Err(AppError::Conflict {
                key: "backup.remoteFailed",
                params: json!({ "message": failure_message(&repo, &e) }),
            });
        }
    }

    let mut tx = state.pool.begin().await?;
    let mut s = settings::load(&mut *tx).await?;
    s.remote = Some(remote);
    settings::save(&mut *tx, &s).await?;
    tx.commit().await?;
    Ok(AxumJson(status(&state).await?))
}

#[utoipa::path(
    delete,
    path = "/api/backup/remote",
    responses(
        (status = 204, description = "원격 설정 삭제. 원격 저장소 내용은 그대로"),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
    )
)]
async fn delete_remote(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<StatusCode> {
    current.require(Role::Owner)?;
    let mut tx = state.pool.begin().await?;
    let mut s = settings::load(&mut *tx).await?;
    if s.remote.take().is_some() {
        settings::save(&mut *tx, &s).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/backup/run",
    responses(
        (status = 202, description = "지금 백업 시작. 진행과 결과는 `GET /api/backup` 의 `runs`", body = RunStarted),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
        (status = 409, description = "`backup.notConfigured`, `backup.running`, `backup.unsupportedPostgres`(`major`, `required`)", body = ErrorBody),
    )
)]
async fn run_now(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<(StatusCode, AxumJson<RunStarted>)> {
    current.require(Role::Owner)?;
    let started = run::start(&state, Trigger::Manual).await?;
    let run_id = started.id();
    // 서버 종료 시 런타임 종료와 함께 취소(restic 종료, 덤프 삭제, 잠금 해제). 기록은 다음 실행 때 interrupted.
    tokio::spawn(async move {
        if let Err(e) = started.finish(&state).await {
            tracing::error!(error = ?e, %run_id, "manual backup record failed");
        }
    });
    Ok((StatusCode::ACCEPTED, AxumJson(RunStarted { run_id })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_shape() {
        let key = generate_key();
        assert_eq!(key.len(), KEY_GROUPS * KEY_GROUP_LEN + KEY_GROUPS - 1);
        assert!(key.len() >= NEW_KEY_MIN);
        for group in key.split('-') {
            assert_eq!(group.len(), KEY_GROUP_LEN);
            assert!(group.bytes().all(|b| KEY_ALPHABET.contains(&b)), "{key}");
        }
        assert_ne!(generate_key(), key);
    }

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn key_of(r: AppResult<Remote>) -> &'static str {
        match r {
            Err(AppError::BadRequest { key, .. }) => key,
            other => panic!("{:?}", other.map(|r| r.repository)),
        }
    }

    #[test]
    fn remote_allow_list() {
        for (repository, pairs) in [
            (
                "s3:https://s3.example/bucket",
                vec![
                    ("AWS_ACCESS_KEY_ID", "id"),
                    ("AWS_SECRET_ACCESS_KEY", "secret"),
                    ("AWS_DEFAULT_REGION", "us-east-1"),
                    ("AWS_SESSION_TOKEN", "token"),
                ],
            ),
            (
                "b2:bucket:path",
                vec![("B2_ACCOUNT_ID", "id"), ("B2_ACCOUNT_KEY", "key")],
            ),
            ("rest:https://u:p@host/repo/", vec![]),
            (
                "azure:container:/path",
                vec![
                    ("AZURE_ACCOUNT_NAME", "n"),
                    ("AZURE_ACCOUNT_KEY", "k"),
                    ("AZURE_ACCOUNT_SAS", "s"),
                    ("AZURE_ENDPOINT_SUFFIX", "core.windows.net"),
                ],
            ),
            (
                "gs:bucket:/path",
                vec![
                    ("GOOGLE_PROJECT_ID", "p"),
                    ("GOOGLE_APPLICATION_CREDENTIALS", "/run/secrets/gcs.json"),
                ],
            ),
        ] {
            let remote = validate_remote(&format!("  {repository}\n"), &env(&pairs)).unwrap();
            assert_eq!(remote.repository, repository);
            assert_eq!(remote.env.len(), pairs.len());
        }
        // 값 앞뒤 공백 제거
        let remote = validate_remote("b2:bucket", &env(&[("B2_ACCOUNT_KEY", " key\n")])).unwrap();
        assert_eq!(remote.env["B2_ACCOUNT_KEY"], "key");

        for repository in [
            "sftp:host:/srv",
            "rclone:remote:path",
            "swift:c:/p",
            "local:/srv",
            "/srv/restic",
            "S3:https://host/b",
            "",
        ] {
            assert_eq!(
                key_of(validate_remote(repository, &BTreeMap::new())),
                "backup.remoteUnsupported",
                "{repository}"
            );
        }
        for repository in ["s3:", "rest:", "s3:https://host/b\nx", "b2:a\0b"] {
            assert_eq!(
                key_of(validate_remote(repository, &BTreeMap::new())),
                "backup.remoteInvalid",
                "{repository:?}"
            );
        }
        let too_long = format!("s3:{}", "a".repeat(REPOSITORY_MAX));
        assert_eq!(
            key_of(validate_remote(&too_long, &BTreeMap::new())),
            "backup.remoteInvalid"
        );

        for (repository, name, value) in [
            ("s3:https://host/b", "LD_PRELOAD", "/tmp/x.so"),
            ("s3:https://host/b", "PATH", "/tmp"),
            ("s3:https://host/b", "RESTIC_PASSWORD_COMMAND", "sh"),
            ("s3:https://host/b", "B2_ACCOUNT_ID", "id"),
            ("rest:https://host/", "AWS_ACCESS_KEY_ID", "id"),
            ("gs:b:/", "GOOGLE_ACCESS_TOKEN", "t"),
            ("s3:https://host/b", "AWS_ACCESS_KEY_ID", ""),
            ("s3:https://host/b", "AWS_ACCESS_KEY_ID", "  "),
            ("s3:https://host/b", "AWS_ACCESS_KEY_ID", "a\0b"),
        ] {
            assert_eq!(
                key_of(validate_remote(repository, &env(&[(name, value)]))),
                "backup.remoteEnvInvalid",
                "{name}={value:?}"
            );
        }
    }

    #[test]
    fn time_and_retention() {
        assert_eq!(
            parse_time("03:00").unwrap(),
            NaiveTime::from_hms_opt(3, 0, 0).unwrap()
        );
        assert_eq!(
            parse_time("23:59").unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap()
        );
        for bad in ["3:00", "24:00", "03:60", "0300", "03:00:00", " 3:00", ""] {
            assert!(parse_time(bad).is_err(), "{bad}");
        }
        let r = |daily, weekly, monthly| Retention {
            daily,
            weekly,
            monthly,
        };
        assert!(validate_retention(r(7, 4, 12)).is_ok());
        assert!(validate_retention(r(0, 0, 1)).is_ok());
        assert!(validate_retention(r(365, 520, 240)).is_ok());
        for bad in [r(0, 0, 0), r(366, 0, 0), r(0, 521, 0), r(0, 0, 241)] {
            assert!(validate_retention(bad).is_err(), "{bad:?}");
        }
    }
}
