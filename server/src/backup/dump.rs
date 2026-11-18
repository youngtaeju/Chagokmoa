//! DB 덤프와 적용. `pg_dump`, `pg_restore` 하위 프로세스. 비밀번호는 `PGPASSWORD` 로만.

use std::{ffi::OsString, path::Path, time::Duration};

use anyhow::Context;
use sqlx::PgPool;
use tokio::process::Command;

use super::{percent_decode, split_url_password};
use crate::{error::AppResult, media::video::run_with_timeout};

/// 멈춤 대비 상한. 덤프 대상은 메타데이터뿐이라 넉넉함.
const TIMEOUT: Duration = Duration::from_secs(3600);

/// 접속한 서버의 주 버전. 예: 17.
pub async fn server_major(pool: &PgPool) -> AppResult<u32> {
    let num =
        sqlx::query_scalar!(r#"SELECT current_setting('server_version_num')::int4 AS "num!""#)
            .fetch_one(pool)
            .await?;
    let major = u32::try_from(num / 10000).context("server_version_num out of range")?;
    Ok(major)
}

/// 접속 정보는 비밀번호 뺀 URL 로(libpq 가 해석), 비밀번호는 환경 변수로.
fn pg_command(program: &str, database_url: &str) -> Command {
    let (url, password) = split_url_password(database_url);
    let mut cmd = Command::new(program);
    if let Some(password) = password {
        cmd.env("PGPASSWORD", percent_decode(&password));
    }
    cmd.arg("--no-password").arg(format!("--dbname={url}"));
    cmd
}

pub fn dump_command(database_url: &str, path: &Path) -> Command {
    let mut cmd = pg_command("pg_dump", database_url);
    let mut file = OsString::from("--file=");
    file.push(path);
    cmd.arg("--format=custom").arg(file);
    cmd
}

pub fn restore_command(database_url: &str, path: &Path) -> Command {
    let mut cmd = pg_command("pg_restore", database_url);
    cmd.args(["--clean", "--if-exists", "--no-owner"]).arg(path);
    cmd
}

pub async fn to_file(database_url: &str, path: &Path) -> AppResult<()> {
    run_with_timeout(dump_command(database_url, path), TIMEOUT).await?;
    Ok(())
}

pub async fn restore_from_file(database_url: &str, path: &Path) -> AppResult<()> {
    run_with_timeout(restore_command(database_url, path), TIMEOUT).await?;
    Ok(())
}
