//! Operational status is independent of public AI evidence and legacy guide flags.
use super::platform_bot_runtime_control_service as controls;
use crate::{errors::AppResult, models::platform_bot_runtime_controls::PlatformBotRuntimeControls};
pub(crate) struct RuntimeStatus {
    pub state: &'static str,
    pub faq_ready: bool,
}
pub(crate) async fn view(
    db: &mongodb::Database,
    row: Option<&PlatformBotRuntimeControls>,
) -> AppResult<RuntimeStatus> {
    let Some(row) = row else {
        return Ok(RuntimeStatus {
            state: "setup_incomplete",
            faq_ready: false,
        });
    };
    if row.retired {
        return Ok(RuntimeStatus {
            state: "retired",
            faq_ready: false,
        });
    }
    let mut session = db.client().start_session().await?;
    let ready = controls::validate_manager_in_session(
        db,
        &mut session,
        &row.platform_config_id,
        &row.manager_bot_id,
        &row.entry,
        None,
    )
    .await?
    .is_some();
    let state = if !ready {
        "connection_repair_required"
    } else if row.emergency_stopped {
        "paused"
    } else if row.personal_enabled || row.public_enabled {
        "active"
    } else {
        "setup_incomplete"
    };
    Ok(RuntimeStatus {
        state,
        faq_ready: ready
            && !row.emergency_stopped
            && row.public_enabled
            && row.public_policy.is_none(),
    })
}
