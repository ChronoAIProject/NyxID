//! Opt-in observation of a connected request. Reads never resume its effect.
use super::*;
use crate::mw::auth::{AuthMethod, AuthUser};
use axum::http::HeaderMap;

pub(crate) const READINESS: &str = "permissions:connected-approval-v1";
pub(crate) const HEADER: &str = "x-nyxid-connected-approval";
tokio::task_local! {
    pub(crate) static OPERATION: String;
}

pub(crate) fn take_operation(headers: &mut HeaderMap) -> AppResult<Option<String>> {
    let mut values = headers.get_all(HEADER).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    let key = value
        .to_str()
        .ok()
        .and_then(|value| value.strip_prefix("v1:"));
    let key = key.filter(|key| valid_key(key) && values.next().is_none());
    let Some(key) = key else {
        return Err(AppError::ValidationError(
            "Invalid connected approval operation".into(),
        ));
    };
    if headers.contains_key("x-nyxid-approval-mode") {
        return Err(AppError::ValidationError(
            "Connected approval cannot request background delivery".into(),
        ));
    }
    let key = key.to_owned();
    headers.remove(HEADER);
    Ok(Some(key))
}

fn valid_key(key: &str) -> bool {
    Uuid::parse_str(key).is_ok_and(|id| id.get_version_num() == 4 && id.to_string() == key)
}

pub(crate) async fn bind(
    state: &crate::AppState,
    context: Option<&Arc<RequestContext>>,
    chat: Option<&acks::ChatAuthority>,
) -> AppResult<()> {
    let Ok(key) = OPERATION.try_with(Clone::clone) else {
        return Ok(());
    };
    let context = context.ok_or_else(unavailable)?;
    if chat.is_some()
        || context.machine.get().is_some()
        || context.descendant.is_some()
        || !matches!(context.principal, Principal::ApiKey(_))
        || !matches!(context.ingress.get(), Some(PreparedAdapter::Llm { .. }))
    {
        return Err(unavailable());
    }
    if context
        .connected_operation
        .get()
        .is_some_and(|old| old != &key)
    {
        return Err(conflict());
    }
    // Recovery of the already admitted connected waiter still follows its
    // existing source/dispatch fences when new observation admission is off.
    if context.connected_operation.get().is_none() {
        require_enabled(state, &context.actor).await?;
        let _ = context.connected_operation.set(key);
    }
    Ok(())
}

pub(crate) async fn require_enabled(state: &crate::AppState, actor: &str) -> AppResult<()> {
    if !crate::services::feature_flag_service::resolve_personal_features(&state.db, actor)
        .await?
        .iter()
        .any(|flag| flag == READINESS)
    {
        return Err(unavailable());
    }
    require_enrolled(&state.db, actor, None).await?;
    crate::services::security_readiness_service::require_serving(&state.db, READINESS).await
}

fn unavailable() -> AppError {
    AppError::Forbidden("Connected approval observation is unavailable".into())
}

pub(super) fn locator(
    actor: &str,
    principal: &Principal,
    child: Option<&str>,
    key: &str,
) -> AppResult<String> {
    fingerprint(
        &json!({"contract":"connected-approval-v1","actor":actor,"principal":principal,"child":child,"operation":key}),
    )
}

#[derive(Serialize)]
pub struct Observation {
    pub version: u32,
    pub operation_key: String,
    pub request_id: String,
    pub provider_operation_id: String,
    pub receipt_revision: String,
    pub created_at: String,
    pub expires_at: String,
    pub review_path: String,
    pub decision: String,
    pub execution_state: String,
    pub effect: &'static str,
    pub failure_code: Option<String>,
}

pub(crate) fn view(row: &ApprovalRequest, key: &str) -> AppResult<Observation> {
    let p = proposal(row)?;
    let effect = match p.state.as_str() {
        "pending" | "queued" | "preparing" | "waiting" => "unclaimed",
        "not_sent" | "cancelled" | "denied" => "not_sent",
        "dispatched" => "dispatch_claimed",
        "completed" => "response_received",
        _ => "unknown",
    };
    Ok(Observation {
        version: 1,
        operation_key: key.into(),
        request_id: row.id.clone(),
        provider_operation_id: p.effect_id.clone(),
        receipt_revision: fingerprint(
            &json!({"request":row.id,"revision":p.revision,"state":p.state,"decision":row.status,"expires":row.expires_at,"failure":p.failure_code,"abandoned":p.delivery_abandoned,"claim":p.claim_id}),
        )?,
        created_at: row.created_at.to_rfc3339(),
        expires_at: p.waiter_until.ok_or_else(conflict)?.to_rfc3339(),
        review_path: crate::services::assistant_links::AssistantPage::Approvals.path(),
        decision: row.status.clone(),
        execution_state: p.state.clone(),
        effect,
        failure_code: p.failure_code.clone(),
    })
}

async fn load(
    state: &crate::AppState,
    auth: &AuthUser,
    key: &str,
) -> AppResult<(ApprovalRequest, authority::Snapshot)> {
    if !valid_key(key) || auth.auth_method != AuthMethod::ApiKey || auth.assistant_chat.is_some() {
        return Err(missing());
    }
    let key_id = auth.api_key_id.as_deref().ok_or_else(missing)?;
    let actor = auth.user_id.to_string();
    let principal = Principal::ApiKey(key_id.into());
    let locator = locator(
        &actor,
        &principal,
        auth.api_key_credential_id.as_deref(),
        key,
    )?;
    let row = state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! {"execution_proposal.version":2,"execution_proposal.v2.locator":locator})
        .await?
        .ok_or_else(missing)?;
    let p = proposal(&row)?;
    if row.user_id != actor
        || p.key_id != key_id
        || p.source_credential_id != auth.api_key_credential_id
        || p.contract.subject.principal != principal
        || p.background_allowed
        || !matches!(p.source, ProposalSource::External { pool: None })
    {
        return Err(missing());
    }
    // Metadata remains readable after a wait is retired, but never after the
    // credential/connection authority changes. It contains no retained result.
    let snapshot = authority::Snapshot::load(
        &state.db,
        &p.contract.binding.connection_id,
        &p.key_id,
        p.source_credential_id.as_deref(),
    )
    .await
    .map_err(|e| if e.is_forbidden() { missing() } else { e })?;
    if snapshot.digest() != p.authority_digest {
        return Err(missing());
    }
    Ok((row, snapshot))
}

pub(crate) async fn observe(
    state: &crate::AppState,
    auth: &AuthUser,
    key: &str,
) -> AppResult<Observation> {
    view(&load(state, auth, key).await?.0, key)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cancel {
    pub request_id: String,
    pub receipt_revision: String,
}

pub(crate) async fn cancel(
    state: &crate::AppState,
    auth: &AuthUser,
    key: &str,
    expected: &Cancel,
) -> AppResult<Observation> {
    let (row, snapshot) = load(state, auth, key).await?;
    let before = view(&row, key)?;
    if row.id != expected.request_id {
        return Err(conflict());
    }
    if !matches!(
        before.execution_state.as_str(),
        "pending" | "queued" | "preparing" | "waiting"
    ) {
        return Ok(before);
    }
    if before.receipt_revision != expected.receipt_revision {
        return Err(conflict());
    }
    blocking::abandon_observed(&state.db, &row, &snapshot, "approval_wait_cancelled").await?;
    observe(state, auth, key).await
}
