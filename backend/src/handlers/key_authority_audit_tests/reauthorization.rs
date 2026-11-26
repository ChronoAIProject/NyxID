//! Both real reauthorization source commits, audit replay, and owner recovery.
use super::*;
use crate::handlers::api_keys;

const GRANTS: &str = "durable_operation_grants";

async fn reauth_fixture() -> Fixture {
    let mut f = Box::pin(fixture(Case::Grant)).await;
    let mut grant = rows(&f.state.db, GRANTS).await.remove(0);
    grant.insert("_id", uuid::Uuid::new_v4().to_string());
    f.state
        .db
        .collection::<Document>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    grant.insert("_id", uuid::Uuid::new_v4().to_string());
    grant.insert("revoked_at", bson::DateTime::now());
    f.state
        .db
        .collection::<Document>(GRANTS)
        .insert_one(grant)
        .await
        .unwrap();
    refresh_plan(&mut f).await;
    f
}

async fn refresh_plan(f: &mut Fixture) {
    let key = f
        .state
        .db
        .collection::<crate::models::api_key::ApiKey>("api_keys")
        .find_one(doc! {"_id": &f.key})
        .await
        .unwrap()
        .unwrap();
    let selections = serde_json::from_value::<Vec<DurableOperationSelection>>(
        f.create_body["selected_operations"].clone(),
    )
    .unwrap();
    let plan = api_key_scope_service::build_scope_plan_with_operations(
        &f.state.db,
        &f.state.node_ws_manager,
        &f.owner,
        None,
        &key.allowed_service_ids,
        &selections,
        key.expires_at,
        false,
    )
    .await
    .unwrap();
    f.create_body["scope_plan_digest"] = plan.normalized_grant_digest.into();
}

async fn select_operations(f: &mut Fixture, count: usize) {
    use crate::models::service_endpoint::ServiceEndpoint;
    let db = f.state.db.clone();
    let endpoint = db
        .collection::<ServiceEndpoint>("service_endpoints")
        .find_one(doc! {"_id":&f.endpoint})
        .await
        .unwrap()
        .unwrap();
    let selection = f.create_body["selected_operations"][0].clone();
    let mut endpoints = Vec::new();
    for n in 1..count {
        let mut endpoint = endpoint.clone();
        endpoint.id = uuid::Uuid::new_v4().to_string();
        endpoint.name = format!("create_{n}");
        endpoint.path = format!("/items/{n}");
        let mut selection = selection.clone();
        selection["endpoint_id"] = endpoint.id.clone().into();
        f.create_body["selected_operations"]
            .as_array_mut()
            .unwrap()
            .push(selection);
        endpoints.push(endpoint);
    }
    db.collection::<ServiceEndpoint>("service_endpoints")
        .insert_many(endpoints)
        .await
        .unwrap();
    refresh_plan(f).await;
}

async fn reauthorize(f: &Fixture, auth: AuthUser) -> Response {
    api_keys::reauthorize_durable_grants(
        State(f.state.clone()),
        auth,
        Path(f.key.clone()),
        Json(
            serde_json::from_value(json!({
                "selected_operations": f.create_body["selected_operations"],
                "scope_plan_digest": f.create_body["scope_plan_digest"],
            }))
            .unwrap(),
        ),
    )
    .await
    .into_response()
}

async fn owner_grants(f: &Fixture, include_revoked: bool) -> Value {
    let response = api_keys::list_durable_grants(
        State(f.state.clone()),
        test_auth_user(&f.owner),
        Path(f.key.clone()),
        axum::extract::Query(api_keys::DurableGrantListQuery { include_revoked }),
    )
    .await
    .unwrap();
    serde_json::to_value(response.0).unwrap()
}

async fn crash_after_unit(second: bool) {
    let f = Box::pin(reauth_fixture()).await;
    let db = &f.state.db;
    {
        let mut monitor = f.monitor.lock().unwrap();
        monitor.stop = true;
        monitor.stop_source = Some(if second {
            |command, collection, _| command == "insert" && collection == GRANTS
        } else {
            |command, collection, _| command == "update" && collection == GRANTS
        });
    }
    let stopped = std::panic::AssertUnwindSafe(Box::pin(
        REQUEST.scope((), reauthorize(&f, test_auth_user(&f.owner))),
    ))
    .catch_unwind()
    .await;
    match stopped {
        Err(error) => assert!(error.is::<ProcessStopped>()),
        Ok(response) => panic!(
            "reauthorization did not reach commit: HTTP {}",
            response.status()
        ),
    }
    let active = owner_grants(&f, false).await;
    assert_eq!(
        active["grants"].as_array().unwrap().len(),
        usize::from(second)
    );
    let all = owner_grants(&f, true).await;
    assert_eq!(
        all["grants"].as_array().unwrap().len(),
        3 + usize::from(second)
    );
    assert!(rows(db, "audit_log").await.is_empty());
    let intents = rows(db, INTENTS).await;
    assert_eq!(
        intents.len(),
        2 + usize::from(second),
        "committed reauthorization unit has no durable audit evidence"
    );
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    db.collection::<Document>(INTENTS)
        .insert_many(&intents)
        .await
        .unwrap();
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    let audit = rows(db, "audit_log").await;
    assert_eq!(audit.len(), intents.len());
    assert_eq!(
        audit
            .iter()
            .filter(|row| row.get_str("event_type") == Ok("durable_grant_revoked"))
            .count(),
        2
    );
    for row in &audit {
        assert!(row.get_i64("seq").is_ok());
        assert!(row.get_str("entry_hash").is_ok());
        if row.get_str("event_type") == Ok("durable_grant_revoked") {
            assert_eq!(
                row.get_document("event_data").unwrap().get_str("reason"),
                Ok("reauthorization")
            );
        }
    }
    assert!(rows(db, INTENTS).await.is_empty());
    if !second {
        let denied = crate::services::durable_operation_grant_service::authorize_and_reserve(
            db,
            &f.state.node_ws_manager,
            &f.owner,
            &f.key,
            &f.connection,
            "POST",
            "/items",
            None,
            &HeaderMap::new(),
            b"",
            &f.grant,
            "gap-fixture",
            false,
        )
        .await;
        assert!(matches!(
            denied,
            Err(crate::errors::AppError::DurableGrantRevoked)
        ));
        // The surviving owner/key still authorize the existing reauthorization route.
        let response = Box::pin(reauthorize(&f, test_auth_user(&f.owner))).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            owner_grants(&f, false).await["grants"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let remaining = rows(db, INTENTS).await;
        assert_eq!(
            remaining.len(),
            1,
            "already revoked grants emit no duplicate occurrence"
        );
        assert_eq!(
            remaining[0]
                .get_document("event")
                .unwrap()
                .get_str("event_type"),
            Ok("durable_grants_reauthorized")
        );
    }
    cleanup(db).await;
}

#[tokio::test]
async fn reauthorization_first_commit_process_death_and_replay() {
    Box::pin(crash_after_unit(false)).await;
}

#[tokio::test]
async fn reauthorization_second_commit_process_death_and_replay() {
    Box::pin(crash_after_unit(true)).await;
}

#[tokio::test]
async fn reauthorization_request_command_counts() {
    let f = Box::pin(reauth_fixture()).await;
    let response = Box::pin(REQUEST.scope((), reauthorize(&f, test_auth_user(&f.owner)))).await;
    assert_eq!(response.status(), StatusCode::OK);
    let commands = f.monitor.lock().unwrap().commands.clone();
    println!(
        "REAUTHORITY_REQUEST count={} commands={commands:?}",
        commands.len()
    );
    assert_eq!(commands.len(), 41, "one intent insert and commit per unit");
    cleanup(&f.state.db).await;
}

#[tokio::test]
async fn reauthorization_rejected_units_preserve_fail_closed_order() {
    for reject in [
        "revocation_intent",
        "replacement_intent",
        "replacement_insert",
    ] {
        let mut f = Box::pin(reauth_fixture()).await;
        select_operations(&mut f, 2).await;
        let db = &f.state.db;
        let before = rows(db, GRANTS).await;
        if reject == "replacement_insert" {
            let old_ids: Vec<_> = before.iter().map(|r| r.get_str("_id").unwrap()).collect();
            db.run_command(doc! {"collMod": GRANTS, "validator": {"_id":{"$in":old_ids}}})
                .await
                .unwrap();
        } else {
            let event = if reject == "revocation_intent" {
                "durable_grant_revoked"
            } else {
                "durable_grants_reauthorized"
            };
            db.run_command(doc! {"collMod":INTENTS,"validator":{"event.event_type":{"$ne":event}}})
                .await
                .unwrap();
        }
        let response = Box::pin(reauthorize(&f, test_auth_user(&f.owner))).await;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{reject}"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error_code"],
            1007
        );
        let after = rows(db, GRANTS).await;
        assert_eq!(after.len(), before.len());
        if reject == "revocation_intent" {
            assert_eq!(after, before);
            assert!(rows(db, INTENTS).await.is_empty());
        } else {
            assert!(after.iter().all(|r| r.get_datetime("revoked_at").is_ok()));
            let intents = rows(db, INTENTS).await;
            assert_eq!(intents.len(), 2);
            assert!(
                intents
                    .iter()
                    .all(|r| r.get_document("event").unwrap().get_str("event_type")
                        == Ok("durable_grant_revoked"))
            );
            assert_eq!(owner_grants(&f, false).await["grants"], json!([]));
        }
        assert!(rows(db, "audit_log").await.is_empty());
        cleanup(db).await;
    }
}

async fn content_parity(revocations: bool) {
    use crate::services::audit_service;
    let f = Box::pin(reauth_fixture()).await;
    let mut auth = test_auth_user(&f.owner);
    auth.ip_address = Some("192.0.2.14".into());
    auth.user_agent = Some("Fixture client/1.0 🦀".into());
    auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    auth.scope = "write".into();
    auth.api_key_id = Some(uuid::Uuid::new_v4().to_string());
    auth.api_key_name = Some("Fixture caller 🦀".into());
    let context = crate::turn_trace::TurnContext::for_turn(&uuid::Uuid::new_v4().to_string());
    let response = crate::turn_trace::scope(context, Box::pin(reauthorize(&f, auth.clone()))).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let receipt: Value = serde_json::from_slice(&bytes).unwrap();
    let db = &f.state.db;
    for intent in rows(db, INTENTS).await {
        let encoded = serde_json::to_string(&intent).unwrap();
        for forbidden in [
            "secret_hash",
            "secret_prefix",
            "key_hash",
            "key_prefix",
            "access_token",
            "refresh_token",
            "fixture-private",
            "constraints",
            "client_audit_binding",
        ] {
            assert!(!encoded.contains(forbidden), "{forbidden}");
        }
    }
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    let source = rows(db, GRANTS).await;
    let event = if revocations {
        "durable_grant_revoked"
    } else {
        "durable_grants_reauthorized"
    };
    let actual: Vec<_> = rows(db, "audit_log")
        .await
        .into_iter()
        .filter(|r| r.get_str("event_type") == Ok(event))
        .collect();
    assert_eq!(actual.len(), if revocations { 2 } else { 1 });
    for mut actual in actual {
        let expected = if revocations {
            let data = actual.get_document_mut("event_data").unwrap();
            // The sole additive fact compared with the existing individual revoke record.
            assert_eq!(data.remove("reason"), Some("reauthorization".into()));
            let grant_id = data.get_str("grant_id").unwrap();
            let old_grant = source
                .iter()
                .find(|r| r.get_str("_id") == Ok(grant_id))
                .unwrap();
            json!({"api_key_id":f.key,"grant_id":grant_id,"endpoint_id":old_grant.get_str("endpoint_id").unwrap(),
                "contract_digest":old_grant.get_str("contract_digest").unwrap(),"decision":"revoked"})
        } else {
            let grants = receipt["grants"].as_array().unwrap();
            json!({"api_key_id":f.key,"grant_ids":grants.iter().map(|g|&g["id"]).collect::<Vec<_>>(),"operation_count":grants.len(),"decision":"authorized"})
        };
        let written = audit_service::notify_on_audit_write_for_user(event, &f.owner);
        crate::turn_trace::scope(context, async {
            audit_service::log_for_user(db.clone(), &auth, event, Some(expected));
        })
        .await;
        let old_id = written.await.unwrap();
        let mut old = db
            .collection::<Document>("audit_log")
            .find_one(doc! {"_id":old_id})
            .await
            .unwrap()
            .unwrap();
        for field in ["_id", "created_at", "seq", "prev_hash", "entry_hash"] {
            actual.remove(field);
            old.remove(field);
        }
        assert_eq!(actual, old, "{event}: field-for-field audit content parity");
    }
    cleanup(db).await;
}

#[tokio::test]
async fn reauthorization_revoked_content_parity() {
    Box::pin(content_parity(true)).await;
}
#[tokio::test]
async fn reauthorization_completed_content_parity() {
    Box::pin(content_parity(false)).await;
}

#[tokio::test]
async fn reauthorization_occurrence_caps_and_maxima() {
    use crate::{
        models::security_effect_intent::SecurityEffect,
        services::{api_key_audit_service as audit, audit_service::AuditActor},
    };
    let raw = "x".repeat(20000);
    let actor = AuditActor {
        user_id: raw.clone(),
        ip_address: Some(raw.clone()),
        user_agent: Some(raw.clone()),
        api_key_id: Some(raw.clone()),
        api_key_name: Some(raw.clone()),
    };
    let mut ids = vec!["i".repeat(256); 30];
    ids.push("i".repeat(238));
    ids.push("i".repeat(256));
    let context = crate::turn_trace::TurnContext::for_turn(&uuid::Uuid::new_v4().to_string());
    let intents = crate::turn_trace::scope(context, async {
        [
            audit::grant_revoked_for_reauthorization(
                &actor,
                &raw,
                &raw,
                &raw,
                &format!("sha256:{}", "d".repeat(64)),
            )
            .unwrap(),
            audit::grants_reauthorized(&actor, &raw, &ids).unwrap(),
        ]
    })
    .await;
    for intent in intents {
        let SecurityEffect::Audit { event } = intent.effect else {
            panic!("audit")
        };
        let bytes = bson::to_vec(&event).unwrap().len();
        assert_eq!(
            bytes,
            if event.event_type == "durable_grant_revoked" {
                3411
            } else {
                11116
            }
        );
        assert!(bytes < 16384);
        println!(
            "REAUTHORITY_OCCURRENCE_MAX {} bytes={bytes} margin={}",
            event.event_type,
            16384 - bytes
        );
        if event.event_type == "durable_grants_reauthorized" {
            let data = event.event_data.as_ref().unwrap();
            assert_eq!(data["grant_ids_count"], ids.len());
            assert_eq!(data["grant_ids_truncated"], true);
            use sha2::Digest;
            assert_eq!(
                data["grant_ids_sha256"],
                hex::encode(sha2::Sha256::digest(ids.join("\n").as_bytes()))
            );
            let doc = bson::to_document(&json!({"a":data["grant_ids"]})).unwrap();
            assert_eq!(bson::to_vec(&doc).unwrap().len() - 8, 8192);
        }
    }
}

#[tokio::test]
async fn reauthorization_caps_accept_oversized_network_context() {
    let f = Box::pin(reauth_fixture()).await;
    let mut auth = test_auth_user(&f.owner);
    auth.ip_address = Some("x".repeat(20000));
    auth.user_agent = Some("界".repeat(7000));
    assert_eq!(
        Box::pin(reauthorize(&f, auth)).await.status(),
        StatusCode::OK
    );
    for intent in rows(&f.state.db, INTENTS).await {
        let event = intent.get_document("event").unwrap();
        for (field, cap) in [("ip_address", 256), ("user_agent", 1024)] {
            let value = event.get_str(field).unwrap();
            assert!(value.len() <= cap);
            assert!(value.ends_with("...[truncated]"));
        }
    }
    cleanup(&f.state.db).await;
}

#[tokio::test]
async fn reauthorization_missing_key_and_stale_plan_preserve_errors_without_intents() {
    for missing in [false, true] {
        let mut f = Box::pin(reauth_fixture()).await;
        if missing {
            f.state
                .db
                .collection::<Document>("api_keys")
                .delete_one(doc! {"_id":&f.key})
                .await
                .unwrap();
        } else {
            f.create_body["scope_plan_digest"] = "stale".into();
        }
        let before = rows(&f.state.db, GRANTS).await;
        let response = Box::pin(reauthorize(&f, test_auth_user(&f.owner))).await;
        assert_eq!(
            response.status(),
            if missing {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::CONFLICT
            }
        );
        assert_eq!(rows(&f.state.db, GRANTS).await, before);
        assert!(rows(&f.state.db, INTENTS).await.is_empty());
        cleanup(&f.state.db).await;
    }
}

#[tokio::test]
#[ignore = "Operating-range probe; through cargo-slot.sh: cargo +1.98.1 test -p nyxid --bin nyxid-server reauthorization_scale_probe -- --ignored --nocapture --test-threads=1"]
async fn reauthorization_scale_probe() {
    let mut f = Box::pin(reauth_fixture()).await;
    let db = f.state.db.clone();
    let template = rows(&db, GRANTS)
        .await
        .into_iter()
        .find(|r| r.get_datetime("revoked_at").is_err())
        .unwrap();
    let extra: Vec<_> = (2..10000)
        .map(|_| {
            let mut r = template.clone();
            r.insert("_id", uuid::Uuid::new_v4().to_string());
            r
        })
        .collect();
    db.collection::<Document>(GRANTS)
        .insert_many(extra)
        .await
        .unwrap();
    select_operations(&mut f, 500).await;
    let request_bytes = serde_json::to_vec(&f.create_body).unwrap().len();
    assert!(request_bytes < 1048576);
    let start = std::time::Instant::now();
    let response = Box::pin(REQUEST.scope((), reauthorize(&f, test_auth_user(&f.owner)))).await;
    let elapsed = start.elapsed();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        db.collection::<Document>(GRANTS)
            .count_documents(doc! {"revoked_at":bson::Bson::Null})
            .await
            .unwrap(),
        500
    );
    assert_eq!(
        db.collection::<Document>(GRANTS)
            .count_documents(doc! {"revoked_at":{"$type":"date"}})
            .await
            .unwrap(),
        10001
    );
    let source = rows(&db, GRANTS).await;
    let revoked_source_bytes: usize = source
        .iter()
        .filter(|r| r.get_datetime("revoked_at").is_ok())
        .map(|r| bson::to_vec(r).unwrap().len())
        .sum();
    let replacement_source_bytes: usize = source
        .iter()
        .filter(|r| r.get_datetime("revoked_at").is_err())
        .map(|r| bson::to_vec(r).unwrap().len())
        .sum();
    let intents = rows(&db, INTENTS).await;
    assert_eq!(intents.len(), 10001);
    let revoked_bytes: usize = intents
        .iter()
        .filter(|r| {
            r.get_document("event").unwrap().get_str("event_type") == Ok("durable_grant_revoked")
        })
        .map(|r| bson::to_vec(r).unwrap().len())
        .sum();
    {
        let m = f.monitor.lock().unwrap();
        println!(
            "REAUTHORITY_SCALE old=10000 new=500 elapsed={elapsed:?} request_bytes={request_bytes} revoked_intent_bytes={revoked_bytes} revoked_source_bytes={revoked_source_bytes} replacement_source_bytes={replacement_source_bytes} transaction_starts={} transaction_write_bytes={} largest_write={} commands={} unit_durations={:?}",
            m.transaction_starts,
            m.transaction_write_bytes,
            m.largest_transaction_write,
            m.commands.len(),
            m.transaction_durations
        );
    }
    cleanup(&db).await;
}
