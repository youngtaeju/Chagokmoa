//! 새 버전 확인. GitHub 최신 릴리스를 조회해 결과를 메모리에 둠. 알림만, 업데이트는 운영자.
//! 보내는 정보는 `User-Agent: chagokmoa/<버전>` 뿐. 설정 `update_check` 가 꺼져 있으면 요청 없음.

pub mod routes;

use std::{sync::LazyLock, time::Duration};

use anyhow::{Context, anyhow};
use serde::Deserialize;
use tokio::{sync::watch, task::JoinHandle};

use crate::{settings::queries as instance, state::AppState};

/// 초안과 사전 릴리스 제외한 최신 릴리스.
pub const RELEASES_URL: &str = "https://api.github.com/repos/youngtaeju/Chagokmoa/releases/latest";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const INTERVAL: Duration = Duration::from_secs(24 * 3600);
const TIMEOUT: Duration = Duration::from_secs(10);

// 리다이렉트로 다른 호스트에 닿지 않게.
static CLIENT: LazyLock<reqwest::Result<reqwest::Client>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("chagokmoa/{CURRENT}"))
        .build()
});

/// 현재보다 새 릴리스.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Latest {
    pub version: String,
    pub url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
}

/// `X.Y.Z` 숫자 3개. 그 밖의 형식(사전 릴리스 접미사 포함)은 None.
fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let triple = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(triple)
}

/// `tag`(`vX.Y.Z`)가 `current`(`X.Y.Z`)보다 새로우면 `X.Y.Z`.
pub fn newer(current: &str, tag: &str) -> Option<String> {
    let version = tag.strip_prefix('v')?;
    (parse(version)? > parse(current)?).then(|| version.to_owned())
}

fn store(state: &AppState, latest: Option<Latest>) {
    // 독점 구간에 패닉 없음. 오염돼도 값은 유효.
    *state
        .update_latest
        .write()
        .unwrap_or_else(|e| e.into_inner()) = latest;
}

/// 현재 결과. 꺼져 있으면 호출 측이 무시.
pub fn latest(state: &AppState) -> Option<Latest> {
    state
        .update_latest
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// 설정 꺼짐 시 결과만 비움. 조회 실패는 `Err`, 직전 결과 유지.
pub async fn check_once(state: &AppState) -> anyhow::Result<()> {
    if !instance::load(&state.pool).await?.update_check {
        store(state, None);
        return Ok(());
    }
    let client = CLIENT.as_ref().map_err(|e| anyhow!("http client: {e}"))?;
    let res = client
        .get(&state.update_url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .context("release request")?;
    let status = res.status();
    if !status.is_success() {
        return Err(anyhow!("release request status {status}"));
    }
    let body = res.bytes().await.context("release body")?;
    let release: Release = serde_json::from_slice(&body).context("release json")?;
    store(
        state,
        newer(CURRENT, &release.tag_name).map(|version| Latest {
            version,
            url: release.html_url,
        }),
    );
    Ok(())
}

/// 기동 직후 1회, 이후 24시간마다 또는 `update_wake` 시. 종료 신호 시 반환.
pub fn spawn(state: AppState, mut shutdown: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(err) = check_once(&state).await {
                tracing::warn!(error = %err, "update check failed");
            }
            tokio::select! {
                _ = tokio::time::sleep(INTERVAL) => {}
                _ = state.update_wake.notified() => {}
                // 송신측 드롭도 종료로 간주.
                _ = shutdown.wait_for(|stop| *stop) => break,
            }
        }
    })
}
