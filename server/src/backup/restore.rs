//! 복구 CLI. 새 서버(앱 중지)에서 restic 스냅숏의 원본과 DB 덤프를 되살리고 파생물 재생성 작업을 큐에.
//! 순서: 서버 버전 → 다른 DB 연결(실행 중인 앱) 확인 → 이전 임시 폴더 정리 → DB, 원본 폴더 확인 → 저장소 열기
//! → 스냅숏 선택 → 임시 폴더로 복원 → DB 적용(덤프 시점 running 기록 삭제) → 원본 이동 → 재생성 큐.

use std::{
    collections::BTreeMap,
    io::{self, BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
    process::Stdio,
    str::FromStr,
    time::{Duration, Instant},
};

use anyhow::Context;
use futures_util::future::BoxFuture;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use uuid::Uuid;

use super::{
    dump, mask_location, queries,
    restic::{self, Repo, RepoState, Snapshot},
    routes,
    run::{self, DbSteps, ServerMajorFn},
    settings,
};
use crate::{error::AppResult, media::regen};

/// 복구 CLI 의 DB 연결 이름. 다른 연결(실행 중인 앱) 판정에서 제외.
pub const APPLICATION_NAME: &str = "chagokmoa-restore";
/// 이전 실행이 남긴 임시 폴더 이름 접두어(`DATA_DIR/tmp/`).
const SCRATCH_PREFIX: &str = "restore-";

/// 덤프 적용. (풀, DATABASE_URL, 덤프 파일).
pub type ApplyFn = for<'a> fn(&'a PgPool, &'a str, &'a Path) -> BoxFuture<'a, AppResult<()>>;

/// DB 단계. 테스트에서 교체(로컬 pg_restore 16 으로 17 덤프 적용 불가).
#[derive(Clone, Copy)]
pub struct Steps {
    pub server_major: ServerMajorFn,
    pub apply: ApplyFn,
}

fn real_apply<'a>(
    pool: &'a PgPool,
    database_url: &'a str,
    dump: &'a Path,
) -> BoxFuture<'a, AppResult<()>> {
    Box::pin(apply(pool, database_url, dump))
}

impl Default for Steps {
    fn default() -> Self {
        Self {
            server_major: DbSteps::default().server_major,
            apply: real_apply,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub database_url: String,
    pub data_dir: PathBuf,
    /// 로컬 restic 저장소(`BACKUP_DIR`). `repository` 없을 때 사용.
    pub backup_dir: PathBuf,
    /// 로컬 절대 경로 또는 원격 주소(`s3:`, `b2:`, `rest:`, `azure:`, `gs:`).
    pub repository: Option<String>,
    /// 스냅숏 id 또는 앞부분. 없으면 최신.
    pub snapshot: Option<String>,
    /// 비어 있지 않은 DB 와 원본 폴더 덮어쓰기(재확인 필요).
    pub force: bool,
}

/// 복구 키와 재확인 입력.
pub trait Prompt {
    fn key(&mut self) -> anyhow::Result<String>;
    /// `yes` 입력이면 true.
    fn confirm(&mut self, message: &str) -> anyhow::Result<bool>;
}

#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error(
        "database is not empty ({users} users, {assets} photos and videos); rerun with --force to replace it"
    )]
    DbNotEmpty { users: i64, assets: i64 },
    #[error(
        "database has {tables} tables that are not from Chagokmoa; rerun with --force to replace them"
    )]
    DbUnknown { tables: i64 },
    #[error("DATA_DIR/originals already has files; rerun with --force to overwrite them")]
    OriginalsNotEmpty,
    #[error(
        "{sessions} other connections to the database; stop the app first (docker compose stop app) and rerun"
    )]
    DatabaseInUse { sessions: i64 },
    #[error("restore cancelled")]
    Aborted,
    #[error(
        "unsupported repository; use an absolute path or an s3:, b2:, rest:, azure: or gs: address"
    )]
    UnsupportedRepository,
    #[error("no backup repository at {0}")]
    NoRepository(String),
    #[error("{0} is not a restic repository")]
    ForeignRepository(String),
    #[error("the recovery key does not open the repository")]
    WrongKey,
    #[error("no Chagokmoa snapshots in the repository")]
    NoSnapshots,
    #[error("snapshot {0} not found")]
    SnapshotNotFound(String),
    #[error("snapshot id {0} matches more than one snapshot")]
    SnapshotAmbiguous(String),
    #[error("snapshot {0} has no database dump or originals")]
    SnapshotIncomplete(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbState {
    /// 테이블 없음, 또는 사용자 0명이고 사진 0건(마이그레이션만 된 새 DB).
    Empty,
    InUse {
        users: i64,
        assets: i64,
    },
    /// 테이블은 있지만 users, assets 가 없음.
    Unknown {
        tables: i64,
    },
}

#[derive(Debug)]
pub struct Summary {
    pub snapshot: Snapshot,
    /// 옮긴 원본 파일 수.
    pub originals: u64,
    /// 큐에 넣은 재생성 작업 수.
    pub queued: u64,
}

/// 복구용 연결 설정. 연결 이름으로 자기 연결 구분.
pub fn connect_options(base: PgConnectOptions) -> PgConnectOptions {
    base.application_name(APPLICATION_NAME)
}

pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    let options = PgConnectOptions::from_str(database_url).context("DATABASE_URL invalid")?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(connect_options(options))
        .await?;
    Ok(pool)
}

/// 같은 DB 의 다른 클라이언트 연결 수(복구 CLI 자신 제외). 0 이 아니면 앱 실행 중으로 판단.
pub async fn other_sessions(pool: &PgPool) -> AppResult<i64> {
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM pg_catalog.pg_stat_activity
           WHERE datname = current_database()
             AND backend_type = 'client backend'
             AND pid <> pg_backend_pid()
             AND application_name IS DISTINCT FROM $1"#,
        APPLICATION_NAME
    )
    .fetch_one(pool)
    .await?;
    Ok(count)
}

pub async fn db_state(pool: &PgPool) -> AppResult<DbState> {
    let tables = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM pg_catalog.pg_tables WHERE schemaname = 'public'"#
    )
    .fetch_one(pool)
    .await?;
    if tables == 0 {
        return Ok(DbState::Empty);
    }
    let ours = sqlx::query_scalar!(
        r#"SELECT to_regclass('public.users') IS NOT NULL
                  AND to_regclass('public.assets') IS NOT NULL AS "ours!""#
    )
    .fetch_one(pool)
    .await?;
    if !ours {
        return Ok(DbState::Unknown { tables });
    }
    let row = sqlx::query!(
        r#"SELECT (SELECT count(*) FROM users) AS "users!",
                  (SELECT count(*) FROM assets) AS "assets!""#
    )
    .fetch_one(pool)
    .await?;
    Ok(match (row.users, row.assets) {
        (0, 0) => DbState::Empty,
        (users, assets) => DbState::InUse { users, assets },
    })
}

/// 빈 스키마에 덤프 적용. 마이그레이션만 된 DB 와 `--force` 대상 모두 기존 객체 제거 후.
/// pg_restore 는 한 트랜잭션, 첫 오류에서 중단(반쯤 복구된 DB 방지). 실패 시 DB 는 테이블 없는 상태라 재실행 가능.
pub async fn apply(pool: &PgPool, database_url: &str, dump_file: &Path) -> AppResult<()> {
    sqlx::query!("DROP SCHEMA IF EXISTS public CASCADE")
        .execute(pool)
        .await?;
    sqlx::query!("CREATE SCHEMA public").execute(pool).await?;
    dump::restore_from_file(database_url, dump_file).await
}

pub async fn run(
    pool: &PgPool,
    opts: &Options,
    steps: Steps,
    prompt: &mut impl Prompt,
) -> anyhow::Result<Summary> {
    let major = (steps.server_major)(pool).await?;
    run::ensure_supported(major)?;
    // 앱이 떠 있으면 스키마 삭제와 원본 이동이 실행 중인 서버(업로드, 처리 작업) 아래에서 일어남.
    let sessions = other_sessions(pool).await?;
    if sessions > 0 {
        return Err(RestoreError::DatabaseInUse { sessions }.into());
    }
    // 중단된 이전 복구의 임시 폴더. 라이브러리 전체 사본과 비밀값 든 덤프라 남기지 않음.
    let removed = remove_leftovers(&opts.data_dir)
        .await
        .context("previous restore temp dirs cleanup failed")?;
    if removed > 0 {
        println!("removed {removed} temp dirs left by an interrupted restore");
    }

    let state = db_state(pool).await?;
    let originals_dir = opts.data_dir.join("originals");
    let files_present = has_files(&originals_dir)
        .with_context(|| format!("{} unreadable", originals_dir.display()))?;
    match state {
        DbState::Empty => {}
        DbState::InUse { users, assets } => {
            println!("database has {users} users and {assets} photos and videos");
        }
        DbState::Unknown { tables } => {
            println!("database has {tables} tables that are not from Chagokmoa");
        }
    }
    if files_present {
        println!("{} already has files", originals_dir.display());
    }
    if state != DbState::Empty || files_present {
        if !opts.force {
            return Err(match state {
                DbState::InUse { users, assets } => RestoreError::DbNotEmpty { users, assets },
                DbState::Unknown { tables } => RestoreError::DbUnknown { tables },
                DbState::Empty => RestoreError::OriginalsNotEmpty,
            }
            .into());
        }
        if !prompt.confirm("This replaces the database and overwrites existing originals.")? {
            return Err(RestoreError::Aborted.into());
        }
    }

    let (repo, snapshots) = open(opts, prompt).await?;
    let snapshot = pick(snapshots, opts.snapshot.as_deref())?;
    let (dump_src, originals_src) = layout(&snapshot)
        .ok_or_else(|| RestoreError::SnapshotIncomplete(snapshot.short_id.clone()))?;
    println!(
        "restoring snapshot {} from {} (see --list for others)",
        snapshot.short_id,
        snapshot.time.to_rfc3339()
    );

    let scratch = Scratch::create(&opts.data_dir).context("restore temp dir create failed")?;
    repo.restore(&snapshot.id, &scratch.0).await?;
    let dump_file = scratch.0.join(dump_src);
    let originals = scratch.0.join(originals_src);
    if !dump_file.is_file() || !originals.is_dir() {
        return Err(RestoreError::SnapshotIncomplete(snapshot.short_id.clone()).into());
    }

    (steps.apply)(pool, &opts.database_url, &dump_file).await?;
    // 남기면 상태 화면에 방금 복구한 백업이 중단(실패)으로 표시
    queries::delete_running(pool).await.context(
        "database restored, but clearing the in-progress backup record failed; \
         rerun with --force once the cause is fixed",
    )?;
    println!("database restored");
    let moved = {
        let originals_dir = originals_dir.clone();
        tokio::task::spawn_blocking(move || move_tree(&originals, &originals_dir))
            .await
            .context("originals move panicked")?
            .context(
                "originals move failed after the database was restored; \
                 rerun with --force once the cause is fixed",
            )?
    };
    println!("restored {moved} originals to {}", originals_dir.display());
    drop(scratch);

    let started = Instant::now();
    let queued = regen::enqueue(pool, &opts.data_dir, true).await.context(
        "database and originals restored, but queueing derivative regeneration failed; \
             start the app and run regenerate-derivatives --missing",
    )?;
    println!(
        "queued {queued} derivative regenerations ({} ms)",
        started.elapsed().as_millis()
    );
    Ok(Summary {
        snapshot,
        originals: moved,
        queued,
    })
}

/// 스냅숏 목록만 출력. DB 와 파일은 건드리지 않음(앱 실행 중에도 가능).
pub async fn list(opts: &Options, prompt: &mut impl Prompt) -> anyhow::Result<Vec<Snapshot>> {
    let (repo, snapshots) = open(opts, prompt).await?;
    println!("snapshots in {}:", mask_location(&repo.location));
    println!(
        "  {:<8}  {:<23}  {:>8}  {:>8}  TAGS",
        "ID", "TIME", "FILES", "SIZE"
    );
    for s in &snapshots {
        let (files, size) = s.summary.as_ref().map_or_else(
            || ("-".to_owned(), "-".to_owned()),
            |m| {
                (
                    m.total_files_processed.to_string(),
                    human_size(m.total_bytes_processed),
                )
            },
        );
        println!(
            "  {}  {}  {files:>8}  {size:>8}  {}",
            s.short_id,
            s.time.format("%Y-%m-%d %H:%M:%S UTC"),
            s.tags.join(",")
        );
    }
    if snapshots.is_empty() {
        println!("  (none)");
    }
    Ok(snapshots)
}

/// 저장소 열기와 앱 스냅숏(고정 호스트) 목록. 오래된 순.
async fn open(opts: &Options, prompt: &mut impl Prompt) -> anyhow::Result<(Repo, Vec<Snapshot>)> {
    let repo = repository(opts, prompt.key()?)?;
    let location = mask_location(&repo.location);
    match repo.probe().await? {
        RepoState::Restic => {}
        RepoState::Empty => return Err(RestoreError::NoRepository(location).into()),
        RepoState::Foreign => return Err(RestoreError::ForeignRepository(location).into()),
        RepoState::WrongKey => return Err(RestoreError::WrongKey.into()),
    }
    let snapshots: Vec<Snapshot> = repo
        .snapshots()
        .await?
        .into_iter()
        .filter(|s| s.hostname == restic::HOST)
        .collect();
    Ok((repo, snapshots))
}

/// 1024 단위 읽기 쉬운 크기.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// 로컬은 절대 경로, 원격은 허용 접두어. 원격 인증값은 같은 이름의 환경 변수(설정 화면과 같은 허용 목록).
fn repository(opts: &Options, key: String) -> anyhow::Result<Repo> {
    let location = opts.repository.as_deref().map_or_else(
        || opts.backup_dir.to_string_lossy().into_owned(),
        |l| l.trim().to_owned(),
    );
    let mut repo = Repo {
        location,
        env: Vec::new(),
        cache_dir: settings::cache_dir(&opts.data_dir),
    };
    if let Some(names) = routes::remote_env_names(&repo.location) {
        let vars: BTreeMap<String, String> = names
            .iter()
            .filter_map(|name| {
                let value = std::env::var(name).ok()?;
                (!value.trim().is_empty()).then(|| ((*name).to_owned(), value))
            })
            .collect();
        let remote = routes::validate_remote(&repo.location, &vars)?;
        repo.location = remote.repository;
        repo.env.extend(remote.env);
    } else if !Path::new(&repo.location).is_absolute() {
        return Err(RestoreError::UnsupportedRepository.into());
    }
    repo.env.push(("RESTIC_PASSWORD".to_owned(), key));
    Ok(repo)
}

/// 오래된 순 목록에서 선택. 없으면 최신, 있으면 id 앞부분이 하나만 맞아야 함.
fn pick(snapshots: Vec<Snapshot>, wanted: Option<&str>) -> Result<Snapshot, RestoreError> {
    let Some(wanted) = wanted.map(str::trim) else {
        return snapshots
            .into_iter()
            .last()
            .ok_or(RestoreError::NoSnapshots);
    };
    let prefix = wanted.to_ascii_lowercase();
    if prefix.is_empty() || !prefix.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(RestoreError::SnapshotNotFound(wanted.to_owned()));
    }
    let mut found = snapshots.into_iter().filter(|s| s.id.starts_with(&prefix));
    match (found.next(), found.next()) {
        (Some(s), None) => Ok(s),
        (Some(_), Some(_)) => Err(RestoreError::SnapshotAmbiguous(wanted.to_owned())),
        (None, _) => Err(RestoreError::SnapshotNotFound(wanted.to_owned())),
    }
}

/// 스냅숏 안 덤프와 원본 경로(뿌리 `/` 뺀 상대 경로). 백업 실행의 경로 구성(`run::dump_path`, `DATA_DIR/originals`) 기준.
fn layout(snapshot: &Snapshot) -> Option<(PathBuf, PathBuf)> {
    let find = |name: &str| {
        snapshot
            .paths
            .iter()
            .map(Path::new)
            .find(|p| p.file_name().is_some_and(|n| n == name))
            .and_then(|p| p.strip_prefix("/").ok())
            .map(Path::to_path_buf)
    };
    Some((find("db.dump")?, find("originals")?))
}

/// 하위 어디든 디렉터리가 아닌 항목이 있으면 true.
fn has_files(dir: &Path) -> io::Result<bool> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || has_files(&entry.path())? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `src` 아래 파일을 같은 상대 경로로 `dst` 에 이름 변경(같은 파일시스템). 같은 이름은 덮어씀. 반환은 파일 수.
fn move_tree(src: &Path, dst: &Path) -> io::Result<u64> {
    std::fs::create_dir_all(dst)?;
    let mut moved = 0;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            moved += move_tree(&entry.path(), &to)?;
        } else {
            std::fs::rename(entry.path(), &to)?;
            moved += 1;
        }
    }
    Ok(moved)
}

/// `DATA_DIR/tmp/restore-*` 전부 삭제. 반환은 지운 수. 복구는 앱 중지 상태에서 하나만 실행되므로 진행 중인 다른 복구 없음.
async fn remove_leftovers(data_dir: &Path) -> io::Result<usize> {
    let mut entries = match tokio::fs::read_dir(data_dir.join("tmp")).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0;
    while let Some(entry) = entries.next_entry().await? {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(SCRATCH_PREFIX)
        {
            continue;
        }
        if entry.file_type().await?.is_dir() {
            tokio::fs::remove_dir_all(entry.path()).await?;
        } else {
            tokio::fs::remove_file(entry.path()).await?;
        }
        removed += 1;
    }
    Ok(removed)
}

/// `DATA_DIR/tmp/restore-{uuid}`. 원본 이동(rename)과 같은 파일시스템, 덤프에 비밀값이 있어 0700. 드롭 시 삭제.
struct Scratch(PathBuf);

impl Scratch {
    fn create(data_dir: &Path) -> io::Result<Scratch> {
        let parent = data_dir.join("tmp");
        std::fs::create_dir_all(&parent)?;
        let dir = parent.join(format!("{SCRATCH_PREFIX}{}", Uuid::now_v7()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&dir)?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.0)
            && e.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(error = %e, path = %self.0.display(), "restore temp dir cleanup failed");
        }
    }
}

/// 터미널 입력. 복구 키는 `RESTIC_PASSWORD` 가 있으면 그 값, 없으면 입력(터미널이면 화면 표시 없이).
pub struct Terminal;

impl Prompt for Terminal {
    fn key(&mut self) -> anyhow::Result<String> {
        if let Some(key) = std::env::var("RESTIC_PASSWORD")
            .ok()
            .filter(|k| !k.is_empty())
        {
            return Ok(key);
        }
        eprint!("recovery key: ");
        io::stderr().flush()?;
        let hidden = HiddenInput::start();
        let line = read_line();
        if hidden.0 {
            eprintln!();
        }
        drop(hidden);
        let key = line?.trim_end_matches(['\r', '\n']).to_owned();
        anyhow::ensure!(!key.is_empty(), "recovery key required");
        Ok(key)
    }

    fn confirm(&mut self, message: &str) -> anyhow::Result<bool> {
        eprint!("{message} Type yes to continue: ");
        io::stderr().flush()?;
        Ok(read_line()?.trim() == "yes")
    }
}

/// 한 줄. 입력 끝이면 빈 문자열.
fn read_line() -> io::Result<String> {
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line)
}

/// 표준 입력이 터미널이면 `stty -echo`, 드롭 시 되돌림. 새 의존성 없이 화면 표시만 끔.
struct HiddenInput(bool);

impl HiddenInput {
    fn start() -> HiddenInput {
        HiddenInput(io::stdin().is_terminal() && stty("-echo"))
    }
}

impl Drop for HiddenInput {
    fn drop(&mut self) {
        if self.0 {
            stty("echo");
        }
    }
}

fn stty(arg: &str) -> bool {
    std::process::Command::new("stty")
        .arg(arg)
        .stdin(Stdio::inherit())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn snapshot(id: &str, paths: &[&str]) -> Snapshot {
        Snapshot {
            id: id.to_owned(),
            short_id: id[..8].to_owned(),
            time: Utc::now(),
            hostname: restic::HOST.to_owned(),
            paths: paths.iter().map(|p| (*p).to_owned()).collect(),
            tags: Vec::new(),
            summary: None,
        }
    }

    #[test]
    fn picks_by_prefix() {
        let list = || vec![snapshot("aaaa1111bbbb", &[]), snapshot("aaaa2222cccc", &[])];
        assert_eq!(pick(list(), None).unwrap().id, "aaaa2222cccc");
        assert_eq!(pick(list(), Some("AAAA1")).unwrap().id, "aaaa1111bbbb");
        assert!(matches!(
            pick(list(), Some("aaaa")),
            Err(RestoreError::SnapshotAmbiguous(_))
        ));
        for bad in ["", "ffff", "--target", "latest"] {
            assert!(
                matches!(
                    pick(list(), Some(bad)),
                    Err(RestoreError::SnapshotNotFound(_))
                ),
                "{bad}"
            );
        }
        assert!(matches!(
            pick(Vec::new(), None),
            Err(RestoreError::NoSnapshots)
        ));
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }

    #[test]
    fn layout_from_paths() {
        let s = snapshot(
            "aaaa1111bbbb",
            &["/data/cache/backup/db.dump", "/data/originals"],
        );
        assert_eq!(
            layout(&s),
            Some((
                PathBuf::from("data/cache/backup/db.dump"),
                PathBuf::from("data/originals")
            ))
        );
        assert_eq!(
            layout(&snapshot("aaaa1111bbbb", &["/data/originals"])),
            None
        );
    }

    #[test]
    fn repository_kinds() {
        let opts = Options {
            database_url: String::new(),
            data_dir: PathBuf::from("/data"),
            backup_dir: PathBuf::from("/backup"),
            repository: None,
            snapshot: None,
            force: false,
        };
        let local = repository(&opts, "k".into()).unwrap();
        assert_eq!(local.location, "/backup");
        assert_eq!(
            local.env,
            vec![("RESTIC_PASSWORD".to_owned(), "k".to_owned())]
        );
        assert_eq!(local.cache_dir, PathBuf::from("/data/cache/restic"));

        let rest = Options {
            repository: Some(" rest:https://u:p@host/repo ".into()),
            ..opts.clone()
        };
        assert_eq!(
            repository(&rest, "k".into()).unwrap().location,
            "rest:https://u:p@host/repo"
        );
        for bad in ["relative/path", "sftp:host:/x", "rclone:x"] {
            let o = Options {
                repository: Some(bad.into()),
                ..opts.clone()
            };
            assert!(repository(&o, "k".into()).is_err(), "{bad}");
        }
    }
}
