//! Independent lifecycle fixtures. Collection names and joins deliberately do
//! not come from the adapter's inventory vector.
use super::*;
use crate::models::user::UserType;
use crate::test_utils::{test_app_state, test_user};

async fn step(state: &crate::AppState) {
    state
        .db
        .collection::<Document>(JOBS)
        .update_many(doc! {}, vec![doc! {"$set":{"retry_at":"$$NOW"}}])
        .await
        .unwrap();
    process_one(state).await.unwrap();
}

async fn until_source_removed(state: &crate::AppState, owner: &str) {
    for _ in 0..240 {
        if state
            .db
            .collection::<Document>(USERS)
            .find_one(doc! {"_id":owner})
            .await
            .unwrap()
            .is_none()
        {
            return;
        }
        step(state).await;
    }
    panic!(
        "source did not drain: {:?}",
        status(&state.db, owner).await.unwrap()
    );
}

#[tokio::test]
async fn accepted_worker_erases_independent_multidomain_graph_and_retains_shared_effect_evidence() {
    let (db, owner) = tests::fixture().await;
    let state = test_app_state(db.clone());
    let other = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&other, UserType::Org))
        .await
        .unwrap();
    let (sa, _) = super::super::service_account_service::create_service_account(
        &db,
        "Personal automation",
        None,
        "openid",
        &[],
        None,
        &owner,
    )
    .await
    .unwrap();
    let (shared_sa, _) = super::super::service_account_service::create_service_account(
        &db,
        "Shared automation",
        None,
        "openid",
        &[],
        None,
        &other,
    )
    .await
    .unwrap();
    // Actor attribution does not transfer resource ownership.
    db.collection::<Document>("service_accounts")
        .update_one(
            doc! {"_id":&shared_sa.id},
            doc! {"$set":{"created_by":&owner}},
        )
        .await
        .unwrap();
    let mut removed = Vec::<(&str, bson::Bson)>::new();
    for (collection, mut row) in [
        (
            "service_preferences",
            doc! {"_id":&owner,"ordered":["catalog-a"]},
        ),
        (
            "usage_workspaces",
            doc! {"_id":&owner,"revision":7_i64,"config":{"label":"private"}},
        ),
        (
            "cli_pairings",
            doc! {"_id":"pair","user_id":&owner,"status":"claimed"},
        ),
        (
            "connect_links",
            doc! {"_id":"connect","user_id":&owner,"status":"pending"},
        ),
        (
            "catalog_delegation_grants",
            doc! {"_id":"catalog-authority","user_id":&owner,"revoked":false},
        ),
        (
            "machine_oauth_consent_requests",
            doc! {"_id":"machine-selection","user_id":&owner},
        ),
        (
            "channel_delivery_receipts",
            doc! {"_id":"receipt","user_id":&owner,"recipient_id":"personal-recipient"},
        ),
        (
            "service_account_key_read_grants",
            doc! {"_id":&sa.id,"owner_id":&owner,"targets":[{"owner_id":&other}]},
        ),
        (
            "service_account_tokens",
            doc! {"_id":"sa-token","service_account_id":&sa.id},
        ),
        (
            "assistant_agents",
            doc! {"_id":"agent-personal","user_id":&owner},
        ),
        (
            "assistant_resources",
            doc! {"_id":"personal-resource","user_id":&owner,"conversation_id":"private-chat"},
        ),
        (
            "assistant_agent_learning_members",
            doc! {"_id":"other-member-on-personal-agent","agent_id":"agent-personal","member_user_id":&other},
        ),
        (
            "assistant_agent_learning_rejections",
            doc! {"_id":"rejection","agent_id":"agent-personal"},
        ),
        (
            "assistant_agent_learning_proposals",
            doc! {"_id":"settled-proposal","agent_id":"agent-personal","owner_id":&owner,"publication":{"started":true,"verified_at":bson::DateTime::now()}},
        ),
        (
            "channel_conversations",
            doc! {"_id":"personal-chat","user_id":&owner},
        ),
        (
            "channel_messages",
            doc! {"_id":"message-joined","conversation_id":"personal-chat","user_id":&other},
        ),
        (
            "channel_event_logs",
            doc! {"_id":"event-joined","conversation_id":"personal-chat"},
        ),
        ("mcp_sessions", doc! {"_id":"mcp","user_id":&owner}),
        ("triggers", doc! {"_id":"trigger","user_id":&owner}),
        (
            "trigger_work",
            doc! {"_id":"legacy-schedule","trigger_id":"trigger","kind":"schedule"},
        ),
        (
            "trigger_work",
            doc! {"_id":"legacy-run","trigger_id":"trigger","kind":"run"},
        ),
        ("trigger_owner_limits", doc! {"_id":&owner,"count":2_i64}),
        ("trigger_run_budgets", doc! {"_id":&owner,"count":4_i64}),
        (
            "saved_logins",
            doc! {"_id":"personal-login","user_id":&owner},
        ),
        ("login_codes", doc! {"_id":"login-code","user_id":&owner}),
        (
            "auth_device_codes",
            doc! {"_id":"device","approved_user_id":&owner},
        ),
        (
            "oauth_broker_bindings",
            doc! {"_id":"binding","user_id":&owner},
        ),
    ] {
        let id = row.get("_id").unwrap().clone();
        row.insert("fixture_marker", "personal-graph");
        db.collection::<Document>(collection)
            .insert_one(row)
            .await
            .unwrap();
        removed.push((collection, id));
    }
    // These are effect/authority records, not ordinary owner-body deletion.
    db.collection::<Document>("durable_operation_grants")
        .insert_one(
            doc! {"_id":"durable-grant","user_id":&owner,"constraints":{"body":"personal-value"}},
        )
        .await
        .unwrap();
    db.collection::<Document>("durable_operation_executions").insert_one(doc! {"_id":"never-dispatched","user_id":&owner,"grant_id":"durable-grant","status":"reserved","request_digest":"personal-request"}).await.unwrap();
    let retained = [
        (
            "assistant_learning_publication_targets",
            doc! {"_id":"landed-shared-target","owner_id":&owner,"agent_id":"agent-personal","state":"landed","operation_id":"settled-publication"},
        ),
        (
            "assistant_agent_learning_proposals",
            doc! {"_id":"org-draft","owner_id":&other,"agent_id":"org-agent"},
        ),
        (
            "assistant_resources",
            doc! {"_id":"org-resource","user_id":&other,"created_by":&owner},
        ),
        (
            "channel_conversations",
            doc! {"_id":"org-chat","user_id":&other,"created_by":&owner},
        ),
        (
            "channel_messages",
            doc! {"_id":"org-message","conversation_id":"org-chat","user_id":&other},
        ),
        (
            "billing_ledger",
            doc! {"_id":"financial-evidence","user_id":&owner,"amount":"0.1"},
        ),
    ];
    for (collection, row) in &retained {
        db.collection::<Document>(collection)
            .insert_one(row)
            .await
            .unwrap();
    }
    let accepted = accept(&db, &owner, &owner, None).await.unwrap();
    assert_eq!(accepted.status, ErasureStatus::Accepted);
    until_source_removed(&state, &owner).await;
    for (collection, id) in removed {
        assert!(
            db.collection::<Document>(collection)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .is_none(),
            "omitted personal source: {collection}"
        );
    }
    for (collection, row) in retained {
        assert_eq!(
            db.collection::<Document>(collection)
                .find_one(doc! {"_id":row.get("_id").unwrap()})
                .await
                .unwrap()
                .unwrap(),
            row,
            "modified retained {collection}"
        );
    }
    assert!(
        db.collection::<Document>("service_accounts")
            .find_one(doc! {"_id":&sa.id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.collection::<Document>("service_accounts")
            .find_one(doc! {"_id":&shared_sa.id})
            .await
            .unwrap()
            .is_some()
    );
    let receipt = db
        .collection::<Document>("personal_erasure_local_receipts")
        .find_one(doc! {"source_id":"never-dispatched"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.get_str("status"), Ok("rejected"));
    assert!(!receipt.contains_key("request_digest"));
    assert!(
        db.collection::<Document>(super::super::erasure_admission::COLLECTION)
            .find_one(doc! {"_id":&owner,"erasure_accepted":true})
            .await
            .unwrap()
            .is_some()
    );
    // Local source absence does not attest unknown vendor history.
    super::super::telemetry_erasure_service::drain_once(&db, &Default::default())
        .await
        .unwrap();
    super::super::telemetry_erasure_service::operator_reconcile_empty_inventory(
        &db,
        &owner,
        "operator",
        "evidence://deployment-inventory",
        "Verified deployment history",
    )
    .await
    .unwrap();
    for _ in 0..10 {
        step(&state).await;
        if status(&db, &owner).await.unwrap().unwrap().status == ErasureStatus::Complete {
            break;
        }
    }
    assert_eq!(
        status(&db, &owner).await.unwrap().unwrap().status,
        ErasureStatus::Complete
    );
    db.drop().await.unwrap();
}

#[derive(Debug)]
struct HeldProvider {
    inner: crate::crypto::local_key_provider::LocalKeyProvider,
    first: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    released: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl crate::crypto::key_provider::KeyProvider for HeldProvider {
    async fn wrap_dek(&self, key: &[u8]) -> AppResult<crate::crypto::key_provider::WrappedKey> {
        if !self.first.swap(true, std::sync::atomic::Ordering::SeqCst) {
            self.entered.notify_one();
            self.released.notified().await;
        }
        self.inner.wrap_dek(key).await
    }
    async fn unwrap_dek(
        &self,
        key: &crate::crypto::key_provider::WrappedKey,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.inner.unwrap_dek(key).await
    }
    fn current_key_id(&self) -> u8 {
        self.inner.current_key_id()
    }
    fn has_key_id(&self, id: u8) -> bool {
        self.inner.has_key_id(id)
    }
    fn has_previous_key(&self) -> bool {
        false
    }
}

#[tokio::test]
async fn held_actual_saved_login_and_trigger_cannot_recreate_rows_after_worker_source_purge() {
    for path in ["saved_login", "trigger"] {
        let (db, owner) = tests::fixture().await;
        let state = test_app_state(db.clone());
        let provider = std::sync::Arc::new(HeldProvider {
            inner: crate::crypto::local_key_provider::LocalKeyProvider::new([117; 32], None),
            first: Default::default(),
            entered: Default::default(),
            released: Default::default(),
        });
        let keys = crate::crypto::aes::EncryptionKeys::with_provider(provider.clone());
        let write_db = db.clone();
        let write_owner = owner.clone();
        let writer = tokio::spawn(async move {
            if path == "saved_login" {
                super::super::saved_login_service::put(
                    &write_db,
                    &keys,
                    &write_owner,
                    &write_owner,
                    None,
                    super::super::saved_login_service::Input {
                        label: "Held private login".into(),
                        allowed_origins: vec!["https://example.test".into()],
                        username: zeroize::Zeroizing::new("distinctive-login".into()),
                        password: None,
                        totp_secret: None,
                        confirm_each_sign_in: true,
                    },
                )
                .await
                .map(|_| ())
            } else {
                super::super::trigger_service::create(
                    &write_db,
                    &keys,
                    super::super::trigger_service::CreateInput {
                        source: crate::models::trigger_schedule::TriggerSource::Webhook,
                        setup_watch_id: None,
                        schedule: None,
                        overlap: Default::default(),
                        user_id: write_owner,
                        label: "Held trigger".into(),
                        user_service_id: None,
                        verification: crate::models::trigger::TriggerVerification::HmacSha256 {
                            header_name: "x-signature".into(),
                        },
                        delivery: crate::models::trigger::TriggerDelivery::Notification,
                    },
                )
                .await
                .map(|_| ())
            }
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            provider.entered.notified(),
        )
        .await
        .unwrap();
        accept(&db, &owner, &owner, None).await.unwrap();
        until_source_removed(&state, &owner).await;
        provider.released.notify_one();
        assert!(
            matches!(writer.await.unwrap(), Err(AppError::Conflict(_))),
            "held {path} writer escaped tombstone"
        );
        for collection in ["saved_logins", "triggers", "trigger_work"] {
            assert_eq!(
                db.collection::<Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn accepted_node_admission_closes_and_unavailable_or_restarted_replica_needs_retirement_evidence()
 {
    use crate::models::node::{Node, NodeConnectionOwner};
    use crate::services::node_owner_service::{NodeOwnerFence, ReplicaIdentity};
    let (db, owner) = tests::fixture().await;
    let state = test_app_state(db.clone());
    let id = uuid::Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    let node:Node=bson::from_document(doc! {"_id":&id,"user_id":&owner,"name":"personal node","status":"online",
        "auth_token_hash":crate::crypto::token::hash_token("distinctive-node-token"),"is_active":true,"created_at":now,"updated_at":now}).unwrap();
    db.collection::<Node>("nodes")
        .insert_one(&node)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let old = ReplicaIdentity {
        instance_name: "retired-replica".into(),
        generation_id: "old-generation".into(),
        internal_base_url: format!("http://{address}"),
    };
    let claimed: NodeConnectionOwner = super::super::node_owner_service::claim(
        &db,
        &id,
        &old,
        "old-connection",
        std::time::Duration::from_secs(3600),
    )
    .await
    .unwrap()
    .unwrap();
    let fence = NodeOwnerFence::from_owner(&id, &claimed);
    accept(&db, &owner, &owner, None).await.unwrap();
    assert!(
        super::super::node_service::validate_auth_token(&db, "distinctive-node-token")
            .await
            .is_err()
    );
    assert!(
        super::super::node_owner_service::claim(
            &db,
            &id,
            &state.replica_identity,
            "successor",
            std::time::Duration::from_secs(60)
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        !super::super::node_owner_service::renew(
            &db,
            &fence,
            std::time::Duration::from_secs(60),
            true
        )
        .await
        .unwrap()
    );
    // A real listener on the same address now serves a DIFFERENT generation.
    // Its signed internal disconnect must reject the old fence; this is not
    // evidence of physical closure of that previous process.
    let mut config = crate::test_utils::test_app_config();
    config.instance_name = "retired-replica".into();
    config.internal_advertise_url = format!("http://{address}");
    let restarted = crate::test_utils::test_app_state_with_config(db.clone(), config);
    let router = super::super::node_dispatch::internal_router(
        restarted.node_dispatch.clone(),
        65536,
        std::time::Duration::from_secs(2),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let inactive = db
        .collection::<Node>("nodes")
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert!(state.node_dispatch.drain_inactive(&inactive).await.is_err());
    let retained = db
        .collection::<Document>("nodes")
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        retained
            .get_document("personal_erasure_drain_owner")
            .unwrap()
            .get_str("generation_id"),
        Ok("old-generation")
    );
    server.abort();
    assert!(state.node_dispatch.drain_inactive(&inactive).await.is_err());
    assert_eq!(
        nodes::pending(&db, &owner, None).await.unwrap()[0]
            .fence
            .as_ref(),
        Some(&fence)
    );
    let mut wrong = fence.clone();
    wrong.generation_id = "unrelated-generation".into();
    assert!(
        nodes::attest_generation_retired(
            &db,
            "operator",
            &owner,
            &wrong,
            "evidence://process-exit"
        )
        .await
        .is_err()
    );
    nodes::attest_generation_retired(
        &db,
        "operator",
        &owner,
        &fence,
        "evidence://verified-old-process-exit",
    )
    .await
    .unwrap();
    until_source_removed(&state, &owner).await;
    assert!(
        db.collection::<Node>("nodes")
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(doc! {"event_type":"user.account.erasure_node_generation_retired"})
            .await
            .unwrap()
            .is_some()
    );
    db.drop().await.unwrap();
}
