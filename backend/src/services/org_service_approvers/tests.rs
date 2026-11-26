use super::*;
use crate::models::{approval_grant::ApprovalGrant, approval_request::COLLECTION_NAME as REQUESTS};
use crate::services::{approval_service as approvals, audit_service::AuditActor};
use crate::test_utils::{connect_transaction_test_database, test_app_state};

async fn fixture(label: &str) -> (crate::AppState, ApprovalRequest) {
    let db = connect_transaction_test_database(label).await;
    let now = bson::DateTime::now();
    db.collection::<Document>(CONFIGS).insert_one(doc! {"_id":"config","user_id":"org","service_id":"catalog","service_name":"Service","approval_required":true,"approval_mode":"grant","approver_user_ids":["alice","bob"],"approver_revision":3_i64,"min_grant_revision":2_i64,"created_at":now,"updated_at":now}).await.unwrap();
    for actor in ["org", "alice", "bob", "other"] {
        db.collection::<Document>(USERS).insert_one(doc! {"_id":actor,"user_type":if actor=="org" {"org"} else {"person"},"is_active":true}).await.unwrap();
        if actor != "org" {
            db.collection::<Document>(MEMBERS).insert_one(doc! {"_id":actor,"org_user_id":"org","member_user_id":actor,"role":"admin","created_at":now}).await.unwrap();
        }
    }
    for id in ["selected", "sibling"] {
        db.collection::<Document>(SERVICES)
            .insert_one(
                doc! {"_id":id,"user_id":"org","catalog_service_id":"catalog","is_active":true},
            )
            .await
            .unwrap();
    }
    db.collection::<Document>(CHANNELS)
        .insert_one(
            doc! {"_id":"link","user_id":"alice","telegram_enabled":true,"telegram_chat_id":25_i64},
        )
        .await
        .unwrap();
    let request = bson::from_document(doc! {"_id":"approval","user_id":"org","service_id":"catalog","service_name":"Service","service_slug":"service","requester_type":"user","requester_id":"requester","operation_summary":"read","approval_mode":"grant","status":"pending","idempotency_key":"test","expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(5)),"created_at":now,"notify_user_ids":["alice","bob","other"],"from_org_policy":true,"org_policy":{"config_id":"config","revision":3_i64,"connection_id":"selected"}}).unwrap();
    db.collection::<ApprovalRequest>(REQUESTS)
        .insert_one(&request)
        .await
        .unwrap();
    (test_app_state(db), request)
}

fn web(actor: &str) -> approvals::DecisionSource {
    approvals::DecisionSource::Web(AuditActor {
        user_id: actor.into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    })
}

async fn decide(
    state: &crate::AppState,
    card: approvals::DecisionRequest,
    source: approvals::DecisionSource,
) -> AppResult<ApprovalRequest> {
    approvals::process_decision_with_access(
        &state.db,
        &state.config,
        &state.http_client,
        None,
        None,
        card,
        true,
        None,
        None,
        source,
        None,
    )
    .await
}

#[tokio::test]
async fn simultaneous_web_and_private_telegram_decisions_commit_one_grant_and_audit() {
    let (state, r) = fixture("org_approvers_race").await;
    let a = approvals::read_decision_request(&state.db, &r.id)
        .await
        .unwrap();
    let b = approvals::read_decision_request(&state.db, &r.id)
        .await
        .unwrap();
    let (left, right) = tokio::join!(
        decide(&state, a, approvals::DecisionSource::Telegram(25)),
        decide(&state, b, web("bob"))
    );
    assert_ne!(left.is_ok(), right.is_ok(), "{left:?} {right:?}");
    assert!(
        matches!(left.as_ref().err().or(right.as_ref().err()),Some(AppError::Conflict(v)) if v=="already_decided")
    );
    let grants = state
        .db
        .collection::<ApprovalGrant>(crate::models::approval_grant::COLLECTION_NAME);
    assert_eq!(grants.count_documents(doc! {}).await.unwrap(), 1);
    assert_eq!(
        grants.find_one(doc! {}).await.unwrap().unwrap().org_policy,
        r.org_policy.map(|stamp| OrgPolicyStamp {
            connection_id: None,
            ..stamp
        })
    );
    let intents = state
        .db
        .collection::<Document>(crate::models::security_effect_intent::COLLECTION_NAME);
    assert_eq!(intents.count_documents(doc! {"event.event_type":"approval_decision","event.user_id":if left.is_ok() {"alice"} else {"bob"}}).await.unwrap(),1);
}

#[tokio::test]
async fn edit_or_demotion_after_card_read_refuses_decision_and_replay() {
    let (state, r) = fixture("org_approvers_policy_race").await;
    let read = || approvals::read_decision_request(&state.db, &r.id);
    let held = read().await.unwrap();
    state
        .db
        .collection::<Document>(MEMBERS)
        .update_one(doc! {"_id":"bob"}, doc! {"$set":{"role":"member"}})
        .await
        .unwrap();
    assert!(matches!(
        decide(&state, held, web("bob")).await,
        Err(AppError::Forbidden(_))
    ));
    let held = read().await.unwrap();
    state
        .db
        .collection::<Document>(CONFIGS)
        .update_one(
            doc! {"_id":"config"},
            doc! {"$inc":{"approver_revision":1_i64}},
        )
        .await
        .unwrap();
    assert!(matches!(
        decide(&state, held, web("alice")).await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(
        approvals::get_request(&state.db, &r.id)
            .await
            .unwrap()
            .status,
        "pending"
    );
    assert!(!request_live(&state.db, &r).await.unwrap());
}

#[tokio::test]
async fn live_named_recipients_filter_siblings_group_actor_and_ambiguous_links() {
    let (state, mut r) = fixture("org_approvers_recipients").await;
    state
        .db
        .collection::<Document>(MEMBERS)
        .update_one(
            doc! {"_id":"bob"},
            doc! {"$set":{"allowed_service_ids":["sibling"]}},
        )
        .await
        .unwrap();
    assert_eq!(recipients(&state.db, &r).await.unwrap(), ["alice"]);
    for actor in ["bob", "other"] {
        assert!(!visible(&state.db, &r, actor).await.unwrap());
        let filter = history_filter(&state.db, actor, Some("org"), false)
            .await
            .unwrap();
        assert_eq!(
            state
                .db
                .collection::<Document>(REQUESTS)
                .count_documents(filter)
                .await
                .unwrap(),
            0
        );
    }
    state
        .db
        .collection::<Document>(CHANNELS)
        .insert_one(doc! {"_id":"duplicate","user_id":"other","telegram_chat_id":25_i64})
        .await
        .unwrap();
    let card = approvals::read_decision_request(&state.db, &r.id)
        .await
        .unwrap();
    assert!(matches!(
        decide(&state, card, approvals::DecisionSource::Telegram(25)).await,
        Err(AppError::Forbidden(_))
    ));
    r.assistant_group = Some(crate::models::approval_request::GroupApprovalBinding {
        group_id: "group".into(),
        conversation_id: "thread".into(),
        actor_user_id: "other".into(),
        request_id: "chain".into(),
    });
    assert!(recipients(&state.db, &r).await.unwrap().is_empty());
}

#[tokio::test]
async fn widening_preserves_grants_pruning_last_admin_keeps_empty_restriction() {
    let (state, r) = fixture("org_approvers_pruning").await;
    let cfgs = state.db.collection::<ServiceApprovalConfig>(CONFIGS);
    let cfg = cfgs.find_one(doc! {}).await.unwrap().unwrap();
    assert!(!revision_live(&cfg, None, false));
    assert!(revision_live(
        &cfg,
        Some(&OrgPolicyStamp {
            revision: 2,
            ..r.org_policy.clone().unwrap()
        }),
        false
    ));
    assert!(!revision_live(
        &cfg,
        Some(&OrgPolicyStamp {
            config_id: "replaced".into(),
            ..r.org_policy.clone().unwrap()
        }),
        false
    ));
    for id in ["alice", "bob"] {
        cfgs.update_many(doc! {"approver_user_ids":id}, prune_update(id))
            .await
            .unwrap();
    }
    let cfg = cfgs.find_one(doc! {}).await.unwrap().unwrap();
    assert_eq!(cfg.approvers.approver_user_ids, Some(vec![]));
    assert_eq!(cfg.approvers.min_grant_revision, 5);
    assert!(!revision_live(&cfg, r.org_policy.as_ref(), false));
    let mut session = state.db.client().start_session().await.unwrap();
    assert!(
        eligible(&state.db, &cfg, Some("selected"), None, false, &mut session)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn reusable_grant_cutoff_uses_origin_for_legacy_same_requester_rows() {
    let (state, r) = fixture("org_approvers_grant_floor").await;
    let grants = state
        .db
        .collection::<Document>(crate::models::approval_grant::COLLECTION_NAME);
    let origin = state.db.collection::<Document>(REQUESTS);
    let op = crate::services::operation_descriptor::build_http_descriptor("GET", "/resource", None);
    let check =
        || approvals::check_approval(&state.db, "org", "catalog", "user", "requester", true, &op);
    let base = doc! {"_id":"legacy","user_id":"org","service_id":"catalog","service_name":"Service","requester_type":"user","requester_id":"requester","approval_request_id":"missing","granted_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(r.expires_at),"revoked":false};
    grants.insert_one(base).await.unwrap();
    assert!(
        !check().await.unwrap(),
        "unattributed evidence after narrowing"
    );
    grants
        .update_one(
            doc! {"_id":"legacy"},
            doc! {"$set":{"approval_request_id":&r.id}},
        )
        .await
        .unwrap();
    origin.update_one(doc! {"_id":&r.id},doc! {"$unset":{"org_policy.connection_id":""},"$set":{"status":"approved","org_policy.revision":2_i64}}).await.unwrap();
    assert!(
        check().await.unwrap(),
        "widening preserves a provable origin"
    );
    origin
        .update_one(
            doc! {"_id":&r.id},
            doc! {"$set":{"org_policy.revision":1_i64}},
        )
        .await
        .unwrap();
    assert!(
        !check().await.unwrap(),
        "narrowing retires the old service-wide grant"
    );
}

#[tokio::test]
async fn policy_replacement_expires_pending_and_preserves_decided_history() {
    let (state, r) = fixture("org_approvers_stale_prompt").await;
    let cfgs = state.db.collection::<Document>(CONFIGS);
    let mut cfg = cfgs.find_one(doc! {}).await.unwrap().unwrap();
    cfg.insert("_id", "replacement");
    cfgs.delete_one(doc! {"_id":"config"}).await.unwrap();
    cfgs.insert_one(cfg).await.unwrap();
    assert!(!request_live(&state.db, &r).await.unwrap());
    assert_eq!(
        approvals::expire_pending_requests(
            &state.db,
            &state.config,
            &state.http_client,
            None,
            None,
            None
        )
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        approvals::get_request(&state.db, &r.id)
            .await
            .unwrap()
            .status,
        "expired"
    );
    let mut legacy = r.clone();
    legacy.id = "legacy-approved".into();
    legacy.status = "approved".into();
    legacy.org_policy = None;
    state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .insert_one(&legacy)
        .await
        .unwrap();
    let audit = approvals::ConfigAudit {
        actor: AuditActor {
            user_id: "alice".into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        user_service_id: Some("selected".into()),
    };
    approvals::delete_service_approval_config(&state.db, "org", "catalog", audit, true)
        .await
        .unwrap();
    let stored = approvals::get_request(&state.db, &legacy.id).await.unwrap();
    assert_eq!(stored.status, "approved");
    assert_eq!(stored.org_policy.unwrap().config_id, "replacement");
}

#[tokio::test]
async fn narrowed_grant_pages_and_retirement_distinguish_legacy_origin_and_execution() {
    let (state, request) = fixture("org_approvers_grant_cleanup").await;
    let db = &state.db;
    let grants = db.collection::<Document>(crate::models::approval_grant::COLLECTION_NAME);
    let requests = db.collection::<Document>(REQUESTS);
    requests
        .update_one(
            doc! {"_id":&request.id},
            doc! {"$set":{"status":"approved","org_policy.revision":1_i64}},
        )
        .await
        .unwrap();
    let template = doc! {"user_id":"org","service_id":"catalog","service_name":"Service","requester_type":"user",
    "requester_id":"requester","approval_request_id":"missing","granted_at":bson::DateTime::now(),
    "expires_at":bson::DateTime::from_chrono(request.expires_at),"revoked":false};
    for (id, fields) in [
        ("legacy", doc! {"approval_request_id":&request.id}),
        ("scoped", doc! {"org_scoped":true,"scope":"read"}),
        ("unknown", doc! {}),
        (
            "kept",
            doc! {"org_policy":bson::to_bson(&request.org_policy).unwrap()},
        ),
        ("permission", doc! {"execution":{"version":1}}),
    ] {
        let mut row = template.clone();
        row.insert("_id", id);
        row.extend(fields);
        grants.insert_one(row).await.unwrap();
    }
    let cfgs = db.collection::<ServiceApprovalConfig>(CONFIGS);
    let visibility = HistoryAccess::load(db, "alice", Some("org"), true)
        .await
        .unwrap();
    for (page, id) in [(1, "kept"), (2, "permission")] {
        let (items, total) =
            approvals::list_visible_grants(db, "org", &[], page, 1, Some(&visibility))
                .await
                .unwrap();
        assert_eq!(total, 2);
        assert_eq!(items[0].id, id);
    }
    cfgs.update_one(
        doc! {"_id":"config"},
        doc! {"$set":{"approver_grants_pending":true}},
    )
    .await
    .unwrap();
    retire_grants(db).await.unwrap();
    retire_grants(db).await.unwrap();
    let mut revoked = grants.distinct("_id", doc! {"revoked":true}).await.unwrap();
    revoked.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    assert_eq!(
        revoked,
        vec![bson::Bson::from("legacy"), bson::Bson::from("scoped")]
    );
    assert_eq!(
        cfgs.count_documents(doc! {"approver_grants_pending":true})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn history_survives_deleted_and_replaced_policy() {
    let (state, mut row) = fixture("approver_history_policy").await;
    let db = &state.db;
    row.status = "approved".into();
    db.collection::<ApprovalRequest>(REQUESTS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    let configs = db.collection::<Document>(CONFIGS);
    let mut cfg = configs.find_one_and_delete(doc! {}).await.unwrap().unwrap();
    assert!(visible(db, &row, "other").await.unwrap(), "deleted config");
    cfg.insert("_id", "replacement");
    configs.insert_one(cfg).await.unwrap();
    assert!(
        visible(db, &row, "other").await.unwrap(),
        "replacement UUID"
    );
}

#[tokio::test]
async fn history_keeps_exact_connection_scope_after_retirement() {
    let (state, mut row) = fixture("approver_history_connection").await;
    let db = &state.db;
    let requests = db.collection::<ApprovalRequest>(REQUESTS);
    row.status = "approved".into();
    requests
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    db.collection::<Document>(MEMBERS)
        .update_one(
            doc! {"_id":"bob"},
            doc! {"$set":{"allowed_service_ids":["selected"]}},
        )
        .await
        .unwrap();
    for update in [
        doc! {"is_active":false},
        doc! {"deleted_at":bson::DateTime::now()},
    ] {
        db.collection::<Document>(SERVICES)
            .update_one(doc! {"_id":"selected"}, doc! {"$set":update})
            .await
            .unwrap();
        for actor in ["alice", "bob"] {
            assert!(
                visible(db, &row, actor).await.unwrap(),
                "{actor} should see the retired connection"
            );
        }
    }
    db.collection::<Document>(MEMBERS)
        .update_one(
            doc! {"_id":"bob"},
            doc! {"$set":{"allowed_service_ids":["sibling"]}},
        )
        .await
        .unwrap();
    assert!(!visible(db, &row, "bob").await.unwrap());
}

#[tokio::test]
async fn org_owned_request_outside_policy_is_listed() {
    let (state, mut row) = fixture("approver_history_owner").await;
    row.org_policy = None;
    row.from_org_policy = false;
    let db = &state.db;
    db.collection::<ApprovalRequest>(REQUESTS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    let filter = history_filter(db, "other", Some("org"), false)
        .await
        .unwrap();
    let (rows, total) =
        approvals::list_requests_with_proposals(db, "org", &[], 1, 10, true, filter)
            .await
            .unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows[0].id, row.id);
}

#[tokio::test]
async fn group_participant_sees_another_actors_org_card() {
    let (state, row) = fixture("approver_group_display").await;
    state.db.collection::<Document>(REQUESTS).update_one(doc!{"_id":&row.id}, doc!{"$set":{"assistant_group":{
        "actor_user_id":"alice","conversation_id":"thread","group_id":"group","request_id":"chain"}}}).await.unwrap();
    let cards =
        crate::services::assistant_group_service::pending_group_approvals(&state.db, "group")
            .await
            .unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(
        cards[0].assistant_group.as_ref().unwrap().actor_user_id,
        "alice"
    );
}

#[tokio::test]
async fn grant_binding_follows_deciders_sibling_scope() {
    for (label, scope, expected) in [
        ("approver_grant_unrestricted", None, None),
        (
            "approver_grant_covers_all",
            Some(vec!["selected", "sibling"]),
            None,
        ),
        (
            "approver_grant_single",
            Some(vec!["selected"]),
            Some("selected"),
        ),
    ] {
        let (state, row) = fixture(label).await;
        let db = &state.db;
        db.collection::<Document>(CONFIGS)
            .update_one(doc! {}, doc! {"$unset":{"approver_user_ids":""}})
            .await
            .unwrap();
        db.collection::<Document>(MEMBERS)
            .update_one(
                doc! {"_id":"alice"},
                doc! {"$set":{"allowed_service_ids":scope}},
            )
            .await
            .unwrap();
        let card = approvals::read_decision_request(db, &row.id).await.unwrap();
        decide(&state, card, web("alice")).await.unwrap();
        let grant = db
            .collection::<ApprovalGrant>(crate::models::approval_grant::COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            grant.org_policy.unwrap().connection_id.as_deref(),
            expected,
            "{label}"
        );
    }
}

async fn monitored_reads(label: &str) -> (Database, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let reads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let record = reads.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        label,
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(e) = event
                && matches!(e.command_name.as_str(), "find" | "aggregate" | "distinct")
            {
                record
                    .lock()
                    .unwrap()
                    .push(e.command.get_str(&e.command_name).unwrap_or("").to_owned());
            }
        }),
    )
    .await;
    (db, reads)
}

#[tokio::test]
async fn chat_poll_without_org_candidates_uses_only_key_and_card_queries() {
    let (db, reads) = monitored_reads("approver_poll").await;
    let now = bson::DateTime::now();
    for id in ["member", "admin", "org1", "org2"] {
        let org = id.starts_with("org");
        db.collection::<Document>(USERS)
            .insert_one(doc! {"_id":id,"is_active":true,"user_type":if org{"org"}else{"person"}})
            .await
            .unwrap();
        if org {
            db.collection::<Document>(MEMBERS).insert_one(doc!{"_id":id,"org_user_id":id,"member_user_id":"admin","role":"admin","scope_source":"inherit","created_at":now}).await.unwrap();
        } else {
            db.collection::<Document>("api_keys").insert_one(doc!{"_id":id,"user_id":id,"is_active":true,"created_at":now,"name":"test","key_prefix":"test","key_hash":"test","scopes":""}).await.unwrap();
        }
    }
    let mut results = Vec::new();
    for actor in ["member", "admin"] {
        reads.lock().unwrap().clear();
        crate::services::assistant_nyxagent::pending_approvals(&db, actor, actor)
            .await
            .unwrap();
        results.push(reads.lock().unwrap().clone());
    }
    println!("CHAT_POLL_READS {results:?}");
    for result in results {
        assert_eq!(result, ["api_keys", REQUESTS]);
    }
}

#[tokio::test]
async fn inherited_legacy_grant_needs_no_origin_read() {
    let (db, reads) = monitored_reads("approver_grant_reads").await;
    let now = bson::DateTime::now();
    db.collection::<Document>(CONFIGS).insert_one(doc!{"_id":"c","user_id":"org","service_id":"svc","service_name":"service","approval_mode":"grant","approval_required":true,"created_at":now,"updated_at":now}).await.unwrap();
    db.collection::<Document>(crate::models::approval_grant::COLLECTION_NAME).insert_one(doc!{"_id":"g","user_id":"org","service_id":"svc","service_name":"service","approval_request_id":"old","requester_id":"user","requester_type":"user","revoked":false,"granted_at":now,"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(5))}).await.unwrap();
    reads.lock().unwrap().clear();
    let descriptor =
        crate::services::operation_descriptor::build_http_descriptor("GET", "/resource", None);
    assert!(
        approvals::check_approval(&db, "org", "svc", "user", "user", true, &descriptor)
            .await
            .unwrap()
    );
    assert!(
        !reads.lock().unwrap().iter().any(|c| c == REQUESTS),
        "legacy grant read its origin"
    );
}

#[tokio::test]
async fn grant_retirement_sweep_has_partial_marker_index() {
    let db = connect_transaction_test_database("approver_retirement_index").await;
    Box::pin(crate::db::ensure_indexes(&db)).await.unwrap();
    let indexes: Vec<_> = db
        .collection::<Document>(CONFIGS)
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        indexes
            .iter()
            .any(|i| i.keys == doc! {"approver_grants_pending":1}
                && i.options
                    .as_ref()
                    .and_then(|o| o.partial_filter_expression.as_ref())
                    == Some(&doc! {"approver_grants_pending":true})),
        "missing partial grant retirement index"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn stale_expiry_edits_primary_message_and_refreshes_original_inboxes() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let (mut state, row) = fixture("approver_expiry_delivery").await;
    // Observe the HTTPS CONNECT at a loopback proxy. Capture only the destination,
    // then close the socket before any TLS handshake or provider traffic.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = reqwest::Proxy::all(format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let client = reqwest::Client::builder().proxy(proxy).build().unwrap();
    let edit = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut line = String::new();
        BufReader::new(socket).read_line(&mut line).await.unwrap();
        line
    });
    state.config.telegram_bot_token = Some("test".into());
    let db = &state.db;
    db.collection::<Document>(REQUESTS).update_one(doc!{"_id":&row.id},doc!{"$set":{"telegram_chat_id":25_i64,"telegram_message_id":42_i64,"notification_channel":"telegram","notify_user_ids":["bob"]}}).await.unwrap();
    db.collection::<Document>(CONFIGS)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert_eq!(
        approvals::expire_pending_requests(db, &state.config, &client, None, None, None)
            .await
            .unwrap(),
        1
    );
    crate::services::shutdown::process().idle().await;
    assert_eq!(
        db.collection::<Document>(CHANNELS)
            .count_documents(doc! {"user_id":"bob"})
            .await
            .unwrap(),
        1
    );
    let destination = tokio::time::timeout(std::time::Duration::from_secs(2), edit)
        .await
        .expect("expiry should edit the primary Telegram message")
        .unwrap();
    assert_eq!(
        destination.trim_end(),
        "CONNECT api.telegram.org:443 HTTP/1.1"
    );
}
