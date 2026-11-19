//! 실제 pg_dump, pg_restore 왕복. 클라이언트와 서버 주 버전이 같을 때만(CI 빌드 이미지는 17, 로컬 Ubuntu 클라이언트 16 은 건너뜀).

use chagokmoa::backup::dump;
use sqlx::{Connection, PgConnection, PgPool};

/// `pg_dump --version` 의 주 버전. 없거나 읽지 못하면 `None`.
fn client_major() -> Option<u32> {
    let out = std::process::Command::new("pg_dump")
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // 예: "pg_dump (PostgreSQL) 17.12 (Debian 17.12-0+deb13u1)"
    let version = text.split(") ").nth(1)?;
    version.split('.').next()?.trim().parse().ok()
}

async fn migration_count(conn: &mut PgConnection) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(conn)
        .await
        .expect("migrations")
}

/// 같은 서버의 다른 DB 를 가리키는 URL.
fn url_for(database: &str) -> String {
    let base = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let (server_part, _) = base.rsplit_once('/').expect("database name");
    format!("{server_part}/{database}")
}

#[sqlx::test(migrations = "./migrations")]
async fn real_dump_restore_round_trip(pool: PgPool) {
    let server = dump::server_major(&pool).await.expect("server major");
    match client_major() {
        Some(client) if client == server => {}
        // CI 빌드 이미지는 서버와 같은 주 버전 전제. 어긋나면 조용히 건너뛰지 않고 실패.
        other if std::env::var_os("CI").is_some() => {
            panic!("CI 에서 pg_dump 주 버전 {other:?} 와 서버 {server} 불일치");
        }
        other => {
            eprintln!("건너뜀: pg_dump 주 버전 {other:?}, 서버 {server}");
            return;
        }
    }

    for (key, value) in [("dump-a", "1"), ("dump-b", "\"둘\""), ("dump-c", "[3]")] {
        sqlx::query("INSERT INTO settings (key, value) VALUES ($1, $2::jsonb)")
            .bind(key)
            .bind(value)
            .execute(&pool)
            .await
            .expect("insert");
    }
    let source_db = pool
        .connect_options()
        .get_database()
        .expect("db name")
        .to_owned();
    // 식별자 63자 상한. 원본 이름에 덧붙이면 잘려 원본과 같아짐.
    let target_db = format!("restore_{}", uuid::Uuid::now_v7().simple());
    sqlx::query(&format!(r#"CREATE DATABASE "{target_db}""#))
        .execute(&pool)
        .await
        .expect("create target db");

    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("db.dump");
    dump::to_file(&url_for(&source_db), &file)
        .await
        .expect("pg_dump");
    dump::restore_from_file(&url_for(&target_db), &file)
        .await
        .expect("pg_restore");

    let mut target = PgConnection::connect(&url_for(&target_db))
        .await
        .expect("connect target");
    let restored: Vec<(String, serde_json::Value)> =
        sqlx::query_as("SELECT key, value FROM settings WHERE key LIKE 'dump-%' ORDER BY key")
            .fetch_all(&mut target)
            .await
            .expect("select settings");
    assert_eq!(
        restored,
        vec![
            ("dump-a".to_owned(), serde_json::json!(1)),
            ("dump-b".to_owned(), serde_json::json!("둘")),
            ("dump-c".to_owned(), serde_json::json!([3])),
        ]
    );
    let source_migrations = {
        let mut conn = pool.acquire().await.expect("acquire");
        migration_count(&mut conn).await
    };
    assert_eq!(migration_count(&mut target).await, source_migrations);

    target.close().await.expect("close target");
    sqlx::query(&format!(r#"DROP DATABASE "{target_db}" WITH (FORCE)"#))
        .execute(&pool)
        .await
        .expect("drop target db");
}
