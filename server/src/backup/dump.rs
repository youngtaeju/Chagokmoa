//! DB 덤프와 적용. `pg_dump`, `pg_restore` 하위 프로세스. 비밀번호는 `PGPASSWORD` 로만.

use std::{ffi::OsString, path::Path, process::Stdio, time::Duration};

use anyhow::{Context, anyhow};
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

/// 빈 스키마 대상. 한 트랜잭션, 첫 오류에서 중단. 소유자와 권한은 접속 사용자 기준(옛 서버 역할 이름과 무관).
pub fn restore_command(database_url: &str, path: &Path) -> Command {
    let mut cmd = pg_command("pg_restore", database_url);
    cmd.args([
        "--single-transaction",
        "--exit-on-error",
        "--no-owner",
        "--no-privileges",
    ])
    .arg(path);
    cmd
}

pub async fn to_file(database_url: &str, path: &Path) -> AppResult<()> {
    run_with_timeout(dump_command(database_url, path), TIMEOUT).await?;
    Ok(())
}

pub async fn restore_from_file(database_url: &str, path: &Path) -> AppResult<()> {
    let out = restore_command(database_url, path)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(TIMEOUT, out)
        .await
        .map_err(|_| anyhow!("pg_restore timed out after {}s", TIMEOUT.as_secs()))?
        .context("pg_restore spawn failed")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!(
            "pg_restore failed ({}): {}",
            out.status,
            error_line(&stderr)
        )
        .into());
    }
    Ok(())
}

/// 실패 원인 줄. `--exit-on-error` 출력은 `pg_restore: error: ...` 뒤에 실패한 SQL 이 이어져 마지막 줄이 `);` 등.
fn error_line(stderr: &str) -> &str {
    stderr
        .lines()
        .find(|l| l.contains("error:"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_error_line() {
        let stderr = "pg_restore: error: could not execute query: ERROR:  relation \"b\" already exists\n\
                      Command was: CREATE TABLE public.b (\n    id integer\n);\n\n";
        assert_eq!(
            error_line(stderr),
            "pg_restore: error: could not execute query: ERROR:  relation \"b\" already exists"
        );
        assert_eq!(error_line("warning only\nlast\n"), "last");
        assert_eq!(error_line(""), "");
    }
}
