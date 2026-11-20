//! 셀프 비밀번호 재설정 메일 작업(`send_reset_mail`). 알림 발송 워커가 실행.

use uuid::Uuid;

use super::{Outgoing, settings, text};
use crate::{
    password_resets::queries::{self as resets, SELF_RESET_HOURS},
    settings::queries as instance,
    state::AppState,
    users,
};

/// 등록 뒤 바뀐 상태 재확인(메일 설정, 정지, 이메일, 대기 시간). 대상 아니면 아무것도 안 하고 `Ok`.
/// 발송 실패 시 만든 행 삭제 후 `Err`. 대기 시간에 안 걸려 바로 재요청 가능.
pub async fn run(state: &AppState, user_id: Uuid) -> anyhow::Result<()> {
    let Some(config) = settings::load(&state.pool).await? else {
        return Ok(());
    };
    let Some(user) = users::queries::find_by_id(&state.pool, user_id).await? else {
        return Ok(());
    };
    let Some(to) = user.email.filter(|_| user.disabled_at.is_none()) else {
        return Ok(());
    };
    // 같은 사용자 작업이 둘 있어도 한 통. 알림 워커 1개라 재확인과 insert 직렬 전제.
    if resets::self_sent_recently(&state.pool, user_id).await? {
        return Ok(());
    }
    let instance_name = instance::load(&state.pool).await?.instance_name;

    let (id, raw, _) = resets::create_self(&mut *state.pool.acquire().await?, user_id).await?;
    let url = format!("{}/reset/{raw}", state.config.public_url);
    let (subject, body) = text::reset(
        &user.locale,
        &instance_name,
        &user.username,
        &url,
        SELF_RESET_HOURS,
    );
    if let Err(err) = super::send(state, &config, Outgoing { to, subject, body }).await {
        resets::delete(&state.pool, id).await?;
        return Err(err.into());
    }
    resets::revoke_self_except(&state.pool, user_id, id).await?;
    Ok(())
}
