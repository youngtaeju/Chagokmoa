//! 백업 설정과 상태 API. 실제 restic, 덤프는 가짜 파일, 원격은 테스트 안 REST 서버.

mod common;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use axum::{
    Router,
    body::Bytes,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use axum_test::{TestResponse, TestServer};
use chagokmoa::{
    app::build_router,
    auth::password,
    backup::{
        queries,
        restic::{Repo, RepoState},
        run::{self, DbSteps, RunStatus},
        settings::{self, BackupSettings, Remote},
    },
    error::{AppError, AppResult},
    media::storage,
    state::AppState,
    users::{self, NewUser, Role},
};
use common::{PASSWORD, XRW, XRW_VALUE, assert_error, test_config};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use sqlx::PgPool;
use tempfile::TempDir;
use uuid::Uuid;

const KEY: &str = "correct-horse-battery-staple-0123456789";
const OTHER_KEY: &str = "another-recovery-key-not-matching-000";

fn fake_dump<'a>(_url: &'a str, path: &'a Path) -> BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        tokio::fs::write(path, b"fake-dump")
            .await
            .map_err(|e| AppError::Internal(e.into()))
    })
}

fn major_17(_pool: &PgPool) -> BoxFuture<'_, AppResult<u32>> {
    Box::pin(async { Ok(17) })
}

struct App {
    state: AppState,
    owner: TestServer,
    tmp: TempDir,
}

impl App {
    fn backup_dir(&self) -> PathBuf {
        self.state.config.backup_dir.clone()
    }

    fn local_repo(&self, key: &str) -> Repo {
        BackupSettings {
            password: Some(key.to_owned()),
            ..BackupSettings::default()
        }
        .local_repo(&self.state.config)
        .unwrap()
    }

    async fn login(&self, username: &str, role: Role) -> TestServer {
        users::queries::create(
            &self.state.pool,
            NewUser {
                username: username.to_owned(),
                password_hash: Some(password::hash(PASSWORD).unwrap()),
                display_name: username.to_owned(),
                role,
                email: None,
                locale: "ko".to_owned(),
            },
        )
        .await
        .unwrap();
        let server = TestServer::builder()
            .save_cookies()
            .build(build_router(self.state.clone()))
            .unwrap();
        server
            .post("/api/auth/login")
            .json(&json!({ "username": username, "password": PASSWORD }))
            .await
            .assert_status_ok();
        server
    }

    async fn status(&self) -> Value {
        let res = self.owner.get("/api/backup").await;
        res.assert_status_ok();
        res.json()
    }

    async fn post(&self, path: &str, body: Value) -> TestResponse {
        self.owner
            .post(path)
            .add_header(XRW, XRW_VALUE)
            .json(&body)
            .await
    }

    async fn put(&self, path: &str, body: Value) -> TestResponse {
        self.owner
            .put(path)
            .add_header(XRW, XRW_VALUE)
            .json(&body)
            .await
    }

    /// 빈 로컬 폴더에 새 키로 켜기.
    async fn enable(&self) {
        self.post(
            "/api/backup/enable",
            json!({ "key": KEY, "confirmed": true }),
        )
        .await
        .assert_status_ok();
    }
}

/// 임시 DATA_DIR, 백업 폴더(`mounted` 면 생성), owner 로그인 서버.
async fn app(pool: &PgPool, mounted: bool) -> App {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config();
    config.data_dir = tmp.path().join("data");
    config.backup_dir = tmp.path().join("backup");
    storage::ensure_dirs(&config.data_dir).unwrap();
    if mounted {
        std::fs::create_dir(&config.backup_dir).unwrap();
    }
    let mut state = AppState::new(pool.clone(), config);
    state.backup_db = DbSteps {
        server_major: major_17,
        dump: fake_dump,
    };
    let owner = TestServer::new(build_router(state.clone())).unwrap();
    let mut app = App { state, owner, tmp };
    app.owner = app.login("mom", Role::Owner).await;
    app
}

async fn stored(pool: &PgPool) -> BackupSettings {
    settings::load(pool).await.unwrap()
}

/// 백업 API 전체. (메서드, 경로, 본문).
fn endpoints() -> Vec<(Method, &'static str, Value)> {
    let remote = json!({ "repository": "s3:https://s3.example/bucket", "env": {} });
    vec![
        (Method::GET, "/api/backup", Value::Null),
        (Method::PUT, "/api/backup", json!({ "enabled": false })),
        (Method::POST, "/api/backup/key", json!({})),
        (
            Method::POST,
            "/api/backup/enable",
            json!({ "key": KEY, "confirmed": true }),
        ),
        (
            Method::POST,
            "/api/backup/key/reveal",
            json!({ "password": PASSWORD }),
        ),
        (Method::POST, "/api/backup/remote/test", remote.clone()),
        (Method::PUT, "/api/backup/remote", remote),
        (Method::DELETE, "/api/backup/remote", Value::Null),
        (Method::POST, "/api/backup/run", json!({})),
    ]
}

async fn call(server: &TestServer, method: &Method, path: &str, body: &Value) -> TestResponse {
    let req = server
        .method(method.clone(), path)
        .add_header(XRW, XRW_VALUE);
    if body.is_null() {
        req.await
    } else {
        req.json(body).await
    }
}

#[sqlx::test]
async fn owner_only(pool: PgPool) {
    let app = app(&pool, true).await;
    let anonymous = TestServer::new(build_router(app.state.clone())).unwrap();
    let guardian = app.login("dad", Role::Guardian).await;
    let family = app.login("grandma", Role::Family).await;
    for (method, path, body) in endpoints() {
        let res = call(&anonymous, &method, path, &body).await;
        assert_error(&res, StatusCode::UNAUTHORIZED, "auth.required");
        for server in [&guardian, &family] {
            let res = call(server, &method, path, &body).await;
            assert_error(&res, StatusCode::FORBIDDEN, "auth.forbidden");
        }
    }
    // 아무것도 바뀌지 않음
    assert_eq!(stored(&pool).await, BackupSettings::default());
    assert!(
        std::fs::read_dir(app.backup_dir())
            .unwrap()
            .next()
            .is_none()
    );
}

#[sqlx::test]
async fn not_mounted_blocks_enable(pool: PgPool) {
    let app = app(&pool, false).await;
    let status = app.status().await;
    assert_eq!(status["mounted"], json!(false));
    assert_eq!(status["repo_state"], Value::Null);
    assert_eq!(status["key_set"], json!(false));

    let res = app.post("/api/backup/key", json!({})).await;
    assert_error(&res, StatusCode::CONFLICT, "backup.notMounted");
    let res = app
        .post(
            "/api/backup/enable",
            json!({ "key": KEY, "confirmed": true }),
        )
        .await;
    assert_error(&res, StatusCode::CONFLICT, "backup.notMounted");
    // restic init 이 폴더를 만들지 않음
    assert!(!app.backup_dir().exists());
    assert_eq!(stored(&pool).await.password, None);

    // 쓸 수 없는 폴더도 연결 안 됨으로
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir(app.backup_dir()).unwrap();
        std::fs::set_permissions(app.backup_dir(), std::fs::Permissions::from_mode(0o555)).unwrap();
        // root 는 권한 무시
        let writable = std::fs::write(app.backup_dir().join(".probe"), b"x").is_ok();
        if !writable {
            assert_eq!(app.status().await["mounted"], json!(false));
            let res = app
                .post(
                    "/api/backup/enable",
                    json!({ "key": KEY, "confirmed": true }),
                )
                .await;
            assert_error(&res, StatusCode::CONFLICT, "backup.notMounted");
        }
        std::fs::set_permissions(app.backup_dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[sqlx::test]
async fn enable_requires_confirmed(pool: PgPool) {
    let app = app(&pool, true).await;
    let status = app.status().await;
    assert_eq!(status["mounted"], json!(true));
    assert_eq!(status["repo_state"], json!("empty"));

    let res = app.post("/api/backup/key", json!({})).await;
    res.assert_status_ok();
    let key = res.json::<Value>()["key"].as_str().unwrap().to_owned();
    assert!(key.len() >= 32, "{key}");
    // 생성만, 저장 안 함
    assert_eq!(stored(&pool).await.password, None);
    let again = app.post("/api/backup/key", json!({})).await.json::<Value>();
    assert_ne!(again["key"], json!(key));

    for body in [
        json!({ "key": key }),
        json!({ "key": key, "confirmed": false }),
    ] {
        let res = app.post("/api/backup/enable", body).await;
        assert_error(&res, StatusCode::BAD_REQUEST, "backup.keyNotConfirmed");
    }
    assert_eq!(stored(&pool).await.password, None);
    assert_eq!(
        app.local_repo(&key).probe().await.unwrap(),
        RepoState::Empty
    );

    // 새 저장소에는 짧은 키 거부
    let res = app
        .post(
            "/api/backup/enable",
            json!({ "key": "short", "confirmed": true }),
        )
        .await;
    assert_error(&res, StatusCode::BAD_REQUEST, "backup.keyInvalid");

    let res = app
        .post(
            "/api/backup/enable",
            json!({ "key": key, "confirmed": true }),
        )
        .await;
    res.assert_status_ok();
    let status: Value = res.json();
    assert_eq!(status["key_set"], json!(true));
    assert_eq!(status["enabled"], json!(true));
    assert_eq!(status["repo_state"], json!("restic"));
    assert!(status["next_run"].is_string(), "{status}");
    let saved = stored(&pool).await;
    assert_eq!(saved.password.as_deref(), Some(key.as_str()));
    assert!(saved.enabled);
    assert_eq!(
        app.local_repo(&key).probe().await.unwrap(),
        RepoState::Restic
    );

    // 이미 설정됨
    let res = app.post("/api/backup/key", json!({})).await;
    assert_error(&res, StatusCode::CONFLICT, "backup.alreadyConfigured");
    let res = app
        .post(
            "/api/backup/enable",
            json!({ "key": key, "confirmed": true }),
        )
        .await;
    assert_error(&res, StatusCode::CONFLICT, "backup.alreadyConfigured");
}

#[sqlx::test]
async fn existing_repo_needs_matching_key(pool: PgPool) {
    let app = app(&pool, true).await;
    app.local_repo(KEY).init().await.unwrap();
    assert_eq!(app.status().await["repo_state"], json!("restic"));

    // 새 키 생성은 빈 폴더에서만
    let res = app.post("/api/backup/key", json!({})).await;
    assert_error(&res, StatusCode::CONFLICT, "backup.repoNotEmpty");

    let res = app
        .post("/api/backup/enable", json!({ "key": OTHER_KEY }))
        .await;
    assert_error(&res, StatusCode::CONFLICT, "backup.wrongKey");
    assert_eq!(stored(&pool).await.password, None);

    // 기존 저장소는 확인 체크 없이
    let res = app.post("/api/backup/enable", json!({ "key": KEY })).await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["key_set"], json!(true));
    assert_eq!(stored(&pool).await.password.as_deref(), Some(KEY));
}

#[sqlx::test]
async fn foreign_dir_rejected(pool: PgPool) {
    let app = app(&pool, true).await;
    std::fs::write(app.backup_dir().join("family.jpg"), b"not restic").unwrap();
    assert_eq!(app.status().await["repo_state"], json!("foreign"));

    let res = app.post("/api/backup/key", json!({})).await;
    assert_error(&res, StatusCode::CONFLICT, "backup.repoNotEmpty");
    let res = app
        .post(
            "/api/backup/enable",
            json!({ "key": KEY, "confirmed": true }),
        )
        .await;
    assert_error(&res, StatusCode::CONFLICT, "backup.foreignRepo");
    assert_eq!(stored(&pool).await.password, None);
    // 기존 파일 그대로, 저장소 안 만듦
    let names: Vec<_> = std::fs::read_dir(app.backup_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec!["family.jpg"]);
}

#[sqlx::test]
async fn update_settings_validates(pool: PgPool) {
    let app = app(&pool, true).await;
    let res = app.put("/api/backup", json!({ "enabled": true })).await;
    assert_error(&res, StatusCode::CONFLICT, "backup.notConfigured");

    for time in ["3:00", "24:00", "03:60", "03:00:00", "noon", ""] {
        let res = app.put("/api/backup", json!({ "time": time })).await;
        assert_error(&res, StatusCode::BAD_REQUEST, "backup.timeInvalid");
    }
    for retention in [
        json!({ "daily": 0, "weekly": 0, "monthly": 0 }),
        json!({ "daily": 366, "weekly": 4, "monthly": 12 }),
        json!({ "daily": 7, "weekly": 521, "monthly": 12 }),
        json!({ "daily": 7, "weekly": 4, "monthly": 241 }),
    ] {
        let res = app
            .put("/api/backup", json!({ "retention": retention }))
            .await;
        assert_error(&res, StatusCode::BAD_REQUEST, "backup.retentionInvalid");
    }
    // 검증 실패는 일부도 반영 안 함
    let res = app
        .put(
            "/api/backup",
            json!({ "time": "04:30", "retention": { "daily": 0, "weekly": 0, "monthly": 0 } }),
        )
        .await;
    assert_error(&res, StatusCode::BAD_REQUEST, "backup.retentionInvalid");
    assert_eq!(stored(&pool).await, BackupSettings::default());

    let res = app
        .put(
            "/api/backup",
            json!({ "time": "04:30", "retention": { "daily": 0, "weekly": 8, "monthly": 0 } }),
        )
        .await;
    res.assert_status_ok();
    let status: Value = res.json();
    assert_eq!(status["time"], json!("04:30"));
    assert_eq!(
        status["retention"],
        json!({ "daily": 0, "weekly": 8, "monthly": 0 })
    );
    assert_eq!(status["enabled"], json!(false));
    assert_eq!(status["next_run"], Value::Null);

    app.enable().await;
    let res = app.put("/api/backup", json!({ "enabled": false })).await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["enabled"], json!(false));
    let saved = stored(&pool).await;
    assert!(!saved.enabled);
    assert_eq!(saved.time.format("%H:%M").to_string(), "04:30");
    // 키는 그대로
    assert_eq!(saved.password.as_deref(), Some(KEY));
    let res = app.put("/api/backup", json!({ "enabled": true })).await;
    res.assert_status_ok();
    assert!(res.json::<Value>()["next_run"].is_string());
}

#[sqlx::test]
async fn key_requires_password(pool: PgPool) {
    let app = app(&pool, true).await;
    let res = app
        .post("/api/backup/key/reveal", json!({ "password": PASSWORD }))
        .await;
    assert_error(&res, StatusCode::CONFLICT, "backup.notConfigured");

    app.enable().await;
    let res = app
        .post(
            "/api/backup/key/reveal",
            json!({ "password": "wrong-password" }),
        )
        .await;
    assert_error(&res, StatusCode::UNAUTHORIZED, "auth.invalidCredentials");
    let res = app.post("/api/backup/key/reveal", json!({})).await;
    assert_error(&res, StatusCode::BAD_REQUEST, "request.invalidBody");

    let res = app
        .post("/api/backup/key/reveal", json!({ "password": PASSWORD }))
        .await;
    res.assert_status_ok();
    res.assert_json(&json!({ "key": KEY }));
}

#[sqlx::test]
async fn status_never_contains_secrets(pool: PgPool) {
    let app = app(&pool, true).await;
    app.enable().await;
    let secrets = [
        KEY,
        "aws-access-id-secret",
        "aws-secret-value",
        "rest-url-secret",
    ];
    let remotes = [
        Remote {
            repository: "s3:https://s3.example/bucket".into(),
            env: BTreeMap::from([
                ("AWS_ACCESS_KEY_ID".into(), "aws-access-id-secret".into()),
                ("AWS_SECRET_ACCESS_KEY".into(), "aws-secret-value".into()),
            ]),
        },
        Remote {
            repository: "rest:https://family:rest-url-secret@backup.example/repo/".into(),
            env: BTreeMap::new(),
        },
    ];
    // 실행 기록도 응답에 포함
    sqlx::query(
        "INSERT INTO backup_runs (id, trigger, status, finished_at, local, error)
         VALUES ($1, 'manual', 'failed', now(), $2, 'local: boom')",
    )
    .bind(Uuid::now_v7())
    .bind(json!({ "error": "boom" }))
    .execute(&pool)
    .await
    .unwrap();

    for remote in remotes {
        let mut s = stored(&pool).await;
        s.remote = Some(remote);
        settings::save(&pool, &s).await.unwrap();

        let res = app.owner.get("/api/backup").await;
        res.assert_status_ok();
        let body = res.text();
        for secret in secrets {
            assert!(!body.contains(secret), "{secret} in {body}");
        }
        let status: Value = res.json();
        assert_eq!(status["key_set"], json!(true));
        assert_eq!(status["runs"][0]["error"], json!("local: boom"));
        assert_eq!(status["last_run"]["status"], json!("failed"));
    }
    let status = app.status().await;
    assert_eq!(
        status["remote"],
        json!({
            "repository": "rest:https://family:***@backup.example/repo/",
            "configured": false,
        })
    );
}

#[sqlx::test]
async fn run_returns_run_id(pool: PgPool) {
    let app = app(&pool, true).await;
    let res = app.post("/api/backup/run", json!({})).await;
    assert_error(&res, StatusCode::CONFLICT, "backup.notConfigured");

    app.enable().await;
    std::fs::write(app.state.config.data_dir.join("originals/a.jpg"), b"photo").unwrap();
    let res = app.post("/api/backup/run", json!({})).await;
    res.assert_status(StatusCode::ACCEPTED);
    let run_id: Uuid = res.json::<Value>()["run_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let mut row = None;
    for _ in 0..300 {
        let r = queries::get(&pool, run_id).await.unwrap().expect("run row");
        if r.status != RunStatus::Running {
            row = Some(r);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let row = row.expect("run finished");
    assert_eq!(row.status, RunStatus::Success, "{:?}", row.error);

    let status = app.status().await;
    assert_eq!(status["runs"][0]["id"], json!(run_id));
    assert_eq!(status["last_run"]["id"], json!(run_id));
    assert_eq!(status["last_run"]["status"], json!("success"));
    assert_eq!(status["last_run"]["trigger"], json!("manual"));
    assert!(status["last_run"]["local"]["snapshot_id"].is_string());
}

#[sqlx::test]
async fn running_shown_only_while_locked(pool: PgPool) {
    let app = app(&pool, true).await;
    app.enable().await;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO backup_runs (id, trigger, status, phase) VALUES ($1, 'scheduled', 'running', 'local')",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();

    // 크래시로 남은 행. 잠금 없음
    let status = app.status().await;
    assert_eq!(status["runs"][0]["id"], json!(id));
    assert_eq!(status["runs"][0]["status"], json!("failed"));
    assert_eq!(status["runs"][0]["phase"], Value::Null);
    assert_eq!(status["runs"][0]["error"], json!("interrupted"));
    assert_eq!(status["last_run"]["id"], json!(id));

    // 다른 프로세스가 실행 중
    let mut other = pool.acquire().await.unwrap();
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(run::LOCK_KEY)
        .fetch_one(&mut *other)
        .await
        .unwrap();
    assert!(locked);
    let status = app.status().await;
    assert_eq!(status["runs"][0]["status"], json!("running"));
    assert_eq!(status["runs"][0]["phase"], json!("local"));
    assert_eq!(status["last_run"], Value::Null);
    let res = app.post("/api/backup/run", json!({})).await;
    assert_error(&res, StatusCode::CONFLICT, "backup.running");
}

/// restic REST 프로토콜 최소 구현. URL 경로를 폴더에 그대로 대응.
async fn rest_handler(State(root): State<PathBuf>, req: Request) -> Response {
    let method = req.method().clone();
    let headers: HeaderMap = req.headers().clone();
    let path = req.uri().path().trim_start_matches('/').to_owned();
    let create = req.uri().query() == Some("create=true");
    if path.split('/').any(|p| p == "..") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let body: Bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
        .await
        .unwrap();
    let target = root.join(&path);
    if create && method == Method::POST {
        for dir in ["data", "keys", "locks", "snapshots", "index"] {
            std::fs::create_dir_all(target.join(dir)).unwrap();
        }
        return StatusCode::OK.into_response();
    }
    if path.ends_with('/') || target.is_dir() {
        if method != Method::GET {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        let Ok(entries) = std::fs::read_dir(&target) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let files: Vec<(String, u64)> = entries
            .map(|e| e.unwrap())
            .map(|e| {
                let len = e.metadata().unwrap().len();
                (e.file_name().to_string_lossy().into_owned(), len)
            })
            .collect();
        let v2 = headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("rest.v2"));
        return if v2 {
            let list: Vec<Value> = files
                .iter()
                .map(|(name, size)| json!({ "name": name, "size": size }))
                .collect();
            (
                [(header::CONTENT_TYPE, "application/vnd.x.restic.rest.v2")],
                serde_json::to_vec(&list).unwrap(),
            )
                .into_response()
        } else {
            let list: Vec<&String> = files.iter().map(|(name, _)| name).collect();
            (
                [(header::CONTENT_TYPE, "application/vnd.x.restic.rest.v1")],
                serde_json::to_vec(&list).unwrap(),
            )
                .into_response()
        };
    }
    match method {
        Method::HEAD => match std::fs::metadata(&target) {
            Ok(m) => ([(header::CONTENT_LENGTH, m.len().to_string())], ()).into_response(),
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        },
        Method::GET => {
            let Ok(data) = std::fs::read(&target) else {
                return StatusCode::NOT_FOUND.into_response();
            };
            let range = headers
                .get(header::RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("bytes="))
                .and_then(|v| v.split_once('-'));
            match range {
                Some((start, end)) => {
                    let start: usize = start.parse().unwrap();
                    let end = end
                        .parse::<usize>()
                        .map_or(data.len(), |e| (e + 1).min(data.len()));
                    (StatusCode::PARTIAL_CONTENT, data[start..end].to_vec()).into_response()
                }
                None => data.into_response(),
            }
        }
        Method::POST => {
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(&target, &body).unwrap();
            StatusCode::OK.into_response()
        }
        Method::DELETE => {
            let _ = std::fs::remove_file(&target);
            StatusCode::OK.into_response()
        }
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// 실제 포트의 REST 서버. 반환은 `127.0.0.1:{port}`.
async fn rest_server(root: PathBuf) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().fallback(rest_handler).with_state(root);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("127.0.0.1:{}", addr.port())
}

fn remote_repo(app: &App, repository: &str, key: &str) -> Repo {
    BackupSettings {
        password: Some(key.to_owned()),
        remote: Some(Remote {
            repository: repository.to_owned(),
            env: BTreeMap::new(),
        }),
        ..BackupSettings::default()
    }
    .remote_repo(&app.state.config)
    .unwrap()
}

#[sqlx::test]
async fn remote_saved_only_after_test_passes(pool: PgPool) {
    let app = app(&pool, true).await;
    let host = rest_server(app.tmp.path().join("rest")).await;
    let fresh = format!("rest:http://family:rest-url-secret@{host}/fresh/");
    let other = format!("rest:http://family:rest-url-secret@{host}/other/");

    // 키 없으면 시험 불가
    let res = app
        .post("/api/backup/remote/test", json!({ "repository": fresh }))
        .await;
    assert_error(&res, StatusCode::CONFLICT, "backup.notConfigured");
    app.enable().await;

    // 허용 목록 밖 주소와 인증값
    for repository in [
        "sftp:user@host:/srv/restic",
        "rclone:remote:path",
        "swift:container:/path",
        "/srv/restic",
        "local:/srv/restic",
    ] {
        let body = json!({ "repository": repository, "env": {} });
        let res = app.post("/api/backup/remote/test", body.clone()).await;
        assert_error(&res, StatusCode::BAD_REQUEST, "backup.remoteUnsupported");
        let res = app.put("/api/backup/remote", body).await;
        assert_error(&res, StatusCode::BAD_REQUEST, "backup.remoteUnsupported");
    }
    for (repository, name) in [
        ("s3:https://s3.example/bucket", "LD_PRELOAD"),
        ("s3:https://s3.example/bucket", "PATH"),
        ("s3:https://s3.example/bucket", "RESTIC_PASSWORD"),
        ("b2:bucket:path", "AWS_SECRET_ACCESS_KEY"),
        (fresh.as_str(), "AWS_ACCESS_KEY_ID"),
    ] {
        let body = json!({ "repository": repository, "env": { name: "x" } });
        let res = app.post("/api/backup/remote/test", body).await;
        res.assert_status(StatusCode::BAD_REQUEST);
        res.assert_json(&json!({ "error": "backup.remoteEnvInvalid", "params": { "name": name } }));
    }
    assert_eq!(stored(&pool).await.remote, None);

    // 다른 키로 만든 원격
    remote_repo(&app, &other, OTHER_KEY).init().await.unwrap();
    let res = app
        .post("/api/backup/remote/test", json!({ "repository": other }))
        .await;
    res.assert_status_ok();
    res.assert_json(&json!({ "ok": false, "state": "wrong_key", "message": null }));
    let res = app
        .put("/api/backup/remote", json!({ "repository": other }))
        .await;
    assert_error(&res, StatusCode::CONFLICT, "backup.wrongKey");
    assert_eq!(stored(&pool).await.remote, None);

    // 시험은 저장도 초기화도 안 함
    let res = app
        .post(
            "/api/backup/remote/test",
            json!({ "repository": fresh, "env": {} }),
        )
        .await;
    res.assert_status_ok();
    res.assert_json(&json!({ "ok": true, "state": "empty", "message": null }));
    assert_eq!(stored(&pool).await.remote, None);
    assert_eq!(app.status().await["remote"], Value::Null);
    assert_eq!(
        remote_repo(&app, &fresh, KEY).probe().await.unwrap(),
        RepoState::Empty
    );

    // 저장은 같은 키로 초기화 뒤
    let res = app
        .put(
            "/api/backup/remote",
            json!({ "repository": fresh, "env": {} }),
        )
        .await;
    res.assert_status_ok();
    let status: Value = res.json();
    let masked = format!("rest:http://family:***@{host}/fresh/");
    assert_eq!(
        status["remote"],
        json!({ "repository": masked, "configured": false })
    );
    assert!(!res.text().contains("rest-url-secret"));
    let saved = stored(&pool).await.remote.unwrap();
    assert_eq!(saved.repository, fresh);
    assert_eq!(
        remote_repo(&app, &fresh, KEY).probe().await.unwrap(),
        RepoState::Restic
    );

    // 이미 저장소면 열림 확인 후 저장
    let res = app
        .post("/api/backup/remote/test", json!({ "repository": fresh }))
        .await;
    res.assert_json(&json!({ "ok": true, "state": "restic", "message": null }));

    let res = app
        .owner
        .delete("/api/backup/remote")
        .add_header(XRW, XRW_VALUE)
        .await;
    res.assert_status(StatusCode::NO_CONTENT);
    assert_eq!(stored(&pool).await.remote, None);
    assert_eq!(stored(&pool).await.password.as_deref(), Some(KEY));
}

/// 응답 하나는 성공, 나머지는 성공 또는 허용된 409.
fn one_ok_rest_conflict(results: [&TestResponse; 2], allowed: &[&str]) {
    let ok = results
        .iter()
        .filter(|r| r.status_code() == StatusCode::OK)
        .count();
    assert!(ok >= 1, "no success");
    for res in results {
        if res.status_code() != StatusCode::OK {
            res.assert_status(StatusCode::CONFLICT);
            let key = res.json::<Value>()["error"].as_str().unwrap().to_owned();
            assert!(allowed.contains(&key.as_str()), "{key}");
        }
    }
}

fn key_files(repo_dir: &Path) -> usize {
    std::fs::read_dir(repo_dir.join("keys")).unwrap().count()
}

#[sqlx::test]
async fn concurrent_enable_inits_once(pool: PgPool) {
    let app = app(&pool, true).await;
    let body = json!({ "key": KEY, "confirmed": true });
    let (a, b) = tokio::join!(
        app.post("/api/backup/enable", body.clone()),
        app.post("/api/backup/enable", body),
    );
    one_ok_rest_conflict([&a, &b], &["backup.running", "backup.alreadyConfigured"]);
    assert_eq!(
        a.status_code() == StatusCode::OK,
        b.status_code() != StatusCode::OK
    );

    // 키 하나, 저장한 키로 열림
    assert_eq!(key_files(&app.backup_dir()), 1);
    assert_eq!(stored(&pool).await.password.as_deref(), Some(KEY));
    assert_eq!(
        app.local_repo(KEY).probe().await.unwrap(),
        RepoState::Restic
    );
    app.local_repo(KEY).check().await.unwrap();
}

#[sqlx::test]
async fn concurrent_remote_save_inits_once(pool: PgPool) {
    let app = app(&pool, true).await;
    let root = app.tmp.path().join("rest");
    let host = rest_server(root.clone()).await;
    app.enable().await;
    let repository = format!("rest:http://family:rest-url-secret@{host}/fresh/");
    let body = json!({ "repository": repository });
    let (a, b) = tokio::join!(
        app.put("/api/backup/remote", body.clone()),
        app.put("/api/backup/remote", body),
    );
    // 먼저 끝난 뒤 시작했으면 둘째는 기존 저장소로 열림 확인 후 저장
    one_ok_rest_conflict([&a, &b], &["backup.running"]);

    assert_eq!(key_files(&root.join("fresh")), 1);
    assert_eq!(stored(&pool).await.remote.unwrap().repository, repository);
    let repo = remote_repo(&app, &repository, KEY);
    assert_eq!(repo.probe().await.unwrap(), RepoState::Restic);
}

#[sqlx::test]
async fn setup_refused_while_run_holds_lock(pool: PgPool) {
    let app = app(&pool, true).await;
    let mut other = pool.acquire().await.unwrap();
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(run::LOCK_KEY)
        .fetch_one(&mut *other)
        .await
        .unwrap();
    assert!(locked);
    let res = app
        .post(
            "/api/backup/enable",
            json!({ "key": KEY, "confirmed": true }),
        )
        .await;
    assert_error(&res, StatusCode::CONFLICT, "backup.running");
    assert!(
        std::fs::read_dir(app.backup_dir())
            .unwrap()
            .next()
            .is_none()
    );
    assert_eq!(stored(&pool).await.password, None);
}
