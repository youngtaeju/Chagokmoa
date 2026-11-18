use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

type Entries = Mutex<HashMap<Uuid, Arc<AsyncMutex<()>>>>;

/// 자산별 파생물 디렉터리 쓰기 락(처리, 재생성). 단일 프로세스 전제.
/// DB 연결을 잡지 않아 긴 영상 변환 중에도 풀 소모 없음.
#[derive(Default)]
pub struct DerivativeLocks {
    locks: Arc<Entries>,
}

/// 드롭 시 해제. 대기자 없으면 항목 제거.
pub struct DerivativeGuard {
    id: Uuid,
    locks: Arc<Entries>,
    guard: Option<OwnedMutexGuard<()>>,
}

impl DerivativeLocks {
    /// 같은 자산의 다른 보유자가 끝날 때까지 대기.
    pub async fn acquire(&self, id: Uuid) -> DerivativeGuard {
        let lock = self
            .locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(id)
            .or_default()
            .clone();
        DerivativeGuard {
            id,
            locks: self.locks.clone(),
            guard: Some(lock.lock_owned().await),
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl Drop for DerivativeGuard {
    fn drop(&mut self) {
        self.guard.take();
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        // 맵만 참조하면 대기자 없음. 복제는 맵 락 안에서만 일어나 판정과 경합 없음.
        if locks
            .get(&self.id)
            .is_some_and(|lock| Arc::strong_count(lock) == 1)
        {
            locks.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn same_id_waits_until_release() {
        let locks = DerivativeLocks::default();
        let id = Uuid::now_v7();
        let guard = locks.acquire(id).await;
        let other = locks.acquire(Uuid::now_v7()).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), locks.acquire(id))
                .await
                .is_err()
        );
        drop(guard);
        let again = locks.acquire(id).await;
        drop((other, again));
        assert_eq!(locks.len(), 0);
    }
}
