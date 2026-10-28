use super::{
    assistant_team_service as team, feature_flag_service as flags, org_agent_service as access,
    org_agent_tests::Fixture, shared_agent_service as sharing,
};
use crate::models::{
    assistant_agent::GuestAccess,
    assistant_sharing::{ShareRole, ShareSubject},
    usage_meter::CredentialClass,
    user::UserType,
};
use mongodb::bson::{Document, doc};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::atomic::Ordering};
use uuid::Uuid;

async fn mcp(state: &crate::AppState, key: &str, method: &str, params: Value) -> Value {
    use super::billing::route_inventory::{BillingIngress, BillingRoutePolicy};
    use axum::{
        Extension,
        body::{Body, to_bytes},
        extract::State,
        http::Request,
    };
    let request = Request::builder()
        .uri("/mcp")
        .method("POST")
        .header("x-api-key", key)
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string(),
        ))
        .unwrap();
    let response = Box::pin(crate::handlers::mcp_transport::mcp_post(
        State(state.clone()),
        Extension(BillingRoutePolicy::Metered(BillingIngress::Mcp)),
        request,
    ))
    .await;
    serde_json::from_slice(&to_bytes(response.into_body(), 65_536).await.unwrap()).unwrap()
}

#[test]
fn shared_agent_widening_classifier_covers_each_authority_change() {
    let alice = "11111111-1111-4111-8111-111111111111".to_owned();
    let bob = "22222222-2222-4222-8222-222222222222".to_owned();
    let person = |subject_id: &str, role| sharing::Audience {
        subject_kind: ShareSubject::Person,
        subject_id: subject_id.to_owned(),
        role,
    };
    let base_audience = vec![person(&alice, ShareRole::User)];
    let base_access = BTreeMap::from([("calendar".to_owned(), GuestAccess::Read)]);
    let input = |enabled, audience, shared_access| sharing::SharingUpdate {
        expected_version: 1,
        enabled,
        audience,
        shared_access,
    };

    assert!(sharing::sharing_widens(
        false,
        &[],
        &BTreeMap::new(),
        &input(true, vec![], BTreeMap::new())
    ));
    assert!(sharing::sharing_widens(
        true,
        &base_audience,
        &base_access,
        &input(
            true,
            vec![
                person(&alice, ShareRole::User),
                person(&bob, ShareRole::User)
            ],
            base_access.clone()
        )
    ));
    assert!(sharing::sharing_widens(
        true,
        &base_audience,
        &base_access,
        &input(
            true,
            vec![person(&alice, ShareRole::Manager)],
            base_access.clone()
        )
    ));
    assert!(sharing::sharing_widens(
        true,
        &base_audience,
        &base_access,
        &input(
            true,
            base_audience.clone(),
            BTreeMap::from([("drive".into(), GuestAccess::Use)])
        )
    ));
    assert!(sharing::sharing_widens(
        true,
        &base_audience,
        &base_access,
        &input(
            true,
            base_audience.clone(),
            BTreeMap::from([("calendar".into(), GuestAccess::All)])
        )
    ));
    assert!(!sharing::sharing_widens(
        true,
        &base_audience,
        &base_access,
        &input(false, vec![], BTreeMap::new())
    ));
    // Removing Alice while adding Bob is still a widening and must raise the
    // owner card before the mixed policy can be applied.
    assert!(sharing::sharing_widens(
        true,
        &base_audience,
        &base_access,
        &input(true, vec![person(&bob, ShareRole::User)], BTreeMap::new())
    ));
}

#[tokio::test]
async fn shared_agent_preview_lists_only_sanitized_delta_and_rejects_stale_versions() {
    let f = Fixture::new("shared_agent_preview_delta").await;
    let agent = publish(&f, false).await;
    let service = f.service(&f.admin, "preview-service").await;
    f.state
        .db
        .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"grants.service_ids": [&service]}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.member},
            doc! {"$set": {"display_name": "Mallory\n(manager)"}},
        )
        .await
        .unwrap();
    let input = sharing::SharingUpdate {
        expected_version: 1,
        enabled: true,
        audience: vec![
            sharing::Audience {
                subject_kind: ShareSubject::Person,
                subject_id: f.outsider.clone(),
                role: ShareRole::User,
            },
            sharing::Audience {
                subject_kind: ShareSubject::Org,
                subject_id: f.org.clone(),
                role: ShareRole::User,
            },
            email_audience(&format!("{}@example.com", f.member), ShareRole::Manager),
        ],
        shared_access: BTreeMap::from([(service.clone(), GuestAccess::All)]),
    };
    let preview = sharing::preview_update(&f.state.db, &f.admin, &agent, &input)
        .await
        .unwrap();
    assert!(preview.widening);
    let member_email = format!("{}@example.com", f.member);
    assert!(preview.summary.contains(&member_email));
    assert!(!preview.summary.contains("Mallory"));
    assert!(!preview.summary.contains("manager)"));
    assert!(!preview.summary.contains('\n'));
    assert!(!preview.summary.contains(&f.outsider));
    assert!(preview.summary.contains("2 existing recipients unchanged"));
    assert!(preview.summary.contains("preview-service"));
    assert!(!preview.summary.contains("existing services unchanged"));

    let stale = sharing::SharingUpdate {
        expected_version: 0,
        enabled: true,
        audience: Vec::new(),
        shared_access: BTreeMap::new(),
    };
    assert!(matches!(
        sharing::preview_update(&f.state.db, &f.admin, &agent, &stale).await,
        Err(crate::errors::AppError::Conflict(_))
    ));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_preview_identity_is_owner_scoped() {
    let f = Fixture::new("shared_agent_preview_identity_scope").await;
    let agent = publish(&f, false).await;

    let unknown = Uuid::new_v4().to_string();
    let unknown_input = sharing::SharingUpdate {
        expected_version: 1,
        enabled: true,
        audience: vec![sharing::Audience {
            subject_kind: ShareSubject::Person,
            subject_id: unknown.clone(),
            role: ShareRole::User,
        }],
        shared_access: BTreeMap::new(),
    };
    let error = sharing::preview_update(&f.state.db, &f.admin, &agent, &unknown_input)
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("@example.com"));
    assert!(!error.contains("Test User"));
    assert!(!error.contains(&unknown));

    let existing_input = sharing::SharingUpdate {
        expected_version: 1,
        enabled: true,
        audience: vec![
            sharing::Audience {
                subject_kind: ShareSubject::Person,
                subject_id: f.outsider.clone(),
                role: ShareRole::Manager,
            },
            sharing::Audience {
                subject_kind: ShareSubject::Org,
                subject_id: f.org.clone(),
                role: ShareRole::User,
            },
        ],
        shared_access: BTreeMap::new(),
    };
    let preview = sharing::preview_update(&f.state.db, &f.admin, &agent, &existing_input)
        .await
        .unwrap();
    assert!(preview.widening);
    assert!(
        preview
            .summary
            .contains(&format!("{}@example.com", f.outsider))
    );
    assert!(!preview.summary.contains("Test User"));

    let outside_org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(&outside_org, UserType::Org))
        .await
        .unwrap();
    let org_input = sharing::SharingUpdate {
        expected_version: 1,
        enabled: true,
        audience: vec![sharing::Audience {
            subject_kind: ShareSubject::Org,
            subject_id: outside_org,
            role: ShareRole::User,
        }],
        shared_access: BTreeMap::new(),
    };
    let org_error = sharing::update(&f.state.db, &f.admin, &agent.id, org_input, true)
        .await
        .unwrap_err()
        .to_string();
    assert!(org_error.contains("Sharing audience is unavailable"));
    assert!(!org_error.contains("Test Org"));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_preview_email_is_directory_free_and_unknown_is_carded() {
    let (f, _, reads) = Fixture::observed("shared_agent_preview_email_directory_free").await;
    let agent = publish(&f, false).await;
    reads.store(0, Ordering::SeqCst);
    let unknown = email_audience("unknown@example.com", ShareRole::User);
    let unknown_preview = sharing::preview_update(
        &f.state.db,
        &f.admin,
        &agent,
        &sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience: vec![unknown],
            shared_access: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    assert!(unknown_preview.widening);
    assert!(unknown_preview.summary.contains("unknown@example.com"));
    // The preview reads only the policy and existing share rows. In
    // particular, it does not perform the owner-directory or email lookup.
    assert_eq!(reads.load(Ordering::SeqCst), 2);

    reads.store(0, Ordering::SeqCst);
    let known_preview = sharing::preview_update(
        &f.state.db,
        &f.admin,
        &agent,
        &sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience: vec![email_audience(
                &format!("{}@example.com", f.member),
                ShareRole::User,
            )],
            shared_access: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    assert!(known_preview.widening);
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    assert!(
        known_preview
            .summary
            .contains(&format!("{}@example.com", f.member))
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_email_write_requires_verified_unique_identity() {
    let f = Fixture::new("shared_agent_email_identity_binding").await;
    let agent = publish(&f, false).await;

    f.state
        .db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.member},
            doc! {"$set": {"email": "MiXeD@Example.COM", "email_verified": true}},
        )
        .await
        .unwrap();
    let mut audience = sharing::audience(&f.state.db, &f.admin, &agent.id)
        .await
        .unwrap();
    audience.push(email_audience("mixed@example.com", ShareRole::User));
    sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience,
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await
    .unwrap();
    let mixed_row = f
        .state
        .db
        .collection::<crate::models::assistant_sharing::AgentShare>(
            crate::models::assistant_sharing::SHARES_COLLECTION_NAME,
        )
        .find_one(doc! {"agent_id": &agent.id, "subject_id": &f.member})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        mixed_row.subject_email.as_deref(),
        Some("mixed@example.com")
    );

    let unverified_agent = publish_named(&f, false, "unverified-target").await;
    f.state
        .db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.member},
            doc! {"$set": {"email": "unverified@example.com", "email_verified": false}},
        )
        .await
        .unwrap();
    let mut unverified_audience = sharing::audience(&f.state.db, &f.admin, &unverified_agent.id)
        .await
        .unwrap();
    unverified_audience.push(email_audience("unverified@example.com", ShareRole::User));
    let unverified_error = sharing::update(
        &f.state.db,
        &f.admin,
        &unverified_agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience: unverified_audience,
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(unverified_error.contains("sharing could not be applied"));
    assert_eq!(
        sharing::policy(&f.state.db, &unverified_agent)
            .await
            .unwrap()
            .version,
        1
    );

    let ambiguous_agent = publish_named(&f, false, "ambiguous-target").await;
    f.state
        .db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.member},
            doc! {"$set": {"email": "ambiguous@example.com", "email_verified": true}},
        )
        .await
        .unwrap();
    let duplicate_id = Uuid::new_v4().to_string();
    let mut duplicate = crate::test_utils::test_user(&duplicate_id, UserType::Person);
    duplicate.email = "AMBIGUOUS@EXAMPLE.COM".into();
    f.state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(duplicate)
        .await
        .unwrap();
    let mut ambiguous_audience = sharing::audience(&f.state.db, &f.admin, &ambiguous_agent.id)
        .await
        .unwrap();
    ambiguous_audience.push(email_audience("ambiguous@example.com", ShareRole::User));
    let ambiguous_error = sharing::update(
        &f.state.db,
        &f.admin,
        &ambiguous_agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience: ambiguous_audience,
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(ambiguous_error.contains("sharing could not be applied"));
    assert_eq!(
        sharing::audience(&f.state.db, &f.admin, &ambiguous_agent.id)
            .await
            .unwrap()
            .len(),
        2
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_approved_unknown_email_writes_nothing() {
    let f = Fixture::new("shared_agent_unknown_email_write").await;
    let agent = publish(&f, false).await;
    let mut audience = sharing::audience(&f.state.db, &f.admin, &agent.id)
        .await
        .unwrap();
    audience.push(email_audience(
        "does-not-exist@example.com",
        ShareRole::User,
    ));
    let error = sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience,
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("sharing could not be applied to one or more of the people listed"));
    let current = sharing::policy(&f.state.db, &agent).await.unwrap();
    assert_eq!(current.version, 1);
    assert_eq!(current.revision, 1);
    assert_eq!(
        sharing::audience(&f.state.db, &f.admin, &agent.id)
            .await
            .unwrap()
            .len(),
        2
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_write_rechecks_widening_and_requires_delegation_approval() {
    let f = Fixture::new("shared_agent_write_widening_fence").await;
    let agent = publish(&f, false).await;
    let input = sharing::SharingUpdate {
        expected_version: 1,
        enabled: true,
        audience: vec![sharing::Audience {
            subject_kind: ShareSubject::Person,
            subject_id: f.outsider.clone(),
            role: ShareRole::User,
        }],
        shared_access: BTreeMap::new(),
    };
    let preview = sharing::preview_update(&f.state.db, &f.admin, &agent, &input)
        .await
        .unwrap();
    assert!(!preview.widening);

    // Simulate an authority change after preview without a version change.
    // The write snapshot now sees false -> true and must fail closed.
    f.state
        .db
        .collection::<Document>(crate::models::assistant_sharing::COLLECTION_NAME)
        .update_one(doc! {"_id": &agent.id}, doc! {"$set": {"enabled": false}})
        .await
        .unwrap();
    let refused = sharing::update(&f.state.db, &f.admin, &agent.id, input, false).await;
    assert!(matches!(
        refused,
        Err(crate::errors::AppError::Forbidden(_))
    ));
    let current = sharing::policy(&f.state.db, &agent).await.unwrap();
    assert!(!current.enabled);
    assert_eq!(current.version, 1);
    assert_eq!(
        sharing::audience(&f.state.db, &f.admin, &agent.id)
            .await
            .unwrap()
            .len(),
        2
    );

    // The first-party human/REST lane explicitly supplies approval.
    let applied = sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience: vec![sharing::Audience {
                subject_kind: ShareSubject::Person,
                subject_id: f.outsider.clone(),
                role: ShareRole::User,
            }],
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await
    .unwrap();
    assert!(applied.enabled);
    assert_eq!(applied.version, 2);
    f.state.db.drop().await.unwrap();
}

async fn publish(f: &Fixture, manager: bool) -> crate::models::assistant_agent::AssistantAgent {
    publish_named(f, manager, "shared-researcher").await
}

async fn publish_named(
    f: &Fixture,
    manager: bool,
    name: &str,
) -> crate::models::assistant_agent::AssistantAgent {
    sharing::ensure_indexes(&f.state.db).await.unwrap();
    flags::set_platform_override(
        &f.state.db,
        sharing::FLAG,
        &flags::FlagTarget::Global,
        true,
        &f.admin,
    )
    .await
    .unwrap();
    let (agent, _) = team::create_specialist(
        &f.state.db,
        &f.state.encryption_keys,
        &f.admin,
        f.request(name),
    )
    .await
    .unwrap()
    .unwrap();
    sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 0,
            enabled: true,
            shared_access: BTreeMap::new(),
            audience: vec![
                email_audience(
                    &format!("{}@example.com", f.outsider),
                    if manager {
                        ShareRole::Manager
                    } else {
                        ShareRole::User
                    },
                ),
                sharing::Audience {
                    subject_kind: ShareSubject::Org,
                    subject_id: f.org.clone(),
                    role: ShareRole::User,
                },
            ],
        },
        true,
    )
    .await
    .unwrap();
    agent
}

async fn seed_shared_activity(
    f: &Fixture,
    agent: &crate::models::assistant_agent::AssistantAgent,
) -> (String, String) {
    let thread = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.outsider, agent)
        .await
        .unwrap();
    let consent_id = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &thread.id},
            doc! {"$set": {
                "active_turn": {"turn_id": "active", "stop_requested": false},
                "nyxagent_session_id": "session-before-share-save",
                "nyxagent_instruction_binding": "binding-before-share-save",
                "nyxagent_last_response_id": "response-before-share-save",
                "pending_events": ["pending-before-share-save"],
            }},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &consent_id,
            "agent_id": &agent.id,
            "owner_id": &f.admin,
            "actor_user_id": &f.outsider,
            "conversation_id": &thread.id,
            "policy_revision": 1_i64,
            "decision": "always",
        })
        .await
        .unwrap();
    (thread.id, consent_id)
}

fn shared_test_skill() -> (
    crate::models::catalog_skill_revision::SkillReference,
    crate::models::assistant_agent::AgentSkillMetadata,
) {
    (
        crate::models::catalog_skill_revision::SkillReference {
            source: "ornn".into(),
            skill_id: "shared-skill".into(),
            name: "Shared skill".into(),
            version: "1.0".into(),
            sha256: "a".repeat(64),
            dependencies: Vec::new(),
            disabled: false,
        },
        crate::models::assistant_agent::AgentSkillMetadata {
            description: "Pinned shared guidance".into(),
            size_bytes: 42,
            preload_paths: Vec::new(),
            preload_bytes: 0,
        },
    )
}

fn email_audience(email: &str, role: ShareRole) -> sharing::Audience {
    serde_json::from_value(json!({
        "subject_kind": "person",
        "email": email,
        "role": role,
    }))
    .unwrap()
}

async fn pin_shared_test_skill(
    f: &Fixture,
    agent: &crate::models::assistant_agent::AssistantAgent,
) {
    let (skill, metadata) = shared_test_skill();
    f.state
        .db
        .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {
                "skills": mongodb::bson::to_bson(&vec![skill]).unwrap(),
                "skill_metadata": mongodb::bson::to_bson(&BTreeMap::from([("shared-skill".to_owned(), metadata)])).unwrap(),
            }},
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn shared_agent_sharing_noop_save_has_no_side_effects() {
    let f = Fixture::new("shared_agent_sharing_noop").await;
    let agent = publish(&f, false).await;
    let current = sharing::policy(&f.state.db, &agent).await.unwrap();
    let audience = sharing::audience(&f.state.db, &f.admin, &agent.id)
        .await
        .unwrap();
    let (thread_id, consent_id) = seed_shared_activity(&f, &agent).await;
    let saved = sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: current.version,
            enabled: current.enabled,
            audience,
            shared_access: current.shared_access.clone(),
        },
        true,
    )
    .await
    .unwrap();
    assert_eq!(saved.revision, current.revision);
    assert_eq!(saved.version, current.version);
    assert_eq!(saved.updated_at, current.updated_at);
    let consent = f
        .state
        .db
        .collection::<Document>(crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME)
        .find_one(doc! {"_id": &consent_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(consent.get_str("decision").unwrap(), "always");
    let thread = f
        .state
        .db
        .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id": &thread_id})
        .await
        .unwrap()
        .unwrap();
    assert!(!thread["active_turn"]["stop_requested"].as_bool().unwrap());
    assert_eq!(
        thread.get_str("nyxagent_session_id").unwrap(),
        "session-before-share-save"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_add_only_save_preserves_existing_consent_and_turn() {
    let f = Fixture::new("shared_agent_sharing_add").await;
    let agent = publish(&f, false).await;
    let (thread_id, consent_id) = seed_shared_activity(&f, &agent).await;
    let mut audience = sharing::audience(&f.state.db, &f.admin, &agent.id)
        .await
        .unwrap();
    audience.push(email_audience(
        &format!("{}@example.com", f.member),
        ShareRole::User,
    ));
    let saved = sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience,
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await
    .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.version, 2);
    let consent = f
        .state
        .db
        .collection::<Document>(crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME)
        .find_one(doc! {"_id": &consent_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(consent.get_str("decision").unwrap(), "always");
    let thread = f
        .state
        .db
        .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id": &thread_id})
        .await
        .unwrap()
        .unwrap();
    assert!(!thread["active_turn"]["stop_requested"].as_bool().unwrap());
    assert_eq!(
        thread.get_str("nyxagent_session_id").unwrap(),
        "session-before-share-save"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_add_only_save_rejects_stale_write_version() {
    let f = Fixture::new("shared_agent_sharing_stale_add").await;
    let agent = publish(&f, false).await;
    let base = sharing::audience(&f.state.db, &f.admin, &agent.id)
        .await
        .unwrap();
    let mut add_alice = base.clone();
    add_alice.push(email_audience(
        &format!("{}@example.com", f.member),
        ShareRole::User,
    ));
    let first = sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience: add_alice,
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await
    .unwrap();
    assert_eq!(first.version, 2);
    assert_eq!(first.revision, 1);

    // A second writer built from version 1 must not silently remove the
    // audience added by the first writer.
    let stale = sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience: base,
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await;
    assert!(matches!(stale, Err(crate::errors::AppError::Conflict(_))));
    let current = sharing::policy(&f.state.db, &agent).await.unwrap();
    assert_eq!(current.version, 2);
    assert_eq!(current.revision, 1);
    assert!(
        sharing::audience(&f.state.db, &f.admin, &agent.id)
            .await
            .unwrap()
            .iter()
            .any(|subject| subject.subject_id == f.member)
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_narrowing_save_revokes_consent_and_stops_turn() {
    let f = Fixture::new("shared_agent_sharing_narrow").await;
    let agent = publish(&f, false).await;
    let (thread_id, consent_id) = seed_shared_activity(&f, &agent).await;
    let audience = vec![sharing::Audience {
        subject_kind: ShareSubject::Org,
        subject_id: f.org.clone(),
        role: ShareRole::User,
    }];
    let saved = sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience,
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await
    .unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.version, 2);
    let consent = f
        .state
        .db
        .collection::<Document>(crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME)
        .find_one(doc! {"_id": &consent_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(consent.get_str("decision").unwrap(), "revoked");
    let thread = f
        .state
        .db
        .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id": &thread_id})
        .await
        .unwrap()
        .unwrap();
    assert!(thread["active_turn"]["stop_requested"].as_bool().unwrap());
    assert!(
        thread
            .get("nyxagent_session_id")
            .is_none_or(|v| matches!(v, mongodb::bson::Bson::Null))
    );
    assert!(
        thread
            .get("pending_events")
            .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty))
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_live_acl_role_and_billing_matrix() {
    let f = Fixture::new("shared_agent_acl").await;
    let agent = publish(&f, true).await;
    let manager = sharing::require(&f.state.db, &f.outsider, &agent)
        .await
        .unwrap();
    assert_eq!(manager.role(), ShareRole::Manager);
    assert_eq!(
        sharing::require(&f.state.db, &f.member, &agent)
            .await
            .unwrap()
            .role(),
        ShareRole::User
    );
    assert!(
        sharing::require(&f.state.db, &f.viewer, &agent)
            .await
            .is_err()
    );
    assert!(
        access::require_maintain(&f.state.db, &f.outsider, &agent)
            .await
            .is_err()
    );
    let edited = team::update_agent(
        &f.state.db,
        &f.outsider,
        &agent.id,
        None,
        Some("A shared research specialist"),
        team::AgentStyle {
            display_name: Some("Research"),
            persona: Some("Explain sources clearly"),
        },
    )
    .await
    .unwrap();
    assert_eq!(edited.persona.as_deref(), Some("Explain sources clearly"));
    assert!(
        team::update_agent(
            &f.state.db,
            &f.outsider,
            &agent.id,
            Some("new-handle"),
            None,
            Default::default()
        )
        .await
        .is_err()
    );
    assert!(
        team::update_agent(
            &f.state.db,
            &f.member,
            &agent.id,
            None,
            Some("Unauthorized edit"),
            Default::default()
        )
        .await
        .is_err()
    );
    assert!(
        sharing::update_memory(
            &f.state.db,
            &f.member,
            &agent,
            sharing::NotesUpdate {
                expected_revision: 1,
                notes: vec!["Unauthorized publication".into()],
            }
        )
        .await
        .is_err()
    );
    let snapshot = access::RequestAccess::from_shared(manager);
    for class in [
        CredentialClass::UserOwned,
        CredentialClass::NyxidManagedMaster,
    ] {
        let payer = f
            .state
            .billing
            .owner_resolver()
            .resolve_for_assistant_execution(&f.outsider, &f.admin, class, Some(&snapshot))
            .await
            .unwrap();
        assert_eq!(payer.owner_id, f.outsider);
        assert!(
            f.state
                .billing
                .owner_resolver()
                .resolve_for_assistant_execution(&f.member, &f.admin, class, Some(&snapshot))
                .await
                .is_err()
        );
    }
    // Sharing must not change the established org, owner or guest payer.
    let (org_agent, _) = f.create().await;
    let org_access = access::resolve_agent_access(&f.state.db, &f.member, &org_agent)
        .await
        .unwrap();
    for (class, expected) in [
        (CredentialClass::UserOwned, &f.org),
        (CredentialClass::NyxidManagedMaster, &f.member),
    ] {
        let payer = f
            .state
            .billing
            .owner_resolver()
            .resolve_for_assistant_execution(&f.member, &f.org, class, org_access.as_deref())
            .await
            .unwrap();
        assert_eq!(&payer.owner_id, expected);
    }
    assert!(
        sharing::personal_owner(&f.state.db, &f.admin, &org_agent.id)
            .await
            .is_err()
    );
    let personal_bot = team::ensure_nyxbot(&f.state.db, &f.admin).await.unwrap();
    assert!(
        sharing::personal_owner(&f.state.db, &f.admin, &personal_bot.id)
            .await
            .is_err()
    );
    let owner_payer = f
        .state
        .billing
        .owner_resolver()
        .resolve_for_assistant_execution(&f.admin, &f.admin, CredentialClass::UserOwned, None)
        .await
        .unwrap();
    assert_eq!(owner_payer.owner_id, f.admin);
    f.revoke(&f.member).await;
    assert!(
        sharing::require(&f.state.db, &f.member, &agent)
            .await
            .is_err()
    );
    flags::set_platform_override(
        &f.state.db,
        sharing::FLAG,
        &flags::FlagTarget::Global,
        false,
        &f.admin,
    )
    .await
    .unwrap();
    assert!(
        sharing::require(&f.state.db, &f.outsider, &agent)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_snapshots_batch_queries_and_leave_owner_paths_unchanged() {
    let (f, _, reads) = Fixture::observed("shared_agent_queries").await;
    let agent = publish(&f, false).await;
    reads.store(0, Ordering::SeqCst);
    assert!(
        access::resolve_agent_access(&f.state.db, &f.admin, &agent)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    let one = sharing::load(&f.state.db, &f.outsider, None).await.unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), one.read_queries);
    assert_eq!(one.agents.len(), 1);
    // Grow the audience fan-in and the number of agents independently of queries.
    for n in 0..12 {
        let mut another = agent.clone();
        another.id = uuid::Uuid::new_v4().to_string();
        another.name = format!("researcher-{n}");
        f.state
            .db
            .collection::<crate::models::assistant_agent::AssistantAgent>(
                crate::models::assistant_agent::COLLECTION_NAME,
            )
            .insert_one(&another)
            .await
            .unwrap();
        sharing::update(
            &f.state.db,
            &f.admin,
            &another.id,
            sharing::SharingUpdate {
                expected_version: 0,
                enabled: true,
                shared_access: BTreeMap::new(),
                audience: vec![email_audience(
                    &format!("{}@example.com", f.outsider),
                    ShareRole::User,
                )],
            },
            true,
        )
        .await
        .unwrap();
    }
    reads.store(0, Ordering::SeqCst);
    let many = sharing::load(&f.state.db, &f.outsider, None).await.unwrap();
    assert_eq!(many.agents.len(), 13);
    assert_eq!(reads.load(Ordering::SeqCst), one.read_queries);
    let snapshot = &many.agents[&agent.id];
    assert!(!snapshot.matches(&f.outsider, &f.admin, "another-agent"));
    let row = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.outsider, &agent)
        .await
        .unwrap();
    let key = f.key(&row).await;
    let snapshot = access::RequestAccess::from_shared(snapshot.clone());
    reads.store(0, Ordering::SeqCst);
    access::key_services(&f.state.db, &key, Some(&snapshot))
        .await
        .unwrap();
    access::key_nodes(&f.state.db, &key, Some(&snapshot))
        .await
        .unwrap();
    assert_eq!(
        reads.load(Ordering::SeqCst),
        0,
        "reuse the loaded connection snapshot"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_private_threads_notes_and_retained_history() {
    let f = Fixture::new("shared_agent_privacy").await;
    let agent = publish(&f, true).await;
    pin_shared_test_skill(&f, &agent).await;
    team::remember(&f.state.db, &f.admin, &agent.id, "Owner private note", None)
        .await
        .unwrap();
    sharing::update_memory(
        &f.state.db,
        &f.outsider,
        &agent,
        sharing::NotesUpdate {
            expected_revision: 1,
            notes: vec!["Published team note".into()],
        },
    )
    .await
    .unwrap();
    sharing::edit_private_note(
        &f.state.db,
        &f.outsider,
        &agent,
        Some("Recipient private note"),
        None,
    )
    .await
    .unwrap();
    let thread = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.outsider, &agent)
        .await
        .unwrap();
    assert!(thread.shared_agent_thread);
    assert_eq!(thread.user_id, f.outsider);
    let key = f.key(&thread).await;
    assert!(!key.allow_all_nodes && !key.allow_auto_connected_services);
    assert!(
        !key.scopes
            .contains(crate::mw::auth::ASSISTANT_ACCOUNT_SCOPE)
    );
    let projected = team::agent_for_conversation(&f.state.db, &thread)
        .await
        .unwrap();
    assert_eq!(projected.skills.len(), 1);
    assert_eq!(
        projected
            .skill_metadata
            .get("shared-skill")
            .map(|metadata| metadata.description.as_str()),
        Some("Pinned shared guidance")
    );
    let notes: Vec<_> = projected.memory.iter().map(|n| n.text.as_str()).collect();
    assert_eq!(notes, ["Published team note", "Recipient private note"]);
    assert!(
        super::assistant_nyxagent::get(&f.state.db, &f.admin, &thread.id)
            .await
            .is_err()
    );
    assert!(
        sharing::private_notes(&f.state.db, &f.admin, &agent.id)
            .await
            .unwrap()
            .is_empty()
    );
    let owner_thread =
        team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.admin, &agent)
            .await
            .unwrap();
    for (actor, conversation, text) in [
        (&f.admin, &owner_thread.id, "Owner private reply"),
        (&f.outsider, &thread.id, "Recipient private reply"),
    ] {
        f.state.db.collection::<Document>("assistant_messages").insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "user_id": actor, "conversation_id": conversation,
            "seq": 1_i64, "turn_id": uuid::Uuid::new_v4().to_string(), "role": "assistant",
            "text": text, "status": "completed", "created_at": mongodb::bson::DateTime::now(),
        }).await.unwrap();
    }
    let summaries = team::tool_delegation_summaries(&f.state.db, &f.outsider, false, 300)
        .await
        .unwrap();
    let shared = summaries
        .iter()
        .find(|summary| summary.id == agent.id)
        .unwrap();
    assert_eq!(
        shared.last_reply.as_ref().unwrap().text,
        "Recipient private reply"
    );
    assert_eq!(
        shared.delegation_name(),
        format!("{}/{}", f.admin, agent.name)
    );
    sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: false,
            audience: Vec::new(),
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await
    .unwrap();
    assert!(
        access::resolve_credential_access(&f.state.db, &key)
            .await
            .is_err()
    );
    assert!(
        super::assistant_nyxagent::get(&f.state.db, &f.outsider, &thread.id)
            .await
            .is_ok()
    );
    assert!(
        team::agent_for_conversation(&f.state.db, &thread)
            .await
            .is_err()
    );
    assert!(
        !team::tool_delegation_summaries(&f.state.db, &f.outsider, false, 300)
            .await
            .unwrap()
            .iter()
            .any(|summary| summary.id == agent.id)
    );
    team::destroy(&f.state.db, &f.admin, &agent.id)
        .await
        .unwrap();
    team::purge(&f.state.db, &f.admin, &agent.id).await.unwrap();
    assert!(
        super::assistant_nyxagent::get(&f.state.db, &f.outsider, &thread.id)
            .await
            .is_ok()
    );
    assert_eq!(
        sharing::private_notes(&f.state.db, &f.outsider, &agent.id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>("assistant_agent_shares")
            .count_documents(doc! {"agent_id": &agent.id})
            .await
            .unwrap(),
        0
    );
    f.state
        .db
        .collection::<Document>("assistant_agent_consents")
        .insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "agent_id": &agent.id,
            "actor_user_id": &f.outsider, "conversation_id": &thread.id,
        })
        .await
        .unwrap();
    super::assistant_nyxagent::delete(&f.state.db, &f.outsider, &thread.id)
        .await
        .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<Document>("assistant_agent_consents")
            .count_documents(doc! {"conversation_id": &thread.id})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_skills_are_pinned_acl_scoped_and_owner_only() {
    use super::mcp_service::operation_effects;

    let f = Fixture::new("shared_agent_skills").await;
    let agent = publish(&f, true).await;
    pin_shared_test_skill(&f, &agent).await;
    let service = f.service(&f.admin, "skill-acl").await;
    f.state
        .db
        .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"grants.service_ids": [&service]}},
        )
        .await
        .unwrap();

    let thread = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.outsider, &agent)
        .await
        .unwrap();
    let projected = team::agent_for_conversation(&f.state.db, &thread)
        .await
        .unwrap();
    assert_eq!(projected.skills.len(), 1);
    assert_eq!(projected.skills[0].skill_id, "shared-skill");
    assert_eq!(
        projected
            .skill_metadata
            .get("shared-skill")
            .map(|metadata| metadata.description.as_str()),
        Some("Pinned shared guidance")
    );

    // The skill is guidance only. Its projection cannot grant an owner service
    // that the live shared_access map withholds.
    let access = sharing::require(&f.state.db, &f.outsider, &agent)
        .await
        .unwrap();
    let write = operation_effects(&reqwest::Method::POST, Default::default());
    assert_eq!(access.level(&service), None);
    assert!(!sharing::permits(access.level(&service), write));

    let selection = crate::models::assistant_agent::SkillSelection {
        expected_revision: 0,
        skills: Vec::new(),
    };
    for actor in [&f.outsider, &f.member] {
        let reader = crate::handlers::agent_skills::Reader {
            state: &f.state,
            person: actor,
            thread_key: None,
            scopes: None,
            chat: None,
        };
        assert!(
            super::agent_skill_service::set(
                &f.state.db,
                &reader,
                actor,
                &agent.id,
                &selection,
                true,
            )
            .await
            .is_err(),
            "shared managers and users cannot change pinned skills"
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_grant_removal_does_not_restore_policy() {
    let f = Fixture::new("shared_agent_regrant").await;
    let mut agent = publish(&f, false).await;
    let service = f.service(&f.admin, "shared-service").await;
    agent.grants.service_ids.push(service.clone());
    f.state
        .db
        .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"grants.service_ids": [&service]}},
        )
        .await
        .unwrap();
    sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience: vec![
                sharing::Audience {
                    subject_kind: ShareSubject::Person,
                    subject_id: f.outsider.clone(),
                    role: ShareRole::User,
                },
                sharing::Audience {
                    subject_kind: ShareSubject::Org,
                    subject_id: f.org.clone(),
                    role: ShareRole::User,
                },
            ],
            shared_access: BTreeMap::from([(service.clone(), GuestAccess::Read)]),
        },
        true,
    )
    .await
    .unwrap();
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    sharing::remove_grants_in_session(
        &f.state.db,
        &agent.id,
        std::slice::from_ref(&service),
        &mut session,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    assert!(
        !sharing::policy(&f.state.db, &agent)
            .await
            .unwrap()
            .shared_access
            .contains_key(&service)
    );
    assert!(
        sharing::require(&f.state.db, &f.outsider, &agent)
            .await
            .unwrap()
            .connections()
            .is_empty()
    );
    f.state.db.drop().await.unwrap();
}

#[test]
fn shared_agent_adapter_withholds_absent_levels_and_uses_guest_effects() {
    use super::mcp_service::{McpDurableEndpointMetadata, operation_effects};
    let read = operation_effects(&reqwest::Method::GET, Default::default());
    let write = operation_effects(&reqwest::Method::DELETE, Default::default());
    assert!(!sharing::permits(None, read));
    assert!(sharing::permits(Some(GuestAccess::Read), read));
    assert!(!sharing::permits(Some(GuestAccess::Use), write));
    assert!(sharing::permits(Some(GuestAccess::All), write));
    let remote = operation_effects(
        &reqwest::Method::DELETE,
        McpDurableEndpointMetadata {
            changes_existing: Some(false),
            ..Default::default()
        },
    );
    assert!(!sharing::permits(Some(GuestAccess::Use), remote));
}

#[tokio::test]
async fn shared_agent_consent_once_race_always_revoke_and_full_access() {
    use super::shared_agent_consent as consent;
    use crate::models::{assistant_sharing::ConsentDecision, user_service::UserService};
    let f = Fixture::new("shared_agent_consent").await;
    let agent = publish(&f, false).await;
    let owner_connection = f.service(&f.admin, "owner-calendar").await;
    let actor_connection = f.service(&f.outsider, "my-calendar").await;
    let catalog = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<Document>("downstream_services")
        .insert_one(doc! {"_id": &catalog, "is_active": true})
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_many(
            doc! {"_id": {"$in": [&owner_connection, &actor_connection]}},
            doc! {"$set": {"catalog_service_id": &catalog, "identity_propagation_mode": "none"}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"grants.service_ids": [&owner_connection]}},
        )
        .await
        .unwrap();
    let row = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.outsider, &agent)
        .await
        .unwrap();
    let row =
        super::assistant_authority_tests::pin_assignment(&f.state.db, &f.outsider, &row.id, None)
            .await;
    let chat = super::assistant_authority_tests::chat_for_row(&f.state, &f.outsider, &row).await;
    assert!(!chat.guest, "shared recipients never use the guest lane");
    assert!(super::assistant_service_execution::is_shared_chat(&chat));
    assert!(!super::assistant_service_execution::owner_model_route_allowed(&chat));
    let access = chat.org_agent_access.as_ref().unwrap().shared().unwrap();
    let service = f
        .state
        .db
        .collection::<UserService>("user_services")
        .find_one(doc! {"_id": &actor_connection})
        .await
        .unwrap()
        .unwrap();
    let binding = super::assistant_service_execution::connection_binding(
        &f.state.db,
        &service.id,
        &chat.api_key_id,
    )
    .await
    .unwrap();
    let operation =
        serde_json::json!({"kind":"http","method":"POST","path":"/events","body":"digest-only"});
    let mut wrong_chat = chat.clone();
    wrong_chat.agent_id = uuid::Uuid::new_v4().to_string();
    assert!(
        consent::authorize(
            &f.state.db,
            &wrong_chat,
            access,
            &service,
            &binding,
            &operation
        )
        .await
        .is_err()
    );
    wrong_chat = chat.clone();
    wrong_chat.turn_stopped = true;
    assert!(
        consent::authorize(
            &f.state.db,
            &wrong_chat,
            access,
            &service,
            &binding,
            &operation
        )
        .await
        .is_err()
    );
    let (first, second) = tokio::join!(
        consent::authorize(&f.state.db, &chat, access, &service, &binding, &operation),
        consent::authorize(&f.state.db, &chat, access, &service, &binding, &operation)
    );
    let card = first.unwrap().err().unwrap();
    assert_eq!(
        card["consent_id"],
        second.unwrap().err().unwrap()["consent_id"]
    );
    let id = card["consent_id"].as_str().unwrap();
    assert!(
        consent::decide(&f.state.db, &f.admin, id, ConsentDecision::Once)
            .await
            .is_err()
    );
    consent::decide(&f.state.db, &f.outsider, id, ConsentDecision::Once)
        .await
        .unwrap();
    assert!(
        consent::prompt(&f.state.db, &chat, access, &service, &binding, &operation)
            .await
            .unwrap()
            .is_none()
    );
    let pending_action = f
        .state
        .db
        .collection::<Document>("assistant_agent_consents")
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending_action.get_str("decision").unwrap(), "once");
    let (a, b) = tokio::join!(
        consent::authorize(&f.state.db, &chat, access, &service, &binding, &operation),
        consent::authorize(&f.state.db, &chat, access, &service, &binding, &operation)
    );
    assert_eq!(
        usize::from(a.is_ok_and(|r| r.is_ok())) + usize::from(b.is_ok_and(|r| r.is_ok())),
        1
    );
    let card = consent::authorize(&f.state.db, &chat, access, &service, &binding, &operation)
        .await
        .unwrap()
        .err()
        .unwrap();
    let id = card["consent_id"].as_str().unwrap();
    consent::decide(&f.state.db, &f.outsider, id, ConsentDecision::Always)
        .await
        .unwrap();
    let permit = consent::authorize(&f.state.db, &chat, access, &service, &binding, &operation)
        .await
        .unwrap()
        .ok()
        .unwrap();
    permit
        .revalidate(&f.state.db, access, &f.outsider)
        .await
        .unwrap();
    consent::decide(&f.state.db, &f.outsider, id, ConsentDecision::Revoked)
        .await
        .unwrap();
    assert!(
        permit
            .revalidate(&f.state.db, access, &f.outsider)
            .await
            .is_err()
    );
    let mut model = serde_json::json!({"kind": "raw_inference", "method": "POST", "path": "/responses", "model": "example", "input_admission": "first"});
    let card = consent::authorize(&f.state.db, &chat, access, &service, &binding, &model)
        .await
        .unwrap()
        .err()
        .unwrap();
    consent::decide(
        &f.state.db,
        &f.outsider,
        card["consent_id"].as_str().unwrap(),
        ConsentDecision::Once,
    )
    .await
    .unwrap();
    assert!(
        consent::authorize(&f.state.db, &chat, access, &service, &binding, &model)
            .await
            .unwrap()
            .is_ok()
    );
    model["input_admission"] = "next".into();
    assert!(
        consent::authorize(&f.state.db, &chat, access, &service, &binding, &model)
            .await
            .unwrap()
            .is_ok()
    );
    let mut next_turn = chat.clone();
    next_turn.turn_id = Some(uuid::Uuid::new_v4().to_string());
    assert!(
        consent::authorize(&f.state.db, &next_turn, access, &service, &binding, &model)
            .await
            .unwrap()
            .is_err()
    );
    let skip = serde_json::json!({"kind":"http","method":"GET","path":"/skip"});
    let card = consent::authorize(&f.state.db, &chat, access, &service, &binding, &skip)
        .await
        .unwrap()
        .err()
        .unwrap();
    consent::decide(
        &f.state.db,
        &f.outsider,
        card["consent_id"].as_str().unwrap(),
        ConsentDecision::Skip,
    )
    .await
    .unwrap();
    let refused = consent::authorize(&f.state.db, &chat, access, &service, &binding, &skip)
        .await
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(refused["error"], "shared_connection_skipped");
    let later = consent::authorize(&f.state.db, &next_turn, access, &service, &binding, &skip)
        .await
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(later["error"], "shared_connection_consent_required");

    // A pinned-skill change changes the behavior digest. The durable Always
    // row remains auditable, but it cannot authorize the changed agent.
    let card = consent::authorize(&f.state.db, &chat, access, &service, &binding, &operation)
        .await
        .unwrap()
        .err()
        .unwrap();
    let pin_consent_id = card["consent_id"].as_str().unwrap();
    consent::decide(
        &f.state.db,
        &f.outsider,
        pin_consent_id,
        ConsentDecision::Always,
    )
    .await
    .unwrap();
    let pin_permit = consent::authorize(&f.state.db, &chat, access, &service, &binding, &operation)
        .await
        .unwrap()
        .unwrap();
    let (skill, _) = shared_test_skill();
    f.state
        .db
        .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {
                "skills": mongodb::bson::to_bson(&vec![skill]).unwrap(),
                "skills_revision": 1_i64,
            }},
        )
        .await
        .unwrap();
    let changed_access = sharing::require(&f.state.db, &f.outsider, &agent)
        .await
        .unwrap();
    assert!(
        pin_permit
            .revalidate(&f.state.db, &changed_access, &f.outsider)
            .await
            .is_err()
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME)
            .find_one(doc! {"_id": pin_consent_id})
            .await
            .unwrap()
            .unwrap()
            .get_str("decision")
            .unwrap(),
        "always"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_http_policy_and_final_egress_revoke_owner_connection() {
    use super::{assistant_service_execution as execution, proxy_service};
    let f = Fixture::new("shared_agent_http_egress").await;
    let agent = publish(&f, false).await;
    let service = f.service(&f.admin, "shared-http").await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id": &service},
            doc! {"$set": {"identity_propagation_mode": "none"}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"grants.service_ids": [&service]}},
        )
        .await
        .unwrap();
    sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience: vec![
                sharing::Audience {
                    subject_kind: ShareSubject::Person,
                    subject_id: f.outsider.clone(),
                    role: ShareRole::User,
                },
                sharing::Audience {
                    subject_kind: ShareSubject::Org,
                    subject_id: f.org.clone(),
                    role: ShareRole::User,
                },
            ],
            shared_access: BTreeMap::from([(service.clone(), GuestAccess::Read)]),
        },
        true,
    )
    .await
    .unwrap();
    let row = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.outsider, &agent)
        .await
        .unwrap();
    let row =
        super::assistant_authority_tests::pin_assignment(&f.state.db, &f.outsider, &row.id, None)
            .await;
    let key = f.key(&row).await;
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    let canonical = super::proxy_authorization::CanonicalPath::from_mcp_built("/records").unwrap();
    for (method, allowed) in [("GET", true), ("POST", false), ("DELETE", false)] {
        let result = super::agent_operation_scope_service::check_non_mcp_context(
            &f.state.db,
            &auth,
            false,
            &service,
            None,
            method,
            Some(&canonical),
            Default::default(),
        )
        .await;
        assert_eq!(result.is_ok(), allowed, "{method}");
    }
    let resolved = proxy_service::resolve_proxy_target_by_user_service_id(
        &f.state.db,
        &f.state.encryption_keys,
        &f.outsider,
        &service,
        None,
        None,
        proxy_service::ProxyExecutionContext::new(None, f.state.platform_user_rate_limit)
            .with_assistant_access(auth.org_agent_access.as_deref()),
    )
    .await
    .unwrap()
    .unwrap();
    let headers = axum::http::HeaderMap::new();
    let decision = execution::check_http(
        &f.state,
        &auth,
        Some(&service),
        Some(&f.admin),
        &resolved.target,
        "GET",
        "/records",
        None,
        b"",
        None,
        None,
        &headers,
    )
    .await
    .unwrap();
    assert!(decision.refusal.is_none());
    let admission = decision.admission.unwrap();
    admission
        .revalidate(&f.state.db, &service, Some(&resolved.target))
        .await
        .unwrap();
    assert!(
        execution::check_http(
            &f.state,
            &auth,
            None,
            Some(&f.admin),
            &resolved.target,
            "GET",
            "/records",
            None,
            b"",
            None,
            None,
            &headers,
        )
        .await
        .is_err()
    );
    let chat = auth.assistant_chat.as_ref().unwrap();
    let (_, refused) = crate::handlers::assistant_team::execute_tool(
        &f.state,
        chat,
        "nyxid__list_subagents",
        &serde_json::json!({}),
    )
    .await;
    assert!(refused);
    sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 2,
            enabled: false,
            audience: Vec::new(),
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await
    .unwrap();
    // A request admitted before unshare cannot dispatch after it commits.
    assert!(
        admission
            .revalidate(&f.state.db, &service, Some(&resolved.target))
            .await
            .is_err()
    );
    assert!(
        crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_rest_and_mcp_dispatch_reads_and_refuse_without_provider_effects() {
    use super::billing::route_inventory::{BillingIngress, BillingRoutePolicy};
    use axum::{
        body::Body,
        extract::{Path, State},
        http::Request,
    };
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    let f = Fixture::new("shared_agent_transport_dispatch").await;
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/records"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"source":"owner"})))
        .expect(2)
        .mount(&upstream)
        .await;
    let agent = publish(&f, false).await;
    let service = f.service(&f.admin, "shared-transport").await;
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! {"user_id": &f.admin, "label": "shared-transport"},
            doc! {"$set": {"url": upstream.uri()}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"grants.service_ids": [&service]}},
        )
        .await
        .unwrap();
    sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience: vec![
                sharing::Audience {
                    subject_kind: ShareSubject::Person,
                    subject_id: f.outsider.clone(),
                    role: ShareRole::User,
                },
                sharing::Audience {
                    subject_kind: ShareSubject::Org,
                    subject_id: f.org.clone(),
                    role: ShareRole::User,
                },
            ],
            shared_access: BTreeMap::from([(service.clone(), GuestAccess::Read)]),
        },
        true,
    )
    .await
    .unwrap();
    let row = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.outsider, &agent)
        .await
        .unwrap();
    let row =
        super::assistant_authority_tests::pin_assignment(&f.state.db, &f.outsider, &row.id, None)
            .await;
    let key = f.key(&row).await;
    let credential = super::assistant_agent_credential_service::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.outsider,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let listed = mcp(&f.state, &credential.raw_key, "tools/list", json!({})).await;
    let tool = listed["result"]["tools"]
        .as_array()
        .expect("shared tools listing")
        .iter()
        .find_map(|tool| {
            tool["name"]
                .as_str()
                .filter(|name| name.ends_with("__request"))
        })
        .expect("shared connection generic tool")
        .to_owned();

    for method in ["GET", "DELETE"] {
        let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
            .await
            .unwrap();
        let mut request = Request::builder()
            .uri(format!("/proxy/{service}/records?_nyxid_via={service}"))
            .method(method)
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
        let response = Box::pin(crate::handlers::proxy::proxy_request_by_slug(
            State(f.state.clone()),
            auth,
            Default::default(),
            Path(("shared-transport".into(), "records".into())),
            request,
        ))
        .await;
        if method == "GET" {
            assert_eq!(
                response
                    .unwrap_or_else(|error| panic!("shared REST GET failed: {error}"))
                    .status(),
                axum::http::StatusCode::OK
            );
        } else {
            assert!(response.is_err());
        }
        let response = mcp(
            &f.state,
            &credential.raw_key,
            "tools/call",
            json!({"name":tool,"arguments":{"method":method,"path":"/records"}}),
        )
        .await;
        assert_eq!(
            response["result"]["isError"].as_bool().unwrap_or(false),
            method != "GET",
            "{response}"
        );
        assert!(response.get("error").is_none(), "{response}");
    }
    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
    sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 2,
            enabled: false,
            audience: Vec::new(),
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await
    .unwrap();
    assert!(
        crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
            .await
            .is_err()
    );
    let response = mcp(
        &f.state,
        &credential.raw_key,
        "tools/call",
        json!({"name":tool,"arguments":{"method":"GET","path":"/records"}}),
    )
    .await;
    assert!(
        response.get("error").is_some() || response["result"]["isError"] == true,
        "{response}"
    );
    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_catalog_connections_exclude_internal_and_preserve_account_selection() {
    let f = Fixture::new("shared_catalog_boundaries").await;
    let agent = publish(&f, false).await;
    let catalog = uuid::Uuid::new_v4().to_string();
    let mut catalog_row = crate::models::downstream_service::test_helpers::dummy_service();
    catalog_row.id = catalog.clone();
    catalog_row.auth_method = "bearer".into();
    catalog_row.credential_encrypted = vec![1];
    catalog_row.platform_key = Some(crate::models::downstream_service::PlatformKeyConfig {
        enabled: true,
        audience: crate::models::downstream_service::PlatformKeyAudience::Public,
        allowed_owner_ids: Vec::new(),
    });
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(catalog_row)
        .await
        .unwrap();
    f.state
        .db
        .collection("service_endpoints")
        .insert_one(crate::test_utils::test_service_endpoint(
            &catalog, "records", "GET", "/records",
        ))
        .await
        .unwrap();
    let owner_service = f.service(&f.admin, "same-calendar").await;
    let actor_service = f.service(&f.outsider, "same-calendar").await;
    f.state.db.collection::<Document>("user_services").update_many(
        doc! {"_id": {"$in": [&owner_service, &actor_service]}},
        doc! {"$set": {"catalog_service_id": &catalog, "identity_propagation_mode": "none", "credential_binding": "platform"}},
    ).await.unwrap();
    f.state
        .db
        .collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"grants.platform_service_ids": [&catalog]}},
        )
        .await
        .unwrap();
    sharing::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 1,
            enabled: true,
            audience: vec![
                sharing::Audience {
                    subject_kind: ShareSubject::Person,
                    subject_id: f.outsider.clone(),
                    role: ShareRole::User,
                },
                sharing::Audience {
                    subject_kind: ShareSubject::Org,
                    subject_id: f.org.clone(),
                    role: ShareRole::User,
                },
            ],
            shared_access: BTreeMap::from([(catalog.clone(), GuestAccess::Read)]),
        },
        true,
    )
    .await
    .unwrap();
    let snapshot = sharing::require(&f.state.db, &f.outsider, &agent)
        .await
        .unwrap();
    assert_eq!(snapshot.catalog_connections(&catalog, false).len(), 2);
    assert_eq!(snapshot.level(&owner_service), Some(GuestAccess::Read));
    assert!(
        snapshot.connection_for_slug("same-calendar").is_err(),
        "ambiguous accounts require an exact connection"
    );
    assert!(snapshot.connection_for_slug("unknown").is_err());
    let owner_metadata = snapshot.discovery_metadata(&owner_service).unwrap();
    let actor_metadata = snapshot.discovery_metadata(&actor_service).unwrap();
    assert_eq!(owner_metadata["credential_source"], "owner_shared");
    assert_eq!(owner_metadata["consent_required"], false);
    assert_eq!(actor_metadata["credential_source"], "actor_personal");
    assert_eq!(actor_metadata["consent_required"], true);
    let access = access::RequestAccess::from_shared(snapshot.clone());
    let listed = super::mcp_service::load_operation_catalog(
        &f.state.db,
        &f.state.node_ws_manager,
        &f.outsider,
        super::mcp_service::NodeScope::Allowed(&[]),
        super::mcp_service::ServiceScope::Organization { access: &access },
    )
    .await
    .unwrap();
    let owner_tool = listed
        .services
        .iter()
        .find(|s| s.service_id == owner_service)
        .unwrap();
    let actor_tool = listed
        .services
        .iter()
        .find(|s| s.service_id == actor_service)
        .unwrap();
    assert_ne!(owner_tool.service_slug, actor_tool.service_slug);
    assert!(
        owner_tool
            .service_name
            .contains("owner's shared connection")
    );
    assert!(actor_tool.service_name.contains("consent required"));
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id": &owner_service},
            doc! {"$set": {"credential_binding": null}},
        )
        .await
        .unwrap();
    let snapshot = sharing::require(&f.state.db, &f.outsider, &agent)
        .await
        .unwrap();
    assert!(
        !snapshot.connections().contains_key(&owner_service),
        "a platform grant never borrows owner BYOK"
    );
    assert_eq!(
        snapshot.connection_for_slug("same-calendar").unwrap().id,
        actor_service
    );
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id": &catalog},
            doc! {"$set": {"service_category": "internal"}},
        )
        .await
        .unwrap();
    let snapshot = sharing::require(&f.state.db, &f.outsider, &agent)
        .await
        .unwrap();
    assert!(
        snapshot.connections().is_empty(),
        "account services are not shared"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_agent_actor_connection_dispatch_requires_consumable_private_consent() {
    use crate::models::assistant_sharing::{AgentConsent, ConsentDecision};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let f = Fixture::new("shared_actor_dispatch_consent").await;
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/records"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"source":"actor"})))
        .expect(1)
        .mount(&upstream)
        .await;
    let agent = publish(&f, false).await;
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = uuid::Uuid::new_v4().to_string();
    catalog.requires_user_credential = true;
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&catalog)
        .await
        .unwrap();
    f.state
        .db
        .collection("service_endpoints")
        .insert_one(crate::test_utils::test_service_endpoint(
            &catalog.id,
            "records",
            "GET",
            "/records",
        ))
        .await
        .unwrap();
    let owner_service = f.service(&f.admin, "calendar").await;
    let actor_service = f.service(&f.outsider, "calendar").await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_many(
            doc! {"_id": {"$in": [&owner_service, &actor_service]}},
            doc! {"$set": {"catalog_service_id": &catalog.id}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! {"user_id": &f.outsider, "label": "calendar"},
            doc! {"$set": {"url": upstream.uri()}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"grants.service_ids": [&owner_service]}},
        )
        .await
        .unwrap();
    // The owner connection stays withheld. Its catalog grants the eligible
    // alternative, but no actor credential may reach the provider yet.
    let row = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.outsider, &agent)
        .await
        .unwrap();
    let row =
        super::assistant_authority_tests::pin_assignment(&f.state.db, &f.outsider, &row.id, None)
            .await;
    let credential = super::assistant_agent_credential_service::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.outsider,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let listed = mcp(&f.state, &credential.raw_key, "tools/list", json!({})).await;
    let tool = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["shared_connection"]["connection_id"] == actor_service)
        .unwrap();
    assert_eq!(tool["shared_connection"]["consent_required"], true);
    assert!(
        !listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["shared_connection"]["connection_id"] == owner_service)
    );
    let args = json!({
        "name": tool["name"],
        "arguments": {}
    });
    let first = mcp(&f.state, &credential.raw_key, "tools/call", args.clone()).await;
    assert_eq!(first["result"]["isError"], true, "{first}");
    assert!(upstream.received_requests().await.unwrap().is_empty());
    let consent = f
        .state
        .db
        .collection::<AgentConsent>("assistant_agent_consents")
        .find_one(doc! {"actor_user_id": &f.outsider, "agent_id": &agent.id, "decision": "pending"})
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("consent row missing after {first}"));
    super::shared_agent_consent::decide(
        &f.state.db,
        &f.outsider,
        &consent.id,
        ConsentDecision::Once,
    )
    .await
    .unwrap();
    let allowed = mcp(&f.state, &credential.raw_key, "tools/call", args.clone()).await;
    assert!(
        !allowed["result"]["isError"].as_bool().unwrap_or(false),
        "{allowed}"
    );
    assert!(allowed.get("error").is_none(), "{allowed}");
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
    let replay = mcp(&f.state, &credential.raw_key, "tools/call", args).await;
    assert_eq!(replay["result"]["isError"], true, "{replay}");
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
    f.state.db.drop().await.unwrap();
}
