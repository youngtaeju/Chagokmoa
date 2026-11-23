//! 새 버전 확인. GitHub 대신 로컬 가짜 서버.

mod common;

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    extract::{RawQuery, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chagokmoa::{state::AppState, update, users::Role};
use common::{XRW, XRW_VALUE, media_app};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::watch;

const CURRENT: &str = env!("CARGO_PKG_VERSION");
const RELEASE_URL: &str = "https://github.com/youngtaeju/Chagokmoa/releases/tag/v99.0.0";

#[derive(Default)]
struct Seen {
    requests: usize,
    user_agent: Option<String>,
    query: Option<String>,
}

#[derive(Clone)]
struct Fake {
    reply: Arc<Mutex<(StatusCode, String)>>,
    seen: Arc<Mutex<Seen>>,
}

impl Fake {
    fn reply(&self, status: StatusCode, body: Value) {
        *self.reply.lock().unwrap() = (status, body.to_string());
    }
    fn requests(&self) -> usize {
        self.seen.lock().unwrap().requests
    }
}

async fn latest(
    State(fake): State<Fake>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> (StatusCode, String) {
    {
        let mut seen = fake.seen.lock().unwrap();
        seen.requests += 1;
        seen.user_agent = headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        seen.query = query;
    }
    fake.reply.lock().unwrap().clone()
}

/// 가짜 releases/latest. 응답 기본값 v99.0.0.
async fn fake_github() -> (Fake, String) {
    let fake = Fake {
        reply: Arc::new(Mutex::new((StatusCode::OK, String::new()))),
        seen: Arc::default(),
    };
    fake.reply(
        StatusCode::OK,
        json!({ "tag_name": "v99.0.0", "html_url": RELEASE_URL }),
    );
    let app = Router::new()
        .route("/latest", get(latest))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/latest", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (fake, url)
}

/// 같은 Arc 를 공유하고 조회 주소만 가짜로 바꾼 state.
fn pointed(state: &AppState, url: &str) -> AppState {
    let mut state = state.clone();
    state.update_url = url.to_owned();
    state
}

#[test]
fn newer_compares_versions() {
    use update::newer;
    assert_eq!(newer("0.14.0", "v0.15.0"), Some("0.15.0".into()));
    assert_eq!(newer("0.14.0", "v1.0.0"), Some("1.0.0".into()));
    assert_eq!(newer("0.14.0", "v0.14.0"), None);
    assert_eq!(newer("0.14.0", "v0.9.9"), None);
    // 문자열 아닌 숫자 비교
    assert_eq!(newer("0.14.0", "v0.14.10"), Some("0.14.10".into()));
    assert_eq!(newer("1.0.0", "v1.0.1-rc.1"), None);
    assert_eq!(newer("1.0.0", "1.0.1"), None);
    assert_eq!(newer("1.0.0", "v1.0"), None);
    assert_eq!(newer("1.0.0", "latest"), None);
}

#[sqlx::test(migrations = "./migrations")]
async fn newer_release_shown_to_owner(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let (fake, url) = fake_github().await;
    update::check_once(&pointed(&app.state, &url))
        .await
        .unwrap();

    let res = app.server.get("/api/update").await;
    res.assert_status_ok();
    res.assert_json(&json!({
        "current": CURRENT,
        "latest": "99.0.0",
        "release_url": RELEASE_URL,
    }));
    let seen = fake.seen.lock().unwrap();
    assert_eq!(seen.requests, 1);
    let agent = seen.user_agent.clone().unwrap_or_default();
    assert_eq!(agent, format!("chagokmoa/{CURRENT}"));
    assert_eq!(seen.query, None);
}

#[sqlx::test(migrations = "./migrations")]
async fn same_version_not_shown(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let (fake, url) = fake_github().await;
    fake.reply(
        StatusCode::OK,
        json!({ "tag_name": format!("v{CURRENT}"), "html_url": RELEASE_URL }),
    );
    update::check_once(&pointed(&app.state, &url))
        .await
        .unwrap();

    let res = app.server.get("/api/update").await;
    res.assert_json(&json!({ "current": CURRENT, "latest": null, "release_url": null }));
}

#[sqlx::test(migrations = "./migrations")]
async fn failure_keeps_previous(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let (fake, url) = fake_github().await;
    let state = pointed(&app.state, &url);
    update::check_once(&state).await.unwrap();

    fake.reply(StatusCode::INTERNAL_SERVER_ERROR, json!({}));
    assert!(update::check_once(&state).await.is_err());
    fake.reply(StatusCode::OK, json!({}));
    assert!(update::check_once(&state).await.is_err());
    fake.reply(
        StatusCode::FORBIDDEN,
        json!({ "message": "API rate limit exceeded" }),
    );
    assert!(update::check_once(&state).await.is_err());

    let res = app.server.get("/api/update").await;
    res.assert_json(&json!({ "current": CURRENT, "latest": "99.0.0", "release_url": RELEASE_URL }));
    assert_eq!(fake.requests(), 4);
}

#[sqlx::test(migrations = "./migrations")]
async fn disabled_sends_nothing(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let (fake, url) = fake_github().await;
    let state = pointed(&app.state, &url);
    update::check_once(&state).await.unwrap();
    assert_eq!(fake.requests(), 1);

    let res = app
        .server
        .patch("/api/settings")
        .add_header(XRW, XRW_VALUE)
        .json(&json!({ "update_check": false }))
        .await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["update_check"], false);
    // 끄면 저장된 결과도 숨김
    let res = app.server.get("/api/update").await;
    res.assert_json(&json!({ "current": CURRENT, "latest": null, "release_url": null }));

    update::check_once(&state).await.unwrap();
    assert_eq!(fake.requests(), 1);
    let res = app.server.get("/api/update").await;
    assert_eq!(res.json::<Value>()["latest"], Value::Null);
}

#[sqlx::test(migrations = "./migrations")]
async fn enable_wakes_check(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let (fake, url) = fake_github().await;
    let patch = |on: bool| {
        app.server
            .patch("/api/settings")
            .add_header(XRW, XRW_VALUE)
            .json(&json!({ "update_check": on }))
    };
    patch(false).await.assert_status_ok();

    let (stop, rx) = watch::channel(false);
    let task = update::spawn(pointed(&app.state, &url), rx);
    // 기동 직후 회차는 꺼져 있어 요청 없음
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fake.requests(), 0);

    patch(true).await.assert_status_ok();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let res = app.server.get("/api/update").await;
        if res.json::<Value>()["latest"] == "99.0.0" {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "not checked after enabling"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // 첫 회차가 켜진 뒤에 돌면 저장된 깨움 신호로 1회 더 조회 가능
    assert!((1..=2).contains(&fake.requests()));

    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("task stops on shutdown")
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn update_owner_only(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    for (name, role) in [("dad", Role::Guardian), ("grandma", Role::Family)] {
        let (server, _) = app.login_as(name, role).await;
        server
            .get("/api/update")
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn settings_update_check_default_and_patch(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let res = app.server.get("/api/settings").await;
    assert_eq!(res.json::<Value>()["update_check"], true);
    let res = app
        .server
        .patch("/api/settings")
        .add_header(XRW, XRW_VALUE)
        .json(&json!({ "update_check": false }))
        .await;
    res.assert_status_ok();
    let res = app.server.get("/api/settings").await;
    assert_eq!(res.json::<Value>()["update_check"], false);
}
