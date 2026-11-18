use std::net::SocketAddr;

use anyhow::Context;
use chagokmoa::{
    app::{build_router, openapi},
    backup,
    config::{self, Config},
    db, jobs, maintenance,
    media::{regen, storage, video},
    push,
    shutdown::{forced, shutdown_signal},
    state::AppState,
    users::reset,
};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "chagokmoa", version, about = "Chagokmoa server")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// HTTP 서버 + 워커 기동
    Serve,
    /// DB 마이그레이션 적용
    Migrate,
    /// 사용자 비밀번호를 새 무작위 값으로 재설정하고 표준 출력에 출력. 세션 전부 삭제
    ResetPassword {
        /// 대상 사용자명
        username: String,
    },
    /// 파생물 재생성 작업을 큐에 넣고 넣은 수 출력. 처리는 실행 중인 서버가 담당
    RegenerateDerivatives {
        /// 파생물 디렉터리가 없는 항목만
        #[arg(long)]
        missing: bool,
    },
    /// 지금 백업하고 결과 출력. 실패면 종료 코드 1
    Backup,
    /// 백업 저장소의 스냅숏에서 DB 와 원본 복구, 파생물 재생성 작업 등록. 앱 중지 상태에서 실행.
    /// 복구 키는 RESTIC_PASSWORD 또는 입력, 원격 인증값은 같은 이름의 환경 변수
    Restore {
        /// restic 저장소(절대 경로 또는 s3:, b2:, rest:, azure:, gs: 주소). 기본은 BACKUP_DIR(/backup)
        #[arg(long, env = "RESTIC_REPOSITORY", hide_env_values = true)]
        repository: Option<String>,
        /// 스냅숏 id 또는 앞부분. 기본은 최신. 목록은 --list
        #[arg(long)]
        snapshot: Option<String>,
        /// 스냅숏 목록만 출력하고 종료(DB 와 파일 변경 없음)
        #[arg(long, conflicts_with_all = ["snapshot", "force"])]
        list: bool,
        /// 사용자나 사진이 있는 DB, 파일이 있는 originals 도 덮어씀(재확인 입력 필요)
        #[arg(long)]
        force: bool,
    },
    /// OpenAPI 문서를 표준 출력으로 출력 (설정, DB 불필요)
    Openapi,
}

#[derive(Debug, PartialEq)]
enum LogFormat {
    Text,
    Json,
}

fn parse_log_format(value: Option<&str>) -> anyhow::Result<LogFormat> {
    match value {
        // 빈 값(`LOG_FORMAT=`)은 미지정과 동일.
        None | Some("" | "text") => Ok(LogFormat::Text),
        Some("json") => Ok(LogFormat::Json),
        Some(_) => anyhow::bail!("LOG_FORMAT must be json or text"),
    }
}

// 로그는 전부 stderr. 서브커맨드 표준 출력(reset-password 등) 오염 방지.
fn init_tracing() -> anyhow::Result<()> {
    use tracing_subscriber::{EnvFilter, fmt};
    let format = parse_log_format(std::env::var("LOG_FORMAT").ok().as_deref())?;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = fmt().with_env_filter(filter).with_writer(std::io::stderr);
    match format {
        LogFormat::Json => builder.json().init(),
        LogFormat::Text => builder.init(),
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // .env 는 개발 편의용. 없어도 무시.
    dotenvy::dotenv().ok();

    let cli = Cli::parse();
    // openapi 는 표준 출력 JSON 오염 방지를 위해 로그 초기화 제외.
    if !matches!(cli.cmd, Cmd::Openapi) {
        init_tracing()?;
    }
    match cli.cmd {
        Cmd::Serve => {
            let config = Config::from_env()?;
            let pool = db::connect(&config.database_url).await?;
            if config.auto_migrate {
                db::migrate(&pool).await?;
            }
            let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
            storage::ensure_dirs(&config.data_dir)
                .with_context(|| format!("DATA_DIR {} 준비 실패", config.data_dir.display()))?;
            let state = AppState::new(pool, config);
            state.vapid().await?;
            let listener = tokio::net::TcpListener::bind(addr).await?;
            tracing::info!(%addr, "listening");
            let (stop, force) = shutdown_signal()?;
            let maintenance = maintenance::spawn(state.clone(), stop.clone());
            let backup_schedule = backup::run::spawn(state.clone(), stop.clone());
            video::check_tools().await;
            let requeued = jobs::requeue_running(&state.pool).await?;
            if requeued > 0 {
                tracing::info!(count = requeued, "requeued interrupted jobs");
            }
            let concurrency = state.config.worker_concurrency;
            let workers = jobs::worker::spawn(state.clone(), concurrency, stop.clone());
            tracing::info!(concurrency, "workers started");
            let push_worker = push::send::spawn_worker(state.clone(), stop.clone());
            let mut serve_stop = stop.clone();
            // 종료 대기 대상은 한 future 로 묶어 유예와 경쟁.
            let drain = async {
                axum::serve(listener, build_router(state))
                    .with_graceful_shutdown(async move {
                        let _ = serve_stop.wait_for(|v| *v).await;
                    })
                    .await?;
                maintenance.await?;
                backup_schedule.await?;
                // 한 워커의 실패가 나머지 join 을 막지 않게.
                for worker in workers {
                    if let Err(err) = worker.await {
                        tracing::error!(%err, "worker task failed");
                    }
                }
                if let Err(err) = push_worker.await {
                    tracing::error!(%err, "push worker task failed");
                }
                anyhow::Ok(())
            };
            tokio::select! {
                res = drain => res?,
                _ = forced(stop, force) => tracing::warn!("shutdown forced"),
            }
            tracing::info!("shutdown complete");
        }
        Cmd::Migrate => {
            let pool = db::connect(&config::database_url()?).await?;
            db::migrate(&pool).await?;
            println!("migrations applied");
        }
        Cmd::ResetPassword { username } => {
            let pool = db::connect(&config::database_url()?).await?;
            match reset::reset_password(&pool, &username).await? {
                Some(password) => println!("{password}"),
                None => {
                    eprintln!("user not found: {username}");
                    std::process::exit(1);
                }
            }
        }
        Cmd::RegenerateDerivatives { missing } => {
            let pool = db::connect(&config::database_url()?).await?;
            let queued = regen::enqueue(&pool, &config::data_dir(), missing).await?;
            println!("queued {queued}");
        }
        Cmd::Backup => {
            let config = Config::from_env()?;
            let pool = db::connect(&config.database_url).await?;
            let state = AppState::new(pool, config);
            let id = backup::run::run_once(&state, backup::run::Trigger::Manual).await?;
            let run = backup::queries::get(&state.pool, id)
                .await?
                .context("backup run record missing")?;
            // 기록에는 비밀값 없음(저장 전 제거).
            println!("status {:?}", run.status);
            for (name, report) in [("local", &run.local), ("remote", &run.remote)] {
                if let Some(report) = report {
                    println!("{name} {report}");
                }
            }
            if let Some(error) = &run.error {
                println!("error {error}");
            }
            if run.status == backup::run::RunStatus::Failed {
                std::process::exit(1);
            }
        }
        Cmd::Restore {
            repository,
            snapshot,
            list,
            force,
        } => {
            let opts = backup::restore::Options {
                database_url: config::database_url()?,
                data_dir: config::data_dir(),
                backup_dir: config::backup_dir(),
                repository,
                snapshot,
                force,
            };
            if list {
                backup::restore::list(&opts, &mut backup::restore::Terminal).await?;
                println!("restore a snapshot with: restore --snapshot <ID>");
                return Ok(());
            }
            let pool = backup::restore::connect(&opts.database_url).await?;
            let steps = backup::restore::Steps::default();
            backup::restore::run(&pool, &opts, steps, &mut backup::restore::Terminal).await?;
            println!("restore complete. start the app: docker compose up -d");
            println!("thumbnails are regenerated in the background after the app starts");
        }
        Cmd::Openapi => println!("{}", serde_json::to_string_pretty(&openapi())?),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_format_parsing() {
        assert_eq!(parse_log_format(None).unwrap(), LogFormat::Text);
        assert_eq!(parse_log_format(Some("")).unwrap(), LogFormat::Text);
        assert_eq!(parse_log_format(Some("text")).unwrap(), LogFormat::Text);
        assert_eq!(parse_log_format(Some("json")).unwrap(), LogFormat::Json);
        let err = parse_log_format(Some("xml")).unwrap_err();
        assert_eq!(err.to_string(), "LOG_FORMAT must be json or text");
    }
}
