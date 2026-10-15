//! One retained detail per primary invocation; component rows keep their charge identities.
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, doc},
    options::IndexOptions,
};

use crate::{
    errors::{AppError, AppResult},
    models::{
        credits::Credits,
        llm_usage::{COLLECTION_NAME, LlmInvocation, LlmInvocationCharge},
        usage_meter::{COLLECTION_NAME as METERS, UsageMeterRow},
    },
};

pub const RETENTION_DAYS: i64 = 365;

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InvocationQuery {
    pub owner_id: Option<String>,
    pub from: Option<chrono::DateTime<Utc>>,
    pub to: Option<chrono::DateTime<Utc>>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub conversation_id: Option<String>,
    pub turn_id: Option<String>,
}

/// Apply the effective org service scope to historical connection identities.
pub fn access_filter(
    owner: &str,
    access: &crate::services::org_service::OwnerAccess,
) -> AppResult<bson::Document> {
    use crate::services::org_service::OwnerAccess;
    let mut filter = doc! {"billing_owner_id":owner,"expires_at":{"$gt":bson::DateTime::from_chrono(Utc::now())}};
    match access {
        OwnerAccess::Forbidden => {
            return Err(AppError::Forbidden("Usage owner is unavailable".into()));
        }
        OwnerAccess::AsOrgAdmin {
            allowed_service_ids: Some(ids),
            ..
        }
        | OwnerAccess::AsOrgMember {
            allowed_service_ids: Some(ids),
            ..
        } => {
            filter.insert("user_service_id", doc! {"$in":ids});
        }
        _ => {}
    }
    Ok(filter)
}

async fn authorized_filter(db: &Database, actor: &str, owner: &str) -> AppResult<bson::Document> {
    uuid::Uuid::parse_str(owner)
        .map_err(|_| AppError::ValidationError("Invalid usage owner".into()))?;
    let access = crate::services::org_service::resolve_owner_access(db, actor, owner).await?;
    access_filter(owner, &access)
}

pub async fn list(
    db: &Database,
    actor: &str,
    query: InvocationQuery,
) -> AppResult<(Vec<LlmInvocation>, Option<String>)> {
    let owner = query.owner_id.as_deref().unwrap_or(actor);
    let mut filter = authorized_filter(db, actor, owner).await?;
    let to = query.to.unwrap_or_else(Utc::now);
    let from = query
        .from
        .or_else(|| to.checked_sub_signed(Duration::days(30)))
        .ok_or_else(|| {
            AppError::ValidationError("Usage window is outside supported dates".into())
        })?;
    let limit = query.limit.unwrap_or(50);
    if to <= from || to - from > Duration::days(31) || !(1..=100).contains(&limit) {
        return Err(AppError::ValidationError(
            "Usage requires a positive window of at most 31 days and limit 1..100".into(),
        ));
    }
    filter.insert(
        "created_at",
        doc! {"$gte":bson::DateTime::from_chrono(from),"$lt":bson::DateTime::from_chrono(to)},
    );
    for (key, value) in [
        ("context.conversation_id", query.conversation_id),
        ("context.turn_id", query.turn_id),
    ] {
        if let Some(value) = value {
            if value.is_empty()
                || value.len() > 128
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
            {
                return Err(AppError::ValidationError(
                    "Invalid usage identity filter".into(),
                ));
            }
            filter.insert(key, value);
        }
    }
    if let Some(cursor) = query.cursor {
        let (date, id) = cursor
            .split_once(':')
            .ok_or_else(|| AppError::ValidationError("Invalid usage cursor".into()))?;
        let date = date
            .parse::<i64>()
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .ok_or_else(|| AppError::ValidationError("Invalid usage cursor".into()))?;
        uuid::Uuid::parse_str(id)
            .map_err(|_| AppError::ValidationError("Invalid usage cursor".into()))?;
        filter.insert(
            "$or",
            vec![
                doc! {"created_at":{"$lt":bson::DateTime::from_chrono(date)}},
                doc! {"created_at":bson::DateTime::from_chrono(date),"_id":{"$lt":id}},
            ],
        );
    }
    let mut rows: Vec<LlmInvocation> = db
        .collection::<LlmInvocation>(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"created_at":-1,"_id":-1})
        .limit(i64::from(limit) + 1)
        .max_time(std::time::Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = if more {
        rows.last()
            .map(|r| format!("{}:{}", r.created_at.timestamp_millis(), r.id))
    } else {
        None
    };
    Ok((rows, next))
}

pub async fn detail(
    db: &Database,
    actor: &str,
    owner: Option<&str>,
    id: &str,
) -> AppResult<LlmInvocation> {
    uuid::Uuid::parse_str(id)
        .map_err(|_| AppError::ValidationError("Invalid invocation identity".into()))?;
    let mut filter = authorized_filter(db, actor, owner.unwrap_or(actor)).await?;
    filter.insert("_id", id);
    db.collection::<LlmInvocation>(COLLECTION_NAME)
        .find_one(filter)
        .max_time(std::time::Duration::from_secs(5))
        .await?
        .ok_or_else(|| AppError::NotFound("Usage invocation not found".into()))
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    let details = db.collection::<LlmInvocation>(COLLECTION_NAME);
    for keys in [
        doc! {"billing_owner_id":1,"created_at":-1,"_id":-1},
        doc! {"billing_owner_id":1,"context.conversation_id":1,"context.turn_id":1,"created_at":-1,"_id":-1},
    ] {
        details
            .create_index(IndexModel::builder().keys(keys).build())
            .await?;
    }
    details
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<UsageMeterRow>(METERS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"llm_detail_pending":1,"llm_detail_retry_at":1,"_id":1})
                .options(
                    IndexOptions::builder()
                        .partial_filter_expression(doc! {"llm_detail_pending":true})
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

/// Hold the complete charge source set before attempting derived detail storage.
/// Read and write the current primary in one transaction so stale workers cannot
/// reinstate holds after a completed snapshot has cleared its marker.
async fn protect_sources(db: &Database, id: &str) -> AppResult<()> {
    let db = db.clone();
    let id = id.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result:AppResult<()> = async {
            let meters=db.collection::<UsageMeterRow>(METERS);
            if let Some(primary)=meters.find_one(doc! {"_id":&id,"llm_detail_pending":true}).session(&mut *session).await? {
                meters.update_many(doc! {"billing_request_id":&primary.billing_request_id,"flush_seq":primary.flush_seq},doc! {"$set":{"llm_retention_hold":true},"$unset":{"expires_at":""}}).session(&mut *session).await?;
            }
            Ok(())
        }.await;
        crate::services::api_key_mutation_service::transaction_result(result)
    }).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub async fn materialize(db: &Database, primary: &UsageMeterRow) -> AppResult<()> {
    protect_sources(db, &primary.id).await?;
    let db = db.clone();
    let primary_id = primary.id.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result = materialize_in_session(&db, &primary_id, session).await;
            crate::services::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

async fn materialize_in_session(
    db: &Database,
    primary_id: &str,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    let meters = db.collection::<UsageMeterRow>(METERS);
    let Some(primary) = meters
        .find_one(doc! {"_id": &primary_id, "llm_detail_pending":true})
        .session(&mut *session)
        .await?
    else {
        return Ok(());
    };
    let Some(observation) = primary.llm_observation.as_ref() else {
        return Ok(());
    };
    let filter =
        doc! {"billing_request_id":&primary.billing_request_id,"flush_seq":primary.flush_seq};
    let mut cursor = meters
        .find(filter.clone())
        .limit(33)
        .session(&mut *session)
        .await?;
    let rows: Vec<UsageMeterRow> = cursor.stream(&mut *session).try_collect().await?;
    drop(cursor);
    if rows.len() > 32 {
        return Err(AppError::Internal(
            "Too many LLM invocation charge components".into(),
        ));
    }
    let row_settled = |r: &UsageMeterRow| {
        r.released
            && r.funding
                .as_ref()
                .map_or(r.wallet_id.is_none(), |f| f.settled)
    };
    let settlement_complete = !rows.is_empty() && rows.iter().all(row_settled);
    let charges = rows
        .iter()
        .map(|r| {
            let funding = r.funding.as_ref();
            let settled = row_settled(r);
            LlmInvocationCharge {
                transaction_id: r.transaction_id.clone(),
                layer: r.layer,
                metric: r.metric,
                quantity: r.quantity,
                settled,
                billable: r.wallet_id.is_some(),
                rate_credits: funding.and_then(|f| f.settlement_rate),
                gross_credits: funding
                    .and_then(|f| f.total_charge)
                    .or_else(|| (settled && r.wallet_id.is_none()).then_some(Credits::ZERO)),
                allowance_credits: funding.and_then(|f| f.allowance_funded),
                grant_credits: funding.and_then(|f| f.grant_funded),
                wallet_credits: funding
                    .and_then(|f| f.wallet_funded)
                    .or_else(|| (settled && r.wallet_id.is_none()).then_some(Credits::ZERO)),
            }
        })
        .collect();
    let detail = LlmInvocation {
        id: primary.id.clone(),
        billing_request_id: primary.billing_request_id.clone(),
        actor_user_id: primary.actor_user_id.clone(),
        billing_owner_id: primary.billing_owner_id.clone(),
        api_key_id: primary.api_key_id.clone(),
        service_id: primary.service_id.clone(),
        user_service_id: primary.user_service_id.clone(),
        service_slug: primary.service_slug.clone(),
        credential_class: primary.credential_class,
        context: primary.llm_context.clone().unwrap_or_default(),
        observation: observation.clone(),
        pool_attempt: primary.pool_attempt.clone(),
        charges,
        settlement_complete,
        created_at: primary.created_at,
        expires_at: primary.created_at + Duration::days(RETENTION_DAYS),
    };
    let details = db.collection::<LlmInvocation>(COLLECTION_NAME);
    let document = bson::to_document(&detail)
        .map_err(|_| AppError::Internal("LLM invocation serialization failed".into()))?;
    details
        .update_one(
            doc! {"_id":&detail.id},
            doc! {"$setOnInsert":document.clone()},
        )
        .upsert(true)
        .session(&mut *session)
        .await?;
    // A stale concurrent worker cannot replace an already complete charge snapshot.
    details
        .update_one(
            doc! {"_id":&detail.id,"settlement_complete":{"$ne":true}},
            doc! {"$set":document},
        )
        .session(&mut *session)
        .await?;
    if settlement_complete {
        // ACK and stale recovery contend on the same rows. TTL restoration and
        // releasing the hold commit with the complete retained charge snapshot.
        meters.update_many(filter.clone(),vec![doc! {"$set":{
            "expires_at":{"$cond":[{"$eq":["$lago_acked",true]},bson::DateTime::from_chrono(Utc::now()+Duration::days(30)),"$$REMOVE"]},
            "llm_retention_hold":"$$REMOVE"
        }}]).session(&mut *session).await?;
        meters
            .update_one(
                doc! {"_id":&primary.id},
                doc! {"$set":{"llm_detail_pending":false},"$unset":{"llm_detail_retry_at":""}},
            )
            .session(&mut *session)
            .await?;
    } else {
        meters
            .update_many(
                filter,
                doc! {"$set":{"llm_retention_hold":true},"$unset":{"expires_at":""}},
            )
            .session(&mut *session)
            .await?;
    }
    Ok(())
}

pub async fn recover(db: &Database) -> AppResult<u64> {
    let rows: Vec<UsageMeterRow> = db.collection::<UsageMeterRow>(METERS)
        .find(doc! {"llm_detail_pending":true, "$or":[{"llm_detail_retry_at":{"$exists":false}}, {"llm_detail_retry_at":{"$lte":bson::DateTime::from_chrono(Utc::now())}}]})
        .sort(doc! {"llm_detail_retry_at":1,"_id":1}).limit(100).await?.try_collect().await?;
    let mut count = 0;
    for row in rows {
        match materialize(db, &row).await {
            Ok(()) => count += 1,
            Err(_) => {
                tracing::warn!(invocation_id=%row.id,"LLM invocation detail retained for retry")
            }
        }
        db.collection::<UsageMeterRow>(METERS).update_one(doc! {"_id":&row.id,"llm_detail_pending":true},doc! {"$set":{"llm_detail_retry_at":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(30))}}).await?;
    }
    Ok(count)
}

pub fn spawn_worker(db: Database) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if recover(&db).await.is_err() {
                tracing::warn!("LLM invocation recovery will retry");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        service_billing::{BillingMetric, PlatformUsage},
        usage_meter::{CredentialClass, UsageFunding},
    };
    use crate::services::billing::{BillingIngress, BillingRouteContext, NodeIntent, meter};
    use crate::services::org_service::OwnerAccess;
    use uuid::Uuid;

    async fn primary(db: &Database) -> (meter::MeteredProxyContext, UsageMeterRow) {
        let owner = Uuid::new_v4().to_string();
        let ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
            Uuid::new_v4().to_string(),
            owner.clone(),
            owner,
            None,
            Some("connection".into()),
            Some("catalog".into()),
            Some("custom".into()),
            NodeIntent::Direct,
            "bearer".into(),
            CredentialClass::UserOwned,
            BillingMetric::Tokens,
            None,
            false,
        )
        .with_platform_metering(false);
        let metered = meter::open(db, &ctx, None).await.unwrap();
        meter::mark_forwarded(db, &metered).await.unwrap();
        let observation = crate::services::llm_observation::extract(&serde_json::json!({"usage":{
            "prompt_tokens":100,"completion_tokens":20,"total_tokens":120,
            "prompt_tokens_details":{"cached_tokens":30,"cache_write_tokens":10}}}))
        .unwrap();
        let usage = PlatformUsage {
            tokens: 120,
            llm_observation: Some(observation),
            ..Default::default()
        };
        let rows = meter::persist_settlement_intent(db, &metered, usage, None, None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        (metered, rows[0].clone())
    }

    async fn raw(db: &Database, id: &str) -> bson::Document {
        db.collection::<bson::Document>(METERS)
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn dispatch_capture_survives_missing_completion_and_first_intent_replaces_it() {
        use crate::models::llm_usage::{UsageOutcome, UsageSource};
        let db = crate::test_utils::connect_transaction_test_database("llm_dispatch_intent").await;
        let owner = Uuid::new_v4().to_string();
        let mut ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
            Uuid::new_v4().to_string(),
            owner.clone(),
            owner.clone(),
            None,
            None,
            None,
            None,
            NodeIntent::Direct,
            "bearer".into(),
            CredentialClass::UserOwned,
            BillingMetric::Tokens,
            None,
            false,
        )
        .with_platform_metering(false);
        ctx.platform_components
            .push(crate::models::service_billing::ResaleSpec {
                metric: BillingMetric::InputTokens,
                lago_metric_code: "input".into(),
            });
        let metered = meter::open(&db, &ctx, None).await.unwrap();
        meter::mark_forwarded(&db, &metered).await.unwrap();
        recover(&db).await.unwrap();
        let rows: Vec<UsageMeterRow> = db
            .collection::<UsageMeterRow>(METERS)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let primary = rows.iter().find(|r| r.llm_detail_pending).unwrap();
        assert_eq!(
            rows.iter().filter(|r| r.llm_observation.is_some()).count(),
            1
        );
        let pending = detail(&db, &owner, None, &primary.id).await.unwrap();
        assert!(!pending.settlement_complete);
        assert_eq!(pending.observation.source, UsageSource::Unknown);
        assert!(
            pending
                .charges
                .iter()
                .all(|c| c.quantity.is_none() && !c.settled)
        );

        let mut guard = meter::UnsettledObservationGuard::new(&db, &metered);
        guard.outcome = UsageOutcome::Failed;
        guard.observation = crate::services::llm_observation::extract(
            &serde_json::json!({"usage":{"input_tokens":10,"output_tokens":2}}),
        );
        drop(guard);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if raw(&db, &primary.id)
                    .await
                    .get_document("llm_observation")
                    .unwrap()
                    .get_str("outcome")
                    == Ok("failed")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        meter::observe_unsettled(
            &db,
            &metered,
            &PlatformUsage {
                llm_observation: Some(crate::services::llm_observation::missing(
                    UsageOutcome::Unknown,
                )),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let current = raw(&db, &primary.id).await;
        let preserved = current.get_document("llm_observation").unwrap();
        assert_eq!(preserved.get_str("outcome"), Ok("failed"));
        assert_eq!(preserved.get_i64("reported_input_tokens"), Ok(10));
        assert!(
            current
                .get("quantity")
                .is_none_or(|v| v == &bson::Bson::Null)
        );
        assert_eq!(current.get_bool("released"), Ok(false));
        let usage = crate::services::llm_usage_service::platform_usage(
            crate::services::llm_usage_service::extract_reported_usage(
                &serde_json::json!({"usage":{"input_tokens":20,"output_tokens":4}}),
            )
            .as_ref(),
            0,
            false,
        );
        meter::settle(&db, &metered, usage, None, None)
            .await
            .unwrap();
        let final_detail = detail(&db, &owner, None, &primary.id).await.unwrap();
        assert!(final_detail.settlement_complete);
        assert_eq!(final_detail.observation.reported_input_tokens, Some(20));
        assert_eq!(final_detail.observation.outcome, UsageOutcome::Failed);
        materialize(&db, primary).await.unwrap();
        assert!(
            !raw(&db, &primary.id)
                .await
                .contains_key("llm_retention_hold")
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn detail_failure_deferred_then_recovered_primary_only_idempotent() {
        let db = crate::test_utils::connect_transaction_test_database("llm_detail_fault").await;
        ensure_indexes(&db).await.unwrap();
        // A real server-side validator failure exercises the storage boundary.
        db.run_command(doc!{"create":COLLECTION_NAME,"validator":{"reject_detail":{"$exists":true}},"validationLevel":"strict"}).await.err();
        // ensure_indexes created the collection; collMod applies the validator.
        db.run_command(doc!{"collMod":COLLECTION_NAME,"validator":{"reject_detail":{"$exists":true}},"validationLevel":"strict"}).await.unwrap();
        let (metered, row) = primary(&db).await;
        meter::settle_persisted(&db, vec![row.clone()])
            .await
            .unwrap();
        let current = raw(&db, &row.id).await;
        assert_eq!(current.get_bool("llm_detail_pending"), Ok(true));
        assert_eq!(current.get_bool("llm_retention_hold"), Ok(true));
        assert!(current.get_document("llm_observation").is_ok());
        assert_eq!(
            db.collection::<bson::Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.run_command(doc! {"collMod":COLLECTION_NAME,"validator":{}})
            .await
            .unwrap();
        assert_eq!(recover(&db).await.unwrap(), 1);
        let complete = detail(&db, &row.actor_user_id, None, &row.id)
            .await
            .unwrap();
        assert!(complete.settlement_complete);
        assert_eq!(complete.charges.len(), 1);
        assert_eq!(complete.observation.input_tokens, Some(60));
        // A stale snapshot, an idempotent settle replay, and recovery cannot re-hold.
        materialize(&db, &row).await.unwrap();
        meter::settle(
            &db,
            &metered,
            PlatformUsage {
                tokens: 999,
                ..Default::default()
            },
            None,
            None,
        )
        .await
        .unwrap();
        recover(&db).await.unwrap();
        let current = raw(&db, &row.id).await;
        assert_eq!(current.get_bool("llm_detail_pending"), Ok(false));
        assert!(!current.contains_key("llm_retention_hold"));
        assert_eq!(current.get_i64("quantity"), Ok(120));
        assert_eq!(
            db.collection::<bson::Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn funding_must_settle_and_flush_scopes_protect_sources_ack_race() {
        let db = crate::test_utils::connect_transaction_test_database("llm_detail_funding").await;
        let (_, mut row) = primary(&db).await;
        row.released = true;
        row.wallet_id = Some("wallet".into());
        row.funding = Some(UsageFunding::default());
        row.lago_acked = true;
        row.expires_at = Some(Utc::now() + Duration::days(1));
        db.collection::<UsageMeterRow>(METERS)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        let mut component = row.clone();
        component.id = Uuid::new_v4().to_string();
        component.transaction_id.push_str(":component");
        component.llm_observation = None;
        component.llm_context = None;
        component.llm_detail_pending = false;
        db.collection::<UsageMeterRow>(METERS)
            .insert_one(&component)
            .await
            .unwrap();
        let mut other = component.clone();
        other.id = Uuid::new_v4().to_string();
        other.flush_seq = Some(1);
        db.collection::<UsageMeterRow>(METERS)
            .insert_one(&other)
            .await
            .unwrap();
        materialize(&db, &row).await.unwrap();
        let incomplete = detail(&db, &row.actor_user_id, None, &row.id)
            .await
            .unwrap();
        assert!(!incomplete.settlement_complete);
        assert_eq!(incomplete.charges.len(), 2);
        for id in [&row.id, &component.id] {
            let held = raw(&db, id).await;
            assert_eq!(held.get_bool("llm_retention_hold"), Ok(true));
            assert!(!held.contains_key("expires_at"));
        }
        assert!(raw(&db, &other.id).await.contains_key("expires_at"));
        db.collection::<bson::Document>(METERS)
            .update_many(
                doc! {"flush_seq":bson::Bson::Null},
                doc! {"$set":{"funding.settled":true,"lago_acked":false}},
            )
            .await
            .unwrap();
        // Both writers touch the same source document. Either serialization
        // order must leave an ACKed, unheld source with a normal expiration.
        let (a, b, c) = tokio::join!(
            materialize(&db, &row),
            super::super::reconcile::mark_lago_acked(&db, &row.id),
            materialize(&db, &row)
        );
        a.unwrap();
        b.unwrap();
        c.unwrap();
        let current = raw(&db, &row.id).await;
        assert_eq!(current.get_bool("llm_detail_pending"), Ok(false));
        assert!(!current.contains_key("llm_retention_hold"));
        assert_eq!(current.get_bool("lago_acked"), Ok(true));
        assert!(current.get_datetime("expires_at").is_ok());
        assert!(
            detail(&db, &row.actor_user_id, None, &row.id)
                .await
                .unwrap()
                .settlement_complete
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn native_realtime_unknown_partition_survives_persistence() {
        let db = crate::test_utils::connect_transaction_test_database("llm_native_realtime").await;
        let owner = Uuid::new_v4().to_string();
        let ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
            Uuid::new_v4().to_string(),
            owner.clone(),
            owner.clone(),
            None,
            None,
            None,
            Some("llm-openai".into()),
            NodeIntent::Direct,
            "bearer".into(),
            CredentialClass::UserOwned,
            BillingMetric::Tokens,
            None,
            false,
        )
        .with_native_provider_origin("https://api.openai.com/v1")
        .with_platform_metering(false);
        let meter = meter::open(&db, &ctx, None).await.unwrap();
        meter::mark_forwarded(&db, &meter).await.unwrap();
        let mut collector =
            crate::services::llm_usage_service::RealtimeLlmUsageCollector::new(true);
        for (id, details) in [
            (
                "one",
                serde_json::json!({"cached_tokens":2,"cache_write_tokens":3}),
            ),
            ("two", serde_json::json!({"cached_tokens":2})),
        ] {
            collector.observe_downstream_text(&serde_json::json!({"type":"response.done","response":{"id":id,"usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12,"input_token_details":details}}}).to_string());
        }
        let summary = collector.finalize();
        let usage = crate::services::llm_usage_service::platform_usage(
            summary.reported_usage.as_ref(),
            0,
            false,
        );
        meter::settle(&db, &meter, usage, None, None).await.unwrap();
        let row = db
            .collection::<UsageMeterRow>(METERS)
            .find_one(doc! {"billing_owner_id":&owner})
            .await
            .unwrap()
            .unwrap();
        let detail = detail(&db, &owner, None, &row.id).await.unwrap();
        assert_eq!(detail.observation.cache_write_tokens, None);
        assert_eq!(detail.observation.input_tokens, None);
        assert_eq!(
            detail.observation.completeness,
            crate::models::llm_usage::UsageCompleteness::Partial
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn old_snapshot_rewrite_preserves_independent_observation_and_old_rows_read() {
        let db = crate::test_utils::connect_transaction_test_database("llm_old_bson").await;
        let (_, row) = primary(&db).await;
        // Emulate old typed PlatformUsage dropping unknown fields when reserialized.
        #[derive(serde::Deserialize, serde::Serialize)]
        struct OldUsage {
            tokens: i64,
        }
        let usage = bson::to_bson(&PlatformUsage {
            tokens: 120,
            llm_observation: row.llm_observation.clone(),
            ..Default::default()
        })
        .unwrap();
        let old: OldUsage = bson::from_bson(usage).unwrap();
        db.collection::<bson::Document>(METERS)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"pending_platform_usage":bson::to_bson(&old).unwrap()}},
            )
            .await
            .unwrap();
        db.collection::<bson::Document>(METERS)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$unset":{"pending_platform_usage":""},"$set":{"released":true}},
            )
            .await
            .unwrap();
        recover(&db).await.unwrap();
        assert_eq!(
            detail(&db, &row.actor_user_id, None, &row.id)
                .await
                .unwrap()
                .observation
                .input_tokens,
            Some(60)
        );
        let mut old = bson::to_document(&row).unwrap();
        for key in ["llm_context", "llm_observation", "llm_detail_pending"] {
            old.remove(key);
        }
        let decoded: UsageMeterRow = bson::from_document(old).unwrap();
        assert!(!decoded.llm_detail_pending);
        assert!(decoded.llm_observation.is_none());
        assert_eq!(decoded.transaction_id, row.transaction_id);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn custom_and_vertex_execution_reports_survive_and_models_do_not_add_missing() {
        let db = crate::test_utils::connect_transaction_test_database("llm_custom_paths").await;
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.inference = Some(crate::models::downstream_service::ServiceInference {
            wire_protocol:
                crate::models::downstream_service::InferenceWireProtocol::OpenaiCompletions,
            model_list: true,
            realtime: false,
            voice: None,
        });
        for path in [
            "/generate",
            "/v1/models/custom:rawPredict",
            "/v1/models/custom:streamRawPredict",
            "/v1/models/custom:predict",
            "/v1/models/gemini:streamGenerateContent",
            "/v1/models",
        ] {
            let owner = Uuid::new_v4().to_string();
            let ctx = BillingRouteContext::new(
                BillingIngress::Proxy,
                Uuid::new_v4().to_string(),
                owner.clone(),
                owner.clone(),
                None,
                None,
                None,
                None,
                NodeIntent::Direct,
                "bearer".into(),
                CredentialClass::UserOwned,
                BillingMetric::Requests,
                None,
                false,
            )
            .with_inference_capture(&service, path)
            .with_platform_metering(false);
            let metered = meter::open(&db, &ctx, None).await.unwrap();
            meter::mark_forwarded(&db, &metered).await.unwrap();
            let usage = if path == "/v1/models" {
                PlatformUsage::default()
            } else if path.ends_with(":streamGenerateContent") {
                let bytes = serde_json::to_vec(&serde_json::json!([
                    {"usageMetadata":{"promptTokenCount":11,"candidatesTokenCount":3,
                      "thoughtsTokenCount":2,"cachedContentTokenCount":4,"totalTokenCount":16},
                     "candidates":[{"finishReason":"STOP"}]}
                ]))
                .unwrap();
                crate::services::llm_usage_service::platform_usage(
                    crate::services::llm_usage_service::usage_from_body(&bytes, path, true)
                        .as_ref(),
                    bytes.len() as i64,
                    false,
                )
            } else {
                crate::services::llm_usage_service::platform_usage(
                    crate::services::llm_usage_service::extract_reported_usage(
                        &serde_json::json!({"usage":{"prompt_tokens":11,"completion_tokens":3}}),
                    )
                    .as_ref(),
                    0,
                    false,
                )
            };
            meter::settle(&db, &metered, usage, None, None)
                .await
                .unwrap();
            let count = db
                .collection::<LlmInvocation>(COLLECTION_NAME)
                .count_documents(doc! {"billing_owner_id":&owner})
                .await
                .unwrap();
            assert_eq!(count, u64::from(path != "/v1/models"), "{path}");
            if path.ends_with(":streamGenerateContent") {
                let retained = db
                    .collection::<LlmInvocation>(COLLECTION_NAME)
                    .find_one(doc! {"billing_owner_id":&owner})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(retained.observation.input_tokens, Some(7));
                assert_eq!(retained.observation.output_tokens, Some(5));
                assert_eq!(retained.observation.cache_read_tokens, Some(4));
                assert_eq!(retained.charges[0].quantity, Some(1));
            }
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn owner_acl_retention_query_bounds_and_pagination() {
        let db = crate::test_utils::connect_transaction_test_database("llm_detail_reads").await;
        let (_, row) = primary(&db).await;
        db.collection::<bson::Document>(METERS)
            .update_one(doc! {"_id":&row.id}, doc! {"$set":{"released":true}})
            .await
            .unwrap();
        recover(&db).await.unwrap();
        let query = || InvocationQuery {
            owner_id: None,
            from: None,
            to: None,
            limit: Some(1),
            cursor: None,
            conversation_id: None,
            turn_id: None,
        };
        assert_eq!(
            list(&db, &row.actor_user_id, query())
                .await
                .unwrap()
                .0
                .len(),
            1
        );
        assert!(
            detail(
                &db,
                &Uuid::new_v4().to_string(),
                Some(&row.billing_owner_id),
                &row.id
            )
            .await
            .is_err()
        );
        assert!(
            detail(&db, &Uuid::new_v4().to_string(), None, &row.id)
                .await
                .is_err()
        );
        for access in [
            OwnerAccess::AsOrgAdmin {
                membership_id: "membership".into(),
                org_user_id: "org-user".into(),
                allowed_service_ids: Some(vec!["other".into()]),
            },
            OwnerAccess::AsOrgMember {
                membership_id: "membership".into(),
                org_user_id: "org-user".into(),
                role: crate::models::org_membership::OrgRole::Member,
                allowed_service_ids: Some(vec!["other".into()]),
            },
        ] {
            let filter = access_filter(&row.billing_owner_id, &access).unwrap();
            assert_eq!(
                db.collection::<LlmInvocation>(COLLECTION_NAME)
                    .count_documents(filter)
                    .await
                    .unwrap(),
                0
            );
        }
        let mut second = detail(&db, &row.actor_user_id, None, &row.id)
            .await
            .unwrap();
        second.id = Uuid::new_v4().to_string();
        db.collection::<LlmInvocation>(COLLECTION_NAME)
            .insert_one(&second)
            .await
            .unwrap();
        let (first, next) = list(&db, &row.actor_user_id, query()).await.unwrap();
        assert_eq!(first.len(), 1);
        let mut page = query();
        page.cursor = Some(next.unwrap());
        let (last, next) = list(&db, &row.actor_user_id, page).await.unwrap();
        assert_eq!(last.len(), 1);
        assert_ne!(first[0].id, last[0].id);
        assert!(next.is_none());
        // Exercise the actual resolver for scoped admins/members/viewers.
        let org = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &org,
                crate::models::user::UserType::Org,
            ))
            .await
            .unwrap();
        second.id = Uuid::new_v4().to_string();
        second.billing_owner_id = org.clone();
        db.collection::<LlmInvocation>(COLLECTION_NAME)
            .insert_one(&second)
            .await
            .unwrap();
        use crate::models::org_membership::{MemberScopeSource, OrgMembership, OrgRole};
        for role in [OrgRole::Admin, OrgRole::Member, OrgRole::Viewer] {
            let actor = Uuid::new_v4().to_string();
            let membership = OrgMembership {
                id: Uuid::new_v4().to_string(),
                org_user_id: org.clone(),
                member_user_id: actor.clone(),
                role,
                scope_source: MemberScopeSource::Override,
                allowed_service_ids: Some(vec!["other".into()]),
                created_at: Utc::now(),
                revoked_at: None,
            };
            let memberships =
                db.collection::<OrgMembership>(crate::models::org_membership::COLLECTION_NAME);
            memberships.insert_one(&membership).await.unwrap();
            assert!(detail(&db, &actor, Some(&org), &second.id).await.is_err());
            memberships
                .update_one(
                    doc! {"_id":&membership.id},
                    doc! {"$set":{"allowed_service_ids":["connection"]}},
                )
                .await
                .unwrap();
            assert!(detail(&db, &actor, Some(&org), &second.id).await.is_ok());
        }
        let mut bad = query();
        bad.to = Some(chrono::DateTime::<Utc>::MIN_UTC);
        assert!(list(&db, &row.actor_user_id, bad).await.is_err());
        let mut bad = query();
        bad.limit = Some(101);
        assert!(list(&db, &row.actor_user_id, bad).await.is_err());
        db.collection::<bson::Document>(COLLECTION_NAME).update_many(doc!{"billing_owner_id":&row.billing_owner_id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
        assert!(
            detail(&db, &row.actor_user_id, None, &row.id)
                .await
                .is_err()
        );
        assert!(
            list(&db, &row.actor_user_id, query())
                .await
                .unwrap()
                .0
                .is_empty()
        );
        db.drop().await.unwrap();
    }
}
