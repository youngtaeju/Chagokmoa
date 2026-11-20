use std::{sync::Arc, time::Duration};

use sqlx::PgPool;
use tokio::sync::{Notify, OnceCell};

use crate::{
    backup::run::DbSteps,
    config::Config,
    error::AppResult,
    mail::MAIL_TIMEOUT,
    media::locks::DerivativeLocks,
    push::vapid::{self, Vapid},
    uploads::{UPLOAD_IDLE_TIMEOUT, locks::UploadLocks},
};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub upload_locks: Arc<UploadLocks>,
    /// 자산별 파생물 디렉터리 쓰기 직렬화(처리, 재생성).
    pub derivative_locks: Arc<DerivativeLocks>,
    /// PATCH 본문 무수신 한도. 기본 `UPLOAD_IDLE_TIMEOUT`, 테스트에서 단축.
    pub upload_idle_timeout: Duration,
    /// 메일 1통 발송 한도. 기본 `MAIL_TIMEOUT`, 테스트에서 단축.
    pub mail_timeout: Duration,
    /// 작업 enqueue 커밋 후 `notify_one`. 대기 중 워커 즉시 깨움.
    pub jobs_wake: Arc<Notify>,
    /// 첫 사용 시 로드 또는 생성. 기동 시 `vapid()` 1회 호출.
    pub vapid: Arc<OnceCell<Vapid>>,
    /// 푸시 잡 enqueue 커밋 후 `notify_one`. 푸시 워커 전용.
    pub push_wake: Arc<Notify>,
    /// 구독 주소 허용 목록 검사. 테스트에서 `false` 로 로컬 가짜 푸시 서버 허용.
    pub push_endpoint_check: bool,
    /// 백업의 서버 버전 확인과 덤프. 테스트에서 가짜로 교체.
    pub backup_db: DbSteps,
}

impl AppState {
    pub fn new(pool: PgPool, config: Config) -> AppState {
        AppState {
            pool,
            config: Arc::new(config),
            upload_locks: Arc::new(UploadLocks::default()),
            derivative_locks: Arc::new(DerivativeLocks::default()),
            upload_idle_timeout: UPLOAD_IDLE_TIMEOUT,
            mail_timeout: MAIL_TIMEOUT,
            jobs_wake: Arc::new(Notify::new()),
            vapid: Arc::new(OnceCell::new()),
            push_wake: Arc::new(Notify::new()),
            push_endpoint_check: true,
            backup_db: DbSteps::default(),
        }
    }

    pub async fn vapid(&self) -> AppResult<&Vapid> {
        self.vapid
            .get_or_try_init(|| vapid::load_or_create(&self.pool))
            .await
    }
}
