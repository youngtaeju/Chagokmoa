use axum::{Json as AxumJson, extract::State};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    auth::extract::CurrentUser,
    error::{AppResult, ErrorBody},
    settings::queries as instance,
    state::AppState,
    users::Role,
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_update))
}

#[derive(Serialize, ToSchema)]
pub struct UpdateStatus {
    /// 실행 중인 버전.
    current: String,
    /// 더 새 릴리스 버전. 없거나 확인 꺼짐, 조회 전, 실패면 null.
    #[schema(required = true)]
    latest: Option<String>,
    /// 그 릴리스 페이지.
    #[schema(required = true)]
    release_url: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/update",
    responses(
        (status = 200, description = "새 버전 확인 결과. GitHub 조회는 서버가 하루 1회", body = UpdateStatus),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "owner 아님", body = ErrorBody),
    )
)]
async fn get_update(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<AxumJson<UpdateStatus>> {
    current.require(Role::Owner)?;
    let enabled = instance::load(&state.pool).await?.update_check;
    let latest = enabled.then(|| super::latest(&state)).flatten();
    Ok(AxumJson(UpdateStatus {
        current: super::CURRENT.to_owned(),
        latest: latest.as_ref().map(|l| l.version.clone()),
        release_url: latest.map(|l| l.url),
    }))
}
