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
    for _ in 0..640 {
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
        if state
            .db
            .collection::<Document>("assistant_voice_sessions")
            .find_one(doc! {"user_id":owner,"fixture_voice":true,"purge_requested":true})
            .await
            .unwrap()
            .is_some()
        {
            // Exercise the real recovery claim before closing an expired
            // terminal session; a stored row's old lease cannot authorize close.
            for voice in
                crate::services::voice::session::claim_orphans(&state.db, "inventory-fixture")
                    .await
                    .unwrap()
            {
                assert_eq!(voice.user_id, owner);
                crate::services::voice::session::close(&state.db, &voice, "account_erasure", true)
                    .await
                    .unwrap();
            }
        }
    }
    panic!(
        "source did not drain: {:?}",
        status(&state.db, owner).await.unwrap()
    );
}

#[tokio::test]
async fn p2_2_quick_access_is_erased_with_the_person_without_changing_a_bystander() {
    let (db, owner) = tests::fixture().await;
    let other = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&other, UserType::Person))
        .await
        .unwrap();
    let items = vec![crate::models::user::QuickAccessItem::Agent {
        id: uuid::Uuid::new_v4().to_string(),
    }];
    for person in [&owner, &other] {
        crate::services::user_preferences_service::save_quick_access(&db, person, items.clone())
            .await
            .unwrap();
    }
    let before = db
        .collection::<Document>(USERS)
        .find_one(doc! { "_id": &other })
        .await
        .unwrap()
        .unwrap();
    let state = test_app_state(db.clone());
    accept(&db, &owner, &owner, None).await.unwrap();
    until_source_removed(&state, &owner).await;
    assert!(
        db.collection::<Document>(USERS)
            .find_one(doc! { "_id": &owner })
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.collection::<Document>(USERS)
            .find_one(doc! { "_id": &other })
            .await
            .unwrap()
            .unwrap(),
        before
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn personal_erasure_preserves_revocation_outbox_until_audit_delivery() {
    use crate::models::{oauth_broker_binding, refresh_token, security_effect_intent};
    use crate::services::{audit_service, oauth_broker_service, security_effect_service};

    let (db, owner) = super::tests::fixture().await;
    let state = test_app_state(db.clone());
    security_effect_service::ensure_indexes(&db).await.unwrap();
    let binding_id =
        oauth_broker_binding::hash_binding_id(&oauth_broker_binding::generate_binding_id());
    let now = bson::DateTime::now();
    let binding: oauth_broker_binding::OauthBrokerBinding = bson::from_document(doc! {
        "_id": &binding_id, "user_id": &owner, "client_id": "outbox-client",
        "refresh_token_jti": "outbox-refresh", "created_at": now, "revoked": false,
    })
    .unwrap();
    db.collection::<oauth_broker_binding::OauthBrokerBinding>(
        oauth_broker_binding::COLLECTION_NAME,
    )
    .insert_one(binding)
    .await
    .unwrap();
    let tokens = db.collection::<refresh_token::RefreshToken>(refresh_token::COLLECTION_NAME);
    for id in ["outbox-refresh", "acceptance-refresh"] {
        let token: refresh_token::RefreshToken = bson::from_document(doc! {
            "_id": id, "jti": id, "user_id": &owner, "client_id": "outbox-client",
            "session_id": null, "expires_at": now, "revoked": false,
            "replaced_by": null, "created_at": now,
        })
        .unwrap();
        tokens.insert_one(token).await.unwrap();
    }
    oauth_broker_service::revoke_binding_by_user_as(
        &db,
        &audit_service::AuditActor {
            user_id: owner.clone(),
            ip_address: Some("192.0.2.1".into()),
            user_agent: Some("fixture-agent".into()),
            api_key_id: None,
            api_key_name: None,
        },
        &binding_id,
        "user_revoked",
    )
    .await
    .unwrap();
    let intents = db.collection::<Document>(security_effect_intent::COLLECTION_NAME);
    let before = intents
        .find(doc! {})
        .sort(doc! {"_id":1})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(
        before.len(),
        2,
        "real revocation commits one audit and one notice"
    );
    let audit = before
        .iter()
        .find(|row| row.get_str("kind") == Ok("audit"))
        .unwrap();
    assert_eq!(
        audit.get_document("event").unwrap().get_str("user_id"),
        Ok(owner.as_str())
    );

    accept(
        &db,
        &owner,
        &owner,
        Some("machine-data-provider-revocation-v2"),
    )
    .await
    .unwrap();
    assert!(
        tokens
            .find_one(doc! {"_id":"acceptance-refresh"})
            .await
            .unwrap()
            .unwrap()
            .revoked
    );
    assert_eq!(
        intents.count_documents(doc! {}).await.unwrap(),
        2,
        "session acceptance revokes authority directly without another outbox occurrence"
    );
    until_source_removed(&state, &owner).await;
    assert!(
        tokens
            .find_one(doc! {"user_id":&owner})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.collection::<Document>(oauth_broker_binding::COLLECTION_NAME)
            .find_one(doc! {"user_id":&owner})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        intents
            .find(doc! {})
            .sort(doc! {"_id":1})
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
        before,
        "personal cleanup must preserve every immutable outbox occurrence"
    );

    security_effect_service::sweep(&db, &state.encryption_keys)
        .await
        .unwrap();
    let event_id = audit.get_document("event").unwrap().get_str("_id").unwrap();
    let chained = db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"_id":event_id})
        .await
        .unwrap()
        .unwrap();
    assert!(chained.get_i64("seq").unwrap() > 0);
    assert_eq!(chained.get_str("user_id"), Ok(owner.as_str()));
    assert!(
        intents
            .find_one(doc! {"kind":"audit"})
            .await
            .unwrap()
            .is_none()
    );
    db.drop().await.unwrap();
}

fn replace_owner(value: &mut bson::Bson, source_owner: &str, target_owner: &str) {
    match value {
        bson::Bson::String(value) if value == source_owner => {
            *value = target_owner.to_owned();
        }
        bson::Bson::Document(document) => {
            let keys = document.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                if let Some(value) = document.get_mut(&key) {
                    replace_owner(value, source_owner, target_owner);
                }
            }
        }
        bson::Bson::Array(values) => {
            for value in values {
                replace_owner(value, source_owner, target_owner);
            }
        }
        _ => {}
    }
}

fn seed_from_inventory_filter(
    filter: &Document,
    source_owner: &str,
    target_owner: &str,
    id: &str,
) -> Document {
    fn copy(document: &Document, source_owner: &str, target_owner: &str) -> Document {
        let mut output = Document::new();
        for (key, value) in document {
            if key.starts_with('$') {
                let bson::Bson::Array(values) = value else {
                    continue;
                };
                if let Some(bson::Bson::Document(first)) = values.first() {
                    output.extend(copy(first, source_owner, target_owner));
                }
                continue;
            }
            let value = match value {
                bson::Bson::Document(operators) => {
                    if let Some(value) = operators.get("$in").and_then(|value| value.as_array()) {
                        value
                            .first()
                            .cloned()
                            .unwrap_or(bson::Bson::String("seed".into()))
                    } else if operators.get_bool("$exists").ok() == Some(false) {
                        continue;
                    } else if operators.contains_key("$ne") {
                        bson::Bson::String("seed-not-equal".into())
                    } else if operators.contains_key("$type") {
                        match operators.get_str("$type").unwrap_or_default() {
                            "binData" => bson::Bson::Binary(bson::Binary {
                                subtype: bson::spec::BinarySubtype::Generic,
                                bytes: vec![1],
                            }),
                            "date" => bson::Bson::DateTime(bson::DateTime::now()),
                            _ => bson::Bson::String("seed".into()),
                        }
                    } else if let Some(value) = operators.get("$eq") {
                        value.clone()
                    } else {
                        bson::Bson::Document(copy(operators, source_owner, target_owner))
                    }
                }
                value => value.clone(),
            };
            let mut value = value;
            replace_owner(&mut value, source_owner, target_owner);
            crate::services::personal_attribution_service::test_support::put(
                &mut output,
                key,
                value,
            );
        }
        output
    }

    let mut row = copy(filter, source_owner, target_owner);
    row.insert("_id", id);
    row.insert("fixture_marker", "registry-table");
    row
}

fn filter_has_owner_id(filter: &Document, owner: &str) -> bool {
    filter.iter().any(|(key, value)| {
        if key == "_id" && value.as_str() == Some(owner) {
            return true;
        }
        match value {
            bson::Bson::Document(document) => filter_has_owner_id(document, owner),
            bson::Bson::Array(values) => values.iter().any(|value| {
                value
                    .as_document()
                    .is_some_and(|document| filter_has_owner_id(document, owner))
            }),
            _ => false,
        }
    })
}

fn assert_owner_row_is_absent_or_redacted(
    row: Option<Document>,
    entry: &crate::services::personal_data_registry::Entry,
    owner: &str,
) {
    match entry.erasure {
        crate::services::personal_data_registry::Erasure::Redacted
            if !super::local::inventory(owner)
                .iter()
                .any(|(name, _)| *name == entry.collection) =>
        {
            let row = row.expect("redacted shared row must remain");
            let encoded = serde_json::to_string(&row).unwrap();
            assert!(!encoded.contains("deleted@example.test"));
            assert!(!encoded.contains("Deleted Person"));
            if entry.collection == "invite_codes" {
                assert_eq!(row.get_i32("used_count"), Ok(1));
                assert_eq!(row.get_array("usages").unwrap().len(), 1);
                assert_eq!(
                    row.get_array("usages").unwrap()[0]
                        .as_document()
                        .unwrap()
                        .get_str("user_id"),
                    Ok(owner)
                );
            }
        }
        _ => assert!(row.is_none(), "target row survived in {}", entry.collection),
    }
}

/// Expand disjunctive selectors so each independently selectable owner field
/// must work. A row with every owner field populated would hide a broken branch.
fn owner_branches(filter: &Document) -> Vec<Document> {
    let mut branches = vec![Document::new()];
    for (key, value) in filter {
        if matches!(key.as_str(), "$or" | "$and") {
            let clauses = value.as_array().unwrap();
            let groups = if key == "$or" {
                vec![
                    clauses
                        .iter()
                        .flat_map(|clause| owner_branches(clause.as_document().unwrap()))
                        .collect::<Vec<_>>(),
                ]
            } else {
                clauses
                    .iter()
                    .map(|clause| owner_branches(clause.as_document().unwrap()))
                    .collect()
            };
            for choices in groups {
                branches = branches
                    .iter()
                    .flat_map(|base| {
                        choices.iter().map(|choice| {
                            let mut row = base.clone();
                            row.extend(choice.clone());
                            row
                        })
                    })
                    .collect();
            }
        } else {
            for branch in &mut branches {
                branch.insert(key, value.clone());
            }
        }
    }
    branches
}

fn settled_durable_fixture(collection: &str, owner: &str, id: &str) -> Option<Document> {
    use crate::models::{
        durable_operation_execution::{DurableExecutionStatus, DurableOperationExecution},
        durable_operation_grant::{
            DurableOperationConstraints, DurableOperationGrant, DurableReplayPolicy,
        },
    };
    let now = chrono::Utc::now();
    Some(match collection {
        "durable_operation_executions" => bson::to_document(&DurableOperationExecution {
            id: id.into(),
            grant_id: "settled-grant".into(),
            operation_id: "settled-operation".into(),
            api_key_id: "settled-key".into(),
            user_id: owner.into(),
            user_service_id: "settled-service".into(),
            endpoint_id: "settled-endpoint".into(),
            contract_digest: "settled-contract".into(),
            request_digest: "personal-request".into(),
            status: DurableExecutionStatus::Completed,
            downstream_attempts: 1,
            node_id: None,
            response_status: Some(200),
            terminal_detail: None,
            dispatched_at: Some(now),
            terminal_at: Some(now),
            created_at: now,
            updated_at: now,
        })
        .unwrap(),
        "durable_operation_grants" => bson::to_document(&DurableOperationGrant {
            id: id.into(),
            user_id: owner.into(),
            api_key_id: "settled-key".into(),
            user_service_id: "settled-service".into(),
            endpoint_id: "settled-endpoint".into(),
            method: "POST".into(),
            normalized_path_template: "/operation".into(),
            contract_digest: "settled-contract".into(),
            constraints: DurableOperationConstraints::default(),
            valid_from: now - chrono::Duration::hours(1),
            expires_at: now,
            total_limit: 1,
            total_used: 1,
            window: None,
            window_started_at: None,
            window_used: 0,
            replay_policy: DurableReplayPolicy::NonReplayable,
            client_audit_binding: None,
            revoked_at: Some(now),
            revoked_by: Some(owner.into()),
            state_version: 1,
            created_by: owner.into(),
            reauthorized_from: None,
            created_at: now,
            updated_at: now,
        })
        .unwrap(),
        _ => return None,
    })
}

/// Registry-driven behavioral coverage. The inventory is the table of cases,
/// so adding a new local predicate automatically adds a target/bystander case.
#[tokio::test]
async fn registry_table_drives_worker_erasure_and_preserves_bystanders() {
    let (db, owner) = tests::fixture().await;
    let state = test_app_state(db.clone());
    let (clients, server) = crate::telemetry::erasure::test_support::clients().await;
    super::super::telemetry_erasure_service::register_destinations(&db, &clients)
        .await
        .unwrap();
    let other = uuid::Uuid::new_v4().to_string();
    let org = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_many([
            test_user(&other, UserType::Person),
            test_user(&org, UserType::Org),
        ])
        .await
        .unwrap();
    accept(&db, &owner, &owner, None).await.unwrap();

    let mut target_rows = Vec::new();
    let mut bystander_rows = Vec::new();
    let mut org_rows = Vec::new();
    let mut retained_rows = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let cases = super::local::inventory(&owner)
        .into_iter()
        .flat_map(|(collection, filter)| {
            owner_branches(&filter)
                .into_iter()
                .map(move |filter| (collection, filter))
        });
    for (index, (collection, filter)) in cases.enumerate() {
        let target_id = format!("registry-target-{index}");
        let bystander_id = format!("registry-bystander-{index}");
        let target = seed_from_inventory_filter(&filter, &owner, &owner, &target_id);
        let bystander = seed_from_inventory_filter(&filter, &owner, &other, &bystander_id);
        // `_id` owner selectors intentionally use the account id. Keep the
        // generated ids for every other collection so repeated inventory
        // predicates cannot collide.
        let target = if filter_has_owner_id(&filter, &owner) {
            let mut row = target;
            row.insert("_id", owner.clone());
            row
        } else {
            target
        };
        let bystander = if filter_has_owner_id(&filter, &owner) {
            let mut row = bystander;
            row.insert("_id", other.clone());
            row
        } else {
            bystander
        };
        let org_row = if filter_has_owner_id(&filter, &owner) {
            let mut row =
                seed_from_inventory_filter(&filter, &owner, &org, &format!("registry-org-{index}"));
            row.insert("_id", org.clone());
            row
        } else {
            seed_from_inventory_filter(&filter, &owner, &org, &format!("registry-org-{index}"))
        };
        let target_key = target.get_str("_id").unwrap_or(&target_id).to_owned();
        if seen.insert((collection, target_key)) {
            db.collection::<Document>(collection)
                .insert_one(target.clone())
                .await
                .unwrap();
            db.collection::<Document>(collection)
                .insert_one(bystander.clone())
                .await
                .unwrap();
            target_rows.push((collection, target.get("_id").cloned().unwrap()));
            bystander_rows.push((
                collection,
                bystander.get("_id").cloned().unwrap(),
                bystander,
            ));
            org_rows.push((collection, org_row.get("_id").cloned().unwrap(), org_row));
        }
    }

    // Domain fixtures exercise each owner field independently. Real storage
    // fixtures supply fields required by typed lifecycle readers; the terminal
    // states below prove what each lifecycle retains rather than pretending
    // every domain is a hard-delete handler.
    let model_inputs: std::collections::BTreeMap<String, Vec<Document>> = serde_json::from_str(
        include_str!("../../../test_data/account-export-models.json"),
    )
    .unwrap();
    let mut domain_rows = Vec::new();
    let mut covered_fields = std::collections::BTreeSet::new();
    for (collection, fields) in super::local::inventory_owner_fields(&owner) {
        for field in fields {
            covered_fields.insert((collection, field));
        }
    }
    for entry in crate::services::personal_data_registry::entries() {
        use crate::services::personal_data_registry::{Erasure, OwnerSelector};
        let OwnerSelector::Fields(fields) = entry.owner else {
            continue;
        };
        for field in fields {
            match entry.erasure {
                Erasure::Retained(_) | Erasure::FeatureRetention(_) => {
                    covered_fields.insert((entry.collection, (*field).to_owned()));
                }
                Erasure::Domain(_) => {
                    covered_fields.insert((entry.collection, (*field).to_owned()));
                    if entry.collection == "users" {
                        continue;
                    }
                    for person in [&owner, &other, &org] {
                        let id = format!("domain-{}-{field}-{person}", entry.collection);
                        let mut row = settled_durable_fixture(entry.collection, person, &id)
                            .or_else(|| {
                                model_inputs
                                    .get(entry.collection)
                                    .map(|rows| rows[0].clone())
                            })
                            .unwrap_or_default();
                        // Never populate both disjunctive owner fields with
                        // the target: that would conceal a broken selector.
                        for owner_field in fields {
                            if !owner_field.contains('.') {
                                row.remove(*owner_field);
                            }
                        }
                        row.insert("_id", &id);
                        crate::services::personal_attribution_service::test_support::put(
                            &mut row,
                            field,
                            person.clone().into(),
                        );
                        match entry.collection {
                            "assistant_learning_publication_targets" => {
                                row.insert("state", "landed");
                            }
                            "assistant_voice_sessions" => {
                                row.insert("state", "closed");
                                row.insert("live_slot", false);
                                row.insert("billing_finalized", true);
                                row.insert("final_usage_confirmed", true);
                                row.insert("fixture_voice", true);
                            }
                            "nodes" => {
                                row.insert("is_active", false);
                                row.insert("status", "offline");
                            }
                            "node_pending_credentials" | "node_service_bindings" => {
                                row.insert("node_id", format!("domain-nodes-user_id-{person}"));
                            }
                            "oracle_pools" => {
                                row.insert("is_active", false);
                                row.insert("deletion_cleanup_pending", false);
                            }
                            "oracle_workers" => {
                                row.insert("pool_id", "shared-org-pool");
                            }
                            "service_accounts" => {
                                row.insert("personal_erasure_credentials_preserved", true);
                            }
                            "org_channel_sender_links" => {
                                row.insert("active", false);
                            }
                            "platform_setups" => {
                                row.insert("state", "cancelled");
                                row.insert("active", false);
                            }
                            "platform_bot_installations" => {
                                row.insert("state", "owner_deleted");
                            }
                            _ => {}
                        }
                        let disposition = match entry.collection {
                            "assistant_learning_publication_targets" | "oracle_pools" => "retained",
                            "oracle_workers" => "oracle_credential",
                            "org_channel_sender_links" => "link_tombstone",
                            _ => "deleted",
                        };
                        db.collection::<Document>(entry.collection)
                            .insert_one(row.clone())
                            .await
                            .unwrap();
                        domain_rows.push((
                            entry.collection,
                            id,
                            row,
                            person == &owner,
                            disposition,
                        ));
                    }
                }
                Erasure::Redacted => {
                    if let Some(spec) = crate::services::personal_data_registry::attribution(
                        entry.collection,
                        field,
                    ) {
                        covered_fields.insert((entry.collection, (*field).to_owned()));
                        for person in [&owner, &other] {
                            let id = format!("attribution-{}-{}-{person}", entry.collection, field);
                            let mut row = doc! {"_id": &id};
                            crate::services::personal_attribution_service::test_support::put(
                                &mut row,
                                field,
                                person.clone().into(),
                            );
                            for path in spec.name_fields {
                                crate::services::personal_attribution_service::test_support::put(
                                    &mut row,
                                    path,
                                    "Deleted Person".into(),
                                );
                            }
                            for path in spec.email_fields {
                                crate::services::personal_attribution_service::test_support::put(
                                    &mut row,
                                    path,
                                    "deleted@example.test".into(),
                                );
                            }
                            db.collection::<Document>(entry.collection)
                                .insert_one(row.clone())
                                .await
                                .unwrap();
                            domain_rows.push((
                                entry.collection,
                                id,
                                row,
                                person == &owner,
                                "attribution",
                            ));
                        }
                    } else if entry.collection == "service_change_events" && *field == "owner_id" {
                        covered_fields.insert((entry.collection, (*field).to_owned()));
                        for person in [&owner, &other, &org] {
                            let id = format!("history-owner-{person}");
                            let row = doc! {"_id": &id, "owner_id": person};
                            db.collection::<Document>(entry.collection)
                                .insert_one(row.clone())
                                .await
                                .unwrap();
                            domain_rows.push((
                                entry.collection,
                                id,
                                row,
                                person == &owner,
                                "deleted",
                            ));
                        }
                    } else if entry.collection == "invite_codes" {
                        covered_fields.insert((entry.collection, (*field).to_owned()));
                    }
                }
                _ => {}
            }
        }
    }
    let expected_fields: std::collections::BTreeSet<_> =
        crate::services::personal_data_registry::entries()
            .iter()
            .flat_map(|entry| match entry.owner {
                crate::services::personal_data_registry::OwnerSelector::Fields(fields) => fields
                    .iter()
                    .map(|field| (entry.collection, (*field).to_owned()))
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect();
    assert_eq!(
        covered_fields, expected_fields,
        "every persisted owner field needs an independent behavioral case"
    );

    let retained_checkpoint = doc! {
        "version": 1_i32, "sequence": 1_i64, "protocol": "responses",
        "final_report": true, "evidence": {"total": 37_i64}, "recovered": false,
        "observed_at": bson::DateTime::now(), "recover_after": bson::DateTime::now(),
    };
    for (index, entry) in crate::services::personal_data_registry::entries()
        .iter()
        .enumerate()
    {
        if !matches!(
            entry.erasure,
            crate::services::personal_data_registry::Erasure::Retained(_)
                | crate::services::personal_data_registry::Erasure::FeatureRetention(_)
        ) {
            continue;
        }
        let fields = match entry.owner {
            crate::services::personal_data_registry::OwnerSelector::Fields(fields) => fields,
            _ => &[],
        };
        for (field_index, field) in fields.iter().enumerate() {
            for person in [&owner, &other] {
                let id = if *field == "_id" {
                    person.clone()
                } else {
                    format!("registry-retained-{index}-{field_index}-{person}")
                };
                let mut row = if entry.collection == "managed_machines" {
                    use crate::models::remote_machine::{AuthorizationState, ResourceState};
                    let mut machine =
                        crate::services::remote_machine_service::CreateMachineInput::fixed(
                            person,
                            true,
                            "sha256:fixture",
                        )
                        .preview(&id, chrono::Utc::now());
                    machine.resource_state = ResourceState::Deleted;
                    machine.authorization_state = AuthorizationState::Revoked;
                    machine.writer_admitted = false;
                    machine.cleanup_state = "complete".into();
                    machine.cleanup_evidence_digest = Some("verified-cleanup".into());
                    bson::to_document(&machine).unwrap()
                } else {
                    doc! {"_id": &id}
                };
                row.insert("fixture_marker", "registry-retained");
                crate::services::personal_attribution_service::test_support::put(
                    &mut row,
                    field,
                    person.into(),
                );
                row.insert("completed", true);
                if entry.collection == "personal_erasure_effects" {
                    row.insert("status", "complete");
                    row.insert("kind", "oauth_revocation");
                }
                if entry.collection == "telemetry_capture_accounts" {
                    row.insert("generation", "registry-generation");
                    row.insert("provider_drained_generation", "registry-generation");
                }
                if entry.collection == "usage_meter" {
                    row.insert("usage_checkpoint", retained_checkpoint.clone());
                }
                db.collection::<Document>(entry.collection)
                    .insert_one(row.clone())
                    .await
                    .unwrap();
                retained_rows.push((entry.collection, id, row));
            }
        }
    }

    db.collection::<Document>("invite_codes")
        .insert_one(doc! {
            "_id": "registry-invite", "created_by": &owner, "note": "Deleted Person",
            "used_count": 1_i32,
            "max_uses": 2_i32,
            "usages": [{"user_id": &owner, "email": "deleted@example.test", "used_at": bson::DateTime::now()}],
        })
        .await
        .unwrap();
    db.collection::<Document>("service_change_events")
        .insert_one(doc! {
            "_id": "registry-org-history",
            "owner_id": &org,
            "actor": {"person_id": &owner, "name": "Deleted Person", "email": "deleted@example.test"},
        })
        .await
        .unwrap();

    for (collection, _, row) in &org_rows {
        db.collection::<Document>(collection)
            .insert_one(row.clone())
            .await
            .unwrap();
    }
    // Joined children have no personal owner field. Seed every registry join
    // with a target parent and child so the worker's authoritative-parent
    // cleanup is exercised automatically when a new join is added.
    let joined_cases: Vec<_> = crate::services::personal_data_registry::JOINED_CHILD_SPECS
        .iter()
        .map(|spec| (spec.child, spec.parent, spec.child_parent_field))
        .collect();
    let registry_joined: std::collections::BTreeSet<_> =
        crate::services::personal_data_registry::entries()
            .iter()
            .filter_map(|entry| match entry.erasure {
                crate::services::personal_data_registry::Erasure::JoinedChild { parent } => {
                    Some((entry.collection, parent))
                }
                _ => None,
            })
            .collect();
    let seeded_joined: std::collections::BTreeSet<_> = joined_cases
        .iter()
        .map(|(child, parent, _)| (*child, *parent))
        .collect();
    assert_eq!(
        seeded_joined, registry_joined,
        "joined-child fixture drifted from registry"
    );
    let mut joined_children = Vec::new();
    for (index, (child, parent, field)) in joined_cases.into_iter().enumerate() {
        let parent_owner_field = crate::services::personal_data_registry::find(parent)
            .and_then(|entry| match entry.owner {
                crate::services::personal_data_registry::OwnerSelector::Fields(fields) => {
                    fields.first().copied()
                }
                crate::services::personal_data_registry::OwnerSelector::NotPersonal => None,
            })
            .unwrap_or("user_id");
        for person in [&owner, &other, &org] {
            let parent_id = format!("registry-joined-parent-{index}-{person}");
            let child_id = if child == "channel_send_claims" {
                format!("{parent_id}:claim")
            } else if field == "_id" {
                parent_id.clone()
            } else {
                format!("registry-joined-child-{index}-{person}")
            };
            let mut parent_row = doc! {"_id": &parent_id};
            parent_row.insert(parent_owner_field, person);
            db.collection::<Document>(parent)
                .insert_one(&parent_row)
                .await
                .unwrap();
            let mut child_row = doc! {"_id": &child_id, "fixture_marker": "registry-joined"};
            if field != "_id" {
                child_row.insert(field, &parent_id);
            }
            if child == "channel_send_claims" {
                child_row.insert("status", "sent");
            }
            db.collection::<Document>(child)
                .insert_one(&child_row)
                .await
                .unwrap();
            if person == &owner {
                joined_children.push((child, child_id));
            } else {
                bystander_rows.push((parent, parent_id.into(), parent_row));
                bystander_rows.push((child, child_id.into(), child_row));
            }
        }
    }
    let agents = db.collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME);
    for (id, fields) in [
        ("acl-user", doc! {"org_user_ids":[&owner,&other]}),
        (
            "acl-maintainer",
            doc! {"org_maintainer_user_ids":[&owner,&other]},
        ),
        (
            "acl-both",
            doc! {"org_user_ids":[&owner,&other],"org_maintainer_user_ids":[&owner,&other]},
        ),
    ] {
        let mut agent =
            doc! {"_id":id,"user_id":&org,"kind":"specialist","org_access_revision":7_i64};
        agent.extend(fields);
        agents.insert_one(agent).await.unwrap();
    }
    // Force one lease expiry between pages to cover interruption/resumption.
    step(&state).await;
    db.collection::<Document>(JOBS)
        .update_many(
            doc! {"user_id": &owner},
            doc! {"$set": {"lease_expires_at": bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    until_source_removed(&state, &owner).await;

    finish_with_vendor_evidence(&state, &owner, &clients, &server).await;
    assert_eq!(
        status(&db, &owner).await.unwrap().unwrap().status,
        ErasureStatus::Complete
    );
    for (id, used, maintained) in [
        ("acl-user", true, false),
        ("acl-maintainer", false, true),
        ("acl-both", true, true),
    ] {
        let agent = agents.find_one(doc! {"_id":id}).await.unwrap().unwrap();
        assert_eq!(agent.get_str("user_id"), Ok(org.as_str()));
        assert_eq!(agent.get_i64("org_access_revision"), Ok(8));
        for (field, revision, pruned) in [
            ("org_user_ids", "org_use_revision", used),
            (
                "org_maintainer_user_ids",
                "org_maintain_revision",
                maintained,
            ),
        ] {
            assert_eq!(agent.get_i64(revision), Ok(i64::from(pruned)));
            if pruned {
                assert_eq!(agent.get(field), Some(&bson::bson!([&other])));
            } else {
                assert!(!agent.contains_key(field));
            }
        }
    }
    for (collection, id, before, target, disposition) in domain_rows {
        let row = db
            .collection::<Document>(collection)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap();
        if !target {
            assert_eq!(row, Some(before), "bystander {collection}");
            continue;
        }
        if matches!(
            collection,
            "durable_operation_executions" | "durable_operation_grants"
        ) {
            let receipt = db
                .collection::<Document>("personal_erasure_local_receipts")
                .find_one(doc! {"source_collection":collection,"source_id":&id,"user_id":&owner})
                .await
                .unwrap()
                .expect("settled durable evidence must have a compact receipt");
            assert_eq!(receipt.get_bool("personal_erasure_retained"), Ok(true));
            assert!(!receipt.contains_key("request_digest"));
            assert!(!receipt.contains_key("constraints"));
        }
        match disposition {
            "deleted" => assert!(row.is_none(), "domain row survived in {collection}"),
            "retained" => assert_eq!(row, Some(before), "retained domain {collection}"),
            "oracle_credential" => {
                let row = row.unwrap();
                assert_eq!(row.get_str("desired_state"), Ok("draining"));
                assert_eq!(
                    row.get_document("enrollment")
                        .unwrap()
                        .get_bool("retired_for_owner_erasure"),
                    Ok(true)
                );
            }
            "link_tombstone" => {
                let row = row.unwrap();
                assert_eq!(row.get_str("user_id"), Ok("erased"));
                assert_eq!(row.get_bool("active"), Ok(false));
            }
            "attribution" => {
                let encoded = serde_json::to_string(&row.unwrap()).unwrap();
                assert!(!encoded.contains("Deleted Person"));
                assert!(!encoded.contains("deleted@example.test"));
            }
            _ => unreachable!(),
        }
    }
    for (collection, id) in target_rows {
        let row = db
            .collection::<Document>(collection)
            .find_one(doc! {"_id": id})
            .await
            .unwrap();
        let entry = crate::services::personal_data_registry::find(collection).unwrap();
        assert_owner_row_is_absent_or_redacted(row, entry, &owner);
    }
    for (collection, id, expected) in bystander_rows {
        assert_eq!(
            db.collection::<Document>(collection)
                .find_one(doc! {"_id": id})
                .await
                .unwrap(),
            Some(expected),
            "bystander changed in {collection}"
        );
    }
    for (collection, id, expected) in org_rows {
        assert_eq!(
            db.collection::<Document>(collection)
                .find_one(doc! {"_id": id})
                .await
                .unwrap(),
            Some(expected),
            "organization row changed in {collection}"
        );
    }
    for (collection, id) in joined_children {
        assert!(
            db.collection::<Document>(collection)
                .find_one(doc! {"_id": id})
                .await
                .unwrap()
                .is_none(),
            "joined child survived in {collection}"
        );
    }
    for (collection, id, expected) in retained_rows {
        let row = db
            .collection::<Document>(collection)
            .find_one(doc! {"_id": id})
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("retained row disappeared from {collection}"));
        let mut row_for_comparison = row.clone();
        row_for_comparison.remove("erasure_completion_fence");
        assert_eq!(
            row_for_comparison, expected,
            "retained row changed in {collection}"
        );
        if collection == "usage_meter" {
            assert_eq!(
                row.get_document("usage_checkpoint").unwrap(),
                &retained_checkpoint,
                "financial recovery evidence changed during erasure"
            );
        }
    }
    let invite = db
        .collection::<Document>("invite_codes")
        .find_one(doc! {"_id": "registry-invite"})
        .await
        .unwrap()
        .unwrap();
    assert!(
        invite.get_array("usages").unwrap()[0]
            .as_document()
            .unwrap()
            .get("email")
            .is_none()
    );
    db.drop().await.unwrap();
}

async fn seed_orphan_delivery_intents(db: &Database, owner: &str) -> Vec<(&'static str, Document)> {
    let intents = crate::models::node_credential_desired::COLLECTION_NAME;
    let receipts = super::super::personal_erasure_registry::REVOCATION_RECEIPTS;
    let mut preserved = Vec::new();
    for kind in [UserType::Org, UserType::Person] {
        let other = uuid::Uuid::new_v4().to_string();
        preserved.push((USERS, bson::to_document(&test_user(&other, kind)).unwrap()));
        preserved.push((
            intents,
            doc! {
                "_id":uuid::Uuid::new_v4().to_string(),
                "node_id":uuid::Uuid::new_v4().to_string(),
                "service_slug":"example-service", "service_id":"example-service-id",
                "owner_id":&other, "actor_user_id":owner,
                "generation":1_i64, "pending":true, "retry_at":bson::DateTime::now(),
            },
        ));
        preserved.push((
            receipts,
            doc! {
                "_id":uuid::Uuid::new_v4().to_string(), "user_id":&other,
                "fixture_marker":"retained-other-owner-receipt",
            },
        ));
    }
    for (collection, row) in &preserved {
        db.collection::<Document>(collection)
            .insert_one(row)
            .await
            .unwrap();
    }
    db.collection::<Document>(intents)
        .insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),
            "node_id":uuid::Uuid::new_v4().to_string(),
            "service_slug":"example-service", "service_id":"example-service-id",
            "owner_id":owner, "actor_user_id":owner,
            "generation":1_i64, "pending":true, "retry_at":bson::DateTime::now(),
        })
        .await
        .unwrap();
    assert_eq!(
        db.collection::<Document>("nodes")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    preserved
}

async fn assert_delivery_bystanders_unchanged(db: &Database, rows: &[(&str, Document)]) {
    for (collection, expected) in rows {
        assert_eq!(
            db.collection::<Document>(collection)
                .find_one(doc! {"_id":expected.get("_id").unwrap()})
                .await
                .unwrap()
                .as_ref(),
            Some(expected),
            "changed another owner in {collection}"
        );
    }
}

async fn finish_with_vendor_evidence(
    state: &crate::AppState,
    owner: &str,
    clients: &crate::telemetry::erasure::ErasureClients,
    server: &crate::telemetry::erasure::test_support::Server,
) {
    use std::sync::atomic::Ordering;
    for _ in 0..16 {
        state
            .db
            .collection::<Document>(crate::models::telemetry_erasure_job::COLLECTION_NAME)
            .update_many(
                doc! {"user_id":owner},
                doc! {"$set":{"retry_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        super::super::telemetry_erasure_service::drain_once(&state.db, clients)
            .await
            .unwrap();
        if server.state.deletes.load(Ordering::SeqCst) > 0 {
            server.state.verified.store(true, Ordering::SeqCst);
        }
        step(state).await;
        if status(&state.db, owner).await.unwrap().unwrap().status == ErasureStatus::Complete {
            assert_eq!(server.state.deletes.load(Ordering::SeqCst), 1);
            return;
        }
    }
    panic!("erasure did not complete with verified vendor evidence");
}

fn stuck_delivery_intent_detection_pipeline() -> Vec<Document> {
    vec![
        doc! {"$match":{"status":"pending","domains":{"$elemMatch":{
            "domain":"source","status":"pending","reason":"source_cleanup_pending"
        }}}},
        doc! {"$lookup":{"from":"node_credential_desired","localField":"user_id",
        "foreignField":"owner_id","as":"intents"}},
        doc! {"$match":{"intents.0":{"$exists":true}}},
        doc! {"$project":{"user_id":1,"accepted_at":1,"updated_at":1,"intents":{"$size":"$intents"}}},
    ]
}

#[tokio::test]
async fn orphan_delivery_intent_does_not_hold_account_erasure_pending() {
    let (db, owner) = tests::fixture().await;
    let state = test_app_state(db.clone());
    let preserved = seed_orphan_delivery_intents(&db, &owner).await;
    let (clients, server) = crate::telemetry::erasure::test_support::clients().await;
    super::super::telemetry_erasure_service::register_destinations(&db, &clients)
        .await
        .unwrap();
    accept(&db, &owner, &owner, None).await.unwrap();
    until_source_removed(&state, &owner).await;
    finish_with_vendor_evidence(&state, &owner, &clients, &server).await;
    assert_eq!(
        status(&db, &owner).await.unwrap().unwrap().status,
        ErasureStatus::Complete
    );
    assert!(
        db.collection::<Document>(USERS)
            .find_one(doc! {"_id":&owner})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.collection::<Document>(crate::models::node_credential_desired::COLLECTION_NAME)
            .count_documents(doc! {"owner_id":&owner})
            .await
            .unwrap(),
        0
    );
    assert_delivery_bystanders_unchanged(&db, &preserved).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn stuck_source_cleanup_recovers_without_repair_or_replaying_completed_domains() {
    use futures::TryStreamExt;
    let (db, owner) = tests::fixture().await;
    let state = test_app_state(db.clone());
    let preserved = seed_orphan_delivery_intents(&db, &owner).await;
    let (clients, server) = crate::telemetry::erasure::test_support::clients().await;
    super::super::telemetry_erasure_service::register_destinations(&db, &clients)
        .await
        .unwrap();
    // Reproduce the stored state left by repeated source_cleanup_pending passes,
    // without accepting a new job or resetting an existing job during recovery.
    db.collection::<Document>(USERS).update_one(doc! {"_id":&owner}, doc! {"$set":{
        "is_active":false,"platform_personal_deleting":true,"platform_personal_retirement_pending":false
    }}).await.unwrap();
    db.collection::<Document>(super::super::erasure_admission::COLLECTION)
        .insert_one(doc! {
            "_id":&owner,"erasure_accepted":true,"cleanup_revision":41_i64
        })
        .await
        .unwrap();
    let at = bson::DateTime::from_millis(bson::DateTime::now().timestamp_millis() - 60_000);
    let domains: Vec<Document> = DOMAINS
        .iter()
        .enumerate()
        .map(|(index, domain)| {
            doc! {
                "domain":*domain,"status":if index < 5 {"complete"} else {"pending"},
                "reason":if *domain == "source" {Some("source_cleanup_pending")} else {None},
                "cursor":if index < 5 {Some(format!("completed:{domain}"))} else {None},
                "completed_at":if index < 5 {Some(at)} else {None},
                "consecutive_failures":0_i32,"retry_generation":0_i64,
            }
        })
        .collect();
    assert_eq!(
        &DOMAINS[..5],
        &[
            "platform",
            "oracle",
            "channel_bots",
            "provider_credentials",
            "local_inventory"
        ]
    );
    let completed_before = domains[..5].to_vec();
    let id = uuid::Uuid::new_v4().to_string();
    db.collection::<Document>(JOBS).insert_one(doc! {
        "_id":&id,"user_id":&owner,"requested_by":&owner,"owner_deletion_policy":bson::Bson::Null,
        "status":"pending","inventory_version":INVENTORY_VERSION,"domains":domains,
        "local_inventory_revision":super::local::LOCAL_INVENTORY_REVISION,
        "accepted_at":at,"updated_at":at,"completed_at":bson::Bson::Null,
        "lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null,"retry_at":bson::Bson::Null,
    }).await.unwrap();
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    super::super::telemetry_erasure_service::enqueue_in_session(&db, &owner, &mut tx)
        .await
        .unwrap();
    super::super::transaction_service::commit(&mut tx)
        .await
        .unwrap();
    let receipts = super::super::personal_erasure_registry::REVOCATION_RECEIPTS;
    db.collection::<Document>(receipts).insert_one(doc! {
        "_id":uuid::Uuid::new_v4().to_string(),"user_id":&owner,"fixture_marker":"old-final-phase-receipt"
    }).await.unwrap();
    let detected: Vec<Document> = db
        .collection::<Document>(JOBS)
        .aggregate(stuck_delivery_intent_detection_pipeline())
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(detected.len(), 1);
    assert_eq!(detected[0].get_str("_id"), Ok(id.as_str()));
    assert_eq!(detected[0].get_str("user_id"), Ok(owner.as_str()));
    assert_eq!(detected[0].get_i32("intents"), Ok(1));

    // Each normal pass deletes only its exact final-metadata page. A user_id
    // receipt still drains before the owner_id intent and final source row.
    for pass in 0..3 {
        step(&state).await;
        let job = db
            .collection::<PersonalErasureJob>(JOBS)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        for (actual, expected) in job.domains[..5].iter().zip(&completed_before) {
            let expected: DomainCheckpoint = bson::from_document(expected.clone()).unwrap();
            assert_eq!(
                bson::to_document(actual).unwrap(),
                bson::to_document(&expected).unwrap()
            );
        }
        assert_eq!(
            job.domains[5].status,
            if pass == 2 {
                DomainStatus::Complete
            } else {
                DomainStatus::Pending
            }
        );
        for domain in &job.domains[6..] {
            assert_eq!(domain.status, DomainStatus::Pending);
            assert!(domain.completed_at.is_none());
            assert!(domain.cursor.is_none());
            assert!(domain.reason.is_none());
        }
        assert_eq!(
            db.collection::<Document>(receipts)
                .count_documents(doc! {"user_id":&owner})
                .await
                .unwrap(),
            0
        );
    }
    assert!(
        db.collection::<Document>(USERS)
            .find_one(doc! {"_id":&owner})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.collection::<Document>(crate::models::node_credential_desired::COLLECTION_NAME)
            .count_documents(doc! {"owner_id":&owner})
            .await
            .unwrap(),
        0
    );
    assert!(
        db.collection::<Document>(super::super::erasure_admission::COLLECTION)
            .find_one(doc! {"_id":&owner,"cleanup_revision":{"$gt":41_i64}})
            .await
            .unwrap()
            .is_some()
    );
    let detected: Vec<Document> = db
        .collection::<Document>(JOBS)
        .aggregate(stuck_delivery_intent_detection_pipeline())
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(detected.is_empty());

    step(&state).await;
    let job = db
        .collection::<PersonalErasureJob>(JOBS)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.domains[6].domain, "machine");
    assert_eq!(job.domains[6].status, DomainStatus::Complete);
    assert!(job.domains[6].completed_at.is_some());
    assert!(job.domains[7].completed_at.is_none());
    step(&state).await;
    let job = db
        .collection::<PersonalErasureJob>(JOBS)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.domains[7].domain, "vendor");
    assert_eq!(job.domains[7].status, DomainStatus::Pending);
    assert_eq!(
        job.domains[7].reason.as_deref(),
        Some("vendor_erasure_pending")
    );
    finish_with_vendor_evidence(&state, &owner, &clients, &server).await;
    let complete = db
        .collection::<PersonalErasureJob>(JOBS)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(complete.status, ErasureStatus::Complete);
    assert!(complete.completed_at.is_some());
    assert!(
        complete
            .domains
            .iter()
            .all(|domain| domain.status == DomainStatus::Complete)
    );
    assert!(complete.domains[7].cursor.is_some());
    for (actual, expected) in complete.domains[..5].iter().zip(completed_before) {
        let expected: DomainCheckpoint = bson::from_document(expected).unwrap();
        assert_eq!(
            bson::to_document(actual).unwrap(),
            bson::to_document(&expected).unwrap()
        );
    }
    let completed_snapshot = db
        .collection::<Document>(JOBS)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert!(!process_one(&state).await.unwrap());
    assert_eq!(
        db.collection::<Document>(JOBS)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap(),
        completed_snapshot
    );
    let detected: Vec<Document> = db
        .collection::<Document>(JOBS)
        .aggregate(stuck_delivery_intent_detection_pipeline())
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(detected.is_empty());
    assert_delivery_bystanders_unchanged(&db, &preserved).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn in_flight_old_inventory_reconciles_before_source_and_survives_legacy_writers() {
    for (revision, orphan_intent) in [(None, false), (Some(2_i32), false), (None, true)] {
        let (db, owner) = tests::fixture().await;
        let state = test_app_state(db.clone());
        let (clients, server) = crate::telemetry::erasure::test_support::clients().await;
        super::super::telemetry_erasure_service::register_destinations(&db, &clients)
            .await
            .unwrap();
        let accepted = accept(&db, &owner, &owner, None).await.unwrap();
        // The completed platform checkpoint has already retired the account.
        db.collection::<Document>(USERS).update_one(doc! {"_id":&owner}, doc! {"$set":{
            "is_active":false,"platform_personal_deleting":true,"platform_personal_retirement_pending":false
        }}).await.unwrap();
        let preserved = if orphan_intent {
            seed_orphan_delivery_intents(&db, &owner).await
        } else {
            Vec::new()
        };
        db.collection::<Document>("assistant_wire_logs")
            .insert_one(
                doc! {"_id":"old-inventory-content","user_id":&owner,"payload":"personal-text"},
            )
            .await
            .unwrap();
        let mut domains = accepted.domains;
        for domain in &mut domains[..5] {
            domain.status = DomainStatus::Complete;
            domain.completed_at = Some(chrono::Utc::now());
        }
        domains[5].status = DomainStatus::Pending;
        domains[5].reason = Some("source_cleanup_pending".into());
        let mut update = doc! {"$set": {
            "domains":bson::to_bson(&domains).unwrap(),"status":"pending",
            "retry_at":bson::DateTime::from_millis(0)
        }};
        if let Some(revision) = revision {
            update
                .get_document_mut("$set")
                .unwrap()
                .insert("local_inventory_revision", revision);
        } else {
            update.insert("$unset", doc! {"local_inventory_revision":""});
        }
        db.collection::<Document>(JOBS)
            .update_one(doc! {"_id":&accepted.id}, update)
            .await
            .unwrap();

        // A stale completed local checkpoint must be selected before source.
        // One bounded pass can remove rows, but cannot stamp the revision yet.
        step(&state).await;
        let current = status(&db, &owner).await.unwrap().unwrap();
        assert_eq!(current.domains[4].status, DomainStatus::Pending);
        assert_eq!(
            current.local_inventory_revision,
            revision.unwrap_or(0) as u32
        );
        assert!(
            db.collection::<Document>("assistant_wire_logs")
                .find_one(doc! {"user_id":&owner})
                .await
                .unwrap()
                .is_none()
        );
        until_source_removed(&state, &owner).await;
        finish_with_vendor_evidence(&state, &owner, &clients, &server).await;
        let complete = status(&db, &owner).await.unwrap().unwrap();
        assert_eq!(complete.status, ErasureStatus::Complete);
        assert_eq!(
            complete.local_inventory_revision,
            super::local::LOCAL_INVENTORY_REVISION
        );
        assert_delivery_bystanders_unchanged(&db, &preserved).await;
        assert_eq!(
            db.collection::<Document>(crate::models::node_credential_desired::COLLECTION_NAME)
                .count_documents(doc! {"owner_id":&owner})
                .await
                .unwrap(),
            0
        );

        // Old serde readers ignore the additive revision field, and their
        // field-level checkpoint writes preserve fields they do not know.
        #[derive(serde::Deserialize)]
        struct LegacyCheckpoint {
            domains: Vec<DomainCheckpoint>,
            status: ErasureStatus,
        }
        let stored = db
            .collection::<Document>(JOBS)
            .find_one(doc! {"_id":&accepted.id})
            .await
            .unwrap()
            .unwrap();
        let legacy: LegacyCheckpoint = bson::from_document(stored.clone()).unwrap();
        assert_eq!(legacy.status, ErasureStatus::Complete);
        db.collection::<Document>(JOBS).update_one(doc! {"_id":&accepted.id},
            doc! {"$set":{"domains":bson::to_bson(&legacy.domains).unwrap(),"status":"complete"}})
            .await.unwrap();
        assert!(!process_one(&state).await.unwrap());
        assert_eq!(
            db.collection::<Document>(JOBS)
                .find_one(doc! {"_id":&accepted.id})
                .await
                .unwrap()
                .unwrap(),
            stored
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn completed_erasure_redacts_every_persisted_attribution_and_global_identity_scan_is_clean() {
    use crate::models::credits::Credits;
    use crate::services::{
        audit_chain_service, audit_service, billing::ledger,
        personal_attribution_service::test_support as attribution,
    };
    let (db, owner) = tests::fixture().await;
    let org = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    db.collection::<Document>(USERS)
        .update_one(
            doc! {"_id": &owner},
            doc! {"$set": {
                "display_name": attribution::NAME, "email": attribution::EMAIL,
            }},
        )
        .await
        .unwrap();
    accept(&db, &owner, &owner, None).await.unwrap();
    let cases = attribution::seed(&db, &owner, &org, 35).await;
    // Delivery intent is drained by node, then swept by exact personal owner
    // in main's final_metadata path. An org actor is attribution, not ownership.
    let desired = crate::models::node_credential_desired::COLLECTION_NAME;
    let org_intent = doc! {
        "_id": "org-delivery-intent", "owner_id": &org, "actor_user_id": &owner,
        "node_id": "org-node", "service_id": "catalog-service", "service_slug": "catalog-slug",
        "generation": 1_i64, "pending": true, "retry_at": bson::DateTime::now(),
    };
    db.collection::<Document>(desired)
        .insert_many([
            doc! {"_id": "personal-delivery-intent", "owner_id": &owner, "actor_user_id": &owner,
            "node_id": "removed-node", "generation": 1_i64},
            org_intent.clone(),
        ])
        .await
        .unwrap();
    // Recipient labels and embedded invite usages are removed by their existing
    // grant/invite paths. They participate in the same global identity scan.
    db.collection::<Document>("assistant_agent_shares")
        .insert_one(doc! {
            "_id": "target-recipient", "owner_id": &org, "subject_kind": "person",
            "subject_id": &owner, "subject_email": attribution::EMAIL,
        })
        .await
        .unwrap();
    db.collection::<Document>("invite_codes")
        .insert_one(doc! {
            "_id": "target-invite", "used_count": 1, "max_uses": 5,
            "usages": [{"user_id": &owner, "email": attribution::EMAIL}],
        })
        .await
        .unwrap();
    super::super::audit_service::log_actor_event(
        db.clone(),
        &super::super::audit_service::AuditActor {
            user_id: owner.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        "attribution.test.immutable",
        Some(serde_json::json!({"name": attribution::NAME, "email": attribution::EMAIL})),
    )
    .await
    .unwrap();
    let audit = db
        .collection::<Document>("audit_log")
        .find_one(doc! {"event_type": "attribution.test.immutable"})
        .await
        .unwrap()
        .unwrap();
    let ledger_key = uuid::Uuid::new_v4().into_bytes();
    let ledger_entry = ledger::exact_entry(
        &owner,
        "immutable-ledger",
        "usage_settled",
        "usage-settled:erasure-fixture".into(),
        ledger::transfer(
            format!("wallet:{owner}"),
            "platform:usage".into(),
            Credits::from_whole(1),
        ),
        None,
    );
    let ledger_entry = ledger::append_chained_entry(&db, ledger_entry, &ledger_key)
        .await
        .unwrap();
    let ledger_before = db
        .collection::<Document>("billing_ledger")
        .find_one(doc! {"_id": &ledger_entry.id})
        .await
        .unwrap()
        .unwrap();
    let state = test_app_state(db.clone());
    until_source_removed(&state, &owner).await;
    // Complete the real vendor-history gate as well as the local cleanup;
    // source absence alone must never stand in for overall completion.
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
    for _ in 0..20 {
        if status(&db, &owner).await.unwrap().unwrap().status == ErasureStatus::Complete {
            break;
        }
        step(&state).await;
    }
    let completed = status(&db, &owner).await.unwrap().unwrap();
    assert_eq!(completed.status, ErasureStatus::Complete, "{completed:?}");
    assert!(
        db.collection::<Document>(desired)
            .find_one(doc! {"_id": "personal-delivery-intent"})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.collection::<Document>(desired)
            .find_one(doc! {"_id": "org-delivery-intent"})
            .await
            .unwrap(),
        Some(org_intent)
    );
    attribution::assert_cases(&db, &cases, true).await;
    attribution::assert_no_identity_outside_audit(&db).await;
    assert_eq!(
        db.collection::<Document>("audit_log")
            .find_one(doc! {"_id": audit.get("_id").unwrap()})
            .await
            .unwrap(),
        Some(audit)
    );
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .find_one(doc! {"_id": &ledger_entry.id})
            .await
            .unwrap(),
        Some(ledger_before)
    );
    let audit_report = audit_chain_service::verify_chain(
        &db,
        audit_service::audit_chain_hmac_key().expect("fixture audit chain key"),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        audit_report.status,
        audit_chain_service::AuditChainStatus::Ok
    );
    assert!(audit_report.checked_count > 0);
    let ledger_report = ledger::verify_chain(&db, &ledger_key, None, None, None)
        .await
        .unwrap();
    assert_eq!(ledger_report.status, ledger::BillingLedgerStatus::Ok);
    assert_eq!(ledger_report.checked_count, 1);
    assert_eq!(
        ledger_report.head_hash.as_deref(),
        Some(ledger_entry.entry_hash.as_str())
    );
}

#[tokio::test]
async fn departing_person_oracle_sessions_are_erased_without_touching_other_owner() {
    let (db, owner) = tests::fixture().await;
    let state = test_app_state(db.clone());
    let other = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&other, UserType::Person))
        .await
        .unwrap();
    let sessions = crate::models::oracle_session::COLLECTION_NAME;
    db.collection::<Document>(sessions)
        .insert_many([
            doc! {
                "_id": format!("oracle-owner-{}", uuid::Uuid::new_v4()),
                "owner_user_id": &owner,
                "prompt": "owner",
            },
            doc! {
                "_id": format!("oracle-other-{}", uuid::Uuid::new_v4()),
                "owner_user_id": &other,
                "prompt": "other",
            },
        ])
        .await
        .unwrap();
    accept(&db, &owner, &owner, None).await.unwrap();
    until_source_removed(&state, &owner).await;
    assert_eq!(
        db.collection::<Document>(sessions)
            .count_documents(doc! {"owner_user_id": &owner})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<Document>(sessions)
            .count_documents(doc! {"owner_user_id": &other})
            .await
            .unwrap(),
        1
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
            "async_service_operation_artifacts",
            doc! {"_id":"pending-file","user_id":&owner,"conversation_id":"deleted-thread","data_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1,2,3]}},
        ),
        (
            "assistant_artifact_deliveries",
            doc! {"_id":"delivery","user_id":&owner,"conversation_id":"deleted-thread"},
        ),
        (
            "generated_artifact_content_series",
            doc! {"_id":"series","user_id":&owner,"conversation_id":"deleted-thread"},
        ),
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
            "channel_whatsapp_native",
            doc! {"_id":"native-wa","owner_id":&owner,"meta_app_id":"123"},
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
            "assistant_tool_results",
            doc! {"_id":"personal-tool-result","user_id":&owner},
        ),
        (
            "assistant_tool_results",
            doc! {"_id":"actor-tool-result","user_id":&other,"actor_id":&owner},
        ),
        (
            "assistant_resources",
            doc! {"_id":"personal-resource","user_id":&owner,"conversation_id":"private-chat"},
        ),
        // An older replica may have deleted the group before it knew these
        // additive collections. Erasure must not depend on a surviving parent.
        (
            "assistant_group_read_states",
            doc! {"_id":"read-state","user_id":&owner,"group_id":"deleted-group","last_seen_seq":42_i64},
        ),
        (
            "assistant_group_mention_notifications",
            doc! {"_id":"pending-mention","user_id":&owner,"group_id":"deleted-group"},
        ),
        (
            "assistant_voice_announcements",
            doc! {"_id":"private-notice","user_id":&owner,"conversation_id":"private-chat","text":"Private notification"},
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
        (
            "consents",
            doc! {"_id":"personal-consent","user_id":&owner,"client_id":"connected-app","scopes":"openid","granted_at":bson::DateTime::now()},
        ),
        (
            "consent_revocation_fences",
            doc! {"_id":format!("{owner}:revoked-app"),"user_id":&owner,"client_id":"revoked-app","revoked_at":bson::DateTime::now(),"version":uuid::Uuid::new_v4().to_string()},
        ),
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
            "consent_revocation_fences",
            doc! {"_id":format!("{other}:revoked-app"),"user_id":&other,"client_id":"revoked-app","revoked_at":bson::DateTime::now(),"version":uuid::Uuid::new_v4().to_string()},
        ),
        (
            "assistant_learning_publication_targets",
            doc! {"_id":"landed-shared-target","owner_id":&owner,"agent_id":"agent-personal","state":"landed","operation_id":"settled-publication"},
        ),
        (
            "assistant_agent_learning_proposals",
            doc! {"_id":"org-draft","owner_id":&other,"agent_id":"org-agent"},
        ),
        (
            "assistant_tool_results",
            doc! {"_id":"other-tool-result","user_id":&other,"actor_id":&other},
        ),
        (
            "assistant_resources",
            doc! {"_id":"org-resource","user_id":&other,"created_by":&owner},
        ),
        (
            "assistant_voice_announcements",
            doc! {"_id":"other-notice","user_id":&other,"conversation_id":"other-chat","text":"Other notification"},
        ),
        (
            "channel_conversations",
            doc! {"_id":"org-chat","user_id":&other,"created_by":&owner},
        ),
        (
            "channel_whatsapp_native",
            doc! {"_id":"org-native-wa","owner_id":&other,"created_by":&owner},
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
async fn shared_redactions_remove_invite_identity_and_preserve_org_history() {
    let (db, owner) = tests::fixture().await;
    let state = test_app_state(db.clone());
    let other = uuid::Uuid::new_v4().to_string();
    let org = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_many([
            test_user(&other, UserType::Person),
            test_user(&org, UserType::Org),
        ])
        .await
        .unwrap();
    let now = bson::DateTime::now();
    db.collection::<Document>("invite_codes")
        .insert_one(doc! {
            "_id": "shared-invite",
            "code": "NYX-SHARED",
            "max_uses": 10_i32,
            "used_count": 2_i32,
            "created_by": &other,
            "usages": [
                {"user_id": &owner, "email": "deleted@example.test", "used_at": now},
                {"user_id": &other, "email": "kept@example.test", "used_at": now},
            ],
        })
        .await
        .unwrap();
    db.collection::<Document>("service_change_events")
        .insert_many([
            doc! {
                "_id": "personal-history",
                "owner_id": &owner,
                "actor": {"person_id": &owner, "name": "Deleted Person", "display_name": "Deleted Person", "email": "deleted@example.test", "email_address": "deleted@example.test", "api_key_id": "key", "app_id": "app"},
                "action": "update",
            },
            doc! {
                "_id": "org-history",
                "owner_id": &org,
                "actor": {"person_id": &owner, "name": "Deleted Person", "display_name": "Deleted Person", "email": "deleted@example.test", "email_address": "deleted@example.test", "api_key_id": "key", "app_id": "app"},
                "action": "update",
                "sequence": 7_i64,
            },
        ])
        .await
        .unwrap();

    accept(&db, &owner, &owner, None).await.unwrap();
    until_source_removed(&state, &owner).await;

    let invite = db
        .collection::<Document>("invite_codes")
        .find_one(doc! {"_id": "shared-invite"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invite.get_i32("used_count"), Ok(2));
    assert_eq!(invite.get_i32("max_uses"), Ok(10));
    let usages = invite.get_array("usages").unwrap();
    assert_eq!(usages.len(), 2);
    assert_eq!(
        usages[0].as_document().unwrap().get_str("user_id"),
        Ok(owner.as_str())
    );
    assert!(usages[0].as_document().unwrap().get("email").is_none());
    assert_eq!(
        usages[1].as_document().unwrap().get_str("user_id"),
        Ok(other.as_str())
    );
    assert_eq!(
        usages[1].as_document().unwrap().get_str("email"),
        Ok("kept@example.test")
    );

    assert!(
        db.collection::<Document>("service_change_events")
            .find_one(doc! {"_id": "personal-history"})
            .await
            .unwrap()
            .is_none()
    );
    let org_history = db
        .collection::<Document>("service_change_events")
        .find_one(doc! {"_id": "org-history"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(org_history.get_i64("sequence"), Ok(7));
    let actor = org_history.get_document("actor").unwrap();
    assert_eq!(actor.get_str("name"), Ok("Deleted account"));
    assert_eq!(actor.get_str("person_id"), Ok(owner.as_str()));
    assert_eq!(actor.get_str("api_key_id"), Ok("key"));
    assert_eq!(actor.get_str("app_id"), Ok("app"));
    assert_eq!(actor.get_str("display_name"), Ok("Deleted account"));
    assert!(actor.get("email").is_none());
    assert!(actor.get("email_address").is_none());
    db.drop().await.unwrap();
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

#[tokio::test]
async fn automation_receipts_are_erased_by_exact_owner_and_block_final_completion() {
    let (db, owner) = tests::fixture().await;
    let state = test_app_state(db.clone());
    let receipts = db.collection::<Document>("assistant_direct_handoffs");
    receipts
        .insert_many([
            doc! {"_id": "pending", "user_id": &owner, "state": "pending"},
            doc! {"_id": "terminal", "user_id": &owner, "state": "delivered"},
            doc! {"_id": "other-owner", "user_id": "another-person", "state": "pending"},
        ])
        .await
        .unwrap();
    accept(&db, &owner, &owner, None).await.unwrap();
    let job = db
        .collection::<PersonalErasureJob>(JOBS)
        .find_one(doc! {"user_id": &owner})
        .await
        .unwrap()
        .unwrap();
    until_source_removed(&state, &owner).await;
    assert_eq!(
        receipts
            .count_documents(doc! {"user_id": &owner})
            .await
            .unwrap(),
        0
    );
    assert!(
        receipts
            .find_one(doc! {"_id": "other-owner"})
            .await
            .unwrap()
            .is_some()
    );
    // The final check uses the same exact-owner inventory even with no hint on
    // the deleted person row. A leftover cannot be declared complete.
    receipts
        .insert_one(doc! {"_id": "leftover", "user_id": &owner, "state": "pending"})
        .await
        .unwrap();
    assert!(!local::source(&db, &job).await.unwrap());
    receipts.delete_one(doc! {"_id": "leftover"}).await.unwrap();
    assert!(local::source(&db, &job).await.unwrap());
}

#[tokio::test]
async fn billing_meter_receipt_and_capture_state_share_financial_erasure_retention() {
    for org in [false, true] {
        let (db, owner) = tests::fixture().await;
        let state = test_app_state(db.clone());
        db.collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
            .insert_one(doc! {
                "_id":"retained-meter",
                "billing_owner_id":&owner,
                "status":"finalized",
                "released":true,
                "lago_acked":true,
            })
            .await
            .unwrap();
        db.collection::<Document>("billing_usage_deliveries")
            .insert_one(doc! {
                "_id":"retained-delivery",
                "owner_id":&owner,
                "wallet_id":"wallet",
                "transaction_id":"retained-transaction",
                "state":"acknowledged",
            })
            .await
            .unwrap();
        db.collection::<Document>("billing_usage_comparisons")
            .insert_one(doc! {
                "_id":"retained-capture",
                "owner_id":&owner,
                "wallet_id":"wallet",
                "customer_id":"customer",
                "subscription_id":"subscription",
            })
            .await
            .unwrap();
        if org {
            db.collection::<Document>(USERS)
                .update_one(doc! {"_id":&owner}, doc! {"$set":{"user_type":"org"}})
                .await
                .unwrap();
            crate::services::org_service::delete_org_user(
                &db,
                &owner,
                &crate::services::org_service::audit::fixture_actor(),
            )
            .await
            .unwrap();
        } else {
            accept(&db, &owner, &owner, None).await.unwrap();
            until_source_removed(&state, &owner).await;
        }
        assert!(
            db.collection::<Document>(USERS)
                .find_one(doc! {"_id":&owner})
                .await
                .unwrap()
                .is_none()
        );

        for collection in [
            crate::models::usage_meter::COLLECTION_NAME,
            "billing_usage_deliveries",
            "billing_usage_comparisons",
        ] {
            assert!(
                db.collection::<Document>(collection)
                    .find_one(doc! {"$or":[{"billing_owner_id":&owner},{"owner_id":&owner}]})
                    .await
                    .unwrap()
                    .is_some(),
                "financial collection {collection} must retain the owner's record"
            );
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn notification_feed_is_erased_for_exact_person_and_blocks_completion_if_leftover() {
    let (db, owner) = tests::fixture().await;
    let state = test_app_state(db.clone());
    let budgets =
        db.collection::<Document>(crate::services::assistant_notification_service::PUSH_BUDGETS);
    let sources =
        db.collection::<Document>(crate::services::assistant_notification_service::PUSH_SOURCES);
    budgets
        .insert_many([doc! {"_id":&owner}, doc! {"_id":"another-person"}])
        .await
        .unwrap();
    sources
        .insert_many([
            doc! {"_id":"removed-source","user_id":&owner},
            doc! {"_id":"retained-source","user_id":"another-person"},
        ])
        .await
        .unwrap();
    let removed = crate::test_utils::seed_notification_purge_probe(&db, &owner, doc! {}).await;
    let retained =
        crate::test_utils::seed_notification_purge_probe(&db, "another-person", doc! {}).await;
    accept(&db, &owner, &owner, None).await.unwrap();
    let job = db
        .collection::<PersonalErasureJob>(JOBS)
        .find_one(doc! {"user_id":&owner})
        .await
        .unwrap()
        .unwrap();
    until_source_removed(&state, &owner).await;
    crate::test_utils::assert_notification_probe(&db, &removed, false).await;
    crate::test_utils::assert_notification_probe(&db, &retained, true).await;
    assert!(
        budgets
            .find_one(doc! {"_id":&owner})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        budgets
            .find_one(doc! {"_id":"another-person"})
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        sources
            .find_one(doc! {"user_id":&owner})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        sources
            .find_one(doc! {"_id":"retained-source"})
            .await
            .unwrap()
            .is_some()
    );
    let leftover = crate::test_utils::seed_notification_purge_probe(&db, &owner, doc! {}).await;
    assert!(!local::source(&db, &job).await.unwrap());
    db.collection::<Document>("assistant_notifications")
        .delete_one(doc! {"_id":leftover})
        .await
        .unwrap();
    assert!(local::source(&db, &job).await.unwrap());
    for (collection, row) in [
        (&budgets, doc! {"_id":&owner}),
        (&sources, doc! {"_id":"leftover-source","user_id":&owner}),
    ] {
        collection.insert_one(row.clone()).await.unwrap();
        assert!(!local::source(&db, &job).await.unwrap());
        collection
            .delete_one(doc! {"_id":row.get_str("_id").unwrap()})
            .await
            .unwrap();
        assert!(local::source(&db, &job).await.unwrap());
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn pending_platform_effects_preserve_parents_children_and_credentials() {
    let (db, owner) = tests::fixture().await;
    let state = test_app_state(db.clone());
    accept(&db, &owner, &owner, None).await.unwrap();
    let job = db
        .collection::<PersonalErasureJob>(JOBS)
        .find_one(doc! {"user_id": &owner})
        .await
        .unwrap()
        .unwrap();
    let rows = [
        (
            "user_services",
            doc! {"_id":"held-service", "user_id": &owner, "name":"Recovery service"},
        ),
        (
            "service_history_heads",
            doc! {"_id":"held-service", "sequence": 5_i64},
        ),
        (
            "platform_bot_installations",
            doc! {"_id":"held-installation", "owner_user_id": &owner, "state":"owner_deleted"},
        ),
        (
            "platform_delivery_tests",
            doc! {"_id":"held-test", "installation_id":"held-installation"},
        ),
        (
            "platform_setups",
            doc! {"_id":"held-setup", "owner_user_id": &owner, "state":"cancelled"},
        ),
        (
            "platform_setup_steps",
            doc! {"_id":"held-step", "setup_id":"held-setup"},
        ),
        (
            "platform_personal_bindings",
            doc! {"_id":"held-binding", "user_id": &owner, "state":"revoked"},
        ),
        (
            "user_provider_credentials",
            doc! {"_id":"held-credential", "user_id": &owner, "encrypted":"recovery-capability"},
        ),
    ];
    for (collection, row) in &rows {
        db.collection::<Document>(collection)
            .insert_one(row.clone())
            .await
            .unwrap();
    }
    db.collection::<Document>("platform_bot_cleanup")
        .insert_one(doc! {"_id":"pending", "owner_user_id":&owner, "completed":false})
        .await
        .unwrap();
    assert!(!super::local::pass(&state, &job).await.unwrap());
    for (collection, row) in &rows {
        assert_eq!(
            db.collection::<Document>(collection)
                .find_one(doc! {"_id":row.get("_id").unwrap()})
                .await
                .unwrap(),
            Some(row.clone())
        );
    }
    db.collection::<Document>("platform_bot_cleanup")
        .update_one(doc! {"_id":"pending"}, doc! {"$set":{"completed":true}})
        .await
        .unwrap();
    let bystander = doc! {"_id":"other-owner", "user_id":"other-person", "name":"Unrelated"};
    db.collection::<Document>("user_services")
        .insert_one(bystander.clone())
        .await
        .unwrap();
    for _ in 0..100 {
        if super::local::pass(&state, &job).await.unwrap() {
            break;
        }
    }
    for (collection, row) in &rows {
        assert!(
            db.collection::<Document>(collection)
                .find_one(doc! {"_id": row.get("_id").unwrap()})
                .await
                .unwrap()
                .is_none(),
            "{collection}"
        );
    }
    assert_eq!(
        db.collection::<Document>("user_services")
            .find_one(doc! {"_id":"other-owner"})
            .await
            .unwrap(),
        Some(bystander)
    );
    assert_eq!(
        db.collection::<Document>("platform_bot_cleanup")
            .count_documents(doc! {"completed":true})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn complete_job_retires_active_personal_links_and_waiting_telegram_requests() {
    use crate::models::{
        platform_link_intent::{PlatformLinkChallenge, PlatformLinkIntent},
        platform_personal_binding::PlatformPersonalBinding,
        telegram_bot_request::TelegramBotRequest,
    };
    use chrono::Utc;
    let (db, owner) = tests::fixture().await;
    let state = test_app_state(db.clone());
    let (clients, server) = crate::telemetry::erasure::test_support::clients().await;
    crate::services::telemetry_erasure_service::register_destinations(&db, &clients)
        .await
        .unwrap();
    let bystander = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>(USERS)
        .insert_one(test_user(&bystander, UserType::Person))
        .await
        .unwrap();
    let fixtures: std::collections::BTreeMap<String, Vec<Document>> = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("test_data/account-export-models.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut targets = Vec::new();
    let mut others = Vec::new();
    for (person, prefix, sender) in [
        (&owner, "target", 100_i64),
        (&bystander, "bystander", 200_i64),
    ] {
        for (collection, suffix, owner_field, state_field, initial) in [
            (
                "platform_personal_bindings",
                "binding",
                "user_id",
                "state",
                "active",
            ),
            (
                "platform_personal_link_intents",
                "intent",
                "claimant_user_id",
                "state",
                "pending",
            ),
            (
                "platform_personal_link_challenges",
                "review",
                "reviewed_user_id",
                "state",
                "review",
            ),
            (
                "platform_personal_link_challenges",
                "awaiting",
                "reviewed_user_id",
                "state",
                "awaiting_chat_confirmation",
            ),
            (
                "telegram_bot_requests",
                "telegram",
                "owner_user_id",
                "status",
                "waiting_telegram",
            ),
        ] {
            let mut row = fixtures[collection][0].clone();
            row.insert("_id", format!("{prefix}-{suffix}"));
            row.insert(owner_field, person);
            row.insert(state_field, initial);
            row.insert(
                "expires_at",
                bson::DateTime::from_chrono(Utc::now() + chrono::Duration::hours(1)),
            );
            if collection == "telegram_bot_requests" {
                row.insert("actor_user_id", person);
            } else {
                // Different external senders: retiring the target's consent must
                // not invalidate another person's pending chat confirmation.
                row.insert("telegram_sender_id", sender);
                row.insert("telegram_chat_id", sender);
                if collection == "platform_personal_bindings" {
                    row.insert("active_turn", bson::Bson::Null);
                } else {
                    row.insert(
                        "purge_after",
                        bson::DateTime::from_chrono(Utc::now() + chrono::Duration::days(1)),
                    );
                    if collection == "platform_personal_link_challenges" {
                        row.insert("intent_id", format!("{prefix}-intent"));
                    }
                }
            }
            // Store real serialized models, including checked platform identity
            // projections, rather than constructing inventory-shaped documents.
            let row = match collection {
                "platform_personal_bindings" => {
                    bson::to_document(&bson::from_document::<PlatformPersonalBinding>(row).unwrap())
                }
                "platform_personal_link_intents" => {
                    bson::to_document(&bson::from_document::<PlatformLinkIntent>(row).unwrap())
                }
                "platform_personal_link_challenges" => {
                    bson::to_document(&bson::from_document::<PlatformLinkChallenge>(row).unwrap())
                }
                _ => bson::to_document(&bson::from_document::<TelegramBotRequest>(row).unwrap()),
            }
            .unwrap();
            db.collection::<Document>(collection)
                .insert_one(row.clone())
                .await
                .unwrap();
            if person == &owner {
                targets.push((collection, row));
            } else {
                others.push((collection, row));
            }
        }
    }
    accept(&db, &owner, &owner, None).await.unwrap();
    until_source_removed(&state, &owner).await;
    finish_with_vendor_evidence(&state, &owner, &clients, &server).await;
    assert_eq!(
        status(&db, &owner).await.unwrap().unwrap().status,
        ErasureStatus::Complete
    );
    for (collection, row) in targets {
        assert!(
            db.collection::<Document>(collection)
                .find_one(doc! {"_id": row.get("_id").unwrap()})
                .await
                .unwrap()
                .is_none(),
            "target survived in {collection}"
        );
    }
    for (collection, expected) in others {
        let actual = db
            .collection::<Document>(collection)
            .find_one(doc! {"_id": expected.get("_id").unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            bson::to_vec(&actual).unwrap(),
            bson::to_vec(&expected).unwrap(),
            "bystander changed in {collection}"
        );
    }
    db.drop().await.unwrap();
}

/// Existing accepted jobs may still have active local nodes. Seed that persisted
/// state explicitly after acceptance, without relaxing normal node admission.
async fn pending_stop_fixture() -> (
    crate::AppState,
    PersonalErasureJob,
    crate::models::node::Node,
) {
    use crate::services::node_service;
    let (db, owner) = tests::fixture().await;
    let state = test_app_state(db.clone());
    let (_, token, _) =
        node_service::create_registration_token(&db, &owner, "stop-recovery", 10, 300)
            .await
            .unwrap();
    let (mut node, _, _) = node_service::register_node(&db, &state.encryption_keys, &token, None)
        .await
        .unwrap();
    node.status = crate::models::node::NodeStatus::Online;
    node.machine = Some(nyxid_machine::MachineProfile {
        version: nyxid_machine::PROTOCOL_VERSION,
        runtime_id: uuid::Uuid::new_v4().to_string(),
        shell: true,
        exact_cancel: true,
        ..Default::default()
    });
    accept(&db, &owner, &owner, None).await.unwrap();
    db.collection::<crate::models::node::Node>("nodes")
        .replace_one(doc! {"_id":&node.id}, &node)
        .await
        .unwrap();
    let job = status(&db, &owner).await.unwrap().unwrap();
    (state, job, node)
}

async fn insert_pending_personal_stop(
    db: &Database,
    owner: &str,
    node: Option<&crate::models::node::Node>,
) -> String {
    use crate::models::{
        platform_bot_runtime_controls::*, platform_entry_projection::*,
        platform_personal_binding::*,
    };
    let now = chrono::Utc::now();
    let row = PlatformPersonalStopEffect {
        id: uuid::Uuid::new_v4().to_string(),
        active_turn: PersonalActiveTurn {
            conversation_id: format!("nyxa-{}", uuid::Uuid::new_v4().simple()),
            turn_id: uuid::Uuid::new_v4().to_string(),
            origin: PersonalTurnProof {
                platform_config_id: "stop-profile".into(),
                manager_bot_id: "1234".into(),
                entry: StoredEntryIdentity::TelegramV1 {
                    telegram_bot_id: 1234,
                },
                sender: StoredSenderIdentity::TelegramV1 {
                    telegram_sender_id: 100,
                    telegram_chat_id: 100,
                },
                manager_authority_epoch: 1,
                personal_control_revision: 1,
                binding_id: "stop-binding".into(),
                binding_generation: 1,
                user_id: owner.into(),
                nyxbot_agent_id: uuid::Uuid::new_v4().to_string(),
                source: PersonalOriginSource::ThreadEvent {
                    admission_id: uuid::Uuid::new_v4().to_string(),
                },
            },
        },
        machine_scopes: node
            .map(|n| {
                vec![ExactMachineScope {
                    node_id: n.id.clone(),
                    runtime_id: n.machine.as_ref().unwrap().runtime_id.clone(),
                }]
            })
            .unwrap_or_default(),
        machine_jobs: Vec::new(),
        stop_requested_at: now,
        progress: None,
        completed: false,
        claim_id: None,
        lease_expires_at: None,
        retry_at: now,
        created_at: now,
    };
    db.collection::<PlatformPersonalStopEffect>(STOP_EFFECTS)
        .insert_one(&row)
        .await
        .unwrap();
    row.id
}

async fn run_real_personal_stop_processor(state: &crate::AppState) {
    let producers = crate::services::platform_manager_transport::compose(
        state.clone(),
        crate::handlers::assistant_team::owner_turn_settled_hook(state.clone()),
        crate::handlers::agent_skills::preload_context(state.clone()),
    );
    let result = producers
        .lifecycle
        .recover_personal_lifecycle(&state.db, producers.personal.as_ref(), 1)
        .await
        .unwrap();
    assert_eq!(result.processed, 1);
}

async fn assert_stop_completed(db: &Database, id: &str, completed: bool) {
    let row = db
        .collection::<Document>("platform_personal_stop_effects")
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_bool("completed"), Ok(completed));
}

#[tokio::test]
async fn pending_personal_stop_preserves_active_local_node_until_real_processor_completes() {
    use crate::services::node_ws_manager::{NodeCapabilitiesMsg, NodeOutboundMessage};
    let (state, job, node) = pending_stop_fixture().await;
    let db = &state.db;
    let id = insert_pending_personal_stop(db, &job.user_id, Some(&node)).await;
    db.collection::<Document>("user_services")
        .insert_one(doc! {"_id":"recovery-parent","user_id":&job.user_id})
        .await
        .unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    crate::test_utils::register_test_node_connection(&state, &node.id, sender).await;
    let caps: NodeCapabilitiesMsg =
        serde_json::from_value(serde_json::json!({"machine":node.machine})).unwrap();
    state.node_ws_manager.record_capabilities(&node.id, &caps);
    crate::services::machine_service::capable(&node, nyxid_machine::Operation::CancelExact)
        .unwrap();
    let before = db
        .collection::<Document>("nodes")
        .find_one(doc! {"_id":&node.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!local::pass(&state, &job).await.unwrap());
    assert_eq!(
        db.collection::<Document>("nodes")
            .find_one(doc! {"_id":&node.id})
            .await
            .unwrap(),
        Some(before)
    );
    assert!(
        db.collection::<Document>("user_services")
            .find_one(doc! {"_id":"recovery-parent"})
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        receiver.try_recv().is_err(),
        "blocked pass must not drain or disconnect"
    );
    let manager = state.node_ws_manager.clone();
    let node_id = node.id.clone();
    let signing = crate::services::node_service::get_node_signing_secret(
        db,
        &state.encryption_keys,
        &node.id,
    )
    .await
    .unwrap();
    let peer = tokio::spawn(async move {
        let NodeOutboundMessage::Text(text) =
            tokio::time::timeout(std::time::Duration::from_secs(15), receiver.recv())
                .await
                .expect("real processor must dispatch the cancel")
                .unwrap()
        else {
            panic!("cancel request")
        };
        let request: nyxid_machine::Request = serde_json::from_str(&text).unwrap();
        assert_eq!(request.operation, nyxid_machine::Operation::CancelExact);
        nyxid_machine::signing::ReplayGuard::default()
            .verify(&request, &node_id, &signing, chrono::Utc::now().timestamp())
            .unwrap();
        manager.deliver_machine_result(
            &node_id,
            nyxid_machine::Response {
                request_id: request.request_id,
                result: serde_json::json!({"stopped":true}),
            },
        );
    });
    run_real_personal_stop_processor(&state).await;
    tokio::time::timeout(std::time::Duration::from_secs(20), peer)
        .await
        .unwrap()
        .unwrap();
    assert_stop_completed(db, &id, true).await;
    until_source_removed(&state, &job.user_id).await;
    assert!(
        db.collection::<Document>("nodes")
            .find_one(doc! {"_id":&node.id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.collection::<Document>("user_services")
            .find_one(doc! {"_id":"recovery-parent"})
            .await
            .unwrap()
            .is_none()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn independent_personal_stop_completes_through_real_processor_after_erasure() {
    let (db, owner) = tests::fixture().await;
    let state = test_app_state(db.clone());
    let (clients, server) = crate::telemetry::erasure::test_support::clients().await;
    crate::services::telemetry_erasure_service::register_destinations(&db, &clients)
        .await
        .unwrap();
    accept(&db, &owner, &owner, None).await.unwrap();
    let id = insert_pending_personal_stop(&db, &owner, None).await;
    let effect = db
        .collection::<Document>("platform_personal_stop_effects")
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    // The stale binding reference is not needed by the processor or completion.
    db.collection::<Document>("platform_personal_bindings").insert_one(doc! {
        "_id":"stop-binding","user_id":&owner,"state":"revoked","active_turn":effect.get("active_turn").unwrap(),
    }).await.unwrap();
    until_source_removed(&state, &owner).await;
    finish_with_vendor_evidence(&state, &owner, &clients, &server).await;
    assert_eq!(
        status(&db, &owner).await.unwrap().unwrap().status,
        ErasureStatus::Complete
    );
    assert_stop_completed(&db, &id, false).await;
    assert!(
        db.collection::<Document>("platform_personal_bindings")
            .find_one(doc! {"_id":"stop-binding"})
            .await
            .unwrap()
            .is_none()
    );
    run_real_personal_stop_processor(&state).await;
    assert_stop_completed(&db, &id, true).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn stop_dependency_matches_only_active_owned_local_nodes() {
    let (state, job, node) = pending_stop_fixture().await;
    let db = &state.db;
    let id = insert_pending_personal_stop(db, &job.user_id, Some(&node)).await;
    for (set, blocked) in [
        (
            doc! {"is_active":true,"user_id":&job.user_id,"metadata.provisioning_source":"ordinary"},
            true,
        ),
        (doc! {"is_active":false}, false),
        (doc! {"is_active":true,"user_id":"other-owner"}, false),
        (
            doc! {"user_id":&job.user_id,"metadata.provisioning_source":"remote_machine"},
            false,
        ),
    ] {
        db.collection::<Document>("nodes")
            .update_one(doc! {"_id":&node.id}, doc! {"$set":set})
            .await
            .unwrap();
        assert_eq!(
            local::remote_cleanup_pending(db, &job.user_id)
                .await
                .unwrap(),
            blocked
        );
    }
    db.collection::<Document>("nodes")
        .delete_one(doc! {"_id":&node.id})
        .await
        .unwrap();
    assert!(
        !local::remote_cleanup_pending(db, &job.user_id)
            .await
            .unwrap()
    );
    // One capture can cover two nodes. The node-specific pre-drain check must
    // not hold inactive A just because the same capture still needs active B.
    let mut inactive = node.clone();
    inactive.is_active = false;
    let mut other = node.clone();
    other.id = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::node::Node>("nodes")
        .insert_many([&inactive, &other])
        .await
        .unwrap();
    db.collection::<Document>("platform_personal_stop_effects")
        .update_one(
            doc! {"_id":&id},
            doc! {"$push":{"machine_scopes":{
                "node_id":&other.id,"runtime_id":&other.machine.as_ref().unwrap().runtime_id,
            }}},
        )
        .await
        .unwrap();
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .await
        .unwrap();
    for (node_id, blocked) in [
        (Some(node.id.as_str()), false),
        (Some(other.id.as_str()), true),
        (None, true),
    ] {
        assert_eq!(
            local::stop_effect_needs_local_node(db, &job.user_id, node_id, &mut tx)
                .await
                .unwrap(),
            blocked,
            "node-specific dependency: {node_id:?}"
        );
    }
    tx.abort_transaction().await.unwrap();
    assert_stop_completed(db, &id, false).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn stop_inserted_after_pass_start_prevents_node_drain_and_transactional_delete() {
    let (state, job, node) = pending_stop_fixture().await;
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    crate::test_utils::register_test_node_connection(&state, &node.id, sender).await;
    let hook = std::sync::Arc::new(local::BeforeNodeDrainHook::default());
    local::BEFORE_NODE_DRAIN.insert(state.db.name().into(), hook.clone());
    let pass_state = state.clone();
    let pass_job = job.clone();
    let pass = tokio::spawn(async move { local::pass(&pass_state, &pass_job).await });
    tokio::time::timeout(std::time::Duration::from_secs(10), hook.reached.notified())
        .await
        .unwrap();
    insert_pending_personal_stop(&state.db, &job.user_id, Some(&node)).await;
    let before = state
        .db
        .collection::<Document>("nodes")
        .find_one(doc! {"_id":&node.id})
        .await
        .unwrap()
        .unwrap();
    hook.resume.notify_one();
    assert!(!pass.await.unwrap().unwrap());
    local::BEFORE_NODE_DRAIN.remove(state.db.name());
    assert!(
        receiver.try_recv().is_err(),
        "racing effect must prevent drain/disconnect"
    );
    assert_eq!(
        state
            .db
            .collection::<Document>("nodes")
            .find_one(doc! {"_id":&node.id})
            .await
            .unwrap(),
        Some(before)
    );
    // Exercise the same transaction guard used immediately before the node delete.
    assert!(matches!(
        local::erase_page(&state.db, &job.user_id, "nodes", doc! {"_id":&node.id}).await,
        Err(AppError::Conflict(_))
    ));
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn node_stop_wait_escalates_and_admin_node_delete_then_operator_retry_completes_erasure() {
    let (state, mut job, node) = pending_stop_fixture().await;
    let db = &state.db;
    let id = insert_pending_personal_stop(db, &job.user_id, Some(&node)).await;
    let index = job
        .domains
        .iter()
        .position(|d| d.domain == "local_inventory")
        .unwrap();
    // Let the real preceding domains close admission and finish retirement;
    // then age the node-blocked local phase beyond its documented wait bound.
    for _ in 0..20 {
        job = status(db, &job.user_id).await.unwrap().unwrap();
        if job.domains[..index]
            .iter()
            .all(|d| d.status == DomainStatus::Complete)
        {
            break;
        }
        step(&state).await;
    }
    assert!(
        job.domains[..index]
            .iter()
            .all(|d| d.status == DomainStatus::Complete)
    );
    job.domains[index].remote_wait_started_at =
        Some(chrono::Utc::now() - chrono::Duration::hours(25));
    db.collection::<PersonalErasureJob>(JOBS)
        .replace_one(doc! {"_id":&job.id}, &job)
        .await
        .unwrap();
    step(&state).await;
    let blocked = status(db, &job.user_id).await.unwrap().unwrap();
    assert_eq!(blocked.status, ErasureStatus::OperatorAction);
    assert_eq!(
        blocked.domains[index].reason.as_deref(),
        Some("remote_cleanup_wait_exceeded")
    );
    crate::services::node_service::admin_delete_node(db, &node.id)
        .await
        .unwrap();
    let inactive = db
        .collection::<crate::models::node::Node>("nodes")
        .find_one(doc! {"_id":&node.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!inactive.is_active);
    assert_eq!(inactive.status, crate::models::node::NodeStatus::Offline);
    assert!(
        crate::services::machine_service::dispatch(
            &state,
            &inactive,
            nyxid_machine::Operation::CancelExact,
            serde_json::json!({})
        )
        .await
        .is_err()
    );
    let request = worker::DomainRetryRequest {
        domain: "local_inventory".into(),
        job_id: job.id.clone(),
        expected_retry_generation: blocked.domains[index].retry_generation,
        idempotency_key: uuid::Uuid::new_v4().to_string(),
    };
    worker::retry_domain(db, &job.user_id, &job.user_id, &request)
        .await
        .unwrap();
    until_source_removed(&state, &job.user_id).await;
    crate::services::telemetry_erasure_service::drain_once(db, &Default::default())
        .await
        .unwrap();
    crate::services::telemetry_erasure_service::operator_reconcile_empty_inventory(
        db,
        &job.user_id,
        "operator",
        "evidence://deployment-inventory",
        "Verified deployment history",
    )
    .await
    .unwrap();
    for _ in 0..20 {
        if status(db, &job.user_id).await.unwrap().unwrap().status == ErasureStatus::Complete {
            break;
        }
        step(&state).await;
    }
    assert_eq!(
        status(db, &job.user_id).await.unwrap().unwrap().status,
        ErasureStatus::Complete
    );
    assert!(
        db.collection::<Document>("nodes")
            .find_one(doc! {"_id":&node.id})
            .await
            .unwrap()
            .is_none()
    );
    // NodeUnavailable remains non-terminal; admin deletion only clears the local dependency.
    run_real_personal_stop_processor(&state).await;
    assert_stop_completed(db, &id, false).await;
    db.drop().await.unwrap();
}
