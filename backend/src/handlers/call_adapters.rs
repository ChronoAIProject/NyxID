//! HTTP DTOs and transport only; admission and provider behavior live in services.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::channel_adapter::{Binding, CredentialBinding},
    mw::auth::AuthUser,
    services::{
        call_channels::{binding, coordinator, custom_llm, management, profiles, session},
        channel_adapters, channel_capability,
    },
};
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::convert::Infallible;
use tokio_stream::wrappers::ReceiverStream;

#[derive(Serialize)]
pub struct BindingDto {
    pub id: String,
    pub label: String,
    pub adapter_id: String,
    pub contract_version: u32,
    pub revision: i64,
    pub agent_id: String,
    pub credential: CredentialBinding,
    pub account_id: String,
    pub resource_id: String,
    pub phone_number: String,
    pub application_id: String,
    pub voice: String,
    pub language: String,
    pub enabled: bool,
    pub verified_at: String,
    pub webhook_url: String,
    pub availability: binding::Availability,
}
async fn dto(state: &AppState, b: Binding) -> BindingDto {
    let availability = binding::availability(state, &b).await;
    let webhook_url = coordinator::webhook_url(state, &b);
    BindingDto {
        id: b.id,
        label: b.label,
        adapter_id: profiles::id(b.profile).into(),
        contract_version: b.contract_version,
        revision: b.revision,
        agent_id: b.agent_id,
        credential: b.credential,
        account_id: b.account_id,
        resource_id: b.resource_id,
        phone_number: b.phone_number,
        application_id: b.application_id,
        voice: b.voice,
        language: b.language,
        enabled: b.enabled,
        verified_at: b.verified_at.to_rfc3339(),
        webhook_url,
        availability,
    }
}
pub async fn catalog(State(state): State<AppState>, _auth: AuthUser) -> Json<Value> {
    let descriptors: Vec<_> = channel_adapters::registered_interfaces(&state.token_exchange_cache)
        .into_iter()
        .map(|a| match a {
            channel_adapters::RegisteredInterface::Messaging(a) => {
                channel_capability::describe(a.as_ref())
            }
            channel_adapters::RegisteredInterface::Nyxbot(a) => a.descriptor(),
        })
        .collect();
    Json(json!({"adapters":descriptors}))
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(page): Query<Page>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    page.validate()?;
    let mut rows =
        management::list(&state, &auth.user_id.to_string(), page.after.as_deref()).await?;
    let next_after_id = page_cursor(&mut rows, |r| r.id.clone());
    let mut result = Vec::new();
    for b in rows {
        result.push(dto(&state, b).await);
    }
    Ok(Json(
        json!({"bindings":result,"next_after_id":next_after_id}),
    ))
}
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<binding::SetupInput>,
) -> AppResult<Json<BindingDto>> {
    super::login_client_context::require_first_party_human(&auth)?;
    binding::require_enabled(&state.db, &auth.user_id.to_string()).await?;
    let b = binding::create(&state, &auth.user_id.to_string(), input).await?;
    Ok(Json(dto(&state, b).await))
}
pub async fn disable(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    management::disable(&state, &auth.user_id.to_string(), &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn remove(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    management::remove(&state, &auth.user_id.to_string(), &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn reverify(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<BindingDto>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let b = management::reverify(&state, &auth.user_id.to_string(), &id).await?;
    Ok(Json(dto(&state, b).await))
}
pub async fn custom_setup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let b = management::owned(&state, &auth.user_id.to_string(), &id).await?;
    let result = custom_llm::setup(&state, &b)?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_adapter_custom_setup_issued",
        Some(json!({"binding_id":b.id,"revision":b.revision})),
    );
    Ok(Json(result))
}
pub async fn custom_session(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let b = management::owned(&state, &auth.user_id.to_string(), &id).await?;
    let result = custom_llm::create_session(&state, &b).await?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_adapter_custom_session_issued",
        Some(json!({"binding_id":b.id,"revision":b.revision,"session_id":result["session_id"]})),
    );
    Ok(Json(result))
}
pub async fn account(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<management::AccountInput>,
) -> AppResult<StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    management::configure_account(&state, &auth.user_id.to_string(), input).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn allocate(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<management::AllocationInput>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    let row = management::allocate(&state, &auth.user_id.to_string(), input).await?;
    Ok(Json(
        json!({"id":row.id,"owner_id":row.user_id,"resource_id":row.resource_id,"revision":row.revision}),
    ))
}
pub async fn revoke_allocation(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    management::revoke_allocation(&state, &id).await?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_adapter_allocation_revoked",
        Some(json!({"allocation_id":id})),
    );
    Ok(StatusCode::NO_CONTENT)
}
pub async fn accounts(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(page): Query<Page>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    page.validate()?;
    let mut rows = management::accounts(&state, page.after.as_deref()).await?;
    let next_after_id = page_cursor(&mut rows, |r| r.id.clone());
    Ok(Json(
        json!({"next_after_id":next_after_id,"accounts":rows.into_iter().map(|r|json!({"id":r.id,"catalog_service_id":r.catalog_service_id,"provider":r.provider,"account_id":r.account_id,"webhook_key_configured":r.webhook_public_key.is_some(),"revision":r.revision,"configured_by":r.configured_by,"updated_at":r.updated_at.to_rfc3339()})).collect::<Vec<_>>()}),
    ))
}
pub async fn allocations(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(page): Query<Page>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    page.validate()?;
    let mut rows = management::allocations(&state, page.after.as_deref()).await?;
    let next_after_id = page_cursor(&mut rows, |r| r.id.clone());
    Ok(Json(
        json!({"next_after_id":next_after_id,"allocations":rows.into_iter().map(|r|json!({"id":r.id,"catalog_service_id":r.catalog_service_id,"owner_id":r.user_id,"provider":r.provider,"account_id":r.account_id,"resource_id":r.resource_id,"phone_number":r.phone_number,"application_id":r.application_id,"revision":r.revision,"tariff_version":r.tariff_version,"tariff_source":r.tariff_source,"includes_all_provider_charges":r.includes_all_provider_charges,"tariff_verified_at":r.tariff_verified_at.to_rfc3339()})).collect::<Vec<_>>()}),
    ))
}
pub async fn remove_account(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    management::remove_account(&state, &id).await?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_adapter_account_revoked",
        Some(json!({"account_id":id})),
    );
    Ok(StatusCode::NO_CONTENT)
}
pub async fn end_custom_session(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, sid)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    let b = management::owned(&state, &auth.user_id.to_string(), &id).await?;
    let s = session::load(&state, &sid).await?;
    if s.binding_id != b.id
        || s.profile != crate::models::channel_adapter::Profile::ElevenlabsCustomLlm
    {
        return Err(AppError::NotFound("Session unavailable".into()));
    }
    session::finish_noncall(&state, &sid, "owner_closed").await?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_adapter_custom_session_ended",
        Some(json!({"binding_id":b.id,"revision":b.revision,"session_id":sid})),
    );
    Ok(StatusCode::NO_CONTENT)
}
pub async fn webhook(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: axum::extract::Request,
) -> AppResult<Response> {
    let (parts, body) = request.into_parts();
    let bytes = super::body_limit::read_body(
        body,
        crate::services::call_channels::protocol::MAX_INGRESS,
        "Carrier webhook",
    )
    .await?;
    let reply = coordinator::ingress(state, &id, &parts.headers, &bytes).await?;
    Ok(([("content-type", "application/xml")], reply).into_response())
}
pub async fn sms_receipt(
    State(state): State<AppState>,
    Path((id, send)): Path<(String, String)>,
    request: axum::extract::Request,
) -> AppResult<Response> {
    if uuid::Uuid::parse_str(&send).is_err() {
        return Err(AppError::ChannelWebhookVerificationFailed(
            "Invalid receipt identity".into(),
        ));
    }
    let (parts, body) = request.into_parts();
    let bytes = super::body_limit::read_body(
        body,
        crate::services::call_channels::protocol::MAX_INGRESS,
        "Carrier receipt",
    )
    .await?;
    let reply =
        coordinator::ingress_correlated(state, &id, &parts.headers, &bytes, Some(&send)).await?;
    Ok(([("content-type", "application/xml")], reply).into_response())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocketQuery {
    ticket: String,
}
pub async fn socket(
    State(state): State<AppState>,
    Path((id, sid)): Path<(String, String)>,
    Query(q): Query<SocketQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> AppResult<Response> {
    let b = binding::load(&state, &id).await?;
    let s = session::load(&state, &sid).await?;
    if s.binding_id != b.id {
        return Err(AppError::AssistantTurnRequired);
    }
    coordinator::authenticate_socket(&state, &b, &s, &headers, &q.ticket).await?;
    Ok(ws
        .max_message_size(16384)
        .max_frame_size(16384)
        .on_upgrade(move |socket| coordinator::socket(state, b, s, socket)))
}
pub async fn completions(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: axum::extract::Request,
) -> AppResult<Response> {
    let (parts, body) = request.into_parts();
    let bytes = super::body_limit::read_body(body, 65536, "Custom LLM request").await?;
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::BadRequest("Invalid Custom LLM request".into()))?;
    let b = binding::load(&state, &id).await?;
    let s = custom_llm::authenticate(&state, &b, &parts.headers, &body).await?;
    let profiles::Adapter::CustomLlm(adapter) = channel_adapters::resolve_nyxbot_profile(
        profiles::id(b.profile),
        &state.token_exchange_cache,
    )?
    else {
        return Err(AppError::ChannelPlatformSendUnsupported);
    };
    let (mut output, worker) = adapter.start(state.clone(), &b, &s, &body).await?;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, Infallible>>(64);
    tokio::spawn(async move {
        let mut completed = false;
        let deadline = tokio::time::sleep(std::time::Duration::from_secs(1800));
        tokio::pin!(deadline);
        loop {
            let out = tokio::select! {_=tx.closed()=>break,_=&mut deadline=>break,out=output.recv()=>{let Some(out)=out else{break};out}};
            if out.error {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    tx.send(Ok(bytes::Bytes::from_static(
                        b"data: {\"error\":{\"code\":\"agent_response_failed\"}}\n\n",
                    ))),
                )
                .await;
                break;
            }
            if session::fence(&state, &s).await.is_err() {
                break;
            }
            let chunk = custom_llm::chunk(&s.id, &out.text, out.last);
            if !matches!(
                tokio::time::timeout(std::time::Duration::from_secs(5), tx.send(Ok(chunk.into())))
                    .await,
                Ok(Ok(()))
            ) {
                break;
            }
            if tx.is_closed() {
                break;
            }
            if out.last {
                completed = true;
                break;
            }
        }
        if completed {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                tx.send(Ok(bytes::Bytes::from_static(b"data: [DONE]\n\n"))),
            )
            .await;
        } else {
            let _ = crate::services::assistant_nyxagent::request_stop(
                &state.db,
                &s.user_id,
                &s.conversation_id,
            )
            .await;
        }
        let _ = worker.await;
    });
    Ok((
        [
            ("content-type", "text/event-stream"),
            ("cache-control", "no-cache"),
            ("x-accel-buffering", "no"),
        ],
        Body::from_stream(ReceiverStream::new(rx)),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    after: Option<String>,
}
impl Page {
    fn validate(&self) -> AppResult<()> {
        if self
            .after
            .as_ref()
            .is_some_and(|s| uuid::Uuid::parse_str(s).is_err())
        {
            return Err(AppError::BadRequest("Invalid page cursor".into()));
        }
        Ok(())
    }
}
fn page_cursor<T>(rows: &mut Vec<T>, id: impl Fn(&T) -> String) -> Option<String> {
    if rows.len() > 100 {
        rows.truncate(100);
        rows.last().map(id)
    } else {
        None
    }
}
