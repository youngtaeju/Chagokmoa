//! 복구 CLI. 실제 restic 저장소, DB 적용은 덤프 자리의 SQL 을 실행하는 가짜.

mod common;

use std::path::{Path, PathBuf};

use chagokmoa::{
    backup::{
        restic::Repo,
        restore::{self, Options, Prompt, RestoreError, Steps},
        run::ServerMajorFn,
    },
    error::{AppError, AppResult},
    media::storage,
    users::{self, NewUser, Role},
};
use futures_util::future::BoxFuture;
use sqlx::{
    Connection, PgConnection, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tempfile::TempDir;
use uuid::Uuid;

const KEY: &str = "correct-horse-battery-staple-0123";

fn major_17(_pool: &PgPool) -> BoxFuture<'_, AppResult<u32>> {
    Box::pin(async { Ok(17) })
}

fn major_16(_pool: &PgPool) -> BoxFuture<'_, AppResult<u32>> {
    Box::pin(async { Ok(16) })
}

/// pg_restore 대신. 실제 복구처럼 스키마를 비우고 마이그레이션만 한 뒤 덤프 파일 내용을 SQL 로 실행.
fn sql_apply<'a>(pool: &'a PgPool, _url: &'a str, dump: &'a Path) -> BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let sql = tokio::fs::read_to_string(dump).await.unwrap();
        sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
            .execute(pool)
            .await?;
        chagokmoa::db::migrate(pool).await?;
        sqlx::raw_sql(&sql).execute(pool).await?;
        Ok(())
    })
}

/// 복구 CLI 와 같은 연결 이름의 풀. 다른 연결 판정에서 제외됨.
async fn cli_pool(conn: &PgConnectOptions) -> PgPool {
    PgPoolOptions::new()
        .connect_with(restore::connect_options(conn.clone()))
        .await
        .unwrap()
}

fn no_apply<'a>(_pool: &'a PgPool, _url: &'a str, _dump: &'a Path) -> BoxFuture<'a, AppResult<()>> {
    panic!("apply must not run");
}

fn steps(server_major: ServerMajorFn) -> Steps {
    Steps {
        server_major,
        apply: sql_apply,
    }
}

struct Answers {
    key: String,
    confirm: bool,
    asked: usize,
}

impl Prompt for Answers {
    fn key(&mut self) -> anyhow::Result<String> {
        Ok(self.key.clone())
    }

    fn confirm(&mut self, _message: &str) -> anyhow::Result<bool> {
        self.asked += 1;
        Ok(self.confirm)
    }
}

fn answers(confirm: bool) -> Answers {
    Answers {
        key: KEY.to_owned(),
        confirm,
        asked: 0,
    }
}

struct Fixture {
    tmp: TempDir,
    opts: Options,
}

impl Fixture {
    fn new() -> Fixture {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        storage::ensure_dirs(&data_dir).unwrap();
        let opts = Options {
            database_url: String::new(),
            data_dir,
            backup_dir: tmp.path().join("backup"),
            repository: None,
            snapshot: None,
            force: false,
        };
        Fixture { tmp, opts }
    }

    fn data(&self) -> &Path {
        &self.opts.data_dir
    }

    fn repo(&self) -> Repo {
        Repo {
            location: self.opts.backup_dir.to_string_lossy().into_owned(),
            env: vec![("RESTIC_PASSWORD".to_owned(), KEY.to_owned())],
            cache_dir: self.tmp.path().join("cache"),
        }
    }

    /// 옛 서버 자리(`src`)에 덤프와 원본을 두고 백업과 같은 경로 구성으로 스냅숏. 반환은 스냅숏 id.
    async fn snapshot(&self, sql: &str, originals: &[(&str, &[u8])]) -> String {
        let src = self.tmp.path().join("src");
        let _ = std::fs::remove_dir_all(&src);
        write(&src.join("cache/backup/db.dump"), sql.as_bytes());
        std::fs::create_dir_all(src.join("originals")).unwrap();
        for (rel, bytes) in originals {
            write(&src.join(rel), bytes);
        }
        let repo = self.repo();
        if !self.opts.backup_dir.join("config").exists() {
            repo.init().await.unwrap();
        }
        let paths = [src.join("cache/backup/db.dump"), src.join("originals")];
        repo.backup(&paths, "manual").await.unwrap().snapshot_id
    }
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// 덤프 대신 쓰는 SQL. 소유자 1명과 사진 1장(`ready`).
fn owner_and_photo(user: Uuid, asset: Uuid, original: &str) -> String {
    format!(
        "INSERT INTO users (id, username, password_hash, display_name, role)
         VALUES ('{user}', 'mom', 'x', 'mom', 'owner');
         {}",
        asset_sql(asset, user, original, "ready", None)
    )
}

fn asset_sql(id: Uuid, user: Uuid, original: &str, status: &str, deleted: Option<&str>) -> String {
    let deleted = deleted.map_or_else(|| "NULL".to_owned(), |d| format!("'{d}'"));
    format!(
        "INSERT INTO assets (id, kind, uploaded_by, original_path, original_filename, mime,
                             size_bytes, sha256, taken_at, taken_at_source, status, deleted_at)
         VALUES ('{id}', 'photo', '{user}', '{original}', 'a.jpg', 'image/jpeg', 1,
                 decode(md5('{id}'), 'hex'), now(), 'upload_time', '{status}', {deleted});"
    )
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

/// `tmp/` 아래 복구 임시 폴더 잔재.
fn leftovers(data: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(data.join("tmp"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("restore")
        })
        .collect()
}

#[sqlx::test]
async fn restore_refuses_nonempty_db(_: PgPoolOptions, conn: PgConnectOptions) {
    let pool = cli_pool(&conn).await;
    let fx = Fixture::new();
    let (user, asset) = (Uuid::now_v7(), Uuid::now_v7());
    let rel = "originals/2026/11/a.jpg";
    fx.snapshot(&owner_and_photo(user, asset, rel), &[(rel, b"photo-a")])
        .await;
    users::queries::create(
        &pool,
        NewUser {
            username: "dad".to_owned(),
            password_hash: "x".to_owned(),
            display_name: "dad".to_owned(),
            role: Role::Owner,
            email: None,
            locale: "ko".to_owned(),
        },
    )
    .await
    .unwrap();

    // --force 없음: 내용 출력 후 거부, 아무것도 안 바뀜
    let no_apply = Steps {
        server_major: major_17,
        apply: no_apply,
    };
    let mut prompt = answers(true);
    let err = restore::run(&pool, &fx.opts, no_apply, &mut prompt)
        .await
        .unwrap_err();
    match err.downcast_ref::<RestoreError>() {
        Some(RestoreError::DbNotEmpty { users, assets }) => {
            assert_eq!((*users, *assets), (1, 0));
        }
        other => panic!("{other:?} {err:#}"),
    }
    assert_eq!(prompt.asked, 0);
    assert!(!fx.data().join(rel).exists());

    // --force 지만 재확인에서 거절
    let mut opts = Options {
        force: true,
        ..fx.opts.clone()
    };
    let mut prompt = answers(false);
    let err = restore::run(&pool, &opts, no_apply, &mut prompt)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<RestoreError>(),
            Some(RestoreError::Aborted)
        ),
        "{err:#}"
    );
    assert_eq!(prompt.asked, 1);
    assert!(!fx.data().join(rel).exists());
    assert!(leftovers(fx.data()).is_empty());

    // --force 와 재확인: 진행
    opts.force = true;
    let mut prompt = answers(true);
    restore::run(&pool, &opts, steps(major_17), &mut prompt)
        .await
        .unwrap();
    assert_eq!(prompt.asked, 1);
    assert_eq!(std::fs::read(fx.data().join(rel)).unwrap(), b"photo-a");
    assert_eq!(count(&pool, "SELECT count(*) FROM assets").await, 1);
    // 기존 DB 내용은 지워지고 덤프 내용만
    let names: Vec<String> = sqlx::query_scalar("SELECT username FROM users")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(names, vec!["mom".to_owned()]);
}

#[sqlx::test]
async fn restore_refuses_while_app_connected(_: PgPoolOptions, conn: PgConnectOptions) {
    let pool = cli_pool(&conn).await;
    let fx = Fixture::new();
    let rel = "originals/2026/11/a.jpg";
    fx.snapshot(
        &owner_and_photo(Uuid::now_v7(), Uuid::now_v7(), rel),
        &[(rel, b"photo-a")],
    )
    .await;
    // 실행 중인 앱의 연결(연결 이름 다름)
    let app = PgConnection::connect_with(&conn).await.unwrap();

    let no_apply = Steps {
        server_major: major_17,
        apply: no_apply,
    };
    let opts = Options {
        force: true,
        ..fx.opts.clone()
    };
    let mut prompt = answers(true);
    let err = restore::run(&pool, &opts, no_apply, &mut prompt)
        .await
        .unwrap_err();
    match err.downcast_ref::<RestoreError>() {
        Some(RestoreError::DatabaseInUse { sessions }) => assert_eq!(*sessions, 1),
        other => panic!("{other:?} {err:#}"),
    }
    assert!(format!("{err:#}").contains("docker compose stop app"));
    assert_eq!(prompt.asked, 0);
    assert!(!fx.data().join(rel).exists());

    // 앱 연결이 끊기면 진행
    app.close().await.unwrap();
    restore::run(&pool, &fx.opts, steps(major_17), &mut answers(false))
        .await
        .unwrap();
    assert!(fx.data().join(rel).exists());
}

#[sqlx::test]
async fn restore_removes_interrupted_scratch(_: PgPoolOptions, conn: PgConnectOptions) {
    let pool = cli_pool(&conn).await;
    let fx = Fixture::new();
    let rel = "originals/2026/11/a.jpg";
    fx.snapshot(
        &owner_and_photo(Uuid::now_v7(), Uuid::now_v7(), rel),
        &[(rel, b"photo-a")],
    )
    .await;
    // 강제 종료된 이전 복구 두 번의 잔재. 업로드 임시 파일은 그대로
    let old = fx.data().join("tmp/restore-0192/data/cache/backup/db.dump");
    write(&old, b"secret dump");
    write(
        &fx.data().join("tmp/restore-0193/data/originals/x.jpg"),
        b"x",
    );
    write(&fx.data().join("tmp/uploads/u.part"), b"u");

    restore::run(&pool, &fx.opts, steps(major_17), &mut answers(false))
        .await
        .unwrap();

    assert!(leftovers(fx.data()).is_empty());
    assert!(fx.data().join("tmp/uploads/u.part").exists());
    assert!(fx.data().join(rel).exists());
}

#[sqlx::test]
async fn restore_list_makes_no_changes(_: PgPoolOptions, conn: PgConnectOptions) {
    let pool = cli_pool(&conn).await;
    let fx = Fixture::new();
    let user = Uuid::now_v7();
    let older = fx
        .snapshot(
            &owner_and_photo(user, Uuid::now_v7(), "originals/a.jpg"),
            &[("originals/a.jpg", b"a")],
        )
        .await;
    let newer = fx
        .snapshot(
            &owner_and_photo(user, Uuid::now_v7(), "originals/b.jpg"),
            &[("originals/b.jpg", b"b")],
        )
        .await;
    // 사용 중인 서버(사용자, 원본, 앱 연결)에서도 목록은 가능
    sqlx::raw_sql(&owner_and_photo(
        Uuid::now_v7(),
        Uuid::now_v7(),
        "originals/c.jpg",
    ))
    .execute(&pool)
    .await
    .unwrap();
    write(&fx.data().join("originals/c.jpg"), b"c");
    let _app = PgConnection::connect_with(&conn).await.unwrap();

    let mut prompt = answers(true);
    let list = restore::list(&fx.opts, &mut prompt).await.unwrap();

    let ids: Vec<&str> = list.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec![older.as_str(), newer.as_str()]);
    assert_eq!(prompt.asked, 0);
    assert_eq!(count(&pool, "SELECT count(*) FROM assets").await, 1);
    assert!(!fx.data().join("originals/a.jpg").exists());
    assert!(!fx.data().join("originals/b.jpg").exists());
    assert_eq!(
        std::fs::read(fx.data().join("originals/c.jpg")).unwrap(),
        b"c"
    );
    assert!(leftovers(fx.data()).is_empty());
}

#[sqlx::test]
async fn restore_refuses_unsupported_postgres(pool: PgPool) {
    let fx = Fixture::new();
    let err = restore::run(&pool, &fx.opts, steps(major_16), &mut answers(true))
        .await
        .unwrap_err();
    match err.downcast_ref::<AppError>() {
        Some(AppError::Conflict { key, .. }) => assert_eq!(*key, "backup.unsupportedPostgres"),
        other => panic!("{other:?} {err:#}"),
    }
}

#[sqlx::test]
async fn restore_accepts_migrated_empty_db(_: PgPoolOptions, conn: PgConnectOptions) {
    let pool = cli_pool(&conn).await;
    let fx = Fixture::new();
    let (user, asset) = (Uuid::now_v7(), Uuid::now_v7());
    let rel = "originals/2026/11/a.jpg";
    let nested = "originals/2025/01/b.jpg";
    let id = fx
        .snapshot(
            &owner_and_photo(user, asset, rel),
            &[(rel, b"photo-a"), (nested, b"photo-b")],
        )
        .await;

    let mut prompt = answers(false);
    let summary = restore::run(&pool, &fx.opts, steps(major_17), &mut prompt)
        .await
        .unwrap();

    // 빈 DB 는 재확인 없이
    assert_eq!(prompt.asked, 0);
    assert_eq!(summary.snapshot.id, id);
    assert_eq!(summary.originals, 2);
    assert_eq!(std::fs::read(fx.data().join(rel)).unwrap(), b"photo-a");
    assert_eq!(std::fs::read(fx.data().join(nested)).unwrap(), b"photo-b");
    assert_eq!(count(&pool, "SELECT count(*) FROM users").await, 1);
    assert!(leftovers(fx.data()).is_empty());
}

#[sqlx::test]
async fn restore_db_state_without_tables(pool: PgPool) {
    assert_eq!(
        restore::db_state(&pool).await.unwrap(),
        restore::DbState::Empty
    );
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        restore::db_state(&pool).await.unwrap(),
        restore::DbState::Empty
    );
    sqlx::raw_sql("CREATE TABLE other (id int)")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        restore::db_state(&pool).await.unwrap(),
        restore::DbState::Unknown { tables: 1 }
    );
}

#[sqlx::test]
async fn restore_refuses_existing_originals(_: PgPoolOptions, conn: PgConnectOptions) {
    let pool = cli_pool(&conn).await;
    let fx = Fixture::new();
    let (user, asset) = (Uuid::now_v7(), Uuid::now_v7());
    let rel = "originals/2026/11/a.jpg";
    fx.snapshot(&owner_and_photo(user, asset, rel), &[(rel, b"photo-a")])
        .await;
    write(&fx.data().join(rel), b"other");

    let no_apply = Steps {
        server_major: major_17,
        apply: no_apply,
    };
    let err = restore::run(&pool, &fx.opts, no_apply, &mut answers(true))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<RestoreError>(),
            Some(RestoreError::OriginalsNotEmpty)
        ),
        "{err:#}"
    );
    assert_eq!(std::fs::read(fx.data().join(rel)).unwrap(), b"other");

    // --force: 재확인 후 덮어씀
    let opts = Options {
        force: true,
        ..fx.opts.clone()
    };
    let mut prompt = answers(true);
    restore::run(&pool, &opts, steps(major_17), &mut prompt)
        .await
        .unwrap();
    assert_eq!(prompt.asked, 1);
    assert_eq!(std::fs::read(fx.data().join(rel)).unwrap(), b"photo-a");
}

#[sqlx::test]
async fn restore_enqueues_regen(_: PgPoolOptions, conn: PgConnectOptions) {
    let pool = cli_pool(&conn).await;
    let fx = Fixture::new();
    let user = Uuid::now_v7();
    let (ready, trashed, processing) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let done_run = Uuid::now_v7();
    let paths = [
        "originals/2026/11/ready.jpg",
        "originals/2026/11/trashed.jpg",
        "originals/2026/11/processing.jpg",
    ];
    let sql = format!(
        "{}
         {}
         {}
         INSERT INTO jobs (id, kind, payload)
         VALUES ('{}', 'process_asset', jsonb_build_object('asset_id', '{processing}'));
         INSERT INTO backup_runs (id, trigger, status, finished_at)
         VALUES ('{done_run}', 'scheduled', 'success', now());
         INSERT INTO backup_runs (id, trigger, status, phase)
         VALUES ('{}', 'scheduled', 'running', 'dump');",
        owner_and_photo(user, ready, paths[0]),
        asset_sql(
            trashed,
            user,
            paths[1],
            "ready",
            Some("2026-11-01T00:00:00Z")
        ),
        asset_sql(processing, user, paths[2], "processing", None),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    let files: Vec<(&str, &[u8])> = paths.iter().map(|p| (*p, &b"x"[..])).collect();
    fx.snapshot(&sql, &files).await;

    let summary = restore::run(&pool, &fx.opts, steps(major_17), &mut answers(false))
        .await
        .unwrap();

    // 휴지통 포함 ready 만 재생성. 처리 중 항목은 덤프의 처리 작업 그대로
    assert_eq!(summary.queued, 2);
    let mut regen: Vec<Uuid> = sqlx::query_scalar(
        "SELECT (payload ->> 'asset_id')::uuid FROM jobs WHERE kind = 'regenerate_derivatives'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    regen.sort();
    let mut expected = vec![ready, trashed];
    expected.sort();
    assert_eq!(regen, expected);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM jobs WHERE kind = 'process_asset' AND status = 'queued'"
        )
        .await,
        1
    );
    // 덤프 시점의 자기 실행 기록만 삭제
    let runs: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM backup_runs")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(runs, vec![done_run]);
}

#[sqlx::test]
async fn restore_picks_snapshot(_: PgPoolOptions, conn: PgConnectOptions) {
    let pool = cli_pool(&conn).await;
    let fx = Fixture::new();
    let user = Uuid::now_v7();
    let (first, second) = (Uuid::now_v7(), Uuid::now_v7());
    let older = fx
        .snapshot(
            &owner_and_photo(user, first, "originals/a.jpg"),
            &[("originals/a.jpg", b"a")],
        )
        .await;
    let newer = fx
        .snapshot(
            &owner_and_photo(user, second, "originals/b.jpg"),
            &[("originals/b.jpg", b"b")],
        )
        .await;
    assert_ne!(older, newer);

    // 짧은 id 로 이전 스냅숏 선택
    let opts = Options {
        snapshot: Some(older[..8].to_owned()),
        ..fx.opts.clone()
    };
    let summary = restore::run(&pool, &opts, steps(major_17), &mut answers(false))
        .await
        .unwrap();
    assert_eq!(summary.snapshot.id, older);
    assert!(fx.data().join("originals/a.jpg").exists());
    assert!(!fx.data().join("originals/b.jpg").exists());

    // 없는 id
    let opts = Options {
        snapshot: Some("deadbeef".to_owned()),
        force: true,
        ..fx.opts.clone()
    };
    let err = restore::run(&pool, &opts, steps(major_17), &mut answers(true))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<RestoreError>(),
            Some(RestoreError::SnapshotNotFound(_))
        ),
        "{err:#}"
    );
}

#[sqlx::test]
async fn restore_latest_by_default(_: PgPoolOptions, conn: PgConnectOptions) {
    let pool = cli_pool(&conn).await;
    let fx = Fixture::new();
    let user = Uuid::now_v7();
    fx.snapshot(
        &owner_and_photo(user, Uuid::now_v7(), "originals/a.jpg"),
        &[("originals/a.jpg", b"a")],
    )
    .await;
    let newer = fx
        .snapshot(
            &owner_and_photo(user, Uuid::now_v7(), "originals/b.jpg"),
            &[("originals/b.jpg", b"b")],
        )
        .await;
    let summary = restore::run(&pool, &fx.opts, steps(major_17), &mut answers(false))
        .await
        .unwrap();
    assert_eq!(summary.snapshot.id, newer);
    assert!(fx.data().join("originals/b.jpg").exists());
    assert!(!fx.data().join("originals/a.jpg").exists());
}

#[sqlx::test]
async fn restore_wrong_key(_: PgPoolOptions, conn: PgConnectOptions) {
    let pool = cli_pool(&conn).await;
    let fx = Fixture::new();
    fx.snapshot("SELECT 1;", &[]).await;
    let mut prompt = Answers {
        key: "not-the-key".to_owned(),
        confirm: false,
        asked: 0,
    };
    let err = restore::run(&pool, &fx.opts, steps(major_17), &mut prompt)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<RestoreError>(),
            Some(RestoreError::WrongKey)
        ),
        "{err:#}"
    );
    assert!(!format!("{err:#}").contains("not-the-key"));
}

#[sqlx::test]
async fn restore_empty_repository(_: PgPoolOptions, conn: PgConnectOptions) {
    let pool = cli_pool(&conn).await;
    let fx = Fixture::new();
    let err = restore::run(&pool, &fx.opts, steps(major_17), &mut answers(false))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<RestoreError>(),
            Some(RestoreError::NoRepository(_))
        ),
        "{err:#}"
    );
}
