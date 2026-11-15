use axum::{Json, extract::State, http::StatusCode};
use utoipa_axum::{router::OpenApiRouter, routes};

use super::{ListQuery, NotificationPage, UnreadCount, queries};
use crate::{
    assets::{Page, cursor},
    auth::extract::CurrentUser,
    comments::into_page,
    error::{AppError, AppResult, ErrorBody},
    request::Query,
    settings,
    state::AppState,
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_notifications))
        .routes(routes!(mark_seen))
        .routes(routes!(unread_count))
}

/// 목록 `limit` 검증. 1~100, 기본 30, 벗어나면 400 `request.invalidQuery`.
fn check_limit(limit: Option<i64>) -> AppResult<i64> {
    let limit = limit.unwrap_or(30);
    if !(1..=100).contains(&limit) {
        return Err(AppError::bad_request("request.invalidQuery"));
    }
    Ok(limit)
}

#[utoipa::path(
    get,
    path = "/api/notifications",
    params(ListQuery),
    responses(
        (status = 200, description = "내 알림 최신순. 대상 사진이 휴지통이면 제외", body = NotificationPage),
        (status = 400, description = "쿼리 오류 (`request.invalidQuery`, `asset.invalidCursor`)", body = ErrorBody),
        (status = 401, description = "인증 필요", body = ErrorBody),
    )
)]
async fn list_notifications(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<NotificationPage>> {
    let limit = check_limit(q.limit)?;
    let after = q.cursor.as_deref().map(cursor::decode).transpose()?;
    let user_id = current.user.id;
    let mut conn = state.pool.acquire().await?;
    let timezone = settings::queries::load(&mut *conn).await?.timezone;
    let items = queries::list(&mut conn, &timezone, user_id, after, limit + 1).await?;
    let Page { items, next_cursor } = into_page(items, limit, |n| (n.updated_at, n.id));
    let seen_at = queries::seen_at(&mut *conn, user_id).await?;
    Ok(Json(NotificationPage {
        items,
        next_cursor,
        seen_at,
    }))
}

#[utoipa::path(
    post,
    path = "/api/notifications/seen",
    responses(
        (status = 204, description = "지금까지의 알림을 읽음으로"),
        (status = 401, description = "인증 필요", body = ErrorBody),
        (status = 403, description = "CSRF 헤더 누락", body = ErrorBody),
    )
)]
async fn mark_seen(State(state): State<AppState>, current: CurrentUser) -> AppResult<StatusCode> {
    queries::mark_seen(&state.pool, current.user.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/notifications/unread-count",
    responses(
        (status = 200, description = "본 시각 이후 알림 수. 100 에서 멈춤", body = UnreadCount),
        (status = 401, description = "인증 필요", body = ErrorBody),
    )
)]
async fn unread_count(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<Json<UnreadCount>> {
    let count = queries::unread_count(&state.pool, current.user.id).await?;
    Ok(Json(UnreadCount { count }))
}
