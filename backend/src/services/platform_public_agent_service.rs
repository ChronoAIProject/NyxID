//! Public routing fails closed; local revocation and cleanup remain operational.
use super::{
    api_key_mutation_service as transactions, platform_manager_contract::*,
    platform_public_policy as policy,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as KEYS},
        platform_public_session::{
            CLEANUP_RECORDS, COLLECTION_NAME as SESSIONS, PlatformPublicSession,
            PublicCleanupRecord, PublicRevocationMarker, READINESS_RECORDS, REVOCATION_SCOPES,
        },
    },
};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::{IndexOptions, ReturnDocument},
};
use std::sync::Arc;
use tokio::sync::mpsc;

const MAX_REVOKE_SESSIONS: usize = 256;
const CLEANUP_LEASE_SECS: i64 = 60;

pub fn producer(
    state: AppState,
    execution: Arc<dyn NyxAgentExecutionProducer>,
    ingress: Arc<dyn ManagerIngressTurnStore>,
) -> Arc<dyn PublicTurnProducer> {
    // No dependency can override the local absence of runtime proof.
    Arc::new(PublicProducer {
        state,
        execution,
        ingress,
    })
}
struct PublicProducer {
    state: AppState,
    execution: Arc<dyn NyxAgentExecutionProducer>,
    ingress: Arc<dyn ManagerIngressTurnStore>,
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    // Draft versions used the deterministic scope identity as _id. Preserve
    // revocation cutoffs and pending drains before building the unique index.
    db.collection::<Document>(REVOCATION_SCOPES)
        .update_many(
            doc! {"scope_key":{"$exists":false}},
            vec![doc! {"$set":{"scope_key":"$_id"}}],
        )
        .await?;
    db.collection::<PlatformPublicSession>(SESSIONS).create_indexes([
        IndexModel::builder().keys(doc! {"credential_api_key_id":1})
            .options(IndexOptions::builder().unique(true).name("public_key_binding_unique".to_owned()).build()).build(),
        IndexModel::builder().keys(doc! {"profile_id":1,"manager_bot_id":1,"telegram_bot_id":1,"sender_id":1,"chat_id":1})
            .options(IndexOptions::builder().unique(true).partial_filter_expression(doc! {"revoked_at":bson::Bson::Null})
                .name("public_sender_live_unique".to_owned()).build()).build(),
        IndexModel::builder().keys(doc! {"revoked_at":1,"expires_at":1}).build(),
        IndexModel::builder().keys(doc! {"sponsor_owner_user_id":1,"revoked_at":1}).build(),
    ]).await?;
    db.collection::<PublicCleanupRecord>(CLEANUP_RECORDS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"completed":1,"retry_at":1,"lease_expires_at":1})
                .build(),
        )
        .await?;
    db.collection::<Document>(READINESS_RECORDS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"candidate_digest":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<PublicRevocationMarker>(REVOCATION_SCOPES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"drain_pending":1,"_id":1})
                .build(),
        )
        .await?;
    db.collection::<PublicCleanupRecord>(CLEANUP_RECORDS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"scope_marker_id":1,"completed":1,"retry_at":1})
                .build(),
        )
        .await?;
    db.collection::<PublicRevocationMarker>(REVOCATION_SCOPES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"scope_key":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    // No TTL: expired authority is revoked before local/upstream cleanup.
    Ok(())
}

#[async_trait]
impl PublicTurnProducer for PublicProducer {
    async fn discover_public_profiles(
        &self,
        db: &Database,
        reviewed: &ManagerRuntimePolicy,
    ) -> AppResult<PublicProfileDiscovery> {
        Ok(PublicProfileDiscovery {
            profiles: Vec::new(),
            default_profile: None,
            readiness: policy::assess(db, reviewed).await?,
        })
    }
    async fn probe_public_readiness(
        &self,
        db: &Database,
        reviewed: &ManagerRuntimePolicy,
    ) -> AppResult<PublicReadiness> {
        policy::assess(db, reviewed).await
    }
    async fn fence_public_readiness(
        &self,
        _db: &Database,
        _session: &mut ClientSession,
        _reviewed: &ManagerRuntimePolicy,
        _evidence: &PublicReadinessEvidence,
    ) -> AppResult<FenceOutcome> {
        // Even a caller-constructed Ready snapshot cannot create deployment proof.
        Ok(FenceOutcome::Unavailable(UnavailableReason::PublicNotReady))
    }
    async fn prepare_public_context(
        &self,
        _db: &Database,
        admitted: &PublicIngress,
    ) -> AppResult<PublicExecutionContext> {
        super::platform_public_credential_service::issue_context(
            &self.state,
            self.ingress.clone(),
            admitted,
        )
        .await
    }
    async fn fence_public_execution(
        &self,
        db: &Database,
        session: &mut ClientSession,
        context: &PublicExecutionContext,
    ) -> AppResult<FenceOutcome> {
        if policy::verified_context(context).is_err() {
            return Ok(FenceOutcome::Unavailable(UnavailableReason::PublicNotReady));
        }
        let row = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&context.public_session_id})
            .session(&mut *session)
            .await?
            .ok_or_else(changed)?;
        super::platform_public_budget_service::fence_context(
            db,
            session,
            context,
            &row.credential_api_key_id,
        )
        .await?;
        Ok(FenceOutcome::Authorized)
    }
    async fn run_public_turn(
        &self,
        db: &Database,
        context: PublicExecutionContext,
        input: TurnInput,
        events: mpsc::Sender<ExecutionEvent>,
    ) -> AppResult<TurnRef> {
        policy::verified_context(&context)?;
        let turn = context.turn.clone();
        self.execution
            .execute_nyxagent_turn(db, NyxAgentExecutionContext::Public(context), input, events)
            .await?;
        Ok(turn)
    }
    async fn fence_public_reply(
        &self,
        db: &Database,
        session: &mut ClientSession,
        destination: &ReplyDestination,
    ) -> AppResult<FenceOutcome> {
        let Ok((_, evidence)) = policy::verified_readiness() else {
            return Ok(FenceOutcome::Unavailable(UnavailableReason::PublicNotReady));
        };
        let ReplyAuthority::Public {
            ingress,
            public_session_id,
            ..
        } = &destination.authority
        else {
            return Err(changed());
        };
        let row = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":public_session_id})
            .session(&mut *session)
            .await?
            .ok_or_else(changed)?;
        if evidence.manager != ingress.manager
            || evidence.public_policy_revision != ingress.public_policy_revision
            || evidence.policy_digest != row.policy_digest
            || evidence.sponsor_owner_user_id != row.sponsor_owner_user_id
            || evidence.nyxagent_profile != row.nyxagent_profile
        {
            return Err(changed());
        }
        super::platform_public_credential_service::fence_reply_in_session(db, session, destination)
            .await?;
        Ok(FenceOutcome::Authorized)
    }
    async fn revoke_public_sender(
        &self,
        db: &Database,
        session: &mut ClientSession,
        manager: &ManagerAuthority,
        sender_id: i64,
        chat_id: i64,
    ) -> AppResult<()> {
        revoke_in_session(
            db,
            session,
            &PublicRevocationScope::Sender {
                manager: manager.clone(),
                sender_id,
                chat_id,
            },
            PublicRevocationReason::PersonalLinkConfirmed,
        )
        .await
        .map(|_| ())
    }
    async fn revoke_public_sessions_in_session(
        &self,
        db: &Database,
        session: &mut ClientSession,
        scope: &PublicRevocationScope,
        reason: PublicRevocationReason,
    ) -> AppResult<PublicRevocationRecorded> {
        revoke_in_session(db, session, scope, reason).await
    }
}

fn reason_label(reason: PublicRevocationReason) -> &'static str {
    match reason {
        PublicRevocationReason::PersonalLinkConfirmed => "personal_link_confirmed",
        PublicRevocationReason::SenderUnlinked => "sender_unlinked",
        PublicRevocationReason::ManagerRetired => "manager_retired",
        PublicRevocationReason::SessionExpired => "session_expired",
    }
}

fn scope_filter(scope: &PublicRevocationScope) -> Document {
    let mut filter = doc! {"revoked_at":bson::Bson::Null};
    match scope {
        PublicRevocationScope::Sender {
            manager,
            sender_id,
            chat_id,
        } => {
            filter.extend(manager_filter(manager));
            filter.insert("sender_id", *sender_id);
            filter.insert("chat_id", *chat_id);
        }
        PublicRevocationScope::Manager { manager } => filter.extend(manager_filter(manager)),
        PublicRevocationScope::Sponsor {
            sponsor_owner_user_id,
        } => {
            filter.insert("sponsor_owner_user_id", sponsor_owner_user_id);
        }
        PublicRevocationScope::Session { public_session_id } => {
            filter.insert("_id", public_session_id);
        }
    }
    filter
}
fn manager_filter(manager: &ManagerAuthority) -> Document {
    doc! {"profile_id":&manager.profile_id,"manager_bot_id":&manager.manager_bot_id,
    "telegram_bot_id":manager.telegram_bot_id,"manager_authority_epoch":{"$lte":manager.manager_authority_epoch}}
}
fn changed() -> AppError {
    AppError::Conflict("Public session authority changed".into())
}
fn empty_binary() -> bson::Binary {
    bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes: Vec::new(),
    }
}

fn marker_identity(scope: &PublicRevocationScope) -> (String, Document, Option<i64>) {
    use sha2::{Digest, Sha256};
    let mut filter = scope_filter(scope);
    filter.remove("revoked_at");
    let epoch = filter
        .remove("manager_authority_epoch")
        .and_then(|v| v.as_document().and_then(|d| d.get_i64("$lte").ok()));
    let bytes = bson::to_vec(&filter).expect("fixed scope selector encodes");
    (hex::encode(Sha256::digest(bytes)), filter, epoch)
}

fn marker_filter(marker: &PublicRevocationMarker) -> Document {
    let mut filter = marker.session_filter.clone();
    filter.insert("revoked_at", bson::Bson::Null);
    filter.insert(
        "created_at",
        doc! {"$lte":marker.revoked_before.map(bson::DateTime::from_chrono)},
    );
    if let Some(epoch) = marker.maximum_epoch {
        filter.insert("manager_authority_epoch", doc! {"$lte":epoch});
    }
    filter
}

/// Joins the caller transaction. The scope marker denies all matching old
/// sessions immediately, including sessions beyond the first bounded key page.
pub(crate) async fn revoke_in_session(
    db: &Database,
    session: &mut ClientSession,
    scope: &PublicRevocationScope,
    reason: PublicRevocationReason,
) -> AppResult<PublicRevocationRecorded> {
    let (id, filter, epoch) = marker_identity(scope);
    let marker = db.collection::<PublicRevocationMarker>(REVOCATION_SCOPES)
        .find_one_and_update(doc! {"scope_key":&id}, vec![doc! {"$set": {
            "_id":{"$ifNull":["$_id",uuid::Uuid::new_v4().to_string()]},"scope_key":&id,
            "session_filter":{"$literal":filter},
            "maximum_epoch": match epoch { Some(epoch) => bson::Bson::Document(doc! {"$max":[{"$ifNull":["$maximum_epoch",0_i64]},epoch]}), None => bson::Bson::Null },
            "revoked_before":"$$NOW", "drain_pending":true,"reason":reason_label(reason),
            "effect_revision":{"$add":[{"$ifNull":["$effect_revision",0_i64]},1_i64]},
        }}]).upsert(true).return_document(ReturnDocument::After)
        .session(&mut *session).await?.ok_or_else(changed)?;
    revoke_page_in_session(db, session, &marker).await?;
    Ok(PublicRevocationRecorded)
}

async fn revoke_page_in_session(
    db: &Database,
    session: &mut ClientSession,
    marker: &PublicRevocationMarker,
) -> AppResult<usize> {
    let mut cursor = db
        .collection::<PlatformPublicSession>(SESSIONS)
        .find(marker_filter(marker))
        .sort(doc! {"_id":1})
        .limit(MAX_REVOKE_SESSIONS as i64)
        .session(&mut *session)
        .await?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.next(&mut *session).await.transpose()? {
        rows.push(row);
    }
    let count = rows.len();
    if db.collection::<PublicRevocationMarker>(REVOCATION_SCOPES).update_one(
        doc! {"_id":&marker.id,"effect_revision":marker.effect_revision},
        doc! {"$set":{"drain_pending":count == MAX_REVOKE_SESSIONS},"$inc":{"effect_revision":1_i64}},
    ).session(&mut *session).await?.matched_count != 1 { return Err(changed()); }
    if count == 0 {
        return Ok(0);
    }
    let record_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let record = PublicCleanupRecord {
        id: record_id.clone(),
        session_ids: rows.iter().map(|row| row.id.clone()).collect(),
        scope_marker_id: Some(marker.id.clone()),
        reason: marker.reason.clone(),
        completed: false,
        claim_id: None,
        attempts: 0,
        blocked_reason: None,
        lease_expires_at: None,
        retry_at: now,
        created_at: now,
    };
    db.collection::<PublicCleanupRecord>(CLEANUP_RECORDS)
        .insert_one(&record)
        .session(&mut *session)
        .await?;
    for row in rows {
        let key = db
            .collection::<ApiKey>(KEYS)
            .find_one(doc! {"_id":&row.credential_api_key_id})
            .session(&mut *session)
            .await?;
        if let Some(key) = key {
            if key.purpose != ApiKeyPurpose::PublicNyxbot
                || key.user_id != row.sponsor_owner_user_id
            {
                return Err(changed());
            }
            let result = transactions::update_one(
                db,
                doc! {"_id":&key.id,"user_id":&row.sponsor_owner_user_id,"purpose":"public_nyxbot"},
                doc! {"$set":{"is_active":false}},
                Some(&mut *session),
            )
            .await?;
            if result.matched_count != 1 {
                return Err(changed());
            }
        }
        let mut fields = doc! {"revoked_at":bson::DateTime::from_chrono(now),"revocation_reason":&marker.reason,
        "cleanup_record_id":&record_id,"credential_ciphertext":empty_binary()};
        if row.active_turn.is_some() {
            fields.insert("active_turn.stop_requested", true);
        }
        if db.collection::<PlatformPublicSession>(SESSIONS).update_one(
            doc! {"_id":&row.id,"credential_api_key_id":&row.credential_api_key_id,"revoked_at":bson::Bson::Null},
            doc! {"$set":fields,"$inc":{"effect_revision":1_i64}},
        ).session(&mut *session).await?.matched_count != 1 { return Err(changed()); }
        // Existing child credentials can only be corrupt/legacy data, but must
        // not survive a public revocation either. No child secret is returned.
        super::api_key_credential_service::revoke_children(
            db,
            &row.credential_api_key_id,
            crate::models::api_key_credential::CredentialRevokedReason::ParentRevoked,
            Some(&mut *session),
        )
        .await?;
    }
    Ok(count)
}

async fn drain_scope(db: &Database, id: &str) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run((db.clone(), id.to_owned()), |session, (db, id)| {
            Box::pin(async move {
                let result: AppResult<_> = async {
                    if let Some(marker) = db
                        .collection::<PublicRevocationMarker>(REVOCATION_SCOPES)
                        .find_one(doc! {"_id":&*id,"drain_pending":true})
                        .session(&mut *session)
                        .await?
                    {
                        revoke_page_in_session(db, session, &marker).await?;
                    }
                    Ok(())
                }
                .await;
                transactions::transaction_result(result)
            })
        })
        .await
        .map_err(transactions::map_transaction_error)
}

/// Local component of the public fence. This never establishes runtime proof.
/// Session issuance and every dispatch/reply write the same scope documents as
/// revocation, so a concurrent marker cannot slip between validation and effect.
pub(crate) async fn fence_local_session_in_session(
    db: &Database,
    session: &mut ClientSession,
    row: &PlatformPublicSession,
) -> AppResult<bool> {
    if row.revoked_at.is_some() || row.expires_at <= Utc::now() {
        return Ok(false);
    }
    let manager = ManagerAuthority {
        profile_id: row.profile_id.clone(),
        manager_bot_id: row.manager_bot_id.clone(),
        telegram_bot_id: row.telegram_bot_id,
        manager_authority_epoch: row.manager_authority_epoch,
    };
    for scope in [
        PublicRevocationScope::Session {
            public_session_id: row.id.clone(),
        },
        PublicRevocationScope::Sponsor {
            sponsor_owner_user_id: row.sponsor_owner_user_id.clone(),
        },
        PublicRevocationScope::Manager {
            manager: manager.clone(),
        },
        PublicRevocationScope::Sender {
            manager,
            sender_id: row.sender_id,
            chat_id: row.chat_id,
        },
    ] {
        let (id, filter, _) = marker_identity(&scope);
        let marker = db.collection::<PublicRevocationMarker>(REVOCATION_SCOPES).find_one_and_update(
            doc! {"scope_key":&id}, doc! {"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),"scope_key":&id,"session_filter":filter,"maximum_epoch":bson::Bson::Null,
                "revoked_before":bson::Bson::Null,"drain_pending":false,"reason":""},"$inc":{"effect_revision":1_i64}},
        ).upsert(true).return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(changed)?;
        if marker
            .revoked_before
            .is_some_and(|cutoff| row.created_at <= cutoff)
            && marker
                .maximum_epoch
                .is_none_or(|epoch| row.manager_authority_epoch <= epoch)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn process_cleanup(db: &Database, id: &str) -> AppResult<()> {
    let claim_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let Some(record) = db.collection::<PublicCleanupRecord>(CLEANUP_RECORDS).find_one_and_update(
        doc! {"_id":id,"completed":false,"retry_at":{"$lte":bson::DateTime::from_chrono(now)},
            "$or":[{"lease_expires_at":bson::Bson::Null},{"lease_expires_at":{"$lte":bson::DateTime::from_chrono(now)}}]},
        doc! {"$set":{"claim_id":&claim_id,"lease_expires_at":bson::DateTime::from_chrono(now+Duration::seconds(CLEANUP_LEASE_SECS))},"$inc":{"attempts":1_i64}},
    ).return_document(ReturnDocument::After).await? else { return Ok(()); };
    let mut blocked = None;
    for session_id in &record.session_ids {
        let row = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {
                "_id":session_id,"cleanup_record_id":id,"revoked_at":{"$ne":bson::Bson::Null},
            })
            .await?;
        let Some(row) = row else {
            blocked = Some("local_session_missing");
            continue;
        };
        if row.cleanup_complete {
            continue;
        }
        if db
            .collection::<ApiKey>(KEYS)
            .count_documents(doc! {"_id":&row.credential_api_key_id,"is_active":true})
            .limit(1)
            .await?
            != 0
        {
            blocked = Some("execution_key_not_revoked");
            continue;
        }
        if row.upstream_may_exist || !row.upstream_binding_ciphertext.is_empty() {
            // DELETE currently requires the revoked original key. Never restore
            // execution to perform cleanup, nor report TTL as verified erasure.
            blocked = Some("upstream_cleanup_contract_unproven");
            continue;
        }
        let result = db.collection::<PlatformPublicSession>(SESSIONS).update_one(doc! {
            "_id":session_id,"cleanup_record_id":id,"revoked_at":{"$ne":bson::Bson::Null},
            "upstream_may_exist":false,"upstream_binding_ciphertext":empty_binary(),
        }, doc! {"$set":{"cleanup_complete":true,"active_turn":bson::Bson::Null,
            "credential_ciphertext":empty_binary(),"upstream_binding_ciphertext":empty_binary()},
            "$inc":{"effect_revision":1_i64}}).await?;
        if result.matched_count != 1 {
            blocked = Some("local_session_authority_changed");
        }
    }
    let result = db
        .collection::<PublicCleanupRecord>(CLEANUP_RECORDS)
        .update_one(
            doc! {"_id":id,"claim_id":&claim_id,"completed":false},
            doc! {"$set":{"completed":blocked.is_none(),"blocked_reason":blocked,
            "claim_id":bson::Bson::Null,"lease_expires_at":bson::Bson::Null,
            "retry_at":bson::DateTime::from_chrono(Utc::now()+Duration::hours(1))}},
        )
        .await?;
    if result.matched_count != 1 {
        return Err(changed());
    }
    Ok(())
}
/// Bounded local-only sweep. There is no network call or erasure assertion for
/// an upstream session whose cleanup contract remains unproven.
pub async fn sweep_public_lifecycle(state: &AppState, limit: u32) -> AppResult<RecoveryBatch> {
    let db = &state.db;
    let limit = limit.clamp(1, 100);
    let scopes: Vec<PublicRevocationMarker> = db
        .collection::<PublicRevocationMarker>(REVOCATION_SCOPES)
        .find(doc! {"drain_pending":true})
        .sort(doc! {"_id":1})
        .limit(i64::from(limit))
        .await?
        .try_collect()
        .await?;
    for marker in &scopes {
        drain_scope(db, &marker.id).await?;
    }
    let expired_turns:Vec<PlatformPublicSession>=db.collection::<PlatformPublicSession>(SESSIONS).find(doc! {
        "active_turn":{"$ne":bson::Bson::Null},"active_turn.expires_at":{"$lte":bson::DateTime::now()},
        "$or":[{"turn_recovery_retry_at":{"$exists":false}},{"turn_recovery_retry_at":{"$lte":bson::DateTime::now()}}],
    }).sort(doc! {"active_turn.expires_at":1,"_id":1}).limit(i64::from(limit)).await?.try_collect().await?;
    for row in &expired_turns {
        if super::platform_public_credential_service::recover_expired_turn(db, row)
            .await
            .is_err()
        {
            db.collection::<Document>(SESSIONS).update_one(doc! {"_id":&row.id},doc! {"$set":{
                "turn_recovery_error":"revalidation_or_storage_failed",
                "turn_recovery_retry_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(1)),
            }}).await?;
        }
    }
    let expired: Vec<PlatformPublicSession> = db
        .collection::<PlatformPublicSession>(SESSIONS)
        .find(doc! {"revoked_at":bson::Bson::Null,"expires_at":{"$lte":bson::DateTime::now()}})
        .sort(doc! {"expires_at":1,"_id":1})
        .limit(i64::from(limit))
        .await?
        .try_collect()
        .await?;
    let mut processed = (scopes.len() + expired_turns.len()) as u32;
    for row in expired {
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .and_run((db.clone(), row.id), |session, (db, session_id)| {
                Box::pin(async move {
                    transactions::transaction_result(
                        revoke_in_session(
                            db,
                            session,
                            &PublicRevocationScope::Session {
                                public_session_id: session_id.clone(),
                            },
                            PublicRevocationReason::SessionExpired,
                        )
                        .await,
                    )
                })
            })
            .await
            .map_err(transactions::map_transaction_error)?;
        processed += 1;
    }
    let due = doc! {"completed":false,"retry_at":{"$lte":bson::DateTime::now()},
    "$or":[{"lease_expires_at":bson::Bson::Null},{"lease_expires_at":{"$lte":bson::DateTime::now()}}]};
    let records: Vec<PublicCleanupRecord> = db
        .collection::<PublicCleanupRecord>(CLEANUP_RECORDS)
        .find(due.clone())
        .sort(doc! {"retry_at":1,"_id":1})
        .limit(i64::from(limit))
        .await?
        .try_collect()
        .await?;
    for record in records {
        process_cleanup(db, &record.id).await?;
        processed += 1;
    }
    let has_more = db
        .collection::<PublicRevocationMarker>(REVOCATION_SCOPES)
        .count_documents(doc! {"drain_pending":true})
        .limit(1)
        .await?
        > 0
        || db
            .collection::<PublicCleanupRecord>(CLEANUP_RECORDS)
            .count_documents(due)
            .limit(1)
            .await?
            > 0
        || db
            .collection::<PlatformPublicSession>(SESSIONS)
            .count_documents(
                doc! {"revoked_at":bson::Bson::Null,"expires_at":{"$lte":bson::DateTime::now()}},
            )
            .limit(1)
            .await?
            > 0;
    Ok(RecoveryBatch {
        processed,
        has_more,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::models::platform_public_session::PublicTurnBinding;

    pub(crate) async fn fixture(
        db: &Database,
        sender_id: i64,
        upstream_may_exist: bool,
    ) -> PlatformPublicSession {
        let now = Utc::now();
        let row = PlatformPublicSession {
            id: uuid::Uuid::new_v4().to_string(),
            profile_id: "profile".into(),
            manager_bot_id: "99".into(),
            telegram_bot_id: 99,
            manager_authority_epoch: 3,
            sender_id,
            chat_id: sender_id,
            sponsor_owner_user_id: uuid::Uuid::new_v4().to_string(),
            public_policy_revision: 1,
            policy_digest: "candidate-only".into(),
            nyxagent_profile: "unverified-public".into(),
            credential_api_key_id: uuid::Uuid::new_v4().to_string(),
            credential_ciphertext: vec![10, 20, 30],
            upstream_binding_ciphertext: if upstream_may_exist {
                vec![40, 50, 60]
            } else {
                vec![]
            },
            upstream_may_exist,
            active_turn: Some(PublicTurnBinding {
                conversation_id: uuid::Uuid::new_v4().to_string(),
                turn_id: uuid::Uuid::new_v4().to_string(),
                receipt_id: uuid::Uuid::new_v4().to_string(),
                claim_id: uuid::Uuid::new_v4().to_string(),
                stop_requested: false,
                expires_at: now + Duration::minutes(5),
            }),
            effect_revision: 0,
            cleanup_record_id: None,
            cleanup_complete: false,
            revocation_reason: None,
            revoked_at: None,
            created_at: now,
            expires_at: now + Duration::hours(24),
        };
        // No raw credential or live account is created by this fixture.
        db.collection::<Document>(KEYS)
            .insert_one(doc! {
                "_id":&row.credential_api_key_id,"user_id":&row.sponsor_owner_user_id,
                "name":"public test capability","key_prefix":"fixture",
                "key_hash":"unusable-test-hash","scopes":"", "is_active":true,
                "purpose":"public_nyxbot","created_at":bson::DateTime::from_chrono(now),
            })
            .await
            .unwrap();
        db.collection::<PlatformPublicSession>(SESSIONS)
            .insert_one(&row)
            .await
            .unwrap();
        row
    }

    async fn session_row(db: &Database, row: &PlatformPublicSession) -> PlatformPublicSession {
        db.collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn public_discovery_and_caller_supplied_proof_cannot_enable_execution() {
        let db = crate::test_utils::connect_transaction_test_database("public_no_proof").await;
        ensure_indexes(&db).await.unwrap();
        let now = Utc::now();
        let reviewed = ManagerRuntimePolicy {
            manager: ManagerAuthority {
                profile_id: "profile".into(),
                manager_bot_id: "99".into(),
                telegram_bot_id: 99,
                manager_authority_epoch: 1,
            },
            personal_enabled: true,
            public_enabled: true,
            emergency_stopped: false,
            personal_control_revision: 1,
            public_policy_revision: 1,
            public_policy: Some(PublicPolicySelection {
                sponsor_owner_user_id: "sponsor".into(),
                nyxagent_profile: "chat".into(),
                total_daily_credits: crate::models::credits::Credits::from_whole(10),
                sender_daily_credits: crate::models::credits::Credits::from_whole(1),
            }),
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let public = super::super::platform_manager_transport::compose(
            state.clone(),
            crate::handlers::assistant_team::owner_turn_settled_hook(state),
        )
        .public
        .clone();
        for _ in 0..2 {
            let discovery = public
                .discover_public_profiles(&db, &reviewed)
                .await
                .unwrap();
            assert!(discovery.profiles.is_empty());
            assert!(discovery.default_profile.is_none());
            assert!(matches!(
                discovery.readiness,
                PublicReadiness::Unavailable(UnavailableReason::PublicNotReady)
            ));
        }
        assert_eq!(
            db.collection::<Document>(READINESS_RECORDS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        let evidence = PublicReadinessEvidence {
            manager: reviewed.manager.clone(),
            public_policy_revision: 1,
            policy_digest: "forged".into(),
            proof_record_id: "forged".into(),
            candidate_digest: policy::candidate_digest(&reviewed).unwrap(),
            evidence_digest: "forged".into(),
            sponsor_owner_user_id: "sponsor".into(),
            nyxagent_build: policy::UPSTREAM_SOURCE_BUILD.into(),
            nyxagent_profile: "chat".into(),
            model_operations: vec![],
            limits: PublicExecutionLimits {
                max_duration_secs: 1,
                max_model_calls: 1,
                max_tool_calls: 1,
                max_continuations: 0,
                max_input_tokens: 1,
                max_output_tokens: 1,
                max_input_bytes: 1,
                max_context_bytes: 1,
                max_output_bytes: 1,
            },
            checked_at: now,
            expires_at: now + Duration::minutes(5),
        };
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert_eq!(
            public
                .fence_public_readiness(&db, &mut session, &reviewed, &evidence)
                .await
                .unwrap(),
            FenceOutcome::Unavailable(UnavailableReason::PublicNotReady)
        );
        session.abort_transaction().await.unwrap();
        assert_eq!(
            db.collection::<Document>(KEYS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<Document>(SESSIONS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn public_revocation_rolls_back_with_its_caller_transaction() {
        let db = crate::test_utils::connect_transaction_test_database("public_revoke_abort").await;
        ensure_indexes(&db).await.unwrap();
        let row = fixture(&db, 5, true).await;
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        revoke_in_session(
            &db,
            &mut session,
            &PublicRevocationScope::Session {
                public_session_id: row.id.clone(),
            },
            PublicRevocationReason::PersonalLinkConfirmed,
        )
        .await
        .unwrap();
        assert_eq!(
            db.collection::<PublicRevocationMarker>(REVOCATION_SCOPES)
                .count_documents(doc! {})
                .session(&mut session)
                .await
                .unwrap(),
            1
        );
        let in_transaction = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .session(&mut session)
            .await
            .unwrap()
            .unwrap();
        assert!(in_transaction.revoked_at.is_some());
        assert!(in_transaction.credential_ciphertext.is_empty());
        assert!(in_transaction.active_turn.unwrap().stop_requested);
        session.abort_transaction().await.unwrap();
        let unchanged = session_row(&db, &row).await;
        assert!(unchanged.revoked_at.is_none());
        assert_eq!(unchanged.credential_ciphertext, row.credential_ciphertext);
        assert!(!unchanged.active_turn.unwrap().stop_requested);
        assert_eq!(
            db.collection::<PublicCleanupRecord>(CLEANUP_RECORDS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert!(
            db.collection::<ApiKey>(KEYS)
                .find_one(doc! {"_id":&row.credential_api_key_id})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
    }

    #[tokio::test]
    async fn cleanup_retains_unknown_upstream_and_completes_only_never_dispatched_sessions() {
        let db = crate::test_utils::connect_transaction_test_database("public_cleanup_proof").await;
        ensure_indexes(&db).await.unwrap();
        let local = fixture(&db, 5, false).await;
        let remote = fixture(&db, 6, true).await;
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        revoke_in_session(
            &db,
            &mut session,
            &PublicRevocationScope::Manager {
                manager: ManagerAuthority {
                    profile_id: "profile".into(),
                    manager_bot_id: "99".into(),
                    telegram_bot_id: 99,
                    manager_authority_epoch: 3,
                },
            },
            PublicRevocationReason::ManagerRetired,
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        sweep_public_lifecycle(&state, 100).await.unwrap();
        let id = session_row(&db, &remote).await.cleanup_record_id.unwrap();
        let local = session_row(&db, &local).await;
        let remote_after = session_row(&db, &remote).await;
        assert!(local.cleanup_complete);
        assert!(local.active_turn.is_none());
        assert!(!remote_after.cleanup_complete);
        assert!(remote_after.active_turn.unwrap().stop_requested);
        assert_eq!(
            remote_after.upstream_binding_ciphertext,
            remote.upstream_binding_ciphertext
        );
        assert!(remote_after.credential_ciphertext.is_empty());
        assert_eq!(
            db.collection::<ApiKey>(KEYS)
                .count_documents(doc! {"is_active":true})
                .await
                .unwrap(),
            0
        );
        let record = db
            .collection::<PublicCleanupRecord>(CLEANUP_RECORDS)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        assert!(!record.completed);
        assert_eq!(
            record.blocked_reason.as_deref(),
            Some("upstream_cleanup_contract_unproven")
        );
        assert_eq!(record.attempts, 1);
        process_cleanup(&db, &id).await.unwrap();
        let record = db
            .collection::<PublicCleanupRecord>(CLEANUP_RECORDS)
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.attempts, 1,
            "a pending retry must respect its due time"
        );
    }

    #[tokio::test]
    async fn oversized_scope_denies_undrained_sessions_then_drains_bounded_pages() {
        let db = crate::test_utils::connect_transaction_test_database("public_revoke_pages").await;
        ensure_indexes(&db).await.unwrap();
        let row = fixture(&db, 5, true).await;
        let mut extra = Vec::new();
        for n in 0..MAX_REVOKE_SESSIONS {
            let mut next = row.clone();
            next.id = uuid::Uuid::new_v4().to_string();
            next.credential_api_key_id = uuid::Uuid::new_v4().to_string();
            next.sender_id = 100 + n as i64;
            next.chat_id = next.sender_id;
            extra.push(next);
        }
        db.collection::<PlatformPublicSession>(SESSIONS)
            .insert_many(extra)
            .await
            .unwrap();
        let manager = ManagerAuthority {
            profile_id: "profile".into(),
            manager_bot_id: "99".into(),
            telegram_bot_id: 99,
            manager_authority_epoch: 3,
        };
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        revoke_in_session(
            &db,
            &mut session,
            &PublicRevocationScope::Manager { manager },
            PublicRevocationReason::ManagerRetired,
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        let recovery = db
            .collection::<PublicRevocationMarker>(REVOCATION_SCOPES)
            .find_one(doc! {"drain_pending":true})
            .await
            .unwrap()
            .unwrap()
            .id;
        assert_eq!(
            db.collection::<PlatformPublicSession>(SESSIONS)
                .count_documents(doc! {"revoked_at":{"$ne":bson::Bson::Null}})
                .await
                .unwrap(),
            MAX_REVOKE_SESSIONS as u64
        );
        let undrained = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"revoked_at":bson::Bson::Null})
            .await
            .unwrap()
            .unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            !fence_local_session_in_session(&db, &mut session, &undrained)
                .await
                .unwrap()
        );
        session.abort_transaction().await.unwrap();
        drain_scope(&db, &recovery).await.unwrap();
        assert_eq!(
            db.collection::<PlatformPublicSession>(SESSIONS)
                .count_documents(doc! {"revoked_at":bson::Bson::Null})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<PublicCleanupRecord>(CLEANUP_RECORDS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            2
        );
        drain_scope(&db, &recovery).await.unwrap();
        assert_eq!(
            db.collection::<PublicCleanupRecord>(CLEANUP_RECORDS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn legacy_scope_markers_keep_cutoffs_and_drain_state_after_index_upgrade() {
        let db =
            crate::test_utils::connect_transaction_test_database("public_marker_upgrade").await;
        let row = fixture(&db, 5, true).await;
        let manager = ManagerAuthority {
            profile_id: row.profile_id.clone(),
            manager_bot_id: row.manager_bot_id.clone(),
            telegram_bot_id: row.telegram_bot_id,
            manager_authority_epoch: row.manager_authority_epoch,
        };
        let mut ids = Vec::new();
        for scope in [
            PublicRevocationScope::Manager { manager },
            PublicRevocationScope::Sponsor {
                sponsor_owner_user_id: row.sponsor_owner_user_id.clone(),
            },
        ] {
            let (id, filter, epoch) = marker_identity(&scope);
            ids.push(id.clone());
            db.collection::<Document>(REVOCATION_SCOPES).insert_one(doc! {"_id":id,"session_filter":filter,"maximum_epoch":epoch,
                "revoked_before":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(1)),"drain_pending":true,"reason":"manager_retired","effect_revision":2_i64}).await.unwrap();
        }
        ensure_indexes(&db).await.unwrap();
        for id in ids {
            let marker = db
                .collection::<PublicRevocationMarker>(REVOCATION_SCOPES)
                .find_one(doc! {"scope_key":&id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(marker.id, id);
            assert!(marker.drain_pending && marker.revoked_before.is_some());
        }
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            !fence_local_session_in_session(&db, &mut session, &row)
                .await
                .unwrap()
        );
        session.abort_transaction().await.unwrap();
    }

    #[test]
    fn revocation_scope_preserves_exact_manager_identity_and_future_epochs() {
        let manager = ManagerAuthority {
            profile_id: "profile".into(),
            manager_bot_id: "99".into(),
            telegram_bot_id: 99,
            manager_authority_epoch: 3,
        };
        let filter = scope_filter(&PublicRevocationScope::Sender {
            manager,
            sender_id: 5,
            chat_id: 5,
        });
        assert_eq!(filter.get_str("profile_id").unwrap(), "profile");
        assert_eq!(filter.get_i64("sender_id").unwrap(), 5);
        assert_eq!(
            filter
                .get_document("manager_authority_epoch")
                .unwrap()
                .get_i64("$lte")
                .unwrap(),
            3
        );
        assert_eq!(filter.get("revoked_at"), Some(&bson::Bson::Null));
    }
}
