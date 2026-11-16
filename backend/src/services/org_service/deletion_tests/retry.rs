use super::*;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Units {
    pub stop_at: Option<usize>,
    pub completed: Vec<Vec<String>>,
    started: HashMap<i32, (String, Option<String>)>,
    transactions: HashMap<String, Vec<String>>,
}
impl Units {
    pub fn observe(&mut self, event: &CommandEvent) -> bool {
        match event {
            CommandEvent::Started(e) => {
                let key = e
                    .command
                    .get_i64("txnNumber")
                    .ok()
                    .filter(|_| e.command.get_bool("autocommit") == Ok(false))
                    .map(|n| format!("{:?}:{n}", e.command.get_document("lsid").unwrap()));
                let name = if matches!(
                    e.command_name.as_str(),
                    "insert" | "delete" | "update" | "findAndModify"
                ) {
                    format!(
                        "{}:{}",
                        e.command_name,
                        e.command.get_str(&e.command_name).unwrap()
                    )
                } else {
                    e.command_name.clone()
                };
                self.started.insert(e.request_id, (name, key));
            }
            CommandEvent::Succeeded(e) => {
                let Some((name, key)) = self.started.remove(&e.request_id) else {
                    return false;
                };
                if name == "abortTransaction" {
                    if let Some(key) = key {
                        self.transactions.remove(&key);
                    }
                } else if name == "commitTransaction" {
                    if let Some(writes) = key.and_then(|k| self.transactions.remove(&k)) {
                        self.completed.push(writes);
                    }
                } else if name.contains(':') {
                    if let Some(key) = key {
                        self.transactions.entry(key).or_default().push(name);
                    } else {
                        self.completed.push(vec![name]);
                    }
                }
                if self.stop_at == Some(self.completed.len()) {
                    self.stop_at = None;
                    return true;
                }
            }
            CommandEvent::Failed(e) => {
                self.started.remove(&e.request_id);
            }
            _ => {}
        }
        false
    }
}

// Actual handler, with a crash only after MongoDB acknowledges a standalone
// write or an entire transaction. Fixture and inspection commands are excluded.
async fn rich_fixture() -> (Fixture, Vec<(String, Document)>) {
    let f = fixture().await;
    let mut inventory = Vec::new();
    let now = bson::DateTime::now();
    let sa = uuid::Uuid::new_v4().to_string();
    let client = uuid::Uuid::new_v4().to_string();
    let catalog = uuid::Uuid::new_v4().to_string();
    let conversation = uuid::Uuid::new_v4().to_string();
    let mut records = vec![
        (
            "service_accounts",
            doc! {"_id":&sa,"owner_user_id":&f.org,"is_active":false},
        ),
        ("service_account_tokens", doc! {"service_account_id":&sa}),
        ("user_provider_tokens", doc! {"user_id":&sa}),
        ("oauth_states", doc! {"target_user_id":&sa}),
        (
            "oauth_clients",
            doc! {"_id":&client,"created_by":&f.org,"is_active":false},
        ),
        ("refresh_tokens", doc! {"client_id":&client}),
        ("consents", doc! {"client_id":&client}),
        (
            "authorization_codes",
            doc! {"client_id":&client,"used":false},
        ),
        (
            "downstream_services",
            doc! {"_id":&catalog,"owner_user_id":&f.org,"is_active":false},
        ),
        ("service_endpoints", doc! {"service_id":&catalog}),
        (
            "service_provider_requirements",
            doc! {"service_id":&catalog},
        ),
        (
            "channel_conversations",
            doc! {"_id":&conversation,"user_id":&f.org,"is_active":false,
            "platform":"device","platform_conversation_id":"fixture","platform_conversation_type":"private",
            "agent_api_key_id":"fixture","created_at":now,"updated_at":now},
        ),
        ("channel_event_logs", doc! {"conversation_id":&conversation}),
    ];
    for c in [
        "approval_grants",
        "api_keys",
        "agent_service_bindings",
        "oauth_states",
        "user_provider_tokens",
        "node_registration_tokens",
        "node_service_bindings",
        "nodes",
        "user_provider_credentials",
        "saved_logins",
        "machine_setups",
        "machine_jobs",
        "machine_desktops",
        "machine_updates",
        "machine_update_attempts",
        "channel_activity_notifications",
        "channel_email_subscriptions",
        "channel_email_sends",
        "channel_email_batches",
        "channel_email_receipts",
        "channel_messages",
        "channel_bots",
        "notification_channels",
        "org_channel_messages",
    ] {
        records.push((
            c,
            doc! {"user_id":&f.org,"is_active":false,"revoked":true,"status":"revoked"},
        ));
    }
    for c in [
        "org_channel_sender_links",
        "org_channel_link_challenges",
        "org_channel_lanes",
    ] {
        records.push((c, doc! {"org_id":&f.org}));
    }
    records.push(("openclaw_channel_mappings", doc! {"nyxid_user_id":&f.org}));
    records.push(("org_invites", doc! {"org_user_id":&f.org}));
    records.push(("org_role_scopes", doc! {"org_user_id":&f.org}));
    records.push(("feature_flag_overrides", doc! {"org_user_id":&f.org}));
    records.push((
        "assistant_migrations",
        doc! {"org_id":&f.org,"migration":"issue-2482"},
    ));
    for (collection, mut row) in records {
        if !row.contains_key("_id") {
            row.insert("_id", uuid::Uuid::new_v4().to_string());
        }
        inventory.push((
            collection.to_owned(),
            doc! {"_id":row.get("_id").unwrap().clone()},
        ));
        f.state
            .db
            .collection::<Document>(collection)
            .insert_one(row)
            .await
            .unwrap();
    }
    let service = uuid::Uuid::new_v4().to_string();
    let mut row = crate::test_utils::test_user_service(
        &service,
        &f.org,
        "fixture",
        "endpoint",
        Some("catalog"),
        None,
    );
    row.is_active = false;
    f.state
        .db
        .collection::<crate::models::user_service::UserService>("user_services")
        .insert_one(row)
        .await
        .unwrap();
    inventory.push(("user_services".into(), doc! {"_id":&service}));
    let hidden = format!("nyxa-{}", uuid::Uuid::new_v4().simple());
    f.state.db.collection::<Document>("assistant_conversations").insert_one(doc!{
        "_id":&hidden,"user_id":&f.actor,"title":"fixture","model":"fixture","credential_api_key_id":"absent",
        "message_count":0_i64,"active_turn":null,"created_at":now,"updated_at":now,
        "channel":{"nyxbot_channel_id":"fixture","partition":"fixture","platform":"device",
            "org_execution":{"version":1,"org_id":&f.org,"revision":"fixture","agent_id":"fixture","actor_id":&f.actor,"guest":false,"sender_hash":"fixture"}}
    }).await.unwrap();
    inventory.push(("assistant_conversations".into(), doc! {"_id":&hidden}));
    for c in [
        crate::models::async_service_operation::COLLECTION_NAME,
        crate::models::generated_artifact::COLLECTION_NAME,
        crate::models::generated_artifact::DELIVERIES,
        crate::models::generated_artifact::SERIES,
        crate::models::cma_session_origin::COLLECTION_NAME,
        crate::models::assistant_voice::REQUESTS,
        crate::models::assistant_voice_session::COLLECTION_NAME,
        crate::models::assistant_voice_announcement::COLLECTION_NAME,
        crate::models::assistant_acknowledgement::COLLECTION_NAME,
        crate::services::assistant_service_execution::COLLECTION,
        crate::models::assistant_attachment::COLLECTION_NAME,
        crate::models::assistant_resource::COLLECTION_NAME,
        crate::models::assistant_tool_result::COLLECTION_NAME,
        crate::models::nyxbot_gateway_response::COLLECTION_NAME,
        crate::models::nyxbot_image_delivery::COLLECTION_NAME,
        crate::models::nyxbot_image_delivery::LEGACY_COLLECTION_NAME,
        crate::models::execution_permission_channel::COLLECTION_NAME,
        crate::models::assistant_upload_retention::TOMBSTONES,
        crate::models::assistant_agent_credential::COLLECTION_NAME,
        crate::models::assistant_message::COLLECTION_NAME,
        crate::models::assistant_message_feedback::COLLECTION_NAME,
        crate::models::org_channel::MESSAGES,
        crate::models::org_channel::LANES,
    ] {
        let id = uuid::Uuid::new_v4().to_string();
        f.state
            .db
            .collection::<Document>(c)
            .insert_one(doc! {"_id":&id,"conversation_id":&hidden,"user_id":&f.actor})
            .await
            .unwrap();
        inventory.push((c.into(), doc! {"_id":&id}));
    }
    // Two admins exercise protection against competing deleting administrators;
    // 101 ordinary members exercise both enrollment-repair page boundaries.
    let mut members = vec![test_membership(
        &f.org,
        &uuid::Uuid::new_v4().to_string(),
        OrgRole::Admin,
        None,
    )];
    for _ in 0..101 {
        members.push(test_membership(
            &f.org,
            &uuid::Uuid::new_v4().to_string(),
            OrgRole::Member,
            None,
        ));
    }
    f.state
        .db
        .collection::<OrgMembership>("org_memberships")
        .insert_many(members)
        .await
        .unwrap();
    inventory.push(("org_memberships".into(), doc! {"org_user_id":&f.org}));
    inventory.push(("users".into(), doc! {"_id":&f.org}));
    inventory.push(("approval_requests".into(), doc! {"user_id":&f.org}));
    inventory.push(("user_service_connections".into(), doc! {"user_id":&f.org}));
    (f, inventory)
}
async fn leftovers(f: &Fixture, inventory: &[(String, Document)]) -> Vec<String> {
    let mut left = Vec::new();
    for (c, query) in inventory {
        if f.state
            .db
            .collection::<Document>(c)
            .count_documents(query.clone())
            .await
            .unwrap()
            != 0
        {
            left.push(c.clone());
        }
    }
    // Inspect every collection created/touched by the actual cascade, including
    // empty conditional children, using only this fixture's known identities.
    // Recovery/evidence records retain their own protocol lifetime.
    let ids: Vec<_> = inventory
        .iter()
        .filter_map(|(_, q)| q.get_str("_id").ok())
        .collect();
    let references: Vec<_> = [
        "_id",
        "user_id",
        "org_user_id",
        "owner_user_id",
        "created_by",
        "org_id",
        "target_user_id",
        "client_id",
        "service_account_id",
        "service_id",
        "conversation_id",
        "channel.org_execution.org_id",
    ]
    .into_iter()
    .map(|field| doc! {field:{"$in":&ids}})
    .collect();
    for c in f.state.db.list_collection_names().await.unwrap() {
        if [
            INTENTS,
            "audit_log",
            "audit_chain_heads",
            "service_history_heads",
            "service_change_events",
            "execution_enrollment_jobs",
            "execution_enrollment_repairs",
            crate::models::machine_access::OUTBOX,
            crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME,
        ]
        .contains(&c.as_str())
        {
            continue;
        }
        let mut query = doc! {"$or":&references};
        if c == "authorization_codes" {
            query.insert("used", false);
        }
        if f.state
            .db
            .collection::<Document>(&c)
            .count_documents(query)
            .await
            .unwrap()
            != 0
        {
            left.push(c);
        }
    }
    left.sort();
    left.dedup();
    left
}
#[tokio::test]
async fn every_cascade_commit_is_retryable_from_the_same_admin() {
    let (f, inventory) = rich_fixture().await;
    let response = REQUEST.scope((), request(&f)).await;
    let status = response.status();
    if status != axum::http::StatusCode::NO_CONTENT {
        panic!(
            "baseline handler failed: {status} {:?}; units={:?}",
            response_json(response).await,
            f.monitor.lock().unwrap().units.completed
        );
    }
    assert!(leftovers(&f, &inventory).await.is_empty());
    let units = f.monitor.lock().unwrap().units.completed.clone();
    println!(
        "CASCADE_UNITS {} commands={} {units:?}",
        units.len(),
        f.monitor.lock().unwrap().commands.len()
    );
    f.state.db.drop().await.unwrap();
    let mut failures = Vec::new();
    for (index, unit) in units.iter().enumerate() {
        let (f, inventory) = rich_fixture().await;
        f.monitor.lock().unwrap().units.stop_at = Some(index + 1);
        let result = std::panic::AssertUnwindSafe(Box::pin(REQUEST.scope((), request(&f))))
            .catch_unwind()
            .await;
        assert!(result.unwrap_err().is::<ProcessStopped>(), "{unit:?}");
        let complete = count(&f, INTENTS, COMPLETED).await == 1;
        let status = request(&f).await.status();
        let expected = if complete {
            axum::http::StatusCode::NOT_FOUND
        } else {
            axum::http::StatusCode::NO_CONTENT
        };
        let left = leftovers(&f, &inventory).await;
        if status != expected || !left.is_empty() {
            failures.push(format!(
                "after {index} {unit:?}: status={status}, remaining={left:?}"
            ));
        }
        f.state.db.drop().await.unwrap();
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn rejected_completion_preserves_all_admins_and_their_repairs() {
    let (f, _) = rich_fixture().await;
    let admins: Vec<_> = rows(&f, "org_memberships")
        .await
        .into_iter()
        .filter(|r| r.get_str("role") == Ok("admin"))
        .collect();
    reject_event(&f, COMPLETED).await;
    assert_eq!(
        request(&f).await.status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(rows(&f, "org_memberships").await, admins);
    for admin in &admins {
        assert!(
            f.state
                .db
                .collection::<Document>("execution_enrollment_repairs")
                .find_one(doc! {"actor_id":admin.get_str("member_user_id").unwrap()})
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(count(&f, INTENTS, COMPLETED).await, 0);
    f.state.db.drop().await.unwrap();
}

async fn client_db(
    f: &Fixture,
    app: &str,
    handler: Option<EventHandler<CommandEvent>>,
) -> mongodb::Database {
    let uri = std::env::var("NYXID_TEST_DATABASE_URL").expect("owned MongoDB URL");
    let mut options = mongodb::options::ClientOptions::parse(uri).await.unwrap();
    options.app_name = Some(app.into());
    options.command_event_handler = handler;
    mongodb::Client::with_options(options)
        .unwrap()
        .database(f.state.db.name())
}
async fn seed_client(f: &Fixture, n: usize) -> String {
    let client = uuid::Uuid::new_v4().to_string();
    f.state.db.collection::<Document>("oauth_clients").insert_one(doc!{"_id":&client,"created_by":&f.org,"is_active":false,"machine_external_security_revision":0_i64}).await.unwrap();
    let rows = (0..n).map(|i| doc!{"_id":format!("{client}-{i}"),"client_id":&client,"user_id":&f.actor,
        "jti":uuid::Uuid::new_v4().to_string(),"session_id":uuid::Uuid::new_v4().to_string(),"scope":"openid profile offline_access",
        "allowed_service_ids":["fixture-service"],"allow_all_services":false,"resource_uris":["https://nyx.example/api/v1/proxy/s/fixture"],"replaced_by":null,"revoked_at":null,
        "expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::days(30)),"created_at":bson::DateTime::now(),"revoked":false,"touch":0_i64});
    f.state
        .db
        .collection::<Document>("refresh_tokens")
        .insert_many(rows)
        .await
        .unwrap();
    for c in ["consents", "authorization_codes"] {
        f.state
            .db
            .collection::<Document>(c)
            .insert_one(
                doc! {"_id":uuid::Uuid::new_v4().to_string(),"client_id":&client,"used":false},
            )
            .await
            .unwrap();
    }
    f.state
        .db
        .collection::<Document>("authorization_codes")
        .insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"client_id":&client,"used":true})
        .await
        .unwrap();
    client
}

#[tokio::test]
async fn client_child_delete_failure_rolls_back_the_whole_client_unit() {
    let f = fixture().await;
    let client = seed_client(&f, 3).await;
    use crate::models::machine_external_grant::{
        ExternalGrantBinding, GrantStatus, MachineExternalGrant,
    };
    let now = chrono::Utc::now();
    f.state
        .db
        .collection::<MachineExternalGrant>(crate::models::machine_external_grant::COLLECTION_NAME)
        .insert_one(MachineExternalGrant {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: f.actor.clone(),
            binding: ExternalGrantBinding::OAuth {
                oauth_client_id: client.clone(),
                consent_id: uuid::Uuid::new_v4().to_string(),
                consent_revision: uuid::Uuid::new_v4().to_string(),
            },
            policy: Default::default(),
            restrictions: Default::default(),
            revision: 1,
            status: GrantStatus::Active,
            admission_fence: 0,
            user_security_revision: 0,
            client_security_revision: Some(0),
            expires_at: now + chrono::Duration::days(30),
            created_at: now,
            updated_at: now,
            revoked_at: None,
            revoked_reason: None,
        })
        .await
        .unwrap();
    let collections = [
        "oauth_clients",
        "refresh_tokens",
        "consents",
        "authorization_codes",
        crate::models::machine_external_grant::COLLECTION_NAME,
        crate::models::machine_access::OUTBOX,
    ];
    let mut before = Vec::new();
    for c in collections {
        before.push(rows(&f, c).await);
    }
    // A readable view deterministically rejects the second child delete without
    // changing a server-wide failpoint or depending on command timing.
    f.state
        .db
        .collection::<Document>("consent_source")
        .insert_many(before[2].clone())
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("consents")
        .drop()
        .await
        .unwrap();
    f.state
        .db
        .run_command(doc! {"create":"consents","viewOn":"consent_source","pipeline":[]})
        .await
        .unwrap();
    let db = &f.state.db;
    let result = super::super::delete_inactive_org_oauth_client(db, &f.org, &client).await;
    let response = result
        .expect_err("all child deletes must be part of the guarded unit")
        .into_response();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(response_json(response).await["error_code"], 1007);
    for (c, expected) in collections.into_iter().zip(before) {
        assert_eq!(rows(&f, c).await, expected, "{c}");
    }
    f.state
        .db
        .collection::<Document>("consents")
        .drop()
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("consents")
        .insert_many(rows(&f, "consent_source").await)
        .await
        .unwrap();
    super::super::delete_inactive_org_oauth_client(db, &f.org, &client)
        .await
        .unwrap();
    for c in ["oauth_clients", "refresh_tokens", "consents"] {
        assert!(rows(&f, c).await.is_empty());
    }
    let codes = rows(&f, "authorization_codes").await;
    assert_eq!(codes.len(), 1);
    assert_eq!(codes[0].get_bool("used"), Ok(true));
    assert_eq!(
        rows(&f, crate::models::machine_access::OUTBOX).await.len(),
        1
    );
    let error = crate::services::oauth_client_service::admin_update_client(
        db,
        &client,
        crate::services::oauth_client_service::AdminUpdateClient {
            is_active: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.into_response().status(),
        axum::http::StatusCode::NOT_FOUND
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn reactivated_client_keeps_children_and_returns_existing_conflict() {
    let f = fixture().await;
    let client = seed_client(&f, 3).await;
    f.state
        .db
        .collection::<Document>("oauth_clients")
        .update_one(doc! {"_id":&client}, doc! {"$set":{"is_active":true}})
        .await
        .unwrap();
    let mut before = Vec::new();
    for c in ["refresh_tokens", "consents", "authorization_codes"] {
        before.push(rows(&f, c).await);
    }
    let error = super::super::delete_inactive_org_oauth_client(&f.state.db, &f.org, &client)
        .await
        .unwrap_err();
    assert_eq!(
        error.into_response().status(),
        axum::http::StatusCode::CONFLICT
    );
    for (c, expected) in ["refresh_tokens", "consents", "authorization_codes"]
        .into_iter()
        .zip(before)
    {
        assert_eq!(rows(&f, c).await, expected);
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
#[ignore = "contention probe: cargo test -p nyxid --bin nyxid-server client_deletion_scale -- --ignored --nocapture --test-threads 1"]
async fn client_deletion_scale() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for repetition in 1..=3 {
        let f = fixture().await;
        let client = seed_client(&f, 20_000).await;
        for collection in ["consents", "authorization_codes"] {
            f.state.db.collection::<Document>(collection).insert_many((0..999).map(|_| doc!{"_id":uuid::Uuid::new_v4().to_string(),"client_id":&client,"user_id":uuid::Uuid::new_v4().to_string(),"used":false,"scopes":"openid profile"})).await.unwrap();
        }
        super::super::cascade::ensure_indexes(&f.state.db)
            .await
            .unwrap();
        for keys in [doc! {"jti":1}, doc! {"session_id":1}, doc! {"expires_at":1}] {
            f.state
                .db
                .collection::<Document>("refresh_tokens")
                .create_index(mongodb::IndexModel::builder().keys(keys).build())
                .await
                .unwrap();
        }
        f.state
            .db
            .collection::<Document>("consents")
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! {"user_id":1,"client_id":1})
                    .build(),
            )
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>("authorization_codes")
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! {"code_hash":1})
                    .build(),
            )
            .await
            .unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let seen = attempts.clone();
        let db = client_db(
            &f,
            "org-client-scale",
            Some(EventHandler::callback(move |e| {
                if let CommandEvent::Started(e) = e
                    && e.command.get_bool("startTransaction") == Ok(true)
                {
                    seen.fetch_add(1, Ordering::SeqCst);
                }
            })),
        )
        .await;
        let stop = tokio_util::sync::CancellationToken::new();
        let done = stop.clone();
        let writer_db = f.state.db.clone();
        let writer_client = client.clone();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let writer = crate::services::shutdown::process().spawn(async move {
            let mut writes = 0usize;
            let mut ready_tx = Some(ready_tx);
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(40));
            loop {
                tokio::select! { _ = done.cancelled() => break, _ = interval.tick() => {} }
                writer_db
                    .collection::<Document>("refresh_tokens")
                    .update_one(
                        doc! {"_id":format!("{writer_client}-{}", ((writes % 1000) * 7919) % 20_000)},
                        doc! {"$inc":{"touch":1_i64}},
                    )
                    .await
                    .unwrap();
                writes += 1;
                if let Some(ready) = ready_tx.take() {
                    let _ = ready.send(());
                }
            }
            writes
        });
        ready_rx.await.unwrap();
        let started = std::time::Instant::now();
        let result = super::super::delete_inactive_org_oauth_client(&db, &f.org, &client).await;
        let elapsed = started.elapsed();
        stop.cancel();
        let writes = writer.await.unwrap();
        println!(
            "CLIENT_SCALE repetition={repetition} tokens=20000 consents=1000 unused_codes=1000 elapsed={elapsed:?} retries={} concurrent_writes={writes} result={result:?}",
            attempts.load(Ordering::SeqCst).saturating_sub(1)
        );
        result.unwrap();
        assert!(rows(&f, "refresh_tokens").await.is_empty());
        assert!(rows(&f, "oauth_clients").await.is_empty());
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn competing_cleanup_keeps_both_admins_until_exact_completion() {
    let (f, _) = rich_fixture().await;
    let before: Vec<_> = rows(&f, "org_memberships")
        .await
        .into_iter()
        .filter(|r| r.get_str("role") == Ok("admin"))
        .collect();
    let (one, two) = tokio::join!(
        crate::services::execution_permission_enrollment::jobs::delete_org_memberships(
            &f.state.db,
            &f.org
        ),
        crate::services::execution_permission_enrollment::jobs::delete_org_memberships(
            &f.state.db,
            &f.org
        )
    );
    one.unwrap();
    two.unwrap();
    assert_eq!(rows(&f, "org_memberships").await, before);
    let actor = actor(&f);
    let marker = super::super::deletion::start(&f.state.db, &f.org, &actor)
        .await
        .unwrap();
    let (one, two) = tokio::join!(
        super::super::deletion::complete(&f.state.db, &f.org, &marker, &actor),
        super::super::deletion::complete(&f.state.db, &f.org, &marker, &actor)
    );
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    let err = one.err().or(two.err()).unwrap();
    assert_eq!(
        err.into_response().status(),
        axum::http::StatusCode::NOT_FOUND
    );
    assert_eq!(count(&f, INTENTS, COMPLETED).await, 1);
    assert!(rows(&f, "org_memberships").await.is_empty());
    for admin in &before {
        assert!(
            f.state
                .db
                .collection::<Document>("execution_enrollment_repairs")
                .find_one(doc! {"actor_id":admin.get_str("member_user_id").unwrap()})
                .await
                .unwrap()
                .is_some()
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
#[ignore = "fanout probe: cargo test -p nyxid --bin nyxid-server final_admin_memberships_scale -- --ignored --nocapture --test-threads 1"]
async fn final_admin_memberships_scale() {
    let f = fixture().await;
    f.state
        .db
        .collection::<OrgMembership>("org_memberships")
        .insert_many((0..999).map(|_| {
            test_membership(
                &f.org,
                &uuid::Uuid::new_v4().to_string(),
                OrgRole::Admin,
                None,
            )
        }))
        .await
        .unwrap();
    let actor = actor(&f);
    let marker = super::super::deletion::start(&f.state.db, &f.org, &actor)
        .await
        .unwrap();
    let now = std::time::Instant::now();
    REQUEST
        .scope(
            (),
            super::super::deletion::complete(&f.state.db, &f.org, &marker, &actor),
        )
        .await
        .unwrap();
    let units = f.monitor.lock().unwrap().units.completed.clone();
    println!(
        "FINAL_ADMIN_SCALE admins=1000 elapsed={:?} committed_units={} commands={}",
        now.elapsed(),
        units.len(),
        f.monitor.lock().unwrap().commands.len()
    );
    assert!(rows(&f, "org_memberships").await.is_empty());
    assert_eq!(rows(&f, "execution_enrollment_repairs").await.len(), 1000);
    f.state.db.drop().await.unwrap();
}
