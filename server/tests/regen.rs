mod common;

use std::{path::PathBuf, time::Duration};

use axum::http::StatusCode;
use chagokmoa::{
    assets::{self, Asset, AssetStatus, TakenAtSource},
    jobs::{self, Job, worker},
    media::{regen, storage},
    settings,
    users::Role,
};
use chrono::{DateTime, Utc};
use common::{
    AssetSeed, MediaApp, XRW, XRW_VALUE, assert_error, insert_asset, media_app, upload_bytes,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(path).unwrap()
}

/// 픽스처 업로드 + finalize + 처리. `ready` 자산 id.
async fn ready_photo(app: &MediaApp, name: &str) -> Uuid {
    let id = finalize(app, &fixture(name), name).await;
    assert!(worker::run_next(&app.state).await.unwrap());
    assert_eq!(asset(app, id).await.status, AssetStatus::Ready);
    id
}

/// 업로드 + finalize. 처리 작업만 큐에.
async fn finalize(app: &MediaApp, bytes: &[u8], filename: &str) -> Uuid {
    let upload = upload_bytes(app, bytes, filename).await;
    let res = app
        .server
        .post(&format!("/api/uploads/{upload}/finalize"))
        .add_header(XRW, XRW_VALUE)
        .json(&json!({}))
        .await;
    res.assert_status(StatusCode::CREATED);
    res.json::<Value>()["id"].as_str().unwrap().parse().unwrap()
}

async fn asset(app: &MediaApp, id: Uuid) -> Asset {
    assets::queries::get(&app.state.pool, id, app.user.id)
        .await
        .unwrap()
        .unwrap()
}

fn dir(app: &MediaApp, id: Uuid) -> PathBuf {
    storage::derivative_dir(app.data.path(), id)
}

/// `derivatives/` 아래 uuid 이외 이름(임시 디렉터리 잔재).
fn leftovers(app: &MediaApp) -> Vec<String> {
    std::fs::read_dir(app.data.path().join("derivatives"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| Uuid::try_parse(name).is_err())
        .collect()
}

struct JobRow {
    kind: String,
    status: String,
    asset_id: Option<String>,
    last_error: Option<String>,
}

async fn job_rows(pool: &PgPool) -> Vec<JobRow> {
    sqlx::query_as!(
        JobRow,
        r#"SELECT kind, status::text AS "status!", payload ->> 'asset_id' AS asset_id, last_error
           FROM jobs ORDER BY created_at"#
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn regen_replaces_derivatives_keeps_ready(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let id = ready_photo(&app, "orient6.jpg").await;
    let taken = "2023-01-02T03:04:05Z".parse::<DateTime<Utc>>().unwrap();
    assets::queries::update_meta(&pool, id, Some(Some("메모")), Some(taken))
        .await
        .unwrap();
    std::fs::write(dir(&app, id).join("stale.txt"), b"x").unwrap();
    assert!(!dir(&app, id).join("preview.avif").exists());
    settings::queries::set(&pool, "derivatives_avif", json!(true))
        .await
        .unwrap();

    let queued = regen::enqueue(&pool, app.data.path(), false).await.unwrap();
    assert_eq!(queued, 1);
    // 대기 중에도 상태 유지.
    assert_eq!(asset(&app, id).await.status, AssetStatus::Ready);
    assert!(worker::run_next(&app.state).await.unwrap());

    let a = asset(&app, id).await;
    assert_eq!(a.status, AssetStatus::Ready);
    assert_eq!(a.caption.as_deref(), Some("메모"));
    assert_eq!(a.taken_at, taken);
    assert_eq!(a.taken_at_source, TakenAtSource::Manual);
    assert_eq!(a.derivatives["preview"]["formats"], json!(["webp", "avif"]));
    assert!(!dir(&app, id).join("stale.txt").exists());
    for file in ["thumb.webp", "preview.webp", "preview.avif", "large.webp"] {
        assert!(dir(&app, id).join(file).is_file(), "{file}");
    }
    assert!(leftovers(&app).is_empty());
    assert!(job_rows(&pool).await.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn regen_failure_keeps_old(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let id = ready_photo(&app, "orient6.jpg").await;
    let thumb = std::fs::read(dir(&app, id).join("thumb.webp")).unwrap();
    let before = asset(&app, id).await;
    // 원본 손상. 형식 판별만 통과하는 JPEG 머리.
    let rel = sqlx::query_scalar!("SELECT original_path FROM assets WHERE id = $1", id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let original = app.data.path().join(rel);
    std::fs::write(&original, [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 4]).unwrap();

    assert_eq!(
        regen::enqueue(&pool, app.data.path(), false).await.unwrap(),
        1
    );
    assert!(worker::run_next(&app.state).await.unwrap());

    let a = asset(&app, id).await;
    assert_eq!(a.status, AssetStatus::Ready);
    assert_eq!(a.processing_error, None);
    assert_eq!(a.derivatives, before.derivatives);
    assert_eq!(a.blurhash, before.blurhash);
    assert_eq!(
        std::fs::read(dir(&app, id).join("thumb.webp")).unwrap(),
        thumb
    );
    assert!(leftovers(&app).is_empty());
    let rows = job_rows(&pool).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "regenerate_derivatives");
    assert_eq!(rows[0].status, "failed");
    assert!(rows[0].last_error.is_some());
    assert!(
        !rows[0]
            .last_error
            .as_deref()
            .unwrap()
            .contains(&*app.data.path().to_string_lossy())
    );

    // 다시 넣으면 실패 행 대체.
    assert_eq!(
        regen::enqueue(&pool, app.data.path(), false).await.unwrap(),
        1
    );
    let rows = job_rows(&pool).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "queued");
}

#[sqlx::test(migrations = "./migrations")]
async fn regen_restores_stale_when_live_missing(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let id = ready_photo(&app, "orient6.jpg").await;
    let thumb = std::fs::read(dir(&app, id).join("thumb.webp")).unwrap();
    // 두 이름 변경 사이 중단 상태. 라이브 없음, `.stale` 만 존재. 원본 손상.
    std::fs::rename(
        dir(&app, id),
        dir(&app, id).with_extension(regen::STALE_EXT),
    )
    .unwrap();
    let rel = sqlx::query_scalar!("SELECT original_path FROM assets WHERE id = $1", id)
        .fetch_one(&pool)
        .await
        .unwrap();
    std::fs::write(
        app.data.path().join(rel),
        [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 4],
    )
    .unwrap();
    jobs::enqueue(&pool, &Job::RegenerateDerivatives { asset_id: id })
        .await
        .unwrap();

    assert!(worker::run_next(&app.state).await.unwrap());

    assert_eq!(
        std::fs::read(dir(&app, id).join("thumb.webp")).unwrap(),
        thumb
    );
    assert!(leftovers(&app).is_empty());
    assert_eq!(job_rows(&pool).await[0].status, "failed");
}

#[sqlx::test(migrations = "./migrations")]
async fn regen_includes_trash(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let id = ready_photo(&app, "orient6.jpg").await;
    app.server
        .delete(&format!("/api/assets/{id}"))
        .add_header(XRW, XRW_VALUE)
        .await
        .assert_status(StatusCode::NO_CONTENT);
    std::fs::remove_dir_all(dir(&app, id)).unwrap();
    // 처리 중 자산과 원본 없는 자산은 제외.
    let mut processing = AssetSeed::photo(app.user.id);
    processing.status = "processing";
    insert_asset(&pool, processing).await;
    insert_asset(&pool, AssetSeed::photo(app.user.id)).await;

    assert_eq!(
        regen::enqueue(&pool, app.data.path(), false).await.unwrap(),
        1
    );
    assert!(worker::run_next(&app.state).await.unwrap());

    let a = asset(&app, id).await;
    assert!(a.deleted_at.is_some());
    assert_eq!(a.status, AssetStatus::Ready);
    assert!(dir(&app, id).join("thumb.webp").is_file());
    assert!(job_rows(&pool).await.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn regen_missing_only(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let kept = ready_photo(&app, "orient6.jpg").await;
    let missing = ready_photo(&app, "plain.png").await;
    std::fs::remove_dir_all(dir(&app, missing)).unwrap();
    let kept_thumb = dir(&app, kept).join("thumb.webp");
    let kept_mtime = std::fs::metadata(&kept_thumb).unwrap().modified().unwrap();

    assert_eq!(
        regen::enqueue(&pool, app.data.path(), true).await.unwrap(),
        1
    );
    // 대기 중인 자산은 다시 넣지 않음.
    assert_eq!(
        regen::enqueue(&pool, app.data.path(), true).await.unwrap(),
        0
    );
    let rows = job_rows(&pool).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].asset_id, Some(missing.to_string()));

    assert!(worker::run_next(&app.state).await.unwrap());
    assert!(!worker::run_next(&app.state).await.unwrap());
    assert!(dir(&app, missing).join("thumb.webp").is_file());
    assert_eq!(
        std::fs::metadata(&kept_thumb).unwrap().modified().unwrap(),
        kept_mtime
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn regen_after_process_jobs(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let old = ready_photo(&app, "orient6.jpg").await;
    assert_eq!(
        regen::enqueue(&pool, app.data.path(), false).await.unwrap(),
        1
    );
    // 재생성보다 나중에 들어온 새 업로드.
    let new = finalize(&app, &fixture("plain.png"), "b.png").await;

    let kinds = ["process_asset", "regenerate_derivatives"];
    let first = jobs::claim(&pool, &kinds).await.unwrap().unwrap();
    assert_eq!(first.job, Job::ProcessAsset { asset_id: new });
    let second = jobs::claim(&pool, &kinds).await.unwrap().unwrap();
    assert_eq!(second.job, Job::RegenerateDerivatives { asset_id: old });
}

#[sqlx::test(migrations = "./migrations")]
async fn regen_and_process_same_asset_serialize(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let id = ready_photo(&app, "orient6.jpg").await;
    std::fs::write(dir(&app, id).join("stale.txt"), b"x").unwrap();
    assert_eq!(
        regen::enqueue(&pool, app.data.path(), false).await.unwrap(),
        1
    );
    jobs::enqueue(&pool, &Job::ProcessAsset { asset_id: id })
        .await
        .unwrap();

    // 락 보유 중에는 두 작업 모두 파생물에 손대지 않음.
    let held = app.state.derivative_locks.acquire(id).await;
    let tasks: Vec<_> = (0..2)
        .map(|_| {
            let state = app.state.clone();
            tokio::spawn(async move { worker::run_next(&state).await })
        })
        .collect();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let running = sqlx::query_scalar!(
            r#"SELECT count(*) AS "count!" FROM jobs WHERE status = 'running'"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        if running == 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "running jobs {running}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(dir(&app, id).join("stale.txt").is_file());
    assert!(leftovers(&app).is_empty());
    drop(held);
    for task in tasks {
        assert!(task.await.unwrap().unwrap());
    }

    let asset = asset(&app, id).await;
    assert_eq!(asset.status, AssetStatus::Ready);
    let mut files: Vec<String> = std::fs::read_dir(dir(&app, id))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["large.webp", "preview.webp", "thumb.webp"]);
    assert!(leftovers(&app).is_empty());
    assert!(job_rows(&pool).await.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn regen_api_owner_only(pool: PgPool) {
    let app = media_app(&pool, Role::Owner).await;
    let id = ready_photo(&app, "orient6.jpg").await;
    std::fs::remove_dir_all(dir(&app, id)).unwrap();
    ready_photo(&app, "plain.png").await;

    let (guardian, _) = app.login_as("dad", Role::Guardian).await;
    let res = guardian
        .post("/api/derivatives/regenerate")
        .add_header(XRW, XRW_VALUE)
        .json(&json!({ "missing_only": false }))
        .await;
    assert_error(&res, StatusCode::FORBIDDEN, "auth.forbidden");
    let res = guardian.get("/api/derivatives/regenerate").await;
    assert_error(&res, StatusCode::FORBIDDEN, "auth.forbidden");
    assert!(job_rows(&pool).await.is_empty());

    let res = app
        .server
        .post("/api/derivatives/regenerate")
        .add_header(XRW, XRW_VALUE)
        .json(&json!({ "missing_only": true }))
        .await;
    res.assert_status_ok();
    res.assert_json(&json!({ "queued": 1 }));

    let res = app.server.get("/api/derivatives/regenerate").await;
    res.assert_status_ok();
    res.assert_json(&json!({ "remaining": 1, "failed": 0 }));

    assert!(worker::run_next(&app.state).await.unwrap());
    let res = app.server.get("/api/derivatives/regenerate").await;
    res.assert_json(&json!({ "remaining": 0, "failed": 0 }));
    assert!(dir(&app, id).join("thumb.webp").is_file());
}
