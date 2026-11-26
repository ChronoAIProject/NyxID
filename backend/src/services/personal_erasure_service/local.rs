//! Explicit personal-owner inventory. Every pass removes at most 32 rows in a
//! majority transaction. Ownership predicates are declared here, never inferred
//! from creator/updater attribution. Durable admission tombstones survive the
//! final source deletion and remain the writer barrier.
use super::*;
use crate::services::consent_service::REVOCATION_FENCES;
use futures::TryStreamExt;

const PAGE: i64 = 32;

/// Bump only when the local inventory gains coverage. `INVENTORY_VERSION` is
/// the durable domain shape and intentionally remains unchanged for old jobs.
pub(crate) const LOCAL_INVENTORY_REVISION: u32 = 5;

pub(crate) fn inventory(owner: &str) -> Vec<(&'static str, Document)> {
    use crate::models as m;
    let mut rows: Vec<(&'static str, Document)> = vec![
        (
            m::client_error_report::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_cma_sandbox::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_cma_agent::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_cma_creation::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            super::preparation::PREPARATIONS,
            doc! {"$or":[{"_id":owner},{"user_id":owner}]},
        ),
        (
            provider::EFFECTS,
            doc! {"user_id":owner,"kind":"oauth_issuance","status":"complete",
            "reason":{"$in":["credential_installed","protocol_no_issuance"]},
            "capability_encrypted":{"$exists":false},"completion_basis":{"$exists":false}},
        ),
        (m::channel_message::COLLECTION_NAME, doc! {"user_id":owner}),
        (
            m::channel_reply_delivery::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::channel_connect_link::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (m::service_preference::COLLECTION_NAME, doc! {"_id":owner}),
        (m::usage_workspace::COLLECTION_NAME, doc! {"_id":owner}),
        (m::cli_pairing::COLLECTION_NAME, doc! {"user_id":owner}),
        (
            m::catalog_delegation_grant::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::machine_oauth_consent_request::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (m::channel_delivery::COLLECTION_NAME, doc! {"user_id":owner}),
        (
            m::execution_permission_channel::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::channel_whatsapp_native::COLLECTION_NAME,
            doc! {"owner_id":owner},
        ),
        (
            m::service_account_key_read_grant::COLLECTION_NAME,
            doc! {"$or":[{"_id":owner},{"owner_id":owner}]},
        ),
        // Link webhook state is an outbox, not an ordinary callback URL. Only
        // unclaimed, never-reserved links are eligible here; terminal effect
        // receipts remain in their existing bounded lifecycle.
        (
            m::connect_link::COLLECTION_NAME,
            doc! {"user_id":owner,
            "completion_claim_id":bson::Bson::Null,"webhook_event_reserved_at":bson::Bson::Null},
        ),
        (m::login_code::COLLECTION_NAME, doc! {"user_id":owner}),
        (m::login_approval::COLLECTION_NAME, doc! {"user_id":owner}),
        (
            m::oauth_broker_binding::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::oauth_delegation_authority::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::oauth_consent_request::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::mcp_session::MCP_SESSION_SLOT_COLLECTION,
            doc! {"user_id":owner},
        ),
        (m::trigger_delivery::COLLECTION_NAME, doc! {"user_id":owner}),
        (m::trigger_run::COLLECTION_NAME, doc! {"user_id":owner}),
        (
            m::assistant_direct_handoff::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (m::trigger::COLLECTION_NAME, doc! {"user_id":owner}),
        (super::super::trigger_schedule::WORK, doc! {"user_id":owner}),
        (
            super::super::trigger_schedule::OWNER_LIMITS,
            doc! {"_id":owner},
        ),
        (super::super::trigger_schedule::BUDGETS, doc! {"_id":owner}),
        (
            m::service_pool_member_health::COLLECTION_NAME,
            doc! {"owner_id":owner},
        ),
        (m::service_pool::COLLECTION_NAME, doc! {"user_id":owner}),
        (
            m::device_onboard_credential::COLLECTION_NAME,
            doc! {"owner_user_id":owner},
        ),
        (
            m::auth_device_code::COLLECTION_NAME,
            doc! {"approved_user_id":owner},
        ),
        (
            m::auth_device_code::V2_COLLECTION_NAME,
            doc! {"approved_user_id":owner},
        ),
        (
            m::agent_key_login_request::COLLECTION_NAME,
            doc! {"approved_user_id":owner},
        ),
        (
            m::oracle_session::COLLECTION_NAME,
            doc! {"owner_user_id":owner},
        ),
        (
            m::device_code::COLLECTION_NAME,
            doc! {"approved_by_user_id":owner,"approved_org_id":bson::Bson::Null},
        ),
        // An unredeemed invite is authority granted by this departing admin;
        // its org is retained, as are redeemed attribution/audit records.
        (
            m::org_invite::COLLECTION_NAME,
            doc! {"created_by":owner,"redeemed_at":bson::Bson::Null},
        ),
        (
            m::assistant_group::REQUESTS_COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (m::channel_adapter::BINDINGS, doc! {"user_id":owner}),
        (m::channel_adapter::ALLOCATIONS, doc! {"user_id":owner}),
        (
            m::channel_adapter::SESSIONS,
            doc! {"user_id":owner,"state":"closed","billing_finalized":true},
        ),
        (
            m::channel_adapter::EVENTS,
            doc! {"user_id":owner,"$or":[{"event_kind":{"$ne":"sms_send"}},{"billing_finalized":true}]},
        ),
        (m::channel_adapter::CLAIMS, doc! {"user_id":owner}),
        (
            m::channel_meta::SESSIONS,
            doc! {"$or":[{"owner_id":owner},{"actor_id":owner}]},
        ),
        (m::channel_email::SUBSCRIPTIONS, doc! {"user_id":owner}),
        (m::channel_email::SENDS, doc! {"user_id":owner}),
        (m::channel_email::BATCHES, doc! {"user_id":owner}),
        (m::channel_email::RECEIPTS, doc! {"user_id":owner}),
        (
            m::channel_bot::COLLECTION_NAME,
            doc! {"user_id":owner,"personal_erasure_cleanup_complete":true},
        ),
        (
            m::async_service_operation::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::cma_session_origin::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::async_service_operation::QUOTAS_COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::generated_artifact::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (m::generated_artifact::DELIVERIES, doc! {"user_id":owner}),
        (m::generated_artifact::SERIES, doc! {"user_id":owner}),
        (m::saved_login::COLLECTION_NAME, doc! {"user_id":owner}),
        (m::machine_setup::COLLECTION_NAME, doc! {"user_id":owner}),
        (m::machine_desktop::COLLECTION_NAME, doc! {"user_id":owner}),
        (m::machine_update::COLLECTION_NAME, doc! {"user_id":owner}),
        (
            m::machine_update::ATTEMPTS_COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::channel_activity::NOTIFICATIONS_COLLECTION,
            doc! {"user_id":owner},
        ),
        (m::session::COLLECTION_NAME, doc! {"user_id":owner}),
        (m::session::MFA_CHALLENGES, doc! {"user_id":owner}),
        (
            m::mobile_social::MobileSocialDelivery::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (m::refresh_token::COLLECTION_NAME, doc! {"user_id":owner}),
        (m::api_key::COLLECTION_NAME, doc! {"user_id":owner}),
        (
            m::api_key_credential::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::permission_policy::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_notification::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            super::super::assistant_notification_service::PUSH_BUDGETS,
            doc! {"_id":owner},
        ),
        (
            super::super::assistant_notification_service::PUSH_SOURCES,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_acknowledgement::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_attachment::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_tool_result::COLLECTION_NAME,
            doc! {"$or":[{"user_id":owner},{"actor_id":owner}]},
        ),
        (m::lark_base_write::COLLECTION_NAME, doc! {"user_id":owner}),
        (
            m::lark_base_write_photo::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_resource::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::nyxbot_image_delivery::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::channel_text_delivery::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::nyxbot_image_delivery::LEGACY_COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_upload_retention::TOMBSTONES,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_agent_credential::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_message::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_invocation::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (m::assistant_invocation::TURNS, doc! {"user_id":owner}),
        (
            m::assistant_message_feedback::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_checkpoint::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            super::super::assistant_service_execution::COLLECTION,
            doc! {"user_id":owner},
        ),
        (
            m::nyxbot_channel::THREADS_COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::nyxbot_channel::EVENTS_COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::nyxbot_event_binding::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::nyxbot_channel::WATCHES_COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::nyxbot_gateway_response::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_voice_announcement::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (m::assistant_voice::REQUESTS, doc! {"user_id":owner}),
        (m::assistant_settings::COLLECTION_NAME, doc! {"_id":owner}),
        (m::assistant_onboarding::COLLECTION_NAME, doc! {"_id":owner}),
        (
            m::assistant_sharing::COLLECTION_NAME,
            doc! {"owner_id": owner},
        ),
        (
            m::assistant_sharing::SHARES_COLLECTION_NAME,
            doc! {"$or": [{"owner_id": owner}, {"subject_id": owner, "subject_kind": "person"}]},
        ),
        (
            m::assistant_sharing::NOTES_COLLECTION_NAME,
            doc! {"actor_user_id": owner},
        ),
        (
            m::assistant_sharing::CONSENTS_COLLECTION_NAME,
            doc! {"$or": [{"actor_user_id": owner}, {"owner_id": owner}]},
        ),
        (
            m::assistant_skill_draft_attempt::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_agent_learning::AUTHORED_CLAIMS_COLLECTION_NAME,
            doc! {"owner_id":owner},
        ),
        (
            m::assistant_agent_learning::CONFIG_COLLECTION_NAME,
            doc! {"owner_id":owner},
        ),
        (
            m::assistant_agent_learning::RUNS_COLLECTION_NAME,
            doc! {"owner_id":owner},
        ),
        (
            m::assistant_agent_learning::PROPOSALS_COLLECTION_NAME,
            doc! {"owner_id":owner},
        ),
        (
            m::assistant_agent_learning::ROOTS_COLLECTION_NAME,
            doc! {"owner_id":owner},
        ),
        (
            m::user_service_connection::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::user_api_key::COLLECTION_NAME,
            doc! {"user_id":owner,"personal_erasure_preserved":true},
        ),
        (
            m::user_provider_token::COLLECTION_NAME,
            doc! {"user_id":owner,"personal_erasure_preserved":true},
        ),
        (
            m::user_provider_credentials::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (m::mfa_factor::COLLECTION_NAME, doc! {"user_id":owner}),
        (
            m::authorization_code::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (m::oauth_state::COLLECTION_NAME, doc! {"user_id":owner}),
        (m::consent::COLLECTION_NAME, doc! {"user_id":owner}),
        (REVOCATION_FENCES, doc! {"user_id":owner}),
        (
            m::mcp_session::MCP_SESSION_COLLECTION,
            doc! {"user_id":owner},
        ),
        (m::approval_request::COLLECTION_NAME, doc! {"user_id":owner}),
        (m::approval_grant::COLLECTION_NAME, doc! {"user_id":owner}),
        (
            m::service_approval_config::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::notification_channel::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_group_collaboration::READ_STATES,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_group_collaboration::MENTION_NOTIFICATIONS,
            doc! {"user_id":owner},
        ),
        (
            m::node_registration_token::COLLECTION_NAME,
            doc! {"$or":[{"user_id":owner},{"issuer_user_id":owner}]},
        ),
        (
            m::agent_service_binding::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (m::user_service::COLLECTION_NAME, doc! {"user_id":owner}),
        (m::user_endpoint::COLLECTION_NAME, doc! {"user_id":owner}),
        (
            m::org_membership::COLLECTION_NAME,
            doc! {"member_user_id":owner},
        ),
        // Parents come after payloads so no retained child loses its owner join.
        (
            m::assistant_group::MESSAGES_COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_group_collaboration::INVITES,
            doc! {"$or":[{"issuer_user_id":owner},{"accepted_by_user_id":owner},{"intended_user_id":owner}]},
        ),
        (
            m::assistant_group_collaboration::PROPOSALS,
            doc! {"proposed_by":owner},
        ),
        (
            m::assistant_group_collaboration::SLOTS,
            doc! {"original_requester_id":owner,"conversation_id":null},
        ),
        (m::assistant_group::COLLECTION_NAME, doc! {"user_id":owner}),
        (
            m::assistant_conversation::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (m::assistant_agent::COLLECTION_NAME, doc! {"user_id":owner}),
    ];
    // This person's membership in another owner's learning scope is personal
    // participation, not ownership of its shared proposals or published skill.
    rows.push((
        m::assistant_agent_learning::MEMBERS_COLLECTION_NAME,
        doc! {"member_user_id":owner},
    ));
    // Collections added after the original inventory. Keep these selectors
    // explicit so a mixed-fleet rollout cannot silently leave personal rows.
    rows.extend([
        ("assistant_action_receipts", doc! {"user_id": owner}),
        ("assistant_voice_windows", doc! {"user_id": owner}),
        ("assistant_wire_logs", doc! {"user_id": owner}),
        ("openclaw_channel_mappings", doc! {"nyxid_user_id": owner}),
        ("telegram_bot_requests", doc! {"owner_user_id": owner, "active": false, "status": {"$in": ["connected", "cancelled", "expired", "suspended"]}}),
        (
            "telegram_bot_claims",
            doc! {"owner_user_id": owner, "status":"redeemed", "delivered": true},
        ),
        (
            "feature_flag_overrides",
            doc! {"target_kind": "user", "target_key": owner},
        ),
        ("invocation_diagnostics", doc! {"actor_user_id": owner}),
        (
            "platform_personal_setup_selections",
            doc! {"user_id": owner},
        ),
        (
            "platform_onboarding_binding_requests",
            doc! {"approved_user_id": owner},
        ),
        ("platform_onboarding_bindings", doc! {"user_id": owner}),
        (
            "platform_personal_decision_spacing",
            doc! {"_id": owner},
        ),
        ("assistant_group_agent_access", doc! {"owner_id": owner}),
    ]);
    rows.extend([
        ("platform_personal_bindings", doc! {"user_id": owner, "state": "revoked"}),
        ("platform_personal_link_challenges", doc! {"reviewed_user_id": owner, "state": {"$nin": ["review", "awaiting_chat_confirmation"]}}),
        ("platform_personal_link_intents", doc! {"claimant_user_id": owner, "state": {"$ne": "pending"}}),
    ]);
    rows.push((
        "platform_public_probe_consents",
        doc! {"actor_user_id": owner},
    ));
    rows
}

/// Return the owner field paths used by the inventory filters. This is kept
/// next to the filters so registry drift cannot silently change the orphan
/// sweep or export owner selector.
#[cfg(test)]
pub(crate) fn inventory_owner_fields(
    owner: &str,
) -> std::collections::BTreeMap<&'static str, std::collections::BTreeSet<String>> {
    fn collect(
        value: &bson::Bson,
        path: Option<&str>,
        owner: &str,
        output: &mut std::collections::BTreeSet<String>,
    ) {
        match value {
            bson::Bson::String(value) if value == owner => {
                if let Some(path) = path {
                    output.insert(path.to_owned());
                }
            }
            bson::Bson::Document(document) => {
                for (key, value) in document {
                    if key.starts_with('$') {
                        collect(value, path, owner, output);
                    } else {
                        let next = match path {
                            Some(path) => format!("{path}.{key}"),
                            None => key.clone(),
                        };
                        collect(value, Some(&next), owner, output);
                    }
                }
            }
            bson::Bson::Array(values) => {
                for value in values {
                    collect(value, path, owner, output);
                }
            }
            _ => {}
        }
    }

    let mut fields = std::collections::BTreeMap::new();
    let acl_filter = super::super::org_agent_service::member_list_filter(owner);
    for (collection, filter) in inventory(owner)
        .into_iter()
        .chain([(crate::models::assistant_agent::COLLECTION_NAME, acl_filter)])
    {
        let mut row = std::collections::BTreeSet::new();
        collect(&bson::Bson::Document(filter), None, owner, &mut row);
        fields
            .entry(collection)
            .or_insert_with(std::collections::BTreeSet::new)
            .extend(row);
    }
    fields
}

/// Redact shared records after personal ownership has been removed. These
/// updates preserve counters and ordering/integrity fields while removing the
/// departing person's human-readable identity.
async fn redact_page(
    db: &Database,
    owner: &str,
    collection: &str,
    filter: Document,
    update: Vec<Document>,
) -> AppResult<bool> {
    if db
        .collection::<Document>(collection)
        .find_one(filter.clone())
        .projection(doc! {"_id": 1})
        .await?
        .is_none()
    {
        return Ok(false);
    }
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<bool> = async {
        let barrier = db
            .collection::<Document>(super::super::erasure_admission::COLLECTION)
            .update_one(
                doc! {"_id": owner, "erasure_accepted": true},
                doc! {"$inc": {"cleanup_revision": 1_i64}},
            )
            .session(&mut tx)
            .await?;
        if barrier.matched_count != 1 {
            return Err(AppError::Conflict(
                "Personal cleanup admission is unavailable".into(),
            ));
        }
        let mut cursor = db
            .collection::<Document>(collection)
            .find(filter.clone())
            .projection(doc! {"_id": 1})
            .sort(doc! {"_id": 1})
            .limit(PAGE)
            .session(&mut tx)
            .await?;
        let ids: Vec<bson::Bson> = cursor
            .stream(&mut tx)
            .try_collect::<Vec<Document>>()
            .await?
            .into_iter()
            .filter_map(|mut row| row.remove("_id"))
            .collect();
        if ids.is_empty() {
            return Ok(false);
        }
        let mut bounded_filter = Document::new();
        bounded_filter.insert("$and", vec![filter, doc! {"_id": {"$in": ids}}]);
        Ok(db
            .collection::<Document>(collection)
            .update_many(bounded_filter, update)
            .session(&mut tx)
            .await?
            .modified_count
            > 0)
    }
    .await;
    super::super::org_membership_authority::finish(tx, result).await
}

async fn redact_invite_usages(db: &Database, owner: &str) -> AppResult<bool> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<bool> = async {
        let barrier = db
            .collection::<Document>(super::super::erasure_admission::COLLECTION)
            .update_one(
                doc! {"_id": owner, "erasure_accepted": true},
                doc! {"$inc": {"cleanup_revision": 1_i64}},
            )
            .session(&mut tx)
            .await?;
        if barrier.matched_count != 1 {
            return Err(AppError::Conflict(
                "Personal cleanup admission is unavailable".into(),
            ));
        }
        let mut cursor = db
            .collection::<Document>("invite_codes")
            .find(doc! {"usages": {"$elemMatch": {"user_id": owner, "email": {"$exists": true}}}})
            .projection(doc! {"_id": 1, "usages": 1})
            .sort(doc! {"_id": 1})
            .limit(PAGE)
            .session(&mut tx)
            .await?;
        let rows: Vec<Document> = cursor.stream(&mut tx).try_collect().await?;
        let mut changed = false;
        for row in rows {
            let id = row
                .get("_id")
                .cloned()
                .ok_or_else(|| AppError::Internal("Invite code identity missing".into()))?;
            let usages = row.get_array("usages").cloned().unwrap_or_default();
            let mut changed_usage = false;
            let filtered: Vec<bson::Bson> = usages
                .into_iter()
                .map(|usage| {
                    let Some(mut usage) = usage.as_document().cloned() else {
                        return usage;
                    };
                    if usage.get_str("user_id").ok() == Some(owner) && usage.contains_key("email") {
                        usage.remove("email");
                        changed_usage = true;
                    }
                    bson::Bson::Document(usage)
                })
                .collect();
            if !changed_usage {
                continue;
            }
            let result = db
                .collection::<Document>("invite_codes")
                .update_one(
                    doc! {"_id": id, "usages.user_id": owner, "usages.email": {"$exists": true}},
                    doc! {"$set": {"usages": filtered}},
                )
                .session(&mut tx)
                .await?;
            changed |= result.modified_count > 0;
        }
        Ok(changed)
    }
    .await;
    super::super::org_membership_authority::finish(tx, result).await
}

async fn redact_shared(db: &Database, owner: &str) -> AppResult<bool> {
    // Events owned by the deleted person's personal service are erased. Events
    // owned by an organization remain, with the actor's human-readable and
    // person-linked fields redacted.
    if erase_page(db, owner, "service_change_events", doc! {"owner_id": owner}).await? {
        return Ok(true);
    }
    for spec in super::super::personal_data_registry::ATTRIBUTIONS {
        if redact_page(
            db,
            owner,
            spec.collection,
            super::super::personal_attribution_service::pending_filter(spec, Some(owner)),
            super::super::personal_attribution_service::update(spec),
        )
        .await?
        {
            return Ok(true);
        }
    }
    if redact_page(
        db,
        owner,
        "invite_codes",
        doc! {"created_by": owner, "note": {"$exists": true, "$ne": bson::Bson::Null}},
        vec![doc! {"$unset": "note"}],
    )
    .await?
    {
        return Ok(true);
    }
    // Invite usage is embedded in an invite owned by another account. Keep
    // used_count/max_uses and the code itself, but remove the person's usage
    // identity (including the legacy email field).
    redact_invite_usages(db, owner).await
}

/// Local revocation creates durable external-effect outboxes. Their completion,
/// rather than domain ordering alone, permits deleting recovery credentials.
pub(super) async fn settle_telegram_requests(db: &Database, owner: &str) -> AppResult<()> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<()> = async {
        if db.collection::<Document>(super::super::erasure_admission::COLLECTION).update_one(
            doc! {"_id":owner,"erasure_accepted":true},doc! {"$inc":{"cleanup_revision":1_i64}},
        ).session(&mut tx).await?.matched_count != 1 {
            return Err(AppError::Conflict("Personal cleanup barrier is unavailable".into()));
        }
        let mut cursor=db.collection::<Document>("telegram_bot_requests").find(doc! {"owner_user_id":owner,"active":true})
            .sort(doc! {"_id":1}).limit(PAGE).session(&mut tx).await?;
        let rows:Vec<Document>=cursor.stream(&mut tx).try_collect().await?;
        for row in rows {
            let Some(id)=row.get("_id") else {continue;};
            let unstarted=matches!(row.get_str("status").unwrap_or_default(),"waiting_telegram"|"waiting_bot"|"waiting_consent"|"ready");
            let confirmed=db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).find_one(
                doc! {"_id":id,"user_id":owner,"personal_erasure_cleanup_complete":true},
            ).session(&mut tx).await?.is_some();
            if unstarted || confirmed {
                db.collection::<Document>("telegram_bot_requests").update_one(
                    doc! {"_id":id,"owner_user_id":owner,"revision":row.get("revision").cloned().unwrap_or(bson::Bson::Null)},
                    doc! {"$set":{"active":false,"status":"cancelled","auto_connect":false},"$inc":{"revision":1_i64,"connection_generation":1_i64}},
                ).session(&mut tx).await?;
            }
        }
        Ok(())
    }.await;
    super::super::org_membership_authority::finish(tx, result).await
}

pub(crate) async fn remote_cleanup_clear(
    db: &Database,
    owner: &str,
    tx: &mut ClientSession,
) -> AppResult<bool> {
    for (collection, filter) in [
        (
            "platform_bot_cleanup",
            doc! {"owner_user_id": owner, "completed": {"$ne": true}},
        ),
        (
            "telegram_bot_requests",
            doc! {"owner_user_id": owner, "active": true},
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one(filter)
            .projection(doc! {"_id": 1})
            .session(&mut *tx)
            .await?
            .is_some()
        {
            return Ok(false);
        }
    }

    Ok(!stop_effect_needs_local_node(db, owner, None, tx).await?)
}

/// Stop completion uses its persisted capture, not the binding. Only an active
/// local node still provides a cancel capability that this pass would remove.
/// Filter pending effects for this owner first; unwind and lookup keep memory
/// bounded without collecting every effect or node id into a client-side array.
pub(super) async fn stop_effect_needs_local_node(
    db: &Database,
    owner: &str,
    node_id: Option<&str>,
    tx: &mut ClientSession,
) -> AppResult<bool> {
    let mut pending = doc! {
        "active_turn.origin.user_id": owner, "completed": {"$ne": true},
    };
    if let Some(id) = node_id {
        pending.insert("machine_scopes.node_id", id);
    }
    let mut pipeline = vec![
        doc! {"$match": pending},
        doc! {"$project": {"machine_scopes.node_id": 1}},
        doc! {"$unwind": "$machine_scopes"},
    ];
    if let Some(id) = node_id {
        pipeline.push(doc! {"$match": {"machine_scopes.node_id": id}});
    }
    pipeline.extend([
        doc! {"$lookup": {
            "from": crate::models::node::COLLECTION_NAME,
            "localField": "machine_scopes.node_id", "foreignField": "_id",
            "pipeline": [
                {"$match": {"user_id": owner, "is_active": true,
                    "metadata.provisioning_source": {"$ne": "remote_machine"}}},
                {"$project": {"_id": 1}},
            ],
            "as": "local_node",
        }},
        doc! {"$match": {"local_node.0": {"$exists": true}}},
        doc! {"$limit": 1},
        doc! {"$project": {"_id": 1}},
    ]);
    let mut cursor = db
        .collection::<Document>("platform_personal_stop_effects")
        .aggregate(pipeline)
        .max_time(std::time::Duration::from_secs(10))
        .session(&mut *tx)
        .await?;
    Ok(cursor.next(tx).await.transpose()?.is_some())
}

pub(super) async fn erase_page(
    db: &Database,
    owner: &str,
    collection: &str,
    filter: Document,
) -> AppResult<bool> {
    if db
        .collection::<Document>(collection)
        .find_one(filter.clone())
        .projection(doc! {"_id":1})
        .await?
        .is_none()
    {
        return Ok(false);
    }
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<bool> = async {
        if !remote_cleanup_clear(db, owner, &mut tx).await? {
            return Err(AppError::Conflict(
                "Platform cleanup still needs its recovery data".into(),
            ));
        }
        let barrier = db
            .collection::<Document>(super::super::erasure_admission::COLLECTION)
            .update_one(
                doc! {"_id":owner,"erasure_accepted":true},
                doc! {"$inc":{"cleanup_revision":1_i64}},
            )
            .session(&mut tx)
            .await?;
        if barrier.matched_count != 1 {
            return Err(AppError::Conflict(
                "Personal cleanup admission is unavailable".into(),
            ));
        }
        let deleting_threads = collection == crate::models::nyxbot_channel::THREADS_COLLECTION_NAME;
        let projection = if deleting_threads {
            super::super::nyxbot_thread_mutation::projection()
        } else {
            doc! {"_id":1}
        };
        let mut cursor = db
            .collection::<Document>(collection)
            .find(filter.clone())
            .projection(projection)
            .sort(doc! {"_id":1})
            .limit(PAGE)
            .session(&mut tx)
            .await?;
        let rows: Vec<Document> = cursor.stream(&mut tx).try_collect().await?;
        if rows.is_empty() {
            return Ok(false);
        }
        if deleting_threads {
            super::super::nyxbot_thread_mutation::delete_observed(db, &mut tx, &rows).await?;
            return Ok(true);
        }
        let ids: Vec<bson::Bson> = rows
            .into_iter()
            .map(|mut row| {
                row.remove("_id")
                    .ok_or_else(|| AppError::Internal("Invalid personal inventory identity".into()))
            })
            .collect::<AppResult<_>>()?;
        if collection == crate::models::service_account::COLLECTION_NAME {
            for id in &ids {
                let target = id
                    .as_str()
                    .ok_or_else(|| AppError::Internal("Invalid service-account identity".into()))?;
                super::super::erasure_admission::close_in_session(db, target, &mut tx).await?;
            }
        }
        if collection == crate::models::nyxbot_channel::COLLECTION_NAME {
            super::super::org_channel_service::defaults::erase_in_session(
                db,
                doc! {"channel_id":{"$in":&ids}},
                &mut tx,
            )
            .await?;
        }
        if collection == crate::models::channel_bot::COLLECTION_NAME {
            // Meta control rows are keyed by bot and provider participant, so
            // they cannot be found through the owner's user_id inventory.  A
            // bot row is still present in this transaction; remove its
            // participant identifiers before the parent is erased.
            db.collection::<Document>(crate::models::coordination::EVENT_DEDUP_COLLECTION_NAME)
                .delete_many(
                    doc! {"namespace": "meta-inbox-inbound", "channel_bot_id": {"$in": &ids}},
                )
                .session(&mut tx)
                .await?;
            db.collection::<Document>(crate::services::channel_adapters::meta::CONTROLS)
                .delete_many(doc! {"bot_id": {"$in": &ids}})
                .session(&mut tx)
                .await?;
        }
        db.collection::<Document>(collection)
            .delete_many(doc! {"$and":[filter, {"_id":{"$in":ids}}]})
            .session(&mut tx)
            .await?;
        Ok(true)
    }
    .await;
    super::super::org_membership_authority::finish(tx, result).await
}

/// Child records without an owner column join their authoritative parent.
/// Parent deletion occurs only after every bounded child page is durable.
async fn joined_parents(db: &Database, job: &PersonalErasureJob, target: &str) -> AppResult<bool> {
    use crate::models as m;
    let specs: &[(&str, &[(&str, &str)])] = &[
        (
            m::trigger::COLLECTION_NAME,
            &[(super::super::trigger_schedule::WORK, "trigger_id")],
        ),
        (
            m::assistant_agent::COLLECTION_NAME,
            &[
                (
                    m::assistant_agent_learning::MEMBERS_COLLECTION_NAME,
                    "agent_id",
                ),
                (
                    m::assistant_agent_learning::REJECTIONS_COLLECTION_NAME,
                    "agent_id",
                ),
                (m::assistant_agent_learning::CONFIG_COLLECTION_NAME, "_id"),
                (
                    m::assistant_agent_learning::RUNS_COLLECTION_NAME,
                    "agent_id",
                ),
                (
                    m::assistant_agent_learning::PROPOSALS_COLLECTION_NAME,
                    "agent_id",
                ),
                (
                    m::assistant_agent_learning::ROOTS_COLLECTION_NAME,
                    "agent_id",
                ),
            ],
        ),
        (
            m::mcp_session::MCP_SESSION_COLLECTION,
            &[("mcp_session_notifications", "session_id")],
        ),
        (
            m::channel_conversation::COLLECTION_NAME,
            &[
                (m::channel_event_log::COLLECTION_NAME, "conversation_id"),
                (m::channel_message::COLLECTION_NAME, "conversation_id"),
            ],
        ),
        (
            m::service_pool::COLLECTION_NAME,
            &[
                (m::service_pool_member_health::COLLECTION_NAME, "pool_id"),
                (
                    m::service_pool_member_health::OBSERVATION_COLLECTION_NAME,
                    "pool_id",
                ),
            ],
        ),
        (
            m::user_service::COLLECTION_NAME,
            &[(m::service_change_event::HEADS_COLLECTION_NAME, "_id")],
        ),
        (
            "platform_bot_installations",
            &[("platform_delivery_tests", "installation_id")],
        ),
        (
            "platform_setups",
            &[
                ("platform_setup_notices", "setup_id"),
                ("platform_setup_steps", "setup_id"),
            ],
        ),
    ];
    for (parent, children) in specs {
        let owner_field = crate::services::personal_data_registry::find(parent)
            .and_then(|entry| match entry.owner {
                crate::services::personal_data_registry::OwnerSelector::Fields(fields) => {
                    fields.first().copied()
                }
                crate::services::personal_data_registry::OwnerSelector::NotPersonal => None,
            })
            .unwrap_or("user_id");
        if let Some(row) = db
            .collection::<Document>(parent)
            .find_one(doc! {owner_field:target})
            .projection(doc! {"_id":1})
            .sort(doc! {"_id":1})
            .await?
        {
            let id = row
                .get_str("_id")
                .map_err(|_| AppError::Internal("Invalid personal parent identity".into()))?;
            if *parent == m::assistant_agent::COLLECTION_NAME {
                // Uncertain publication and landed target reservations are
                // external-effect locks, not ordinary agent children. The
                // scheduler checks unresolved operations before any local
                // purge; delete only reservations which never dispatched.
                if super::learning::release_unused_targets(db, &job.user_id, id).await? {
                    return Ok(true);
                }
            }
            for (collection, field) in *children {
                let mut filter = Document::new();
                filter.insert(*field, id);
                if erase_page(db, &job.user_id, collection, filter).await? {
                    return Ok(true);
                }
            }
            if *parent == m::channel_conversation::COLLECTION_NAME {
                // Claim IDs use the documented conversation-id prefix; escape
                // the UUID before constructing a literal anchored selector.
                let prefix = format!("^{}:", regex::escape(id));
                if erase_page(
                    db,
                    &job.user_id,
                    m::channel_send_claim::COLLECTION_NAME,
                    doc! {"_id":{"$regex":prefix},"status":"sent"},
                )
                .await?
                {
                    return Ok(true);
                }
                if db.collection::<Document>(m::channel_send_claim::COLLECTION_NAME).find_one(doc! {"_id":{"$regex":format!("^{}:",regex::escape(id))},"status":{"$ne":"sent"}}).await?.is_some() {return Err(AppError::Conflict("An external channel send still needs reconciliation".into()));}
            }
            erase_page(db, &job.user_id, parent, doc! {"_id":id,owner_field:target}).await?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// Retain only the metadata needed by an already settled external-effect
/// lifecycle. Uncertain execution and pending webhooks never enter this path.
async fn retain_settled_evidence(db: &Database, job: &PersonalErasureJob) -> AppResult<bool> {
    for (collection, filter, fields) in [
        (
            crate::models::connect_link::COLLECTION_NAME,
            doc! {"user_id":&job.user_id,"webhook_event_status":{"$in":["delivered","abandoned"]},"completion_claim_id":bson::Bson::Null,"personal_erasure_retained":{"$ne":true}},
            &[
                "_id",
                "user_id",
                "webhook_event_id",
                "webhook_event_status",
                "webhook_event_attempts",
                "webhook_event_delivered_at",
                "created_at",
                "expires_at",
            ][..],
        ),
        (
            crate::models::durable_operation_grant::COLLECTION_NAME,
            doc! {"user_id":&job.user_id,"revoked_at":{"$type":"date"},"personal_erasure_retained":{"$ne":true}},
            &[
                "_id",
                "user_id",
                "revoked_at",
                "revoked_by",
                "state_version",
                "created_at",
            ][..],
        ),
        (
            crate::models::durable_operation_execution::COLLECTION_NAME,
            doc! {"user_id":&job.user_id,"status":{"$in":["reserved","completed","failed","rejected"]},"personal_erasure_retained":{"$ne":true}},
            &[
                "_id",
                "user_id",
                "grant_id",
                "operation_id",
                "status",
                "downstream_attempts",
                "response_status",
                "dispatched_at",
                "terminal_at",
                "created_at",
                "updated_at",
            ][..],
        ),
    ] {
        if let Some(row) = db
            .collection::<Document>(collection)
            .find_one(filter.clone())
            .sort(doc! {"_id":1})
            .await?
        {
            let mut receipt = Document::new();
            for field in fields {
                if let Some(value) = row.get(*field) {
                    receipt.insert(*field, value.clone());
                }
            }
            if collection == crate::models::durable_operation_execution::COLLECTION_NAME
                && receipt.get_str("status") == Ok("reserved")
            {
                receipt.insert("status", "rejected");
                receipt.insert("terminal_detail", "owner_erasure_before_dispatch");
            }
            receipt.insert("personal_erasure_retained", true);
            let mut tx = db.client().start_session().await?;
            tx.start_transaction()
                .read_concern(ReadConcern::snapshot())
                .write_concern(WriteConcern::majority())
                .await?;
            let result:AppResult<bool>=async {
                if db.collection::<Document>(super::super::erasure_admission::COLLECTION)
                    .update_one(doc! {"_id":&job.user_id,"erasure_accepted":true},doc! {"$inc":{"cleanup_revision":1_i64}}).session(&mut tx).await?.matched_count!=1 {
                    return Err(AppError::Conflict("Personal cleanup barrier is unavailable".into()));
                }
                let id=row.get_str("_id").map_err(|_|AppError::Internal("Invalid lifecycle receipt identity".into()))?;
                receipt.insert("_id",format!("{collection}:{id}"));
                receipt.insert("source_collection",collection);receipt.insert("source_id",id);
                let changed=db.collection::<Document>(collection).delete_one(doc! {"$and":[filter,{"_id":id,"$expr":{"$eq":["$$ROOT",{"$literal":&row}]}}]}).session(&mut tx).await?;
                if changed.deleted_count==1 {
                    db.collection::<Document>("personal_erasure_local_receipts").insert_one(receipt).session(&mut tx).await?;
                }
                Ok(changed.deleted_count==1)
            }.await;
            return super::super::org_membership_authority::finish(tx, result).await;
        }
    }
    Ok(false)
}

/// Children drained before their erased node; also pinned to the data registry.
pub(crate) const NODE_DRAIN_COLLECTIONS: &[&str] = &[
    crate::models::node_pending_credential::COLLECTION_NAME,
    crate::models::node_service_binding::COLLECTION_NAME,
    crate::models::node_credential_desired::COLLECTION_NAME,
];

pub(super) async fn remote_cleanup_pending(db: &Database, owner: &str) -> AppResult<bool> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .await?;
    let clear = remote_cleanup_clear(db, owner, &mut tx).await?;
    tx.abort_transaction().await?;
    Ok(!clear)
}

/// One bounded page. The next pass starts from the first unresolved collection,
/// avoiding offset skips when an earlier held writer resumes during cleanup.
pub(super) async fn pass(state: &crate::AppState, job: &PersonalErasureJob) -> AppResult<bool> {
    let db = super::super::transaction_service::durable_database(&state.db);
    settle_telegram_requests(&db, &job.user_id).await?;
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .await?;
    let clear = remote_cleanup_clear(&db, &job.user_id, &mut tx).await?;
    tx.abort_transaction().await?;
    if !clear {
        return Ok(false);
    }
    if redact_page(
        &db,
        &job.user_id,
        crate::models::assistant_agent::COLLECTION_NAME,
        super::super::org_agent_service::member_list_filter(&job.user_id),
        super::super::org_agent_service::prune_member_update(&job.user_id),
    )
    .await?
        || redact_shared(&db, &job.user_id).await?
    {
        return Ok(false);
    }
    if super::super::collaboration_group_service::cleanup::erase_person(&db, &job.user_id).await? {
        return Ok(false);
    }
    super::super::collaboration_group_service::cleanup::sweep(&db).await?;
    if db
        .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"collaboration.original_requester_id":&job.user_id})
        .await?
        .is_some()
    {
        return Ok(false);
    }

    if !super::learning::clear(&db, &job.user_id).await? {
        return Err(AppError::Conflict(
            "Interrupted skill publication requires reconciliation before local erasure".into(),
        ));
    }
    if retain_settled_evidence(&db, job).await? {
        return Ok(false);
    }
    if joined_parents(&db, job, &job.user_id).await? {
        return Ok(false);
    }
    // Voice has its own final settlement/connection shutdown and retention.
    if db
        .collection::<Document>(crate::models::assistant_voice_session::COLLECTION_NAME)
        .find_one(doc! {"user_id":&job.user_id})
        .await?
        .is_some()
    {
        db.collection::<Document>(crate::models::assistant_voice_session::COLLECTION_NAME)
            .update_many(
                doc! {"user_id":&job.user_id},
                doc! {"$set":{"end_requested":true,"purge_requested":true}},
            )
            .await?;
        return Ok(false);
    }
    super::super::org_channel_service::lifecycle::erase_person(&db, &job.user_id).await?;
    // Preserve org routes: release only the person's existing NyxBot links via
    // the routing/key lifecycle before deleting their local link record.
    if let Some(link) = db
        .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .find_one(doc! {"user_id":&job.user_id})
        .await?
    {
        if link.get_document("org_specialist").is_ok() {
            let id = link
                .get_str("_id")
                .map_err(|_| AppError::Internal("Invalid org channel identity".into()))?;
            let context = (db.clone(), id.to_owned());
            super::super::transaction_service::run_db_only(&db, &context, |tx, (db, id)| {
                Box::pin(async move {
                    super::super::org_channel_service::lifecycle::retire_in_session(
                        db, tx, id, None,
                    )
                    .await
                })
            })
            .await?;
            if !super::super::org_channel_service::lifecycle::erase_hidden(
                &db,
                doc! {"channel.nyxbot_channel_id":id,"channel.org_execution":{"$type":"object"}},
            )
            .await?
            {
                return Ok(false);
            }
            for collection in [
                crate::models::org_channel::LINKS,
                crate::models::org_channel::CHALLENGES,
                crate::models::org_channel::LANES,
                crate::models::org_channel::MESSAGES,
            ] {
                db.collection::<Document>(collection)
                    .delete_many(doc! {"channel_id":id})
                    .await?;
            }
        }
        if let Ok(org) = link.get_str("bot_owner_id") {
            if let Ok(route) = link.get_str("route_id") {
                match super::super::channel_routing_service::delete_conversation(&db, route, org)
                    .await
                {
                    Ok(()) | Err(AppError::NotFound(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            if let Ok(key) = link.get_str("route_api_key_id") {
                match super::super::key_service::delete_api_key(&db, org, key).await {
                    Ok(()) | Err(AppError::NotFound(_)) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        erase_page(&db,&job.user_id,crate::models::nyxbot_channel::COLLECTION_NAME,doc! {"user_id":&job.user_id,"_id":link.get("_id").cloned().ok_or_else(||AppError::Internal("Invalid NyxBot link identity".into()))?}).await?;
        return Ok(false);
    }
    if let Some(client) = db
        .collection::<Document>(crate::models::oauth_client::COLLECTION_NAME)
        .find_one(doc! {"created_by":&job.user_id})
        .projection(doc! {"_id":1})
        .await?
    {
        let id = client
            .get_str("_id")
            .map_err(|_| AppError::Internal("Invalid OAuth client identity".into()))?;
        super::super::oauth_client_service::delete_client_for_creator(&db, id, &job.user_id)
            .await?;
        erase_page(
            &db,
            &job.user_id,
            crate::models::oauth_client::COLLECTION_NAME,
            doc! {"_id":id,"created_by":&job.user_id,"is_active":false},
        )
        .await?;
        return Ok(false);
    }
    if let Some(node) = db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .find_one(
            doc! {"user_id":&job.user_id,"metadata.provisioning_source":{"$ne":"remote_machine"}},
        )
        .await?
    {
        let id = node
            .get_str("_id")
            .map_err(|_| AppError::Internal("Invalid node identity".into()))?;
        let mut node_model: crate::models::node::Node = bson::from_document(node.clone())
            .map_err(|_| AppError::Internal("Invalid node drain evidence".into()))?;
        if let Ok(owner) = node.get_document("personal_erasure_drain_owner") {
            node_model.connection_owner = Some(
                bson::from_document(owner.clone())
                    .map_err(|_| AppError::Internal("Invalid retained node owner fence".into()))?,
            );
        }
        #[cfg(test)]
        if let Some(hook) = before_node_drain_test_hook(&db) {
            hook.reached.notify_one();
            hook.resume.notified().await;
        }
        // Capture can still be enqueued by lifecycle recovery after admission.
        // Recheck immediately before external drain/disconnect as well as in
        // every deletion transaction; the pass-start snapshot is insufficient.
        let mut tx = db.client().start_session().await?;
        tx.start_transaction()
            .read_concern(ReadConcern::snapshot())
            .write_concern(WriteConcern::majority())
            .await?;
        let ready: AppResult<bool> = async {
            if stop_effect_needs_local_node(&db, &job.user_id, Some(id), &mut tx).await? {
                return Ok(false);
            }
            if db
                .collection::<Document>(super::super::erasure_admission::COLLECTION)
                .update_one(
                    doc! {"_id":&job.user_id,"erasure_accepted":true},
                    doc! {"$inc":{"cleanup_revision":1_i64}},
                )
                .session(&mut tx)
                .await?
                .matched_count
                != 1
            {
                return Err(AppError::Conflict(
                    "Personal cleanup barrier is unavailable".into(),
                ));
            }
            // Reconcile active nodes left by older acceptance paths only after
            // their captured Stop no longer needs the live signing capability.
            db.collection::<Document>(crate::models::node::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":id,"user_id":&job.user_id,"is_active":true,
                    "metadata.provisioning_source":{"$ne":"remote_machine"}},
                    vec![doc! {"$set":{"is_active":false,"status":"draining",
                        "personal_erasure_drain_required":true,
                        "personal_erasure_drain_owner":{"$ifNull":["$personal_erasure_drain_owner","$connection_owner"]}}}],
                )
                .session(&mut tx)
                .await?;
            Ok(true)
        }
        .await;
        if !super::super::org_membership_authority::finish(tx, ready).await? {
            return Ok(false);
        }
        node_model.is_active = false;
        if node
            .get_bool("personal_erasure_generation_retired")
            .unwrap_or(false)
        {
            state
                .node_ws_manager
                .disconnect_connection(id, 4006, "Account erasure")
                .await;
        } else {
            state.node_dispatch.drain_inactive(&node_model).await?;
        }
        for &collection in NODE_DRAIN_COLLECTIONS {
            if erase_page(&db, &job.user_id, collection, doc! {"node_id":id}).await? {
                return Ok(false);
            }
        }
        erase_page(
            &db,
            &job.user_id,
            crate::models::node::COLLECTION_NAME,
            doc! {"_id":id,"user_id":&job.user_id},
        )
        .await?;
        return Ok(false);
    }
    if let Some(account)=db.collection::<Document>(crate::models::service_account::COLLECTION_NAME).find_one(doc! {"$or":[{"owner_user_id":&job.user_id},{"owner_user_id":bson::Bson::Null,"created_by":&job.user_id}]}).await? {
        let id=account.get_str("_id").map_err(|_|AppError::Internal("Invalid owned service account".into()))?;
        if account.get_bool("personal_erasure_credentials_preserved").ok()!=Some(true) {return Ok(false);}
        for (collection,filter) in inventory(id) {if erase_page(&db,&job.user_id,collection,filter).await? {return Ok(false);}}
        if erase_page(&db,&job.user_id,crate::models::service_account_token::COLLECTION_NAME,doc! {"service_account_id":id}).await? {return Ok(false);}
        erase_page(&db,&job.user_id,crate::models::service_account::COLLECTION_NAME,doc! {"_id":id,"$or":[{"owner_user_id":&job.user_id},{"owner_user_id":bson::Bson::Null,"created_by":&job.user_id}]}).await?;
        return Ok(false);
    }
    for (collection, filter) in inventory(&job.user_id) {
        if erase_page(&db, &job.user_id, collection, filter).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Only the last local phase removes the source. The durable barrier, jobs,
/// effect capabilities, audit and financial/lifecycle receipts remain intact.
pub(super) async fn source(db: &Database, job: &PersonalErasureJob) -> AppResult<bool> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result:AppResult<bool>=async {
        if !remote_cleanup_clear(db,&job.user_id,&mut tx).await? {return Ok(false);}
        if db.collection::<Document>(provider::EFFECTS).find_one(doc! {"user_id":&job.user_id,"$or":[{"status":{"$ne":"complete"}},{"capability_encrypted":{"$exists":true}}]}).session(&mut tx).await?.is_some() {return Ok(false);}
        let acl_filter = super::super::org_agent_service::member_list_filter(&job.user_id);
        for (collection,filter) in inventory(&job.user_id).into_iter().chain([(crate::models::assistant_agent::COLLECTION_NAME, acl_filter)]) {
            if db.collection::<Document>(collection).find_one(filter).projection(doc! {"_id":1}).session(&mut tx).await?.is_some() {return Ok(false);}
        }
        for spec in super::super::personal_data_registry::ATTRIBUTIONS {
            let pending=super::super::personal_attribution_service::pending_filter(spec, Some(&job.user_id));
            if db.collection::<Document>(spec.collection).find_one(pending).projection(doc! {"_id":1}).session(&mut tx).await?.is_some() {return Ok(false);}
        }
        let barrier=db.collection::<Document>(super::super::erasure_admission::COLLECTION).update_one(doc! {"_id":&job.user_id,"erasure_accepted":true},doc! {"$inc":{"cleanup_revision":1_i64}}).session(&mut tx).await?;
        if barrier.matched_count!=1 {return Err(AppError::Conflict("Personal cleanup barrier is unavailable".into()));}
        for (collection, filter) in super::super::personal_erasure_registry::final_metadata(&job.user_id) {
            let mut cursor = db.collection::<Document>(collection).find(filter.clone()).projection(doc! {"_id":1}).limit(PAGE).session(&mut tx).await?;
            let rows: Vec<Document> = cursor.stream(&mut tx).try_collect().await?;
            if !rows.is_empty() {
                // Preserve the registry predicate: delivery intents use owner_id,
                // while revocation receipts use user_id. Keep the exact page fence.
                db.collection::<Document>(collection).delete_many(doc! {"$and":[filter, {"_id":{"$in":rows.into_iter().filter_map(|mut row| row.remove("_id")).collect::<Vec<_>>()}}]}).session(&mut tx).await?;
                return Ok(false);
            }
        }
        db.collection::<Document>(USERS).delete_one(doc! {"_id":&job.user_id,"is_active":false,"platform_personal_deleting":true,"platform_personal_retirement_pending":{"$ne":true}}).session(&mut tx).await?;
        Ok(db.collection::<Document>(USERS).find_one(doc! {"_id":&job.user_id}).session(&mut tx).await?.is_none())
    }.await;
    super::super::org_membership_authority::finish(tx, result).await
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct BeforeNodeDrainHook {
    pub reached: tokio::sync::Notify,
    pub resume: tokio::sync::Notify,
}

#[cfg(test)]
pub(super) static BEFORE_NODE_DRAIN: std::sync::LazyLock<
    dashmap::DashMap<String, std::sync::Arc<BeforeNodeDrainHook>>,
> = std::sync::LazyLock::new(dashmap::DashMap::new);

#[cfg(test)]
fn before_node_drain_test_hook(db: &Database) -> Option<std::sync::Arc<BeforeNodeDrainHook>> {
    BEFORE_NODE_DRAIN.get(db.name()).map(|entry| entry.clone())
}

#[cfg(test)]
mod crash_inventory_tests {
    #[test]
    fn crash_reports_are_erased_by_owner_and_anonymous_counters_are_retained() {
        let rows = super::inventory("owner-sentinel");
        assert!(
            rows.iter()
                .any(|(name, filter)| *name == "client_error_reports"
                    && filter.get_str("user_id") == Ok("owner-sentinel"))
        );
        assert!(
            !rows
                .iter()
                .any(|(name, _)| *name == "client_error_report_counters")
        );
    }
}
