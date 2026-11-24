use super::*;
use std::collections::HashSet;

fn pending_sources() -> Vec<(&'static str, Document)> {
    use crate::models::*;
    let bytes = Bson::Binary(bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes: vec![0xa5],
    });
    let grant = auth_device_code::AuthDeviceAgentKeyGrant {
        api_key_id: "key".into(),
        credential_id: "grant".into(),
        key_was_created: true,
        delivery_credential_encrypted: Some(vec![0xa5]),
    };
    let mut sources = vec![
        (channel_meta::SESSIONS, doc! {"assets_encrypted": &bytes}),
        (
            mobile_social::MobileSocialDelivery::COLLECTION_NAME,
            doc! {"ciphertext": &bytes},
        ),
        (
            assistant_cma_sandbox::COLLECTION_NAME,
            doc! {"request_ciphertext": &bytes},
        ),
        (
            assistant_cma_creation::COLLECTION_NAME,
            doc! {"request_ciphertext": &bytes},
        ),
        (
            nyxbot_event_binding::COLLECTION_NAME,
            doc! {"provider_key_ciphertext": &bytes, "producer_secret_ciphertext": &bytes},
        ),
        (
            auth_device_code::V2_COLLECTION_NAME,
            doc! {"agent_key_grant": bson::to_document(&grant).unwrap()},
        ),
        (
            auth_device_code::V2_COLLECTION_NAME,
            doc! {"delivery_access_token_encrypted": &bytes, "delivery_refresh_token_encrypted": &bytes},
        ),
        (
            nyxbot_channel::EVENTS_COLLECTION_NAME,
            doc! {"delivery": {"target_ciphertext": &bytes}, "event_context_ciphertext": &bytes, "guest_setup_payload": &bytes},
        ),
        (
            nyxbot_channel::THREADS_COLLECTION_NAME,
            doc! {"event_ref_ciphertext": &bytes},
        ),
        (
            nyxbot_gateway_response::COLLECTION_NAME,
            doc! {"payload_ciphertext": &bytes},
        ),
        (
            channel_whatsapp_native::COLLECTION_NAME,
            doc! {"native_verify_token_encrypted": &bytes},
        ),
        (
            assistant_conversation::COLLECTION_NAME,
            doc! {"active_turn": {"restart_admission": {"window": {"input": &bytes}}}},
        ),
    ];
    for version in ["v1", "v2", "v3", "v4"] {
        sources.push((
            approval_request::COLLECTION_NAME,
            doc! {"execution_proposal": {version: {"payload": &bytes, "result": &bytes}}},
        ));
    }
    sources
}

#[test]
fn previous_labels_match_the_ordered_base_registry() {
    use sha2::{Digest, Sha256};

    let labels = serde_json::to_vec(previous_labels::LABELS).unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(labels)),
        "2429aeaea43635d2e7418b096d9a8305888e4aea11a9bc8d5aa0ac864f5761e0"
    );
}

#[test]
fn registered_paths_match_stored_material_and_the_documented_inventory() {
    let inventory = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../docs/security/ENCRYPTED_FIELD_INVENTORY.md"
    ));
    let rows: Vec<_> = inventory
        .lines()
        .filter(|line| line.starts_with("| `"))
        .collect();
    assert_eq!(rows.len(), REGISTRY.len());
    let mut unique = HashSet::new();
    for (entry, line) in REGISTRY.iter().zip(rows) {
        let cells: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
        assert_eq!(cells[0].trim_matches('`'), entry.collection);
        let fields: Vec<_> = cells[1].split(", ").map(|f| f.trim_matches('`')).collect();
        assert_eq!(fields, entry.fields);
        assert_eq!(
            cells[3],
            if matches!(entry.adapter, Adapter::Pending) {
                "Pending"
            } else {
                "Implemented"
            }
        );
        for field in entry.fields {
            assert!(
                unique.insert((entry.collection, *field)),
                "duplicate {}.{field}",
                entry.collection
            );
        }
    }
    for collection in [
        RUNS,
        "node_pending_credentials",
        "managed_machine_bootstrap_deliveries",
        "nyxbot_watches",
    ] {
        assert!(!REGISTRY.iter().any(|entry| entry.collection == collection));
    }
    assert!(!unique.contains(&("nyxbot_events", "event_ref_ciphertext")));
    assert!(!unique.contains(&("auth_device_codes_v2", "delivery_credential_encrypted")));

    fn assert_binary_paths(
        collection: &str,
        doc: &Document,
        prefix: &str,
        fields: &HashSet<(&str, &str)>,
    ) {
        for (key, value) in doc {
            let path = format!("{prefix}{key}");
            match value {
                Bson::Document(child) => {
                    assert_binary_paths(collection, child, &format!("{path}."), fields)
                }
                Bson::Binary(_) => assert!(
                    fields.contains(&(collection, path.as_str())),
                    "{collection}.{path}"
                ),
                _ => {}
            }
        }
    }
    for (collection, source) in pending_sources() {
        let entry = REGISTRY
            .iter()
            .find(|entry| entry.collection == collection)
            .unwrap();
        assert!(matches!(entry.adapter, Adapter::Pending));
        assert!(has_material(entry, &source));
        assert_binary_paths(collection, &source, "", &unique);
        for field in entry.fields {
            assert_eq!(
                field_has_material(&source, field),
                matches!(path(&source, field), Some(Bson::Binary(_)))
            );
            assert!(!field_has_material(&doc! {}, field));
            let mut null = Bson::Null;
            for component in field.split('.').rev() {
                null = doc! {component: null}.into();
            }
            assert!(!field_has_material(null.as_document().unwrap(), field));
            if let Some((_, leaf)) = field.rsplit_once('.') {
                assert!(!field_has_material(
                    &doc! {leaf: 1, "sibling": {leaf: 1}},
                    field
                ));
            }
        }
    }
}

#[tokio::test]
async fn status_selects_the_generations_labels_and_keeps_the_live_digest_fence() {
    let db = crate::test_utils::connect_test_database("registry_status_labels")
        .await
        .unwrap();
    let keys = crate::test_utils::test_encryption_keys();
    let generation = start(&db, &keys, Mode::Verify, None).await.unwrap();
    let runs = db.collection::<Document>(RUNS);
    let original = runs.find_one(doc! {"_id":RUN_ID}).await.unwrap().unwrap();
    let live = registry_digest();
    assert_ne!(live, previous_labels::DIGEST);
    for digest in [
        Some(live.as_str()),
        Some(previous_labels::DIGEST),
        Some("unknown"),
        None,
    ] {
        let mut row = original.clone();
        row.extend(doc! {
            "entry": 30,
            "scanned": 9_i64,
            "rewritten": 3_i64,
            "unresolved": 1_i64,
            "field_outcomes": {
                "21": {"0": {"verification_blocked": 7_i64}},
                "30": {"0": {"authenticated_current": 2_i64}},
            },
        });
        if let Some(digest) = digest {
            row.insert("registry_digest", digest);
        } else {
            row.remove("registry_digest");
        }
        runs.replace_one(doc! {"_id":RUN_ID}, &row).await.unwrap();
        let report = status(&db, &keys).await.unwrap().unwrap();
        assert_eq!(
            (report.scanned, report.rewritten, report.unresolved),
            (9, 3, 1)
        );
        assert_eq!(
            report
                .blockers
                .contains(&"registry_or_primary_provider_changed"),
            digest != Some(live.as_str())
        );
        let expected = match digest {
            Some(previous_labels::DIGEST) => Some((
                "nyxbot_watches",
                "target_ciphertext",
                "delivery_credential_encrypted",
            )),
            Some(value) if value == live => Some((
                "mobile_social_deliveries",
                "ciphertext",
                "agent_key_grant.delivery_credential_encrypted",
            )),
            _ => None,
        };
        if let Some((collection, field, grant)) = expected {
            assert_eq!(report.collection, Some(collection));
            assert!(
                report
                    .field_outcomes
                    .iter()
                    .any(|f| f.collection == collection
                        && f.field == field
                        && f.authenticated_current == 2)
            );
            assert!(
                report
                    .field_outcomes
                    .iter()
                    .any(|f| f.collection == "auth_device_codes_v2"
                        && f.field == grant
                        && f.verification_blocked == 7)
            );
        } else {
            assert!(report.collection.is_none() && report.field_outcomes.is_empty());
        }
        if digest != Some(live.as_str()) {
            assert!(matches!(
                advance(&db, &keys, &generation).await,
                Err(AppError::Conflict(_))
            ));
        }
    }
    let mut damaged_proof = original.clone();
    damaged_proof.insert("provider_proof", Bson::Null);
    runs.replace_one(doc! {"_id":RUN_ID}, damaged_proof)
        .await
        .unwrap();
    let report = status(&db, &keys).await.unwrap().unwrap();
    assert!(
        report
            .blockers
            .contains(&"registry_or_primary_provider_changed")
    );
    assert_eq!(report.collection, Some(REGISTRY[0].collection));
    assert!(!report.field_outcomes.is_empty());

    let entry = &REGISTRY[0];
    let id = "a".repeat(64);
    db.collection::<Document>(FINDINGS)
        .insert_one(doc! {
            "_id": &id, "entry": 0, "collection": entry.collection,
            "fields": entry.fields, "entry_contract": entry_contract(entry),
            "registry_digest": previous_labels::DIGEST,
        })
        .await
        .unwrap();
    for rewrap in [false, true] {
        assert!(matches!(
            lookup_finding(&db, &keys, &id, None, rewrap).await,
            Err(AppError::Conflict(_))
        ));
    }
    let mut previous = original;
    previous.insert("registry_digest", previous_labels::DIGEST);
    runs.replace_one(doc! {"_id": RUN_ID}, previous)
        .await
        .unwrap();
    let next = start(&db, &keys, Mode::Verify, Some(&generation))
        .await
        .unwrap();
    advance(&db, &keys, &next).await.unwrap();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn pending_sources_are_reported_and_preserved_by_both_walk_modes() {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let db = crate::test_utils::connect_test_database("registry_pending_walk")
        .await
        .unwrap();
    let keys = crate::test_utils::test_encryption_keys();
    let mut sources = pending_sources();
    for (index, (collection, source)) in sources.iter_mut().enumerate() {
        let mut row = doc! {"_id": index.to_string()};
        row.extend(std::mem::take(source));
        *source = row;
        db.collection::<Document>(collection)
            .insert_one(&*source)
            .await
            .unwrap();
    }
    for mode in [Mode::Verify, Mode::Migrate] {
        let generation = start(&db, &keys, mode, None).await.unwrap();
        let report = finish(&db, &keys, &generation).await;
        assert_eq!(report.unresolved, sources.len() as i64);
        assert_eq!(report.rewritten, 0);
        for (collection, source) in &sources {
            let stored = db
                .collection::<Document>(collection)
                .find_one(doc! {"_id":source.get_str("_id").unwrap()})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                bson::to_vec(source).unwrap(),
                bson::to_vec(&stored).unwrap()
            );
            let entry = REGISTRY
                .iter()
                .find(|e| e.collection == *collection)
                .unwrap();
            for field in entry
                .fields
                .iter()
                .filter(|field| field_has_material(source, field))
            {
                assert!(
                    report
                        .field_outcomes
                        .iter()
                        .any(|f| f.collection == *collection
                            && f.field == *field
                            && f.verification_blocked > 0
                            && f.authenticated_current == 0)
                );
            }
        }
    }
    db.drop().await.unwrap();
}
