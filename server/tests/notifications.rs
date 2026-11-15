mod common;

use std::time::Duration;

use axum::http::StatusCode;
use axum_test::TestServer;
use base64::{Engine, engine::general_purpose::STANDARD};
use chagokmoa::{
    babies::{self, BabyInput},
    notifications::record::record_upload,
    users::{Role, User},
};
use common::{
    AssetSeed, MediaApp, OFFSET_STREAM, TUS_RESUMABLE, TUS_VERSION, XRW, XRW_VALUE, insert_asset,
    media_app, server_as,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, PartialEq)]
struct Row {
    kind: String,
    actor_id: Uuid,
    asset_id: Option<Uuid>,
    story_id: Option<Uuid>,
    milestone_id: Option<Uuid>,
    asset_comment_id: Option<Uuid>,
    story_comment_id: Option<Uuid>,
}

/// 대상 없는 줄. 대상은 구조체 갱신 문법으로 채움.
fn row(kind: &str, actor: &User) -> Row {
    Row {
        kind: kind.to_string(),
        actor_id: actor.id,
        asset_id: None,
        story_id: None,
        milestone_id: None,
        asset_comment_id: None,
        story_comment_id: None,
    }
}

async fn rows(pool: &PgPool, user: &User) -> Vec<Row> {
    sqlx::query_as!(
        Row,
        "SELECT kind, actor_id, asset_id, story_id, milestone_id, asset_comment_id, story_comment_id
         FROM notifications WHERE user_id = $1 ORDER BY created_at, id",
        user.id
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn total(pool: &PgPool) -> i64 {
    sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM notifications"#)
        .fetch_one(pool)
        .await
        .unwrap()
}

struct Family {
    mom: (TestServer, User),
    dad: (TestServer, User),
    grandma: (TestServer, User),
}

async fn family(pool: &PgPool) -> Family {
    Family {
        mom: server_as(pool, "mom", Role::Owner).await,
        dad: server_as(pool, "dad", Role::Guardian).await,
        grandma: server_as(pool, "grandma", Role::Family).await,
    }
}

fn mention(user: &User) -> String {
    format!("<@{}>", user.id)
}

fn id(v: &Value) -> Uuid {
    v["id"].as_str().unwrap().parse().unwrap()
}

async fn post_created(server: &TestServer, url: &str, body: Value) -> Value {
    let res = server
        .post(url)
        .add_header(XRW, XRW_VALUE)
        .json(&body)
        .await;
    res.assert_status(StatusCode::CREATED);
    res.json()
}

async fn create_story(server: &TestServer, body: &str) -> Uuid {
    let story = post_created(
        server,
        "/api/stories",
        json!({ "entry_date": "2024-06-01", "body": body, "baby_ids": [], "asset_ids": [] }),
    )
    .await;
    id(&story)
}

async fn comment_asset(server: &TestServer, asset: Uuid, body: &str) -> Uuid {
    let c = post_created(
        server,
        &format!("/api/assets/{asset}/comments"),
        json!({ "body": body }),
    )
    .await;
    id(&c)
}

async fn comment_story(server: &TestServer, story: Uuid, body: &str) -> Uuid {
    let c = post_created(
        server,
        &format!("/api/stories/{story}/comments"),
        json!({ "body": body }),
    )
    .await;
    id(&c)
}

#[sqlx::test]
async fn story_notifies_everyone_but_author(pool: PgPool) {
    let f = family(&pool).await;
    let story = create_story(&f.mom.0, "오늘의 이야기").await;
    for user in [&f.dad.1, &f.grandma.1] {
        let want = Row {
            story_id: Some(story),
            ..row("story", &f.mom.1)
        };
        assert_eq!(rows(&pool, user).await, [want]);
    }
    assert!(rows(&pool, &f.mom.1).await.is_empty());
}

#[sqlx::test]
async fn story_mention_replaces_story_row(pool: PgPool) {
    let f = family(&pool).await;
    let body = format!("{} 보세요", mention(&f.grandma.1));
    let story = create_story(&f.mom.0, &body).await;
    let want = Row {
        story_id: Some(story),
        ..row("mention", &f.mom.1)
    };
    assert_eq!(rows(&pool, &f.grandma.1).await, [want]);
    let want = Row {
        story_id: Some(story),
        ..row("story", &f.mom.1)
    };
    assert_eq!(rows(&pool, &f.dad.1).await, [want]);
    assert!(rows(&pool, &f.mom.1).await.is_empty());
}

#[sqlx::test]
async fn milestone_notifies_everyone_but_creator(pool: PgPool) {
    let f = family(&pool).await;
    let baby = babies::queries::create(
        &pool,
        BabyInput {
            name: "하나".to_string(),
            birth_date: "2024-01-01".parse().unwrap(),
            sex: None,
        },
    )
    .await
    .unwrap()
    .id;
    let milestone = post_created(
        &f.dad.0,
        &format!("/api/babies/{baby}/milestones"),
        json!({ "custom_label": "첫 눈", "achieved_on": "2024-02-01", "asset_ids": [] }),
    )
    .await;
    let milestone = id(&milestone);
    for user in [&f.mom.1, &f.grandma.1] {
        let want = Row {
            milestone_id: Some(milestone),
            ..row("milestone", &f.dad.1)
        };
        assert_eq!(rows(&pool, user).await, [want]);
    }
    assert!(rows(&pool, &f.dad.1).await.is_empty());
}

#[sqlx::test]
async fn asset_comment_notifies_owner_and_prior_commenters(pool: PgPool) {
    let f = family(&pool).await;
    let asset = insert_asset(&pool, AssetSeed::photo(f.mom.1.id)).await;
    let first = comment_asset(&f.grandma.0, asset, "예쁘다").await;
    let first_row = Row {
        asset_id: Some(asset),
        asset_comment_id: Some(first),
        ..row("comment", &f.grandma.1)
    };
    assert_eq!(rows(&pool, &f.mom.1).await, [first_row]);
    assert!(rows(&pool, &f.dad.1).await.is_empty());
    assert!(rows(&pool, &f.grandma.1).await.is_empty());

    let second = comment_asset(&f.dad.0, asset, "그러게요").await;
    let second_row = |actor: &User| Row {
        asset_id: Some(asset),
        asset_comment_id: Some(second),
        ..row("comment", actor)
    };
    let first_row = Row {
        asset_id: Some(asset),
        asset_comment_id: Some(first),
        ..row("comment", &f.grandma.1)
    };
    assert_eq!(
        rows(&pool, &f.mom.1).await,
        [first_row, second_row(&f.dad.1)]
    );
    assert_eq!(rows(&pool, &f.grandma.1).await, [second_row(&f.dad.1)]);
    assert!(rows(&pool, &f.dad.1).await.is_empty());
}

#[sqlx::test]
async fn comment_mention_owner_gets_one_row(pool: PgPool) {
    let f = family(&pool).await;
    let asset = insert_asset(&pool, AssetSeed::photo(f.mom.1.id)).await;
    let body = format!("{} 이거 봐요", mention(&f.mom.1));
    let comment = comment_asset(&f.dad.0, asset, &body).await;
    let want = Row {
        asset_id: Some(asset),
        asset_comment_id: Some(comment),
        ..row("mention", &f.dad.1)
    };
    assert_eq!(rows(&pool, &f.mom.1).await, [want]);
    assert!(rows(&pool, &f.grandma.1).await.is_empty());
    assert!(rows(&pool, &f.dad.1).await.is_empty());
}

#[sqlx::test]
async fn story_comment_notifies_author_and_prior_commenters(pool: PgPool) {
    let f = family(&pool).await;
    let story = create_story(&f.mom.0, "오늘의 이야기").await;
    let first = comment_story(&f.grandma.0, story, "좋네요").await;
    let story_row = || Row {
        story_id: Some(story),
        ..row("story", &f.mom.1)
    };
    let first_row = || Row {
        story_id: Some(story),
        story_comment_id: Some(first),
        ..row("comment", &f.grandma.1)
    };
    assert_eq!(rows(&pool, &f.mom.1).await, [first_row()]);
    assert_eq!(rows(&pool, &f.grandma.1).await, [story_row()]);
    assert_eq!(rows(&pool, &f.dad.1).await, [story_row()]);

    let second = comment_story(&f.dad.0, story, "그러게요").await;
    let second_row = || Row {
        story_id: Some(story),
        story_comment_id: Some(second),
        ..row("comment", &f.dad.1)
    };
    assert_eq!(rows(&pool, &f.mom.1).await, [first_row(), second_row()]);
    assert_eq!(rows(&pool, &f.grandma.1).await, [story_row(), second_row()]);
    assert_eq!(rows(&pool, &f.dad.1).await, [story_row()]);
}

#[sqlx::test]
async fn disabled_user_not_notified(pool: PgPool) {
    let f = family(&pool).await;
    sqlx::query!(
        "UPDATE users SET disabled_at = now() WHERE id = $1",
        f.grandma.1.id
    )
    .execute(&pool)
    .await
    .unwrap();
    create_story(&f.mom.0, "오늘의 이야기").await;
    let asset = insert_asset(&pool, AssetSeed::photo(f.mom.1.id)).await;
    let body = format!("{} 보세요", mention(&f.grandma.1));
    comment_asset(&f.dad.0, asset, &body).await;
    assert!(rows(&pool, &f.grandma.1).await.is_empty());
    assert_eq!(rows(&pool, &f.mom.1).await.len(), 1);
}

#[sqlx::test]
async fn edit_does_not_notify(pool: PgPool) {
    let f = family(&pool).await;
    let asset = insert_asset(&pool, AssetSeed::photo(f.mom.1.id)).await;
    let comment = comment_asset(&f.dad.0, asset, "안녕").await;
    let before = total(&pool).await;
    assert_eq!(before, 1);
    f.dad
        .0
        .patch(&format!("/api/comments/{comment}"))
        .add_header(XRW, XRW_VALUE)
        .json(&json!({ "body": format!("{} 안녕", mention(&f.grandma.1)) }))
        .await
        .assert_status_ok();
    assert_eq!(total(&pool).await, before);
}

/// 받는 사람의 `upload` 줄. (행동자, 대표 사진, 묶음 수), 오래된 순.
async fn upload_rows(pool: &PgPool, user: &User) -> Vec<(Uuid, Uuid, i32)> {
    sqlx::query!(
        r#"SELECT actor_id, asset_id AS "asset_id!", count FROM notifications
           WHERE user_id = $1 AND kind = 'upload' ORDER BY created_at, id"#,
        user.id
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| (r.actor_id, r.asset_id, r.count))
    .collect()
}

struct UploadFamily {
    app: MediaApp,
    dad: (TestServer, User),
    grandma: (TestServer, User),
}

/// 임시 `DATA_DIR` 앱의 mom 과 같은 state 의 dad, grandma.
async fn upload_family(pool: &PgPool) -> UploadFamily {
    let app = media_app(pool, Role::Owner).await;
    let dad = app.login_as("dad", Role::Guardian).await;
    let grandma = app.login_as("grandma", Role::Family).await;
    UploadFamily { app, dad, grandma }
}

/// `FF D8 FF` 로 시작하는 가짜 JPEG. 매번 다른 바이트라 중복 판정 회피.
fn fake_jpeg() -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.extend(rand::random::<[u8; 60]>());
    bytes
}

/// tus 생성, PATCH, finalize. 응답 그대로 반환.
async fn upload_as(server: &TestServer, bytes: &[u8]) -> axum_test::TestResponse {
    let res = server
        .post("/api/uploads")
        .add_header(XRW, XRW_VALUE)
        .add_header(TUS_RESUMABLE, TUS_VERSION)
        .add_header("upload-length", bytes.len().to_string())
        .add_header(
            "upload-metadata",
            format!("filename {}", STANDARD.encode("a.jpg")),
        )
        .await;
    res.assert_status(StatusCode::CREATED);
    let location = res.header("location");
    let upload = location.to_str().unwrap();
    server
        .patch(upload)
        .add_header(XRW, XRW_VALUE)
        .add_header(TUS_RESUMABLE, TUS_VERSION)
        .add_header("upload-offset", "0")
        .content_type(OFFSET_STREAM)
        .bytes(bytes.to_vec().into())
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .post(&format!("{upload}/finalize"))
        .add_header(XRW, XRW_VALUE)
        .json(&json!({}))
        .await
}

/// 새 사진 업로드 후 asset id.
async fn upload_photo(server: &TestServer) -> Uuid {
    let res = upload_as(server, &fake_jpeg()).await;
    res.assert_status(StatusCode::CREATED);
    id(&res.json())
}

#[sqlx::test]
async fn upload_notifies_everyone_but_uploader(pool: PgPool) {
    let f = upload_family(&pool).await;
    let asset = upload_photo(&f.app.server).await;
    let mom = f.app.user.id;
    assert_eq!(upload_rows(&pool, &f.dad.1).await, [(mom, asset, 1)]);
    assert_eq!(upload_rows(&pool, &f.grandma.1).await, [(mom, asset, 1)]);
    assert!(upload_rows(&pool, &f.app.user).await.is_empty());
}

#[sqlx::test]
async fn uploads_within_window_grouped(pool: PgPool) {
    let f = upload_family(&pool).await;
    upload_photo(&f.app.server).await;
    let second = upload_photo(&f.app.server).await;
    let mom = f.app.user.id;
    assert_eq!(upload_rows(&pool, &f.dad.1).await, [(mom, second, 2)]);
    assert_eq!(upload_rows(&pool, &f.grandma.1).await, [(mom, second, 2)]);
    assert_eq!(total(&pool).await, 2);
}

#[sqlx::test]
async fn upload_after_window_new_row(pool: PgPool) {
    let f = upload_family(&pool).await;
    let first = upload_photo(&f.app.server).await;
    sqlx::query!("UPDATE notifications SET updated_at = now() - interval '31 minutes'")
        .execute(&pool)
        .await
        .unwrap();
    let second = upload_photo(&f.app.server).await;
    let mom = f.app.user.id;
    for user in [&f.dad.1, &f.grandma.1] {
        assert_eq!(
            upload_rows(&pool, user).await,
            [(mom, first, 1), (mom, second, 1)]
        );
    }
}

#[sqlx::test]
async fn other_actor_separate_row(pool: PgPool) {
    let f = upload_family(&pool).await;
    upload_photo(&f.app.server).await;
    let b = upload_photo(&f.dad.0).await;
    let c = upload_photo(&f.app.server).await;
    let (mom, dad) = (f.app.user.id, f.dad.1.id);
    assert_eq!(
        upload_rows(&pool, &f.grandma.1).await,
        [(mom, c, 2), (dad, b, 1)]
    );
    assert_eq!(upload_rows(&pool, &f.dad.1).await, [(mom, c, 2)]);
    assert_eq!(upload_rows(&pool, &f.app.user).await, [(dad, b, 1)]);
}

#[sqlx::test]
async fn duplicate_upload_no_notification(pool: PgPool) {
    let f = upload_family(&pool).await;
    let bytes = fake_jpeg();
    upload_as(&f.app.server, &bytes)
        .await
        .assert_status(StatusCode::CREATED);
    let before = total(&pool).await;
    upload_as(&f.app.server, &bytes)
        .await
        .assert_status(StatusCode::CONFLICT);
    assert_eq!(total(&pool).await, before);
    let mom = f.app.user.id;
    for user in [&f.dad.1, &f.grandma.1] {
        let rows = upload_rows(&pool, user).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2, 1);
        assert_eq!(rows[0].0, mom);
    }
}

/// 이 DB 에서 잠금 대기 중인 advisory lock 이 생길 때까지 대기.
async fn wait_for_advisory_waiter(pool: &PgPool) {
    for _ in 0..500 {
        let waiting = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM pg_locks
               WHERE locktype = 'advisory' AND NOT granted
                 AND database = (SELECT oid FROM pg_database WHERE datname = current_database())"#
        )
        .fetch_one(pool)
        .await
        .unwrap();
        if waiting > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("두 번째 트랜잭션이 advisory lock 대기에 들어가지 않음");
}

#[sqlx::test]
async fn upload_concurrent_same_actor_one_row(pool: PgPool) {
    let f = family(&pool).await;
    let mom = f.mom.1.id;
    let a = insert_asset(&pool, AssetSeed::photo(mom)).await;
    let b = insert_asset(&pool, AssetSeed::photo(mom)).await;

    // 첫 트랜잭션이 잠금을 쥔 채 미커밋. 두 번째는 잠금 대기 확인 뒤에야 첫 커밋.
    // 잠금이 없으면 두 번째가 미커밋 줄을 못 보고 새 줄 삽입.
    let mut first = pool.begin().await.unwrap();
    let mut second = pool.begin().await.unwrap();
    record_upload(&mut first, mom, a).await.unwrap();
    tokio::join!(
        async {
            record_upload(&mut second, mom, b).await.unwrap();
            second.commit().await.unwrap();
        },
        async {
            wait_for_advisory_waiter(&pool).await;
            first.commit().await.unwrap();
        },
    );

    for user in [&f.dad.1, &f.grandma.1] {
        assert_eq!(upload_rows(&pool, user).await, [(mom, b, 2)]);
    }
}
