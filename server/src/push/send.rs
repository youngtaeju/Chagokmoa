//! 웹푸시 발송 잡과 알림 발송 워커(웹푸시, 재설정 메일).

use std::{collections::HashMap, sync::LazyLock, time::Duration};

use anyhow::{Context, anyhow};
use axum::http::{HeaderValue, StatusCode, Uri};
use serde_json::json;
use tokio::{sync::watch, task::JoinHandle};
use uuid::Uuid;
use web_push_native::{Auth, WebPushBuilder, p256};

use super::{
    queries::{self, Subscription},
    routes::decode_key,
    text,
    vapid::Vapid,
};
use crate::{
    error::AppResult,
    jobs::{self, ClaimedJob, Job, worker::catch_panic},
    mail,
    notifications::{self, NotificationKind},
    settings,
    state::AppState,
};

/// 푸시 서비스 보관 기간(`TTL`).
const TTL: Duration = Duration::from_secs(86400);
/// 요청 1건 시간 제한.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// 깨움 신호를 놓쳤을 때 대비한 큐 재확인 주기.
const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// 대상 요약이 없을 때 이동 주소.
const FALLBACK_URL: &str = "/notifications";

// 구독 주소는 사용자 입력. 리다이렉트로 다른 호스트에 닿지 않게.
static CLIENT: LazyLock<reqwest::Result<reqwest::Client>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(SEND_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
});

/// 알림 줄 1개 발송. 늘어난 장수(`count - pushed_count`)만 알리고 `pushed_count` 갱신.
/// 발송 실패는 로그만. `Err` 는 DB 오류.
pub async fn run(state: &AppState, notification_id: Uuid) -> AppResult<()> {
    let mut conn = state.pool.acquire().await?;
    // 잠금 없음. 발송 중 늘어난 장수는 다음 잡 몫.
    let Some(row) = sqlx::query!(
        r#"SELECT n.user_id, n.kind AS "kind: NotificationKind", n.count, n.pushed_count,
                  u.locale, u.disabled_at IS NOT NULL AS "disabled!"
           FROM notifications n JOIN users u ON u.id = n.user_id
           WHERE n.id = $1"#,
        notification_id
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(());
    };
    let delta = row.count - row.pushed_count;
    let skip = row.disabled
        || delta <= 0
        || queries::muted(&mut *conn, row.user_id)
            .await?
            .contains(&row.kind);
    let settings = settings::queries::load(&mut *conn).await?;
    let note = if skip {
        None
    } else {
        notifications::queries::get(&mut conn, &settings.timezone, row.user_id, notification_id)
            .await?
    };
    // 보내지 않는 경우도 갱신. 다시 켜거나 새로 구독했을 때 쌓인 장수 일괄 발송 방지.
    sqlx::query!(
        "UPDATE notifications SET pushed_count = $2 WHERE id = $1",
        notification_id,
        row.count
    )
    .execute(&mut *conn)
    .await?;
    let Some(note) = note else {
        return Ok(());
    };
    let subscriptions = queries::endpoints(&mut *conn, row.user_id).await?;
    if subscriptions.is_empty() {
        return Ok(());
    }
    let names: HashMap<Uuid, String> = sqlx::query!("SELECT id, display_name FROM users")
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|u| (u.id, u.display_name))
        .collect();
    // 발송 대기 동안 연결 점유 방지.
    drop(conn);

    let locale = text::pick_locale(&row.locale, &settings.default_locale);
    let payload = json!({
        "title": settings.instance_name,
        "body": text::body(locale, &note, delta, row.user_id, &names),
        "url": text::url(&note, delta).unwrap_or_else(|| FALLBACK_URL.to_string()),
    })
    .to_string();
    let vapid = state.vapid().await?;
    for subscription in &subscriptions {
        deliver(state, vapid, subscription, &payload).await?;
    }
    Ok(())
}

/// 구독 1건 요청. 404/410 은 구독 삭제, 그 밖의 실패는 경고 로그만.
async fn deliver(
    state: &AppState,
    vapid: &Vapid,
    subscription: &Subscription,
    payload: &str,
) -> AppResult<()> {
    // 주소 전체는 수신 권한 토큰. 호스트만 기록.
    let host = reqwest::Url::parse(&subscription.endpoint)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default();
    let client = match &*CLIENT {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(%host, error = %err, "push client unavailable");
            return Ok(());
        }
    };
    let request = match build_request(vapid, &state.config.public_url, subscription, payload) {
        Ok(request) => request,
        Err(err) => {
            tracing::warn!(%host, error = %err, "push request build failed");
            return Ok(());
        }
    };
    match client.execute(request).await {
        Ok(res) if res.status().is_success() => {}
        Ok(res) if matches!(res.status(), StatusCode::NOT_FOUND | StatusCode::GONE) => {
            match queries::delete_by_endpoint(&state.pool, &subscription.endpoint).await {
                Ok(()) => {
                    tracing::info!(%host, status = res.status().as_u16(), "push subscription gone, removed");
                }
                Err(err) => tracing::warn!(%host, error = ?err, "push subscription removal failed"),
            }
        }
        Ok(res) => {
            tracing::warn!(%host, status = res.status().as_u16(), "push rejected");
        }
        Err(err) => {
            tracing::warn!(%host, error = ?err.without_url(), "push send failed");
        }
    }
    Ok(())
}

/// RFC 8291 암호화와 VAPID 서명. 에러 문구에 주소 미포함.
fn build_request(
    vapid: &Vapid,
    contact: &str,
    subscription: &Subscription,
    payload: &str,
) -> anyhow::Result<reqwest::Request> {
    let endpoint: Uri = subscription.endpoint.parse().context("invalid endpoint")?;
    let point = decode_key(&subscription.p256dh).context("invalid p256dh")?;
    let public = p256::PublicKey::from_sec1_bytes(&point).map_err(|_| anyhow!("invalid p256dh"))?;
    let auth = decode_key(&subscription.auth)
        .filter(|a| a.len() == 16)
        .context("invalid auth")?;
    let mut request = WebPushBuilder::new(endpoint, public, Auth::clone_from_slice(&auth))
        .with_vapid(&vapid.key_pair, contact)
        .build(payload)?;
    // JWT exp 는 라이브러리 기본(12시간), TTL 헤더만 별도 지정.
    request
        .headers_mut()
        .insert("ttl", HeaderValue::from(TTL.as_secs()));
    request
        .headers_mut()
        .insert("urgency", HeaderValue::from_static("normal"));
    Ok(reqwest::Request::try_from(request)?)
}

/// 알림 발송 잡 1개 처리. 큐가 비었으면 `false`. 재시도 없음.
/// 푸시: 발송 결과와 무관하게 잡 삭제, DB 오류와 패닉만 `failed`.
/// 재설정 메일: 발송 실패도 `failed`.
async fn run_next(state: &AppState) -> AppResult<bool> {
    let Some(ClaimedJob { id, job, .. }) =
        jobs::claim(&state.pool, &["send_push", "send_reset_mail"]).await?
    else {
        return Ok(false);
    };
    let result = match job.clone() {
        Job::SendPush { notification_id } => {
            catch_panic(async { Ok(run(state, notification_id).await?) }).await
        }
        Job::SendResetMail { user_id } => catch_panic(mail::reset_job::run(state, user_id)).await,
        // `claim` 종류 필터로 도달 불가. 방어용.
        Job::ProcessAsset { .. } | Job::RegenerateDerivatives { .. } => {
            jobs::fail(&state.pool, id, "unexpected job kind").await?;
            return Ok(true);
        }
    };
    match result {
        Ok(()) => jobs::finish(&state.pool, id).await?,
        Err(err) => {
            tracing::error!(?job, job_id = %id, error = ?err, "notification job failed");
            jobs::fail(&state.pool, id, &format!("{err:#}")).await?;
        }
    }
    Ok(true)
}

/// 알림 발송 워커 1개(웹푸시, 재설정 메일). 대기 구조는 `jobs::worker::spawn` 과 같고 깨움은 `push_wake`.
pub fn spawn_worker(state: AppState, mut shutdown: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        while !*shutdown.borrow() {
            let idle = match run_next(&state).await {
                Ok(ran) => !ran,
                Err(err) => {
                    // DB 장애 시 바쁜 재시도 방지. 유휴와 같은 대기.
                    tracing::error!(%err, "push queue error");
                    true
                }
            };
            if idle {
                tokio::select! {
                    _ = state.push_wake.notified() => {}
                    _ = tokio::time::sleep(POLL_INTERVAL) => {}
                    // 송신측 드롭도 종료로 간주.
                    _ = shutdown.wait_for(|stop| *stop) => break,
                }
            }
        }
    })
}
