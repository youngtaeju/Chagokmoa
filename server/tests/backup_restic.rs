//! restic, pg_dump 래퍼. 실제 restic 과 임시 디렉터리 저장소 사용.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use chagokmoa::backup::{
    dump,
    restic::{self, Repo, RepoState, Retention},
};
use sqlx::PgPool;
use tempfile::TempDir;

const KEY: &str = "correct-horse-battery-staple-0123";

fn repo_at(tmp: &TempDir, location: &Path, key: &str) -> Repo {
    Repo {
        location: location.to_string_lossy().into_owned(),
        env: vec![("RESTIC_PASSWORD".into(), key.into())],
        cache_dir: tmp.path().join("cache/restic"),
    }
}

/// `DATA_DIR` 흉내. 덤프 파일과 originals 하나씩.
fn source(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let data = tmp.path().join("data");
    let originals = data.join("originals");
    std::fs::create_dir_all(originals.join("2026/11")).unwrap();
    std::fs::write(originals.join("2026/11/a.jpg"), b"photo-a").unwrap();
    let dump = data.join("cache/backup/db.dump");
    std::fs::create_dir_all(dump.parent().unwrap()).unwrap();
    std::fs::write(&dump, b"dump").unwrap();
    (dump, originals)
}

#[tokio::test]
async fn probe_states() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("backup");

    // 폴더 없음
    let repo = repo_at(&tmp, &dir, KEY);
    assert_eq!(repo.probe().await.unwrap(), RepoState::Empty);

    // 빈 폴더, NAS 가 만드는 숨김과 색인 폴더만 있음
    std::fs::create_dir_all(dir.join("@eaDir")).unwrap();
    std::fs::write(dir.join(".DS_Store"), b"x").unwrap();
    assert_eq!(repo.probe().await.unwrap(), RepoState::Empty);

    repo.init().await.unwrap();
    assert_eq!(repo.probe().await.unwrap(), RepoState::Restic);
}

#[tokio::test]
async fn init_then_backup_and_snapshots() {
    let tmp = TempDir::new().unwrap();
    let (dump, originals) = source(&tmp);
    let repo = repo_at(&tmp, &tmp.path().join("backup"), KEY);
    repo.init().await.unwrap();

    let summary = repo
        .backup(&[dump.clone(), originals.clone()], "manual")
        .await
        .unwrap();
    assert_eq!(summary.files_new, 2);
    assert!(summary.data_added > 0);
    assert!(summary.warning.is_none());
    assert!(!summary.snapshot_id.is_empty());

    let snapshots = repo.snapshots().await.unwrap();
    assert_eq!(snapshots.len(), 1);
    let snap = &snapshots[0];
    assert_eq!(snap.id, summary.snapshot_id);
    assert_eq!(snap.tags, vec!["manual".to_owned()]);
    assert_eq!(snap.hostname, "chagokmoa");
    assert!(
        snap.paths
            .contains(&originals.to_string_lossy().into_owned())
    );

    // 두 번째는 바뀐 것 없음
    let again = repo
        .backup(&[dump, originals.clone()], "manual")
        .await
        .unwrap();
    assert_eq!(again.files_new, 0);

    repo.unlock().await.unwrap();
    repo.check().await.unwrap();
    repo.prune().await.unwrap();

    let target = tmp.path().join("restore");
    repo.restore(&summary.snapshot_id, &target).await.unwrap();
    let restored = target
        .join(originals.strip_prefix("/").unwrap())
        .join("2026/11/a.jpg");
    assert_eq!(std::fs::read(restored).unwrap(), b"photo-a");

    // 스냅숏 id 형식 아닌 값은 인자로 넘기지 않음
    assert!(repo.restore("--help", &target).await.is_err());
}

#[tokio::test]
async fn wrong_key_detected() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("backup");
    repo_at(&tmp, &dir, KEY).init().await.unwrap();

    let other = "another-key-that-does-not-open";
    let wrong = repo_at(&tmp, &dir, other);
    assert_eq!(wrong.probe().await.unwrap(), RepoState::WrongKey);

    // 오류 문구에 비밀값 없음
    let err = wrong.snapshots().await.unwrap_err().to_string();
    assert!(!err.contains(other), "{err}");
}

#[tokio::test]
async fn foreign_dir_detected() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("backup");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("photo.jpg"), b"x").unwrap();
    assert_eq!(
        repo_at(&tmp, &dir, KEY).probe().await.unwrap(),
        RepoState::Foreign
    );

    // 폴더 자리에 파일
    let file = tmp.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    assert_eq!(
        repo_at(&tmp, &file, KEY).probe().await.unwrap(),
        RepoState::Foreign
    );
}

#[tokio::test]
async fn forget_applies_policy() {
    let tmp = TempDir::new().unwrap();
    let (dump, originals) = source(&tmp);
    let repo = repo_at(&tmp, &tmp.path().join("backup"), KEY);
    repo.init().await.unwrap();
    let paths = [dump.clone(), originals];
    let mut last = String::new();
    for i in 0..3 {
        std::fs::write(&dump, format!("dump-{i}")).unwrap();
        last = repo.backup(&paths, "scheduled").await.unwrap().snapshot_id;
    }

    // 모두 0 이면 지우지 않음
    repo.forget(Retention {
        daily: 0,
        weekly: 0,
        monthly: 0,
    })
    .await
    .unwrap();
    assert_eq!(repo.snapshots().await.unwrap().len(), 3);

    // 같은 날 3개 중 최신 하나
    repo.forget(Retention {
        daily: 1,
        weekly: 0,
        monthly: 0,
    })
    .await
    .unwrap();
    let left = repo.snapshots().await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, last);
}

/// restic 0.16 (`Err` 숫자 또는 빈 객체) 과 0.17 이상(`message`) 오류 줄.
fn error_line_016(item: &Path, errno: i32) -> String {
    let item = item.to_string_lossy();
    format!(
        r#"{{"message_type":"error","error":{{"Op":"lstat","Path":"{item}","Err":{errno}}},"during":"archival","item":"{item}"}}"#
    )
}

fn error_line_016_opaque(item: &Path) -> String {
    let item = item.to_string_lossy();
    format!(r#"{{"message_type":"error","error":{{}},"during":"archival","item":"{item}"}}"#)
}

fn error_line_018(item: &Path) -> String {
    let item = item.to_string_lossy();
    format!(
        r#"{{"message_type":"error","error":{{"message":"lstat {item}: no such file or directory"}},"during":"scan","item":"{item}"}}"#
    )
}

#[test]
fn missing_file_exit3_is_warning() {
    let tmp = TempDir::new().unwrap();
    let (dump, originals) = source(&tmp);
    let paths = [dump.clone(), originals.clone()];
    let gone_a = originals.join("2026/11/gone-a.jpg");
    let gone_b = originals.join("2026/10");
    let parse = |lines: &[String]| -> Vec<restic::ItemError> {
        lines
            .iter()
            .map(|l| restic::parse_item_error(l).expect(l))
            .collect()
    };

    // originals 아래 사라진 항목뿐: 경고
    let errors = parse(&[
        error_line_016(&gone_a, 2),
        error_line_016_opaque(&gone_a),
        error_line_018(&gone_b),
    ]);
    let warning = restic::exit3_outcome(&paths, &errors).unwrap();
    assert_eq!(warning, "2 files vanished during backup");

    // 상대 경로 뿌리(`..` 포함)도 restic 의 절대 경로 항목과 맞춤
    let relative = [relative_to_cwd(&dump), relative_to_cwd(&originals)];
    assert!(relative[1].is_relative());
    assert_eq!(
        restic::exit3_outcome(&relative, &errors).unwrap(),
        "2 files vanished during backup"
    );

    // originals 아래지만 파일이 남아 있음(권한 등): 실패
    let present = originals.join("2026/11/a.jpg");
    let errors = parse(&[error_line_016(&gone_a, 2), error_line_016(&present, 13)]);
    assert!(restic::exit3_outcome(&paths, &errors).is_err());

    // originals 밖(덤프 파일): 실패
    std::fs::remove_file(&dump).unwrap();
    let errors = parse(&[error_line_018(&dump)]);
    assert!(restic::exit3_outcome(&paths, &errors).is_err());

    // 원인을 못 읽음: 실패
    assert!(restic::exit3_outcome(&paths, &[]).is_err());
    assert!(restic::parse_item_error(r#"{"message_type":"status"}"#).is_none());
}

/// 작업 디렉터리 기준 `../..` 로 시작하는 상대 경로.
fn relative_to_cwd(path: &Path) -> PathBuf {
    let cwd = std::env::current_dir().unwrap();
    let up = cwd.components().count() - 1;
    let mut rel = PathBuf::new();
    for _ in 0..up {
        rel.push("..");
    }
    rel.join(path.strip_prefix("/").unwrap())
}

#[tokio::test]
async fn relative_sources_stored_absolute() {
    let tmp = TempDir::new().unwrap();
    let (dump, originals) = source(&tmp);
    let repo = repo_at(&tmp, &tmp.path().join("backup"), KEY);
    repo.init().await.unwrap();
    let summary = repo
        .backup(
            &[relative_to_cwd(&dump), relative_to_cwd(&originals)],
            "manual",
        )
        .await
        .unwrap();
    assert!(summary.warning.is_none());
    let snap = &repo.snapshots().await.unwrap()[0];
    let mut paths = snap.paths.clone();
    paths.sort();
    let mut expected = vec![
        dump.to_string_lossy().into_owned(),
        originals.to_string_lossy().into_owned(),
    ];
    expected.sort();
    assert_eq!(paths, expected);
}

#[test]
fn repo_debug_hides_secrets() {
    let repo = Repo {
        location: "rest:https://family:rest-secret@backup.example/repo".into(),
        env: vec![
            ("RESTIC_PASSWORD".into(), "restic-secret".into()),
            ("AWS_SECRET_ACCESS_KEY".into(), "aws-secret".into()),
        ],
        cache_dir: PathBuf::from("/data/cache/restic"),
    };
    let debug = format!("{repo:?} {repo:#?}");
    for secret in ["rest-secret", "restic-secret", "aws-secret"] {
        assert!(!debug.contains(secret), "{debug}");
    }
    assert!(
        debug.contains("rest:https://family:***@backup.example/repo"),
        "{debug}"
    );
    assert!(debug.contains("RESTIC_PASSWORD") && debug.contains("AWS_SECRET_ACCESS_KEY"));
}

#[tokio::test]
async fn unreadable_file_exit3_is_failure() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let (dump, originals) = source(&tmp);
    let locked = originals.join("2026/11/locked.jpg");
    std::fs::write(&locked, b"x").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&locked).is_ok() {
        // root 는 권한 무시. 재현 불가
        return;
    }
    let repo = repo_at(&tmp, &tmp.path().join("backup"), KEY);
    repo.init().await.unwrap();
    let err = repo.backup(&[dump, originals], "manual").await.unwrap_err();
    assert!(err.to_string().contains("locked.jpg"), "{err}");
}

#[tokio::test]
async fn missing_source_path_is_failure() {
    let tmp = TempDir::new().unwrap();
    let (dump, originals) = source(&tmp);
    std::fs::remove_file(&dump).unwrap();
    let repo = repo_at(&tmp, &tmp.path().join("backup"), KEY);
    repo.init().await.unwrap();
    assert!(repo.backup(&[dump, originals], "manual").await.is_err());
}

fn has_secret(args: impl Iterator<Item = impl AsRef<OsStr>>, secrets: &[&str]) -> bool {
    args.map(|a| a.as_ref().to_string_lossy().into_owned())
        .any(|a| secrets.iter().any(|s| a.contains(s)))
}

#[test]
fn secrets_not_in_argv() {
    let repo = Repo {
        location: "rest:https://family:rest-secret@backup.example/repo".into(),
        env: vec![
            ("RESTIC_PASSWORD".into(), "restic-secret".into()),
            ("AWS_SECRET_ACCESS_KEY".into(), "aws-secret".into()),
        ],
        cache_dir: PathBuf::from("/data/cache/restic"),
    };
    let secrets = ["rest-secret", "restic-secret", "aws-secret"];
    let cmd = repo.command(&["snapshots", "--json"]);
    let std = cmd.as_std();
    assert!(!has_secret(std.get_args(), &secrets));
    let envs: Vec<_> = std
        .get_envs()
        .filter_map(|(k, v)| Some((k.to_str()?.to_owned(), v?.to_str()?.to_owned())))
        .collect();
    assert!(envs.contains(&("RESTIC_PASSWORD".into(), "restic-secret".into())));
    assert!(envs.contains(&("AWS_SECRET_ACCESS_KEY".into(), "aws-secret".into())));
    assert!(envs.contains(&("RESTIC_REPOSITORY".into(), repo.location.clone())));

    let url = "postgres://chagokmoa:pg%40secret@db:5432/chagokmoa?sslmode=disable";
    for cmd in [
        dump::dump_command(url, Path::new("/data/cache/backup/db.dump")),
        dump::restore_command(url, Path::new("/data/cache/backup/db.dump")),
    ] {
        let std = cmd.as_std();
        assert!(!has_secret(std.get_args(), &["pg%40secret", "pg@secret"]));
        assert!(
            std.get_args()
                .any(|a| a == "--dbname=postgres://chagokmoa@db:5432/chagokmoa?sslmode=disable")
        );
        assert!(
            std.get_envs()
                .any(|(k, v)| k == "PGPASSWORD" && v == Some(OsStr::new("pg@secret")))
        );
    }
}

#[test]
fn dump_arguments() {
    let url = "postgresql://u:p@localhost/db";
    let dump = dump::dump_command(url, Path::new("/x/db.dump"));
    let args: Vec<_> = dump.as_std().get_args().collect();
    assert_eq!(dump.as_std().get_program(), "pg_dump");
    assert!(args.contains(&OsStr::new("--format=custom")));
    assert!(args.contains(&OsStr::new("--file=/x/db.dump")));

    let restore = dump::restore_command(url, Path::new("/x/db.dump"));
    let args: Vec<_> = restore.as_std().get_args().collect();
    assert_eq!(restore.as_std().get_program(), "pg_restore");
    for flag in [
        "--single-transaction",
        "--exit-on-error",
        "--no-owner",
        "--no-privileges",
        "/x/db.dump",
    ] {
        assert!(args.contains(&OsStr::new(flag)), "{flag}");
    }
    assert!(!args.contains(&OsStr::new("--clean")));

    // 비밀번호 없는 URL 은 그대로, PGPASSWORD 미설정
    let plain = dump::dump_command("postgres://u@h/db", Path::new("/x"));
    assert!(
        plain
            .as_std()
            .get_args()
            .any(|a| a == "--dbname=postgres://u@h/db")
    );
    assert!(!plain.as_std().get_envs().any(|(k, _)| k == "PGPASSWORD"));
}

#[sqlx::test]
async fn server_major_matches_version(pool: PgPool) {
    let version: String = sqlx::query_scalar("SELECT version()")
        .fetch_one(&pool)
        .await
        .unwrap();
    // 예: "PostgreSQL 17.6 on x86_64..."
    let expected: u32 = version
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.split('.').next())
        .and_then(|v| v.parse().ok())
        .unwrap();
    assert_eq!(dump::server_major(&pool).await.unwrap(), expected);
}
