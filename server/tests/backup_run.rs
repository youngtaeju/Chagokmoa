//! 백업 실행, 일정, 기록, 실패 알림. 실제 restic, 덤프는 가짜 파일.

mod common;

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use chagokmoa::{
    backup::{
        queries,
        restic::{Repo, RepoState},
        run::{self, DbSteps, RunStatus, Trigger},
        settings::{self, BackupSettings, Remote},
    },
    error::{AppError, AppResult},
    media::storage,
    notifications::{self, NotificationKind},
    settings::queries as instance_settings,
    state::AppState,
};
use chrono::{DateTime, NaiveTime, Utc};
use common::test_config;
use futures_util::future::BoxFuture;
use serde_json::json;
use sqlx::PgPool;
use tempfile::TempDir;
use uuid::Uuid;

const KEY: &str = "correct-horse-battery-staple-0123";
const DUMP: &[u8] = b"fake-dump";

fn fake_dump<'a>(_url: &'a str, path: &'a Path) -> BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        tokio::fs::write(path, DUMP)
            .await
            .map_err(|e| AppError::Internal(e.into()))
    })
}

fn major_17(_pool: &PgPool) -> BoxFuture<'_, AppResult<u32>> {
    Box::pin(async { Ok(17) })
}

fn major_16(_pool: &PgPool) -> BoxFuture<'_, AppResult<u32>> {
    Box::pin(async { Ok(16) })
}

struct Env {
    state: AppState,
    tmp: TempDir,
}

impl Env {
    fn data(&self) -> PathBuf {
        self.state.config.data_dir.clone()
    }

    fn local_repo(&self) -> Repo {
        enabled()
            .local_repo(&self.state.config)
            .expect("local repo")
    }
}

fn enabled() -> BackupSettings {
    BackupSettings {
        enabled: true,
        password: Some(KEY.to_owned()),
        ..BackupSettings::default()
    }
}

/// 임시 DATA_DIR 와 백업 폴더. 로컬 저장소 초기화, 설정 저장.
async fn env(pool: &PgPool) -> Env {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config();
    config.data_dir = tmp.path().join("data");
    config.backup_dir = tmp.path().join("backup");
    storage::ensure_dirs(&config.data_dir).unwrap();
    let mut state = AppState::new(pool.clone(), config);
    state.backup_db = DbSteps {
        server_major: major_17,
        dump: fake_dump,
    };
    let env = Env { state, tmp };
    env.local_repo().init().await.unwrap();
    settings::save(pool, &enabled()).await.unwrap();
    env
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// 스냅숏을 복원한 폴더 안에서 원래 절대 경로 위치.
fn restored(target: &Path, original: &Path) -> PathBuf {
    target.join(original.strip_prefix("/").unwrap())
}

#[sqlx::test]
async fn run_creates_snapshot_with_dump_and_originals(pool: PgPool) {
    let env = env(&pool).await;
    let original = env.data().join("originals/2026/11/a.jpg");
    write(&original, b"photo-a");
    // 이전 프로세스가 남긴 진행 중 기록
    let stale = Uuid::now_v7();
    sqlx::query("INSERT INTO backup_runs (id, trigger, status, phase) VALUES ($1, 'scheduled', 'running', 'local')")
        .bind(stale)
        .execute(&pool)
        .await
        .unwrap();

    let id = run::run_once(&env.state, Trigger::Manual).await.unwrap();

    let row = queries::get(&pool, id).await.unwrap().expect("run row");
    assert_eq!(row.status, RunStatus::Success, "{:?}", row.error);
    assert_eq!(row.trigger, Trigger::Manual);
    assert_eq!(row.phase, None);
    assert!(row.finished_at.is_some());
    assert!(row.remote.is_none());
    let local = row.local.expect("local result");
    let snapshot_id = local["snapshot_id"].as_str().expect("snapshot id");
    assert_eq!(local["files_new"], json!(2));
    assert!(queries::running(&pool).await.unwrap().is_none());

    let old = queries::get(&pool, stale).await.unwrap().unwrap();
    assert_eq!(old.status, RunStatus::Failed);
    assert_eq!(old.error.as_deref(), Some("interrupted"));

    let repo = env.local_repo();
    let snapshots = repo.snapshots().await.unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].id, snapshot_id);
    assert_eq!(snapshots[0].tags, vec!["manual".to_owned()]);

    let target = env.tmp.path().join("restore");
    repo.restore(snapshot_id, &target).await.unwrap();
    let dump = std::path::absolute(env.data().join("cache/backup/db.dump")).unwrap();
    assert_eq!(std::fs::read(restored(&target, &dump)).unwrap(), DUMP);
    let original = std::path::absolute(&original).unwrap();
    assert_eq!(
        std::fs::read(restored(&target, &original)).unwrap(),
        b"photo-a"
    );
    // 덤프 파일은 실행 뒤 정리
    assert!(!dump.exists());
}

#[sqlx::test]
async fn derivatives_excluded(pool: PgPool) {
    let env = env(&pool).await;
    let data = std::path::absolute(env.data()).unwrap();
    write(&data.join("originals/2026/11/a.jpg"), b"photo-a");
    for excluded in [
        "derivatives/x/thumb.webp",
        "tmp/uploads/x.part",
        "orphans/2026/11/b.jpg",
        "cache/restic/marker",
    ] {
        write(&data.join(excluded), b"x");
    }

    let id = run::run_once(&env.state, Trigger::Manual).await.unwrap();
    let row = queries::get(&pool, id).await.unwrap().unwrap();
    assert_eq!(row.status, RunStatus::Success, "{:?}", row.error);

    let repo = env.local_repo();
    let snapshot = repo.snapshots().await.unwrap().pop().unwrap();
    let mut paths = snapshot.paths.clone();
    paths.sort();
    assert_eq!(
        paths,
        vec![
            data.join("cache/backup/db.dump")
                .to_string_lossy()
                .into_owned(),
            data.join("originals").to_string_lossy().into_owned(),
        ]
    );
    let target = env.tmp.path().join("restore");
    repo.restore(&snapshot.id, &target).await.unwrap();
    let root = restored(&target, &data);
    assert!(root.join("originals/2026/11/a.jpg").is_file());
    for excluded in ["derivatives", "tmp", "orphans", "cache/restic"] {
        assert!(!root.join(excluded).exists(), "{excluded}");
    }
}

#[sqlx::test]
async fn concurrent_run_rejected(pool: PgPool) {
    let env = env(&pool).await;
    // 다른 프로세스(CLI 또는 서버)가 실행 중
    let mut other = pool.acquire().await.unwrap();
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(run::LOCK_KEY)
        .fetch_one(&mut *other)
        .await
        .unwrap();
    assert!(locked);

    let err = run::run_once(&env.state, Trigger::Manual)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            AppError::Conflict {
                key: "backup.running",
                ..
            }
        ),
        "{err:?}"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM backup_runs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);

    // 풀린 뒤 실행
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(run::LOCK_KEY)
        .execute(&mut *other)
        .await
        .unwrap();
    run::run_once(&env.state, Trigger::Manual).await.unwrap();
}

/// 배타 잠금을 잡은 채 죽은 restic 흉내. `key passwd` 가 새 키 입력을 기다리는 동안 강제 종료.
async fn leave_stale_lock(repo: &Repo, dir: &Path) {
    let mut cmd = repo.command(&["key", "passwd"]);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = cmd.spawn().unwrap();
    let locks = dir.join("locks");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let held = std::fs::read_dir(&locks).is_ok_and(|mut d| d.next().is_some());
        if held {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "lock not taken");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    child.kill().await.unwrap();
}

#[sqlx::test]
async fn stale_lock_unlocked_before_run(pool: PgPool) {
    let env = env(&pool).await;
    let repo = env.local_repo();
    leave_stale_lock(&repo, &env.state.config.backup_dir).await;
    // 잠금이 남아 그냥 backup 은 실패
    let originals = env.data().join("originals");
    assert!(repo.backup(&[originals], "manual").await.is_err());

    let id = run::run_once(&env.state, Trigger::Manual).await.unwrap();
    let row = queries::get(&pool, id).await.unwrap().unwrap();
    assert_eq!(row.status, RunStatus::Success, "{:?}", row.error);
}

async fn user(pool: &PgPool, name: &str, role: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, role, locale)
         VALUES ($1, $2, 'x', $2, $3::user_role, 'ko')",
    )
    .bind(id)
    .bind(name)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
    id
}

#[sqlx::test]
async fn failure_notifies_owners_once_per_day(pool: PgPool) {
    let env = env(&pool).await;
    let owner = user(&pool, "owner1", "owner").await;
    let owner2 = user(&pool, "owner2", "owner").await;
    user(&pool, "guardian", "guardian").await;
    let disabled = user(&pool, "gone", "owner").await;
    sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1")
        .bind(disabled)
        .execute(&pool)
        .await
        .unwrap();
    // 원격 저장소 없음 + 비밀값 섞인 인증값. 로컬은 성공, 원격만 실패
    let secret = "remote-secret-value-xyz";
    let missing = env.tmp.path().join("no-such-remote");
    let mut s = enabled();
    s.remote = Some(Remote {
        repository: missing.to_string_lossy().into_owned(),
        env: [("AWS_SECRET_ACCESS_KEY".to_owned(), secret.to_owned())].into(),
    });
    settings::save(&pool, &s).await.unwrap();

    for _ in 0..2 {
        let id = run::run_once(&env.state, Trigger::Scheduled).await.unwrap();
        let row = queries::get(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.status, RunStatus::Failed);
        assert!(row.local.unwrap()["snapshot_id"].is_string());
        let remote = row.remote.expect("remote result");
        assert!(remote["error"].is_string());
        let error = row.error.expect("error");
        assert!(!error.contains(secret) && !error.contains(KEY), "{error}");
    }

    let rows: Vec<(Uuid, Uuid, i32)> = sqlx::query_as(
        "SELECT user_id, actor_id, count FROM notifications WHERE kind = 'backup' ORDER BY user_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let mut expected = vec![(owner, owner, 2), (owner2, owner2, 2)];
    expected.sort();
    assert_eq!(rows, expected);
    // 목록과 푸시 본문 조회 경로에서 대상 없이 읽힘
    let mut conn = pool.acquire().await.unwrap();
    let listed = notifications::queries::list(&mut conn, "Asia/Seoul", owner, None, 10)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].kind, NotificationKind::Backup);
    assert_eq!(listed[0].count, 2);
    assert_eq!(listed[0].actor.id, owner);
    // 같은 날 반복은 푸시 한 번
    let pushes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs j JOIN notifications n
           ON n.id = (j.payload->>'notification_id')::uuid
         WHERE j.kind = 'send_push' AND n.kind = 'backup'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pushes, 2);
}

#[sqlx::test]
async fn unsupported_postgres_rejected(pool: PgPool) {
    let mut env = env(&pool).await;
    env.state.backup_db.server_major = major_16;
    let err = run::run_once(&env.state, Trigger::Manual)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            AppError::Conflict {
                key: "backup.unsupportedPostgres",
                ..
            }
        ),
        "{err:?}"
    );
    assert!(env.local_repo().snapshots().await.unwrap().is_empty());
}

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

#[sqlx::test]
async fn scheduled_time_in_instance_timezone(pool: PgPool) {
    let env = env(&pool).await;
    let mut s = enabled();
    s.time = NaiveTime::from_hms_opt(3, 0, 0).unwrap();
    settings::save(&pool, &s).await.unwrap();
    instance_settings::set(&pool, "timezone", json!("Asia/Seoul"))
        .await
        .unwrap();

    // 03:00 KST = 전날 18:00 UTC
    assert!(
        !run::due(&env.state, at("2026-01-09T17:59:00Z"))
            .await
            .unwrap()
    );
    assert!(
        run::due(&env.state, at("2026-01-09T18:00:00Z"))
            .await
            .unwrap()
    );
    // 02:30 UTC 는 KST 11:30, 오늘 일정 지남
    assert!(
        run::due(&env.state, at("2026-01-10T02:30:00Z"))
            .await
            .unwrap()
    );

    instance_settings::set(&pool, "timezone", json!("UTC"))
        .await
        .unwrap();
    assert!(
        !run::due(&env.state, at("2026-01-10T02:30:00Z"))
            .await
            .unwrap()
    );
    assert!(
        run::due(&env.state, at("2026-01-10T03:00:00Z"))
            .await
            .unwrap()
    );

    // 꺼져 있으면 일정 없음
    s.enabled = false;
    settings::save(&pool, &s).await.unwrap();
    assert!(
        !run::due(&env.state, at("2026-01-10T03:00:00Z"))
            .await
            .unwrap()
    );
}

#[sqlx::test]
async fn missed_schedule_runs_once(pool: PgPool) {
    let env = env(&pool).await;
    instance_settings::set(&pool, "timezone", json!("Asia/Seoul"))
        .await
        .unwrap();
    // 03:00 에 서버가 꺼져 있었고 14:00 KST 에 첫 확인
    let now = at("2026-01-10T05:00:00Z");
    let id = run::tick(&env.state, now)
        .await
        .unwrap()
        .expect("missed run");
    let row = queries::get(&pool, id).await.unwrap().unwrap();
    assert_eq!(row.trigger, Trigger::Scheduled);
    assert_eq!(row.status, RunStatus::Success, "{:?}", row.error);

    assert_eq!(run::tick(&env.state, now).await.unwrap(), None);
    assert_eq!(
        run::tick(&env.state, at("2026-01-10T09:00:00Z"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(env.local_repo().snapshots().await.unwrap().len(), 1);
}

#[sqlx::test]
async fn leftover_dump_removed_at_start(pool: PgPool) {
    let env = env(&pool).await;
    // 중단된 이전 실행이 남긴 덤프
    let dump = run::dump_path(&env.data());
    write(&dump, b"old-dump");
    let started = run::start(&env.state, Trigger::Manual).await.unwrap();
    assert!(!dump.exists());
    drop(started);
}

/// 권한 확인 후 파일을 쓰고 실패. 권한이 틀리면 다른 문구.
fn private_then_fail<'a>(_url: &'a str, path: &'a Path) -> BoxFuture<'a, AppResult<()>> {
    use std::os::unix::fs::PermissionsExt;
    Box::pin(async move {
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let dir = mode(path.parent().unwrap());
        let file = mode(path);
        if (dir, file) != (0o700, 0o600) {
            return Err(AppError::Internal(anyhow::anyhow!(
                "bad mode dir {dir:o} file {file:o}"
            )));
        }
        tokio::fs::write(path, DUMP).await.unwrap();
        Err(AppError::Internal(anyhow::anyhow!("injected dump failure")))
    })
}

#[sqlx::test]
async fn dump_private_and_removed_after_failed_run(pool: PgPool) {
    use std::os::unix::fs::PermissionsExt;
    let mut env = env(&pool).await;
    // 이전 버전이 0755 로 만든 폴더
    let dump = run::dump_path(&env.data());
    let dir = dump.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    env.state.backup_db.dump = private_then_fail;

    let id = run::run_once(&env.state, Trigger::Manual).await.unwrap();
    let row = queries::get(&pool, id).await.unwrap().unwrap();
    assert_eq!(row.status, RunStatus::Failed);
    let error = row.error.unwrap();
    assert!(error.contains("injected dump failure"), "{error}");
    assert!(!dump.exists());
}

#[sqlx::test]
async fn mid_run_db_error_still_recorded(pool: PgPool) {
    let env = env(&pool).await;
    let owner = user(&pool, "owner1", "owner").await;
    write(&env.data().join("originals/2026/11/a.jpg"), b"photo-a");
    // 스냅숏 뒤 진행 단계 갱신 실패 흉내
    sqlx::raw_sql(
        "CREATE FUNCTION fail_forget() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'injected phase failure'; END $$;
         CREATE TRIGGER fail_forget BEFORE UPDATE ON backup_runs
           FOR EACH ROW WHEN (NEW.phase = 'forget') EXECUTE FUNCTION fail_forget();",
    )
    .execute(&pool)
    .await
    .unwrap();

    let id = run::run_once(&env.state, Trigger::Scheduled).await.unwrap();
    let row = queries::get(&pool, id).await.unwrap().unwrap();
    assert_eq!(row.status, RunStatus::Failed);
    assert_eq!(row.phase, None);
    assert!(row.finished_at.is_some());
    let error = row.error.unwrap();
    assert!(error.starts_with("internal: "), "{error}");
    assert!(error.contains("injected phase failure"), "{error}");
    // 이미 만든 스냅숏 결과는 기록에 남음
    assert!(row.local.unwrap()["snapshot_id"].is_string());
    assert!(!run::dump_path(&env.data()).exists());
    let notified: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE kind = 'backup' AND user_id = $1",
    )
    .bind(owner)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(notified, 1);
}

#[sqlx::test]
async fn empty_backup_dir_recreated_with_stored_key(pool: PgPool) {
    let env = env(&pool).await;
    write(&env.data().join("originals/2026/11/a.jpg"), b"photo-a");
    // 원격에서 새 서버로 복구한 뒤 빈 백업 폴더 연결
    let backup = env.state.config.backup_dir.clone();
    std::fs::remove_dir_all(&backup).unwrap();
    std::fs::create_dir(&backup).unwrap();

    let id = run::run_once(&env.state, Trigger::Scheduled).await.unwrap();

    let row = queries::get(&pool, id).await.unwrap().unwrap();
    assert_eq!(row.status, RunStatus::Warning, "{:?}", row.error);
    assert!(row.error.is_none());
    let local = row.local.expect("local result");
    assert_eq!(local["warning"], json!("local repository created"));
    let snapshot_id = local["snapshot_id"].as_str().expect("snapshot id");
    let repo = env.local_repo();
    assert_eq!(repo.probe().await.unwrap(), RepoState::Restic);
    let snapshots = repo.snapshots().await.unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].id, snapshot_id);

    // 다음 실행은 경고 없이 성공
    let id = run::run_once(&env.state, Trigger::Scheduled).await.unwrap();
    let row = queries::get(&pool, id).await.unwrap().unwrap();
    assert_eq!(row.status, RunStatus::Success, "{:?}", row.error);
}

#[sqlx::test]
async fn missing_backup_dir_fails_without_creating(pool: PgPool) {
    let env = env(&pool).await;
    let backup = env.state.config.backup_dir.clone();
    std::fs::remove_dir_all(&backup).unwrap();

    let id = run::run_once(&env.state, Trigger::Scheduled).await.unwrap();

    let row = queries::get(&pool, id).await.unwrap().unwrap();
    assert_eq!(row.status, RunStatus::Failed);
    let local = row.local.expect("local result");
    assert!(local["error"].is_string());
    assert!(local.get("warning").is_none());
    assert!(row.error.unwrap().starts_with("local: "));
    assert!(!backup.exists());
}
