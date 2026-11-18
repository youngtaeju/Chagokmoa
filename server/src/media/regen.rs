//! 파생물 재생성. 복구 직후(파생물 없음)나 파생물 형식 변경 때 사용.
//! 상태는 `ready` 유지, 새 파생물을 임시 디렉터리에 만든 뒤 교체. 실패 시 기존 파생물 유지.

use std::{io, path::Path};

use anyhow::Context;
use axum::{Json as AxumJson, extract::State};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::{process, storage};
use crate::{
    assets::{AssetKind, AssetStatus},
    auth::extract::CurrentUser,
    error::{AppError, AppResult, ErrorBody},
    request::Json,
    settings,
    state::AppState,
    users::Role,
};

/// 새 파생물을 만드는 임시 디렉터리 확장자. 성공 시 `derivatives/{id}` 로 이름 변경.
pub const TEMP_EXT: &str = "regen";
/// 교체 중 기존 파생물을 잠시 옮겨 두는 디렉터리 확장자. 교체 직후 삭제.
pub const STALE_EXT: &str = "stale";

/// 원본 파일이 있는 `ready` 자산(휴지통 포함)의 재생성 작업을 큐에. 반환은 넣은 수.
/// `missing_only` 면 파생물 디렉터리가 없는 자산만. 이미 대기 또는 실행 중인 자산은 제외.
/// 이전 실패 작업 행은 삭제(새 실행이 대체).
pub async fn enqueue(pool: &PgPool, data_dir: &Path, missing_only: bool) -> AppResult<u64> {
    let rows = sqlx::query!(
        "SELECT id, original_path FROM assets a
         WHERE status = 'ready'
           AND NOT EXISTS (SELECT 1 FROM jobs j
                           WHERE j.kind = 'regenerate_derivatives'
                             AND j.status IN ('queued', 'running')
                             AND j.payload ->> 'asset_id' = a.id::text)"
    )
    .fetch_all(pool)
    .await?;

    let mut ids = Vec::new();
    for row in rows {
        let original = data_dir.join(&row.original_path);
        if !exists(&original).await? {
            continue;
        }
        if missing_only && exists(&storage::derivative_dir(data_dir, row.id)).await? {
            continue;
        }
        ids.push(row.id);
    }
    let job_ids: Vec<Uuid> = ids.iter().map(|_| Uuid::now_v7()).collect();

    let mut tx = pool.begin().await?;
    sqlx::query!("DELETE FROM jobs WHERE kind = 'regenerate_derivatives' AND status = 'failed'")
        .execute(&mut *tx)
        .await?;
    let res = sqlx::query!(
        "INSERT INTO jobs (id, kind, payload)
         SELECT job_id, 'regenerate_derivatives', jsonb_build_object('asset_id', asset_id)
         FROM UNNEST($1::uuid[], $2::uuid[]) AS t(job_id, asset_id)",
        &job_ids,
        &ids,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected())
}

/// 실패 상태 재생성 작업 수.
pub async fn failed(pool: &PgPool) -> AppResult<i64> {
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM jobs
           WHERE kind = 'regenerate_derivatives' AND status = 'failed'"#
    )
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// 대기와 실행 중 재생성 작업 수.
pub async fn remaining(pool: &PgPool) -> AppResult<i64> {
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM jobs
           WHERE kind = 'regenerate_derivatives' AND status IN ('queued', 'running')"#
    )
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// 자산 1개 재생성. 자산이 없거나 `ready` 가 아니면 할 일 없이 `Ok`(일반 처리 담당).
/// 실패는 에러로 반환. 자산 상태와 기존 파생물은 그대로.
pub async fn run(state: &AppState, asset_id: Uuid) -> anyhow::Result<()> {
    let _lock = state.derivative_locks.acquire(asset_id).await;
    let Some(src) = sqlx::query!(
        r#"SELECT kind AS "kind: AssetKind", status AS "status: AssetStatus", original_path
           FROM assets WHERE id = $1"#,
        asset_id
    )
    .fetch_optional(&state.pool)
    .await?
    else {
        tracing::info!(%asset_id, "asset gone before regeneration");
        return Ok(());
    };
    if src.status != AssetStatus::Ready {
        tracing::info!(%asset_id, status = ?src.status, "regeneration skipped, asset not ready");
        return Ok(());
    }
    let data_dir = &state.config.data_dir;
    let original = data_dir.join(&src.original_path);
    let ty = process::original_type(&original, &src.original_path).await?;
    let settings = settings::queries::load(&state.pool).await?;

    let dir = storage::derivative_dir(data_dir, asset_id);
    let temp = dir.with_extension(TEMP_EXT);
    let stale = dir.with_extension(STALE_EXT);
    // 이전 실행이 중단돼 남은 잔재.
    process::remove_dir(&temp)
        .await
        .context("clear regeneration temp")?;
    restore_stale(&dir, &stale)
        .await
        .context("restore stale derivatives")?;
    process::remove_dir(&stale)
        .await
        .context("clear stale derivatives")?;

    let rendered = process::render(
        src.kind,
        original,
        ty,
        temp.clone(),
        settings.derivatives_avif,
    )
    .await;
    let p = match rendered {
        Ok(p) => p,
        Err(err) => {
            if let Err(rm) = process::remove_dir(&temp).await {
                tracing::warn!(%rm, %asset_id, "regeneration temp removal failed");
            }
            return Err(err);
        }
    };
    swap(&dir, &temp, &stale).await?;

    // 파일에서 나오는 값만 갱신. 촬영일, 위치, 캡션 등 메타데이터와 사용자 편집은 그대로.
    let derivatives: Value =
        serde_json::to_value(&p.rendered.derivatives).context("serialize derivatives")?;
    let res = sqlx::query!(
        "UPDATE assets SET width = $2, height = $3, duration_ms = $4,
             blurhash = $5, dominant_color = $6, derivatives = $7
         WHERE id = $1 AND status = 'ready'",
        asset_id,
        i32::try_from(p.width).context("width")?,
        i32::try_from(p.height).context("height")?,
        p.duration_ms,
        p.rendered.blurhash,
        p.rendered.dominant_color,
        derivatives,
    )
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0
        && crate::assets::queries::source(&state.pool, asset_id)
            .await?
            .is_none()
    {
        process::remove_orphan_derivatives(state, asset_id).await;
    }
    Ok(())
}

/// `temp` 를 `dir` 로 교체. 기존 `dir` 는 `stale` 로 옮긴 뒤 삭제.
/// 두 이름 변경 사이만 디렉터리 부재. 새 디렉터리 이동 실패 시 기존 복원.
async fn swap(dir: &Path, temp: &Path, stale: &Path) -> anyhow::Result<()> {
    let had_old = match tokio::fs::rename(dir, stale).await {
        Ok(()) => true,
        Err(err) if err.kind() == io::ErrorKind::NotFound => false,
        Err(err) => {
            remove_temp(temp).await;
            return Err(err).context("move old derivatives");
        }
    };
    if let Err(err) = tokio::fs::rename(temp, dir).await {
        if had_old && let Err(back) = tokio::fs::rename(stale, dir).await {
            tracing::warn!(%back, path = %dir.display(), "old derivatives restore failed");
        }
        remove_temp(temp).await;
        return Err(err).context("move new derivatives");
    }
    if had_old && let Err(err) = process::remove_dir(stale).await {
        // 남으면 고아 정리가 삭제.
        tracing::warn!(%err, path = %stale.display(), "stale derivatives removal failed");
    }
    Ok(())
}

/// 교체 중 중단돼 라이브 디렉터리 없이 `stale` 만 남았으면 되돌림. 복원했으면 `true`.
/// 자산 파생물 락 보유 상태에서 호출.
pub async fn restore_stale(dir: &Path, stale: &Path) -> io::Result<bool> {
    if tokio::fs::try_exists(dir).await? || !tokio::fs::try_exists(stale).await? {
        return Ok(false);
    }
    tokio::fs::rename(stale, dir).await?;
    Ok(true)
}

async fn remove_temp(temp: &Path) {
    if let Err(err) = process::remove_dir(temp).await {
        tracing::warn!(%err, path = %temp.display(), "regeneration temp removal failed");
    }
}

async fn exists(path: &Path) -> AppResult<bool> {
    tokio::fs::try_exists(path)
        .await
        .with_context(|| format!("check {}", path.display()))
        .map_err(AppError::Internal)
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(regeneration_status, start_regeneration))
}

#[derive(Deserialize, ToSchema)]
pub struct RegenerateRequest {
    /// `true` 면 파생물 디렉터리가 없는 항목만.
    missing_only: bool,
}

#[derive(Serialize, ToSchema)]
pub struct RegenerateQueued {
    /// 이번에 큐에 넣은 수. 이미 대기 중인 항목은 제외.
    pub queued: u64,
}

#[derive(Serialize, ToSchema)]
pub struct RegenerateStatus {
    /// 대기와 실행 중 재생성 작업 수.
    pub remaining: i64,
    /// 실패 상태 재생성 작업 수. 다음 큐 등록 때까지 유지.
    pub failed: i64,
}

#[utoipa::path(
    post,
    path = "/api/derivatives/regenerate",
    request_body = RegenerateRequest,
    responses(
        (status = 200, description = "재생성 작업 등록. 원본이 있는 `ready` 항목(휴지통 포함)", body = RegenerateQueued),
        (status = 400, description = "본문 형식 오류", body = ErrorBody),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님 또는 CSRF 헤더 누락", body = ErrorBody),
    )
)]
async fn start_regeneration(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(req): Json<RegenerateRequest>,
) -> AppResult<AxumJson<RegenerateQueued>> {
    current.require(Role::Owner)?;
    let queued = enqueue(&state.pool, &state.config.data_dir, req.missing_only).await?;
    if queued > 0 {
        // 대량 작업. 대기 중인 워커 전부 깨움.
        state.jobs_wake.notify_waiters();
    }
    Ok(AxumJson(RegenerateQueued { queued }))
}

#[utoipa::path(
    get,
    path = "/api/derivatives/regenerate",
    responses(
        (status = 200, description = "남은 재생성 작업 수와 실패 수", body = RegenerateStatus),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님", body = ErrorBody),
    )
)]
async fn regeneration_status(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<AxumJson<RegenerateStatus>> {
    current.require(Role::Owner)?;
    Ok(AxumJson(RegenerateStatus {
        remaining: remaining(&state.pool).await?,
        failed: failed(&state.pool).await?,
    }))
}
