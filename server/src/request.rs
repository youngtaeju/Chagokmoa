use axum::extract::{
    FromRequest, FromRequestParts, OptionalFromRequest, Request,
    rejection::{JsonRejection, PathRejection, QueryRejection},
};
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::error::AppError;

/// 본문 파싱 실패를 표준 에러 응답으로 변환하는 `axum::Json` 대체.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(AppError))]
pub struct Json<T>(pub T);

/// 선택 본문. `Content-Type` 없으면 `None`, 있으면 `Json` 과 같은 검사.
impl<T, S> OptionalFromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Option<Self>, AppError> {
        let body = <axum::Json<T> as OptionalFromRequest<S>>::from_request(req, state).await?;
        Ok(body.map(|axum::Json(value)| Json(value)))
    }
}

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
        tracing::debug!(error = %rejection, "invalid request body");
        AppError::BadRequest {
            key: "request.invalidBody",
            params: json!({}),
        }
    }
}

/// 경로 파라미터 파싱 실패를 표준 에러 응답으로 변환하는 `axum::extract::Path` 대체.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(AppError))]
pub struct Path<T>(pub T);

impl From<PathRejection> for AppError {
    fn from(rejection: PathRejection) -> Self {
        tracing::debug!(error = %rejection, "invalid path parameter");
        AppError::BadRequest {
            key: "request.invalidPath",
            params: json!({}),
        }
    }
}

/// 쿼리 스트링 파싱 실패를 표준 에러 응답으로 변환하는 `axum::extract::Query` 대체.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(AppError))]
pub struct Query<T>(pub T);

impl From<QueryRejection> for AppError {
    fn from(rejection: QueryRejection) -> Self {
        tracing::debug!(error = %rejection, "invalid query string");
        AppError::BadRequest {
            key: "request.invalidQuery",
            params: json!({}),
        }
    }
}
