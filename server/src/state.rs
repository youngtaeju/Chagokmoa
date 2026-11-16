use std::{sync::Arc, time::Duration};

use sqlx::PgPool;
use tokio::sync::{Notify, OnceCell};

use crate::{
    config::Config,
    error::AppResult,
    push::vapid::{self, Vapid},
    uploads::{UPLOAD_IDLE_TIMEOUT, locks::UploadLocks},
};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub upload_locks: Arc<UploadLocks>,
    /// PATCH 본문 무수신 한도. 기본 `UPLOAD_IDLE_TIMEOUT`, 테스트에서 단축.
    pub upload_idle_timeout: Duration,
    /// 작업 enqueue 커밋 후 `notify_one`. 대기 중 워커 즉시 깨움.
    pub jobs_wake: Arc<Notify>,
    /// 첫 사용 시 로드 또는 생성. 기동 시 `vapid()` 1회 호출.
    pub vapid: Arc<OnceCell<Vapid>>,
    /// 푸시 잡 enqueue 커밋 후 `notify_one`. 푸시 워커 전용.
    pub push_wake: Arc<Notify>,
    /// 구독 주소 허용 목록 검사. 테스트에서 `false` 로 로컬 가짜 푸시 서버 허용.
    pub push_endpoint_check: bool,
}

impl AppState {
    pub fn new(pool: PgPool, config: Config) -> AppState {
        AppState {
            pool,
            config: Arc::new(config),
            upload_locks: Arc::new(UploadLocks::default()),
            upload_idle_timeout: UPLOAD_IDLE_TIMEOUT,
            jobs_wake: Arc::new(Notify::new()),
            vapid: Arc::new(OnceCell::new()),
            push_wake: Arc::new(Notify::new()),
            push_endpoint_check: true,
        }
    }

    pub async fn vapid(&self) -> AppResult<&Vapid> {
        self.vapid
            .get_or_try_init(|| vapid::load_or_create(&self.pool))
            .await
    }
}
