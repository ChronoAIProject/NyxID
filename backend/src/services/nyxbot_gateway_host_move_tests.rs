use super::*;
use crate::{
    models::{
        audit_log::{AuditLog, COLLECTION_NAME as AUDIT},
        nyxbot_gateway_release::ReleaseReason,
    },
    test_utils::{connect_transaction_test_database, test_app_state},
};
use uuid::Uuid;

const OWNER: &str = "11111111-1111-4111-8111-111111111111";
const OLD: &str = "https://bot.chrono-ai.fun/event-gateway/v1";
const CURRENT: &str = "https://cma.nyxid.ai/event-gateway/v1";
const REMOTE: &str = "cmaeg1.ch.1.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

async fn fixture() -> (AppState, Catalog) {
    let state = test_app_state(connect_transaction_test_database("gateway_host_move").await);
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = "cmaeg".into();
    service.base_url = CURRENT.into();
    service.requires_user_credential = false;
    let catalog = Catalog {
        id: service.id.clone(),
        raw_base: service.base_url.clone(),
        base: CURRENT.into(),
    };
    state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    (state, catalog)
}

async fn route(state: &AppState, transport: &str, status: &str, org: bool) -> Route {
    let row = Route {
        id: Uuid::new_v4().to_string(),
        user_id: OWNER.into(),
        bot_owner_id: org.then(|| Uuid::new_v4().to_string()),
        route_api_key_id: Uuid::new_v4().to_string(),
        transport: transport.into(),
        status: status.into(),
    };
    state
        .db
        .collection::<Document>(CHANNELS)
        .insert_one(doc! {
            "_id":&row.id,"user_id":&row.user_id,"bot_owner_id":&row.bot_owner_id,
            "route_api_key_id":&row.route_api_key_id,"transport":transport,"status":status,
        })
        .await
        .unwrap();
    // The mutation service needs only these authoritative fields; unrelated
    // serialized key fields do not influence destination fencing.
    state
        .db
        .collection::<Document>(KEYS)
        .insert_one(doc! {
            "_id":&row.route_api_key_id,"user_id":row.bot_owner_id.as_deref().unwrap_or(OWNER),
            "callback_url":format!("{OLD}/callbacks/{REMOTE}"),"is_active":true,
        })
        .await
        .unwrap();
    row
}

async fn callback(state: &AppState, row: &Route) -> String {
    route_callback(
        &state.db,
        &row.route_api_key_id,
        row.bot_owner_id.as_deref().unwrap_or(OWNER),
    )
    .await
    .unwrap()
    .unwrap()
}

#[test]
fn callback_shapes_rebase_only_a_strict_cma_suffix_onto_the_catalog() {
    let expected = format!("{CURRENT}/callbacks/{REMOTE}");
    for base in [OLD, "https://bot-gateway.chrono-ai.fun/v1"] {
        assert_eq!(
            rebase_callback(&format!("{base}/callbacks/{REMOTE}"), CURRENT).as_deref(),
            Some(expected.as_str())
        );
    }
    assert_eq!(rebase_callback(&expected, CURRENT), None);
    assert_eq!(rebase_callback(&expected, &format!("{CURRENT}/")), None);
    for stored in [
        format!("http://old.example/callbacks/{REMOTE}"),
        format!("https://old.example/callbacks/{REMOTE}?token=secret"),
        format!("https://old.example/callbacks/{REMOTE}#fragment"),
        format!("https://user:secret@old.example/callbacks/{REMOTE}"),
        format!("https://@old.example/callbacks/{REMOTE}"),
        format!("https://old.example/a/../callbacks/{REMOTE}"),
        format!("https://old.example/./callbacks/{REMOTE}"),
        format!("https://old.example/%2e%2e/callbacks/{REMOTE}"),
        format!("https://old.example/%63allbacks/{REMOTE}"),
        format!("https://old.example//callbacks/{REMOTE}"),
        format!("https://old.example/callbacks/{REMOTE}%2fextra"),
        format!("https://old.example\\callbacks/{REMOTE}"),
        format!("https://old.example/not-callbacks/{REMOTE}"),
        format!("https://old.example/callbacks/{REMOTE}/"),
        "https://old.example/callbacks/ordinary-user-callback".into(),
        "https://old.example/callbacks/cmaeg1.".into(),
    ] {
        assert!(rebase_callback(&stored, CURRENT).is_none());
    }
    for base in [
        "https://user@new.example/v1",
        "https://new.example/../v1",
        "https://new.example/v1?x=1",
        "https://new.example/%2e",
        "http://public.example/v1",
    ] {
        assert!(rebase_callback(&format!("{OLD}/callbacks/{REMOTE}"), base).is_none());
    }
}

#[test]
fn actual_cma_channel_format_accepts_exported_ids_and_enforces_wire_bounds() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/fixtures/platform-entry-cma/discord-ready-record.json"
    ))
    .unwrap();
    let id = fixture["channel_id"].as_str().unwrap();
    assert!(id.len() > 200);
    assert!(valid_callback_id(id));
    assert_eq!(
        rebase_callback(&format!("{OLD}/callbacks/{id}"), CURRENT),
        Some(format!("{CURRENT}/callbacks/{id}"))
    );
    let prefix = "cmaeg1.ch.255.";
    assert!(valid_callback_id(&format!(
        "{prefix}{}",
        "A".repeat(4096 - prefix.len())
    )));
    for id in [
        format!("{prefix}{}", "A".repeat(4097 - prefix.len())),
        REMOTE.replace("ch.1.", "ev.1."),
        REMOTE.replace("ch.1.", "ch.01."),
        REMOTE.replace("ch.1.", "ch.256."),
        REMOTE.replace("ch.1.", "ch.+1."),
        format!("{REMOTE}="),
        format!("{REMOTE}.extra"),
        "cmaeg1.ch.1.short".into(),
        "cmaeg1.ch.1.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB".into(), // noncanonical padding bits
    ] {
        assert!(!valid_callback_id(&id));
    }
}

#[tokio::test]
async fn only_referenced_gateway_keys_change_once_and_audit_has_hosts_only() {
    let (state, _) = fixture().await;
    let personal = route(&state, "gateway", "active", false).await;
    let org = route(&state, "gateway", "failed", true).await;
    let direct = route(&state, "direct", "active", false).await;
    let deleted = route(&state, "gateway", "deleted", false).await;
    let foreign = route(&state, "gateway", "active", false).await;
    state
        .db
        .collection::<Document>(KEYS)
        .update_one(
            doc! {"_id":&foreign.route_api_key_id},
            doc! {"$set":{"user_id":"another-owner"}},
        )
        .await
        .unwrap();
    let unrelated = Uuid::new_v4().to_string();
    let old = format!("{OLD}/callbacks/{REMOTE}");
    state
        .db
        .collection::<Document>(KEYS)
        .insert_one(doc! {"_id":&unrelated,"user_id":OWNER,"callback_url":&old})
        .await
        .unwrap();
    let mut reconciler = Reconciler::default();
    assert_eq!(
        reconciler.tick(&state).await.unwrap(),
        Some(Counts {
            nyxbot: 2,
            ..Default::default()
        })
    );
    let next = format!("{CURRENT}/callbacks/{REMOTE}");
    assert_eq!(callback(&state, &personal).await, next);
    assert_eq!(callback(&state, &org).await, next);
    assert_eq!(callback(&state, &direct).await, old);
    assert_eq!(callback(&state, &deleted).await, old);
    assert_eq!(
        route_callback(&state.db, &foreign.route_api_key_id, "another-owner")
            .await
            .unwrap(),
        Some(old.clone())
    );
    assert_eq!(
        route_callback(&state.db, &unrelated, OWNER).await.unwrap(),
        Some(old)
    );
    assert_eq!(reconciler.tick(&state).await.unwrap(), None);
    assert_eq!(
        Reconciler::default().tick(&state).await.unwrap(),
        Some(Counts::default())
    );
    let logs: Vec<AuditLog> = state
        .db
        .collection::<AuditLog>(AUDIT)
        .find(doc! {"event_type":"nyxbot_gateway_callback_rebased"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(logs.len(), 2);
    for log in logs {
        let id = log.event_data.as_ref().unwrap()["row_id"].as_str().unwrap();
        assert!([personal.id.as_str(), org.id.as_str()].contains(&id));
        assert_eq!(
            log.event_data,
            Some(
                json!({"row_kind":"nyxbot","row_id":id,"old_host":"bot.chrono-ai.fun","new_host":"cma.nyxid.ai"})
            )
        );
        assert!(log.seq.is_some());
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn callback_route_reference_and_catalog_compare_and_swap_refuse_stale_scans() {
    let (state, catalog) = fixture().await;
    let row = route(&state, "gateway", "active", false).await;
    let old = callback(&state, &row).await;
    let next = rebase_callback(&old, &catalog.base).unwrap();
    let concurrent = "https://owner.example/new-callback";
    state
        .db
        .collection::<Document>(KEYS)
        .update_one(
            doc! {"_id":&row.route_api_key_id},
            doc! {"$set":{"callback_url":concurrent}},
        )
        .await
        .unwrap();
    assert!(
        !rebase_route(&state, &catalog, &row, &old, &next)
            .await
            .unwrap()
    );
    assert_eq!(callback(&state, &row).await, concurrent);
    state
        .db
        .collection::<Document>(KEYS)
        .update_one(
            doc! {"_id":&row.route_api_key_id},
            doc! {"$set":{"callback_url":&old}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<Document>(CHANNELS)
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"transport":"direct"}})
        .await
        .unwrap();
    assert!(matches!(
        rebase_route(&state, &catalog, &row, &old, &next).await,
        Err(AppError::Conflict(_))
    ));
    state
        .db
        .collection::<Document>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"transport":"gateway", "route_api_key_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    assert!(matches!(
        rebase_route(&state, &catalog, &row, &old, &next).await,
        Err(AppError::Conflict(_))
    ));
    state
        .db
        .collection::<Document>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"route_api_key_id":&row.route_api_key_id}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&catalog.id},
            doc! {"$set":{"base_url":"https://newer.example/v1"}},
        )
        .await
        .unwrap();
    assert!(matches!(
        rebase_route(&state, &catalog, &row, &old, &next).await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(callback(&state, &row).await, old);
    assert_eq!(
        state
            .db
            .collection::<Document>(AUDIT)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(
        !state
            .db
            .collection::<Document>(CHANNELS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap()
            .contains_key("gateway_host_revision")
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn absent_catalog_noop_and_completed_cache_detects_moves_and_hourly_backstop() {
    let (state, catalog) = fixture().await;
    state
        .db
        .collection::<Document>(SERVICES)
        .delete_one(doc! {"_id":&catalog.id})
        .await
        .unwrap();
    let row = route(&state, "gateway", "active", false).await;
    let old = callback(&state, &row).await;
    let mut worker = Reconciler::default();
    assert_eq!(worker.tick(&state).await.unwrap(), None);
    assert_eq!(callback(&state, &row).await, old);
    state.db.drop().await.unwrap();

    let (state, catalog) = fixture().await;
    assert_eq!(worker.tick(&state).await.unwrap(), Some(Counts::default()));
    let row = route(&state, "gateway", "active", false).await;
    assert_eq!(worker.tick(&state).await.unwrap(), None);
    assert_eq!(callback(&state, &row).await, old);
    worker.completed.as_mut().unwrap().2 = Instant::now() - BACKSTOP;
    assert_eq!(worker.tick(&state).await.unwrap().unwrap().nyxbot, 1);
    let newer = "https://newer.example/v2";
    state
        .db
        .collection::<Document>(SERVICES)
        .update_one(doc! {"_id":&catalog.id}, doc! {"$set":{"base_url":newer}})
        .await
        .unwrap();
    assert_eq!(worker.tick(&state).await.unwrap().unwrap().nyxbot, 1);
    assert_eq!(
        callback(&state, &row).await,
        format!("{newer}/callbacks/{REMOTE}")
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pagination_and_two_replica_reconciliation_are_idempotent() {
    let (state, _) = fixture().await;
    // Sparse pages must still advance across unrelated channels.
    let docs: Vec<_> = (0..BATCH + 1)
        .map(|i| doc! {"_id":format!("0000-{i:03}"),"user_id":OWNER,"transport":"direct"})
        .collect();
    state
        .db
        .collection::<Document>(CHANNELS)
        .insert_many(docs)
        .await
        .unwrap();
    let row = route(&state, "gateway", "active", false).await;
    let (a, b) = tokio::join!(
        async { Reconciler::default().tick(&state).await.unwrap() },
        async { Reconciler::default().tick(&state).await.unwrap() }
    );
    assert_eq!(a.unwrap().nyxbot + b.unwrap().nyxbot, 1);
    assert_eq!(
        callback(&state, &row).await,
        format!("{CURRENT}/callbacks/{REMOTE}")
    );
    assert_eq!(
        state
            .db
            .collection::<Document>(AUDIT)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    state.db.drop().await.unwrap();
}

fn release(state: &AppState, catalog: &Catalog) -> NyxbotGatewayRelease {
    let now = chrono::Utc::now();
    NyxbotGatewayRelease {
        gateway_source: None,
        id: Uuid::new_v4().to_string(),
        schema_version: 1,
        owner_user_id: OWNER.into(),
        channel_id: Uuid::new_v4().to_string(),
        channel_bot_id: Uuid::new_v4().to_string(),
        bot_owner_id: OWNER.into(),
        platform: "telegram".into(),
        gateway_service_id: catalog.id.clone(),
        gateway_base_url: OLD.into(),
        creator_issuer: state.config.jwt_issuer.clone(),
        record_id: Some("retained-record".into()),
        gateway_channel_id: Some(REMOTE.into()),
        binding_id: "retained-binding".into(),
        agent_api_key_id: Uuid::new_v4().to_string(),
        route_api_key_id: Uuid::new_v4().to_string(),
        expected_version: Some(3),
        state: ReleaseState::ReviewRequired,
        reason: ReleaseReason::Disconnect,
        dispatch_started_at: None,
        provisional_expires_at: None,
        due_at: Some(now),
        next_attempt_at: None,
        attempt_count: 1,
        claim_id: None,
        lease_until: None,
        revision: 1,
        last_error: Some("gateway_destination_changed".into()),
        observed_version: None,
        resolution: None,
        reviewed_by: None,
        settled_at: None,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn release_move_preserves_service_issuer_version_and_lease_fences() {
    let (state, catalog) = fixture().await;
    let good = release(&state, &catalog);
    let mut foreign = release(&state, &catalog);
    foreign.gateway_service_id = Uuid::new_v4().to_string();
    let mut issuer = release(&state, &catalog);
    issuer.creator_issuer = "https://foreign-issuer.example".into();
    let mut leased = release(&state, &catalog);
    leased.lease_until = Some(chrono::Utc::now() + chrono::Duration::minutes(2));
    leased.claim_id = Some("active-claim".into());
    let mut other_review = release(&state, &catalog);
    other_review.last_error = Some("remote_changed".into());
    state
        .db
        .collection::<NyxbotGatewayRelease>(RELEASES)
        .insert_many([&good, &foreign, &issuer, &leased, &other_review])
        .await
        .unwrap();
    assert!(!releases::destination(&state, &good).await.unwrap());
    let counts = Reconciler::default().tick(&state).await.unwrap().unwrap();
    assert_eq!(
        counts,
        Counts {
            releases: 2,
            releases_review: 2,
            deferred: 1,
            ..Default::default()
        }
    );
    // The transaction also checks MongoDB's clock, even if a replica's
    // preliminary lease check were early because of local clock skew.
    assert!(matches!(
        rebase_release(&state, &catalog, &leased).await,
        Err(AppError::Conflict(_))
    ));
    let live = releases::get(&state.db, &good.id).await.unwrap();
    assert!(releases::destination(&state, &live).await.unwrap());
    assert_eq!(live.state, ReleaseState::Due);
    assert_eq!(live.expected_version, good.expected_version);
    assert_eq!(live.gateway_channel_id, good.gateway_channel_id);
    assert_eq!(live.record_id, good.record_id);
    for row in [&foreign, &issuer, &leased] {
        let stored = releases::get(&state.db, &row.id).await.unwrap();
        assert_eq!(stored.gateway_base_url, OLD);
        assert!(!releases::destination(&state, &stored).await.unwrap());
    }
    let reviewed = releases::get(&state.db, &other_review.id).await.unwrap();
    assert_eq!(reviewed.state, ReleaseState::ReviewRequired);
    assert_eq!(reviewed.last_error, other_review.last_error);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_whatsapp_is_counted_without_unsafe_activation_or_state_change() {
    let (state, _) = fixture().await;
    for (id, phase, base) in [
        ("1", "gateway", OLD),
        ("2", "awaiting_relay", OLD),
        ("3", "relay", OLD),
        ("4", "gateway", CURRENT),
    ] {
        state.db.collection::<Document>(WHATSAPP).insert_one(doc! {"_id":id,"callback":{"phase":phase,"gateway_channel_id":REMOTE,"gateway_webhook_url":format!("{base}/whatsapp/{REMOTE}")}}).await.unwrap();
    }
    assert_eq!(
        Reconciler::default()
            .tick(&state)
            .await
            .unwrap()
            .unwrap()
            .whatsapp_manual,
        2
    );
    assert_eq!(
        state
            .db
            .collection::<Document>(WHATSAPP)
            .count_documents(
                doc! {"callback.gateway_webhook_url":format!("{OLD}/whatsapp/{REMOTE}")}
            )
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        state
            .db
            .collection::<Document>(AUDIT)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn startup_worker_returns_before_unavailable_database_is_polled() {
    let state = crate::test_utils::test_app_state_no_db().await;
    // On this single-thread runtime a synchronous database wait in spawn_worker
    // would block the caller. Only the returned task may start that work.
    let start = Instant::now();
    let task = releases::spawn_worker(state);
    assert!(start.elapsed() < Duration::from_millis(50));
    assert!(!task.is_finished());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn startup_first_tick_repairs_callbacks_without_waiting_for_sweep_interval() {
    let (state, _) = fixture().await;
    let row = route(&state, "gateway", "active", false).await;
    let task = releases::spawn_worker(state.clone());
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if callback(&state, &row).await == format!("{CURRENT}/callbacks/{REMOTE}") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    result.expect("startup repair must run before the 30-second sweep interval");
    state.db.drop().await.unwrap();
}
