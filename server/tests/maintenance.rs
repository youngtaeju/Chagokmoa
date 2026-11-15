mod common;

use chagokmoa::users::Role;
use chagokmoa::{auth::session, maintenance};
use common::{AssetSeed, insert_asset, media_app};
use sqlx::PgPool;

#[sqlx::test]
async fn expired_sessions_are_removed(pool: PgPool) {
    let app = media_app(&pool, Role::Guardian).await;
    let (_, expired_id) = session::create(&pool, app.user.id, None)
        .await
        .expect("session");
    sqlx::query!(
        "UPDATE sessions SET expires_at = now() - interval '1 minute' WHERE id = $1",
        expired_id,
    )
    .execute(&pool)
    .await
    .expect("expire");

    maintenance::run_once(&app.state).await.expect("run_once");

    let remaining = sqlx::query_scalar!("SELECT id FROM sessions")
        .fetch_all(&pool)
        .await
        .expect("list");
    assert_eq!(remaining.len(), 1);
    assert_ne!(remaining[0], expired_id);
    app.server.get("/api/auth/me").await.assert_status_ok();
}

#[sqlx::test]
async fn spawn_stops_on_shutdown_signal(pool: PgPool) {
    let app = media_app(&pool, Role::Guardian).await;
    let (tx, rx) = tokio::sync::watch::channel(false);
    let handle = maintenance::spawn(app.state.clone(), rx);

    tx.send(true).expect("send");

    tokio::time::timeout(std::time::Duration::from_secs(1), handle)
        .await
        .expect("종료 지연")
        .expect("join");
}

#[sqlx::test]
async fn old_notifications_removed(pool: PgPool) {
    let app = media_app(&pool, Role::Guardian).await;
    let asset = insert_asset(&pool, AssetSeed::photo(app.user.id)).await;
    let insert = |days: i32| {
        let pool = pool.clone();
        let user = app.user.id;
        async move {
            sqlx::query_scalar!(
                "INSERT INTO notifications (id, user_id, kind, actor_id, asset_id, created_at,
                                            updated_at)
                 VALUES (gen_random_uuid(), $1, 'upload', $1, $2,
                         now() - make_interval(days => $3), now() - make_interval(days => $3))
                 RETURNING id",
                user,
                asset,
                days
            )
            .fetch_one(&pool)
            .await
            .expect("insert notification")
        }
    };
    insert(91).await;
    let kept = insert(89).await;

    maintenance::run_once(&app.state).await.expect("run_once");

    let remaining = sqlx::query_scalar!("SELECT id FROM notifications")
        .fetch_all(&pool)
        .await
        .expect("list");
    assert_eq!(remaining, [kept]);
}
