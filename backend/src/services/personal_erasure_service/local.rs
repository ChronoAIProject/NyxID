//! Explicit personal-owner inventory. Every pass removes at most 32 rows in a
//! majority transaction. Ownership predicates are declared here, never inferred
//! from creator/updater attribution. Durable admission tombstones survive the
//! final source deletion and remain the writer barrier.
use super::*;
use futures::TryStreamExt;

const PAGE: i64 = 32;

fn inventory(owner: &str) -> Vec<(&'static str, Document)> {
    use crate::models as m;
    let mut rows: Vec<(&'static str, Document)> = vec![
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
            m::async_service_operation::QUOTAS_COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
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
            m::assistant_acknowledgement::COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (
            m::assistant_attachment::COLLECTION_NAME,
            doc! {"user_id":owner},
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
            m::nyxbot_channel::WATCHES_COLLECTION_NAME,
            doc! {"user_id":owner},
        ),
        (m::assistant_voice::REQUESTS, doc! {"user_id":owner}),
        (m::assistant_settings::COLLECTION_NAME, doc! {"_id":owner}),
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
    rows
}

async fn erase_page(
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
        let mut cursor = db
            .collection::<Document>(collection)
            .find(filter.clone())
            .projection(doc! {"_id":1})
            .sort(doc! {"_id":1})
            .limit(PAGE)
            .session(&mut tx)
            .await?;
        let rows: Vec<Document> = cursor.stream(&mut tx).try_collect().await?;
        if rows.is_empty() {
            return Ok(false);
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
    ];
    for (parent, children) in specs {
        if let Some(row) = db
            .collection::<Document>(parent)
            .find_one(doc! {"user_id":target})
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
            erase_page(db, &job.user_id, parent, doc! {"_id":id,"user_id":target}).await?;
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

/// One bounded page. The next pass starts from the first unresolved collection,
/// avoiding offset skips when an earlier held writer resumes during cleanup.
pub(super) async fn pass(state: &crate::AppState, job: &PersonalErasureJob) -> AppResult<bool> {
    let db = super::super::transaction_service::durable_database(&state.db);
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
    // Preserve org routes: release only the person's existing NyxBot links via
    // the routing/key lifecycle before deleting their local link record.
    if let Some(link) = db
        .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .find_one(doc! {"user_id":&job.user_id})
        .await?
    {
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
        for collection in [
            crate::models::node_pending_credential::COLLECTION_NAME,
            crate::models::node_service_binding::COLLECTION_NAME,
        ] {
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
        if db.collection::<Document>(provider::EFFECTS).find_one(doc! {"user_id":&job.user_id,"$or":[{"status":{"$ne":"complete"}},{"capability_encrypted":{"$exists":true}}]}).session(&mut tx).await?.is_some() {return Ok(false);}
        for (collection,filter) in inventory(&job.user_id) {
            if db.collection::<Document>(collection).find_one(filter).projection(doc! {"_id":1}).session(&mut tx).await?.is_some() {return Ok(false);}
        }
        let barrier=db.collection::<Document>(super::super::erasure_admission::COLLECTION).update_one(doc! {"_id":&job.user_id,"erasure_accepted":true},doc! {"$inc":{"cleanup_revision":1_i64}}).session(&mut tx).await?;
        if barrier.matched_count!=1 {return Err(AppError::Conflict("Personal cleanup barrier is unavailable".into()));}
        for (collection, filter) in super::super::personal_erasure_registry::final_metadata(&job.user_id) {
            let mut cursor = db.collection::<Document>(collection).find(filter).projection(doc! {"_id":1}).limit(PAGE).session(&mut tx).await?;
            let rows: Vec<Document> = cursor.stream(&mut tx).try_collect().await?;
            if !rows.is_empty() {
                db.collection::<Document>(collection).delete_many(doc! {"user_id":&job.user_id,"_id":{"$in":rows.into_iter().filter_map(|mut row| row.remove("_id")).collect::<Vec<_>>()}}).session(&mut tx).await?;
                return Ok(false);
            }
        }
        db.collection::<Document>(USERS).delete_one(doc! {"_id":&job.user_id,"is_active":false,"platform_personal_deleting":true,"platform_personal_retirement_pending":{"$ne":true}}).session(&mut tx).await?;
        Ok(db.collection::<Document>(USERS).find_one(doc! {"_id":&job.user_id}).session(&mut tx).await?.is_none())
    }.await;
    super::super::org_membership_authority::finish(tx, result).await
}
