//! Human management and administrator-owned shared account/resource allocation.
use super::{binding, profiles, protocol};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::channel_adapter::*,
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::Deserialize;
pub(crate) fn audit(state: &AppState, actor: &str, event: &str, data: serde_json::Value) {
    crate::services::audit_service::log_async(
        state.db.clone(),
        Some(actor.into()),
        event.into(),
        Some(data),
        None,
        None,
        None,
        None,
    );
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountInput {
    pub catalog_service_id: String,
    pub adapter_id: String,
    pub account_id: String,
    pub webhook_public_key: Option<String>,
}
pub async fn configure_account(
    state: &AppState,
    admin: &str,
    input: AccountInput,
) -> AppResult<()> {
    let adapter = crate::services::channel_adapters::resolve_nyxbot_profile(
        &input.adapter_id,
        &state.token_exchange_cache,
    )?;
    if uuid::Uuid::parse_str(&input.catalog_service_id).is_err()
        || !protocol::handle(&input.account_id)
    {
        return Err(AppError::ValidationError("Invalid provider account".into()));
    }
    if profiles::provider(adapter.profile()) == "telnyx" {
        protocol::validate_public_key(
            input
                .webhook_public_key
                .as_deref()
                .ok_or(AppError::VoiceProviderUnavailable)?,
        )?;
    }
    binding::resolve(
        state,
        admin,
        &CredentialBinding::Platform {
            catalog_service_id: input.catalog_service_id.clone(),
        },
        adapter.profile(),
    )
    .await?;
    let filter =
        doc! {"provider":profiles::provider(adapter.profile()),"account_id":&input.account_id};
    let saved=state.db.collection::<ProviderAccount>(ACCOUNTS).find_one_and_update(filter,doc!{"$set":{"catalog_service_id":input.catalog_service_id,"webhook_public_key":input.webhook_public_key,"configured_by":admin,"updated_at":bson::DateTime::from_chrono(Utc::now())},"$inc":{"revision":1},"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string()}}).upsert(true).return_document(mongodb::options::ReturnDocument::After).await?.ok_or_else(||AppError::Conflict("Trusted account changed".into()))?;
    audit(
        state,
        admin,
        "channel_adapter_account_configured",
        serde_json::json!({"trusted_account_id":saved.id,"revision":saved.revision,"catalog_service_id":saved.catalog_service_id,"provider":saved.provider}),
    );
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocationResources {
    pub adapter_id: String,
    pub credential: CredentialBinding,
    pub account_id: String,
    pub resource_id: String,
    pub phone_number: String,
    pub application_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocationInput {
    pub setup: AllocationResources,
    pub owner_id: String,
    pub tariff_version: String,
    pub tariff_source: String,
    pub includes_all_provider_charges: bool,
}
pub async fn allocate(
    state: &AppState,
    admin: &str,
    input: AllocationInput,
) -> AppResult<ResourceAllocation> {
    let adapter = crate::services::channel_adapters::resolve_nyxbot_profile(
        &input.setup.adapter_id,
        &state.token_exchange_cache,
    )?;
    let CredentialBinding::Platform { catalog_service_id } = &input.setup.credential else {
        return Err(AppError::BadRequest(
            "Platform binding required for allocation".into(),
        ));
    };
    if uuid::Uuid::parse_str(&input.owner_id).is_err()
        || input.tariff_version.is_empty()
        || input.tariff_version.len() > 128
        || !input.tariff_source.starts_with("https://")
        || input.tariff_source.len() > 2048
        || !protocol::phone(&input.setup.phone_number)
        || !protocol::handle(&input.setup.account_id)
        || !protocol::handle(&input.setup.resource_id)
        || !protocol::handle(&input.setup.application_id)
    {
        return Err(AppError::ValidationError(
            "Invalid resource allocation".into(),
        ));
    }
    let owner = state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":&input.owner_id,"is_active":true})
        .await?
        .ok_or_else(|| {
            AppError::ValidationError("Allocation requires an active person account".into())
        })?;
    if !owner.user_type.is_person() {
        return Err(AppError::ValidationError(
            "Allocation requires an active person account".into(),
        ));
    }
    let account = binding::trusted_account(
        state,
        catalog_service_id,
        &input.setup.account_id,
        adapter.profile(),
    )
    .await?;
    let r = binding::resolve(state, admin, &input.setup.credential, adapter.profile()).await?;
    let now = Utc::now();
    let b = Binding {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: input.owner_id.clone(),
        profile: adapter.profile(),
        contract_version: 1,
        revision: 1,
        label: "Allocated provider resource".into(),
        agent_id: String::new(),
        credential: input.setup.credential.clone(),
        account_id: input.setup.account_id.clone(),
        resource_id: input.setup.resource_id.clone(),
        phone_number: input.setup.phone_number.clone(),
        application_id: input.setup.application_id.clone(),
        webhook_public_key: account.webhook_public_key,
        voice: if profiles::provider(adapter.profile()) == "twilio" {
            "en-US-Standard-A"
        } else {
            "Telnyx.Ultra.Callie"
        }
        .into(),
        language: "en-US".into(),
        enabled: false,
        verified_credential_identity: r.identity.clone(),
        trusted_account_revision: Some(account.revision),
        verified_at: now,
        created_at: now,
        updated_at: now,
    };
    for path in binding::resource_paths(&b) {
        binding::authorize(state, &b, &r, "GET", &path, None).await?;
    }
    binding::verify_resources(
        crate::services::proxy_service::no_redirect_http_client(),
        &b,
        &r.key,
        None,
    )
    .await?;
    let allocation = ResourceAllocation {
        id: uuid::Uuid::new_v4().to_string(),
        catalog_service_id: catalog_service_id.clone(),
        user_id: input.owner_id,
        provider: profiles::provider(adapter.profile()).into(),
        account_id: input.setup.account_id,
        resource_id: input.setup.resource_id,
        phone_number: input.setup.phone_number,
        application_id: input.setup.application_id,
        allocated_by: admin.into(),
        revision: 1,
        tariff_version: input.tariff_version,
        tariff_source: input.tariff_source,
        includes_all_provider_charges: input.includes_all_provider_charges,
        tariff_verified_at: now,
        created_at: now,
    };
    let mut tx = state.db.client().start_session().await?;
    tx.start_transaction().await?;
    super::owner_admission::fence(&state.db, &mut tx, &allocation.user_id).await?;
    let trusted = state.db.collection::<ProviderAccount>(ACCOUNTS)
        .update_one(doc!{"_id":&account.id,"revision":account.revision,"catalog_service_id":catalog_service_id},doc!{"$inc":{"allocation_fence":1}})
        .session(&mut tx).await?;
    if trusted.matched_count != 1 {
        return Err(AppError::Conflict(
            "Trusted provider account changed; verify again".into(),
        ));
    }
    binding::claim_resources(&state.db, &mut tx, &b).await?;
    state.db.collection::<ResourceAllocation>(ALLOCATIONS).insert_one(&allocation).session(&mut tx).await.map_err(|e|if binding::duplicate(&e){AppError::Conflict("Number or application is already allocated; revoke the previous allocation explicitly".into())}else{e.into()})?;
    tx.commit_transaction().await?;
    crate::services::audit_service::log_async(
        state.db.clone(),
        Some(admin.into()),
        "channel_adapter_resource_allocated".into(),
        Some(
            serde_json::json!({"allocation_id":allocation.id,"owner_id":allocation.user_id,"revision":allocation.revision}),
        ),
        None,
        None,
        None,
        None,
    );
    Ok(allocation)
}
pub async fn revoke_allocation(state: &AppState, id: &str) -> AppResult<()> {
    let allocation = state
        .db
        .collection::<ResourceAllocation>(ALLOCATIONS)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(|| AppError::NotFound("Allocation not found".into()))?;
    let bindings:Vec<Binding>=state.db.collection::<Binding>(BINDINGS).find(doc!{"user_id":&allocation.user_id,"account_id":&allocation.account_id,"resource_id":&allocation.resource_id}).await?.try_collect().await?;
    for b in &bindings {
        disable(state, &b.user_id, &b.id).await?;
        ensure_idle(state, &b.id).await?;
    }
    let mut tx = state.db.client().start_session().await?;
    tx.start_transaction().await?;
    let deleted = state
        .db
        .collection::<ResourceAllocation>(ALLOCATIONS)
        .delete_one(doc! {"_id":id,"revision":allocation.revision})
        .session(&mut tx)
        .await?;
    if deleted.deleted_count != 1 {
        return Err(AppError::Conflict(
            "Allocation changed; reload before revoking".into(),
        ));
    }
    for old in &bindings {
        let b = state
            .db
            .collection::<Binding>(BINDINGS)
            .find_one(doc! {"_id":&old.id,"enabled":false})
            .session(&mut tx)
            .await?
            .ok_or_else(|| AppError::Conflict("Binding changed".into()))?;
        idle_in_transaction(&state.db, &mut tx, &b.id).await?;
        let deleted = state
            .db
            .collection::<Binding>(BINDINGS)
            .delete_one(doc! {"_id":&b.id,"enabled":false,"revision":b.revision})
            .session(&mut tx)
            .await?;
        if deleted.deleted_count != 1 {
            return Err(AppError::Conflict("Binding changed".into()));
        }
        release_unused_claims(&state.db, &mut tx, &b).await?;
    }
    if bindings.is_empty() {
        for (kind, field, value) in [
            ("number_id", "resource_id", allocation.resource_id.as_str()),
            ("phone", "phone_number", allocation.phone_number.as_str()),
            (
                "application",
                "application_id",
                allocation.application_id.as_str(),
            ),
        ] {
            let mut remaining = doc! {"provider":&allocation.provider};
            remaining.insert(field, value);
            if state
                .db
                .collection::<ResourceAllocation>(ALLOCATIONS)
                .find_one(remaining)
                .session(&mut tx)
                .await?
                .is_none()
            {
                state.db.collection::<ResourceClaim>(CLAIMS).delete_one(doc!{"identity_key":super::session::identity(&[&allocation.provider,kind,value]),"user_id":&allocation.user_id}).session(&mut tx).await?;
            }
        }
    }
    tx.commit_transaction().await?;
    Ok(())
}
pub async fn owned(state: &AppState, owner: &str, id: &str) -> AppResult<Binding> {
    let b = binding::load(state, id).await?;
    if b.user_id != owner {
        return Err(AppError::NotFound("Binding unavailable".into()));
    }
    Ok(b)
}
pub async fn list(state: &AppState, owner: &str, after: Option<&str>) -> AppResult<Vec<Binding>> {
    Ok(state
        .db
        .collection::<Binding>(BINDINGS)
        .find(page_filter(doc! {"user_id":owner}, after))
        .sort(doc! {"_id":1})
        .limit(101)
        .await?
        .try_collect()
        .await?)
}
pub async fn disable(state: &AppState, owner: &str, id: &str) -> AppResult<()> {
    let b = owned(state, owner, id).await?;
    let changed=state.db.collection::<Binding>(BINDINGS).update_one(doc!{"_id":id,"user_id":owner,"revision":b.revision},doc!{"$inc":{"revision":1},"$set":{"enabled":false,"updated_at":bson::DateTime::from_chrono(Utc::now())}}).await?;
    if changed.matched_count != 1 {
        return Err(AppError::Conflict("Binding changed".into()));
    }
    audit(
        state,
        owner,
        "channel_adapter_binding_disabled",
        serde_json::json!({"binding_id":id,"revision":b.revision+1}),
    );
    // Immediate cleanup is bounded; every remaining session becomes eligible
    // for the fair recovery sweep and cannot retain its old generation.
    state
        .db
        .collection::<CallSession>(SESSIONS)
        .update_many(
            doc! {"binding_id":id,"state":{"$ne":"closed"}},
            doc! {"$inc":{"generation":1_i64},"$set":{"lease_expires_at":bson::DateTime::now()}},
        )
        .await?;
    let rows: Vec<CallSession> = state
        .db
        .collection::<CallSession>(SESSIONS)
        .find(doc! {"binding_id":id,"state":{"$ne":"closed"}})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    for s in rows {
        let _ =
            crate::services::assistant_nyxagent::request_stop(&state.db, owner, &s.conversation_id)
                .await;
        if profiles::relay(s.profile) {
            let _ = super::coordinator::close(state, &b, &s, false).await;
        } else {
            super::session::finish_noncall(state, &s.id, "binding_disabled").await?;
        }
    }
    Ok(())
}
pub async fn reverify(state: &AppState, owner: &str, id: &str) -> AppResult<Binding> {
    let mut b = owned(state, owner, id).await?;
    let r = binding::resolve(state, owner, &b.credential, b.profile).await?;
    binding::validate_agent(state, owner, &b.agent_id).await?;
    if let CredentialBinding::Platform { catalog_service_id } = &b.credential {
        binding::allocation(state, &b, catalog_service_id).await?;
        let a =
            binding::trusted_account(state, catalog_service_id, &b.account_id, b.profile).await?;
        b.webhook_public_key = a.webhook_public_key;
        b.trusted_account_revision = Some(a.revision);
    }
    for path in binding::resource_paths(&b) {
        binding::authorize(state, &b, &r, "GET", &path, None).await?;
    }
    binding::verify_resources(
        crate::services::proxy_service::no_redirect_http_client(),
        &b,
        &r.key,
        None,
    )
    .await?;
    let old = b.revision;
    b.revision += 1;
    b.enabled = true;
    b.verified_credential_identity = r.identity;
    b.verified_at = Utc::now();
    b.updated_at = Utc::now();
    let mut tx = state.db.client().start_session().await?;
    let db = state.db.clone();
    let verified = b.clone();
    tx.start_transaction()
        .and_run2(async move |tx| {
            let result: AppResult<()> = async {
                super::owner_admission::fence(&db, tx, &verified.user_id).await?;
                let changed = db
                    .collection::<Binding>(BINDINGS)
                    .replace_one(
                        doc! {"_id":&verified.id,"user_id":&verified.user_id,"revision":old},
                        &verified,
                    )
                    .session(&mut *tx)
                    .await?;
                if changed.matched_count != 1 {
                    return Err(AppError::Conflict("Binding changed".into()));
                }
                Ok(())
            }
            .await;
            crate::services::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    audit(
        state,
        owner,
        "channel_adapter_binding_reverified",
        serde_json::json!({"binding_id":id,"revision":b.revision}),
    );
    Ok(b)
}

/// Disable stops work; deletion is explicit and waits for all admitted evidence.
pub async fn ensure_idle(state: &AppState, id: &str) -> AppResult<()> {
    if state
        .db
        .collection::<CallSession>(SESSIONS)
        .find_one(
            doc! {"binding_id":id,"$or":[{"state":{"$ne":"closed"}},{"billing_finalized":false}]},
        )
        .await?
        .is_some()
        || state
            .db
            .collection::<EventReceipt>(EVENTS)
            .find_one(doc! {"binding_id":id,"event_kind":"sms_send","billing_finalized":false})
            .await?
            .is_some()
    {
        return Err(AppError::Conflict("Session cleanup or admitted usage is pending. Disable the binding and wait for finalization before removal.".into()));
    }
    Ok(())
}
pub async fn remove(state: &AppState, owner: &str, id: &str) -> AppResult<()> {
    let b = owned(state, owner, id).await?;
    if b.enabled {
        return Err(AppError::Conflict(
            "Disable this binding before removing it".into(),
        ));
    }
    ensure_idle(state, id).await?;
    let mut tx = state.db.client().start_session().await?;
    tx.start_transaction().await?;
    idle_in_transaction(&state.db, &mut tx, id).await?;
    let result = state
        .db
        .collection::<Binding>(BINDINGS)
        .delete_one(doc! {"_id":id,"user_id":owner,"revision":b.revision,"enabled":false})
        .session(&mut tx)
        .await?;
    if result.deleted_count != 1 {
        return Err(AppError::Conflict("Binding changed".into()));
    }
    release_unused_claims(&state.db, &mut tx, &b).await?;
    tx.commit_transaction().await?;
    audit(
        state,
        owner,
        "channel_adapter_binding_removed",
        serde_json::json!({"binding_id":id,"revision":b.revision}),
    );
    Ok(())
}
async fn release_unused_claims(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    b: &Binding,
) -> AppResult<()> {
    let provider = profiles::provider(b.profile);
    let fields = if b.profile == Profile::ElevenlabsCustomLlm {
        vec![("agent", "resource_id", b.resource_id.as_str())]
    } else {
        vec![
            ("number_id", "resource_id", b.resource_id.as_str()),
            ("phone", "phone_number", b.phone_number.as_str()),
            ("application", "application_id", b.application_id.as_str()),
        ]
    };
    for (kind, field, value) in fields {
        let mut filter = doc! {"profile":{"$in":if provider=="telnyx" {vec!["telnyx_relay","telnyx_sms"]}else if provider=="twilio"{vec!["twilio_relay","twilio_sms"]}else{vec!["elevenlabs_custom_llm"]}}};
        filter.insert(field, value);
        let bindings = db
            .collection::<Binding>(BINDINGS)
            .find_one(filter)
            .session(&mut *tx)
            .await?
            .is_some();
        let mut allocation = doc! {"provider":provider};
        allocation.insert(field, value);
        let allocated = db
            .collection::<ResourceAllocation>(ALLOCATIONS)
            .find_one(allocation)
            .session(&mut *tx)
            .await?
            .is_some();
        if !bindings && !allocated {
            db.collection::<ResourceClaim>(CLAIMS).delete_one(doc!{"identity_key":super::session::identity(&[provider,kind,value]),"user_id":&b.user_id}).session(&mut *tx).await?;
        }
    }
    Ok(())
}

pub async fn accounts(state: &AppState, after: Option<&str>) -> AppResult<Vec<ProviderAccount>> {
    Ok(state
        .db
        .collection::<ProviderAccount>(ACCOUNTS)
        .find(page_filter(doc! {}, after))
        .sort(doc! {"_id":1})
        .limit(101)
        .await?
        .try_collect()
        .await?)
}
pub async fn allocations(
    state: &AppState,
    after: Option<&str>,
) -> AppResult<Vec<ResourceAllocation>> {
    Ok(state
        .db
        .collection::<ResourceAllocation>(ALLOCATIONS)
        .find(page_filter(doc! {}, after))
        .sort(doc! {"_id":1})
        .limit(101)
        .await?
        .try_collect()
        .await?)
}
pub async fn remove_account(state: &AppState, id: &str) -> AppResult<()> {
    let account = state
        .db
        .collection::<ProviderAccount>(ACCOUNTS)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(|| AppError::NotFound("Trusted account unavailable".into()))?;
    if state
        .db
        .collection::<ResourceAllocation>(ALLOCATIONS)
        .find_one(doc! {"provider":&account.provider,"account_id":&account.account_id})
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Revoke the account's allocations before removing its trust configuration".into(),
        ));
    }
    let mut tx = state.db.client().start_session().await?;
    tx.start_transaction().await?;
    if state
        .db
        .collection::<ResourceAllocation>(ALLOCATIONS)
        .find_one(doc! {"provider":&account.provider,"account_id":&account.account_id})
        .session(&mut tx)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict("Account allocations changed".into()));
    }
    let result = state
        .db
        .collection::<ProviderAccount>(ACCOUNTS)
        .delete_one(doc! {"_id":id,"revision":account.revision})
        .session(&mut tx)
        .await?;
    if result.deleted_count != 1 {
        return Err(AppError::Conflict("Trusted account changed".into()));
    }
    tx.commit_transaction().await?;
    Ok(())
}

async fn idle_in_transaction(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    id: &str,
) -> AppResult<()> {
    if db
        .collection::<CallSession>(SESSIONS)
        .find_one(
            doc! {"binding_id":id,"$or":[{"state":{"$ne":"closed"}},{"billing_finalized":false}]},
        )
        .session(&mut *tx)
        .await?
        .is_some()
        || db
            .collection::<EventReceipt>(EVENTS)
            .find_one(doc! {"binding_id":id,"event_kind":"sms_send","billing_finalized":false})
            .session(&mut *tx)
            .await?
            .is_some()
    {
        return Err(AppError::Conflict(
            "Admitted session evidence is still pending".into(),
        ));
    }
    Ok(())
}

fn page_filter(mut filter: bson::Document, after: Option<&str>) -> bson::Document {
    if let Some(after) = after {
        filter.insert("_id", doc! {"$gt":after});
    }
    filter
}
