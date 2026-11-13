use super::{inference_service as inference, inference_voice as voice};
use crate::models::downstream_service::{test_helpers::dummy_service, *};
use mongodb::bson::{self, doc};

// Frozen pre-correction seed: compare the complete stored array, including
// labels and order, rather than deriving history from today's defaults.
fn legacy_openai_voice_choices() -> Vec<VoiceChoice> {
    [
        ("marin", "Marin"),
        ("alloy", "Alloy"),
        ("ash", "Ash"),
        ("aube", "Aube"),
        ("ballad", "Ballad"),
        ("beacon", "Beacon"),
        ("bossa", "Bossa"),
        ("brise", "Brise"),
        ("cedar", "Cedar"),
        ("cinder", "Cinder"),
        ("coral", "Coral"),
        ("delta", "Delta"),
        ("echo", "Echo"),
        ("flitz", "Flitz"),
        ("gleam", "Gleam"),
        ("harema", "Harema"),
        ("juni", "Juni"),
        ("meridian", "Meridian"),
        ("nira", "Nira"),
        ("noeul", "Noeul"),
        ("nuri", "Nuri"),
        ("quartz", "Quartz"),
        ("ripple", "Ripple"),
        ("sage", "Sage"),
        ("shimmer", "Shimmer"),
        ("shitan", "Shitan"),
        ("sillage", "Sillage"),
        ("stone", "Stone"),
        ("tempo", "Tempo"),
        ("verse", "Verse"),
        ("vesper", "Vesper"),
        ("willow", "Willow"),
    ]
    .into_iter()
    .map(|(id, label)| VoiceChoice {
        id: id.into(),
        label: label.into(),
    })
    .collect()
}

/// The corrected seed: the frozen legacy list unchanged, then appended choices.
fn expected_openai_voice_choices() -> Vec<VoiceChoice> {
    let mut choices = legacy_openai_voice_choices();
    choices.push(VoiceChoice {
        id: "shida".into(),
        label: "Shida".into(),
    });
    choices
}

/// Every historically seeded choice must keep validating, which is what call
/// admission, live revalidation, recovery and dictation all rely on.
fn assert_legacy_choices_selectable(metadata: &VoiceInference, context: &str) {
    for old in legacy_openai_voice_choices() {
        assert_eq!(
            super::voice::selected_voice(metadata, &metadata.models[0].id, Some(&old.id))
                .unwrap_or_else(|_| panic!("{context}: legacy voice {} rejected", old.id)),
            old.id,
            "{context}"
        );
    }
}

#[test]
fn openai_voice_seed_appends_choices_and_keeps_every_legacy_choice_selectable() {
    let mut metadata = voice::default_voice("llm-openai").unwrap();
    // Additive: the exact legacy list (IDs, labels, order) is a prefix.
    assert_eq!(metadata.voices, expected_openai_voice_choices());
    assert_eq!(metadata.voices.len(), 33);
    assert_eq!(metadata.voices[..32], legacy_openai_voice_choices()[..]);
    assert_eq!(metadata.voices[0].id, "marin");
    assert_eq!(metadata.voices[0].label, "Marin");
    assert_eq!(
        metadata
            .voices
            .iter()
            .map(|choice| &choice.id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        33
    );
    voice::validate(&metadata).unwrap();
    assert_legacy_choices_selectable(&metadata, "seed");
    for kept in ["aube", "shitan"] {
        assert!(metadata.voices.iter().any(|choice| choice.id == kept));
    }
    assert_eq!(
        super::voice::selected_voice(&metadata, &metadata.models[0].id, None).unwrap(),
        "marin"
    );
    assert_eq!(
        super::voice::selected_voice(&metadata, &metadata.models[0].id, Some("shida")).unwrap(),
        "shida"
    );

    metadata.voices.push(VoiceChoice {
        id: "admin-added-voice".into(),
        label: "Admin voice".into(),
    });
    voice::validate(&metadata).unwrap();
    assert_eq!(
        super::voice::selected_voice(&metadata, &metadata.models[0].id, Some("admin-added-voice"))
            .unwrap(),
        "admin-added-voice"
    );
}

#[tokio::test]
async fn voice_backfill_appends_only_to_exact_unmodified_legacy_openai_choices() {
    let db = crate::test_utils::connect_transaction_test_database("voice_choices_repair").await;
    let rows = db.collection::<bson::Document>(COLLECTION_NAME);
    let legacy = legacy_openai_voice_choices();
    assert_eq!(legacy.len(), 32);
    let corrected = bson::to_bson(&expected_openai_voice_choices()).unwrap();
    assert_eq!(
        corrected,
        bson::to_bson(&voice::default_voice("llm-openai").unwrap().voices).unwrap()
    );
    let mut fixtures = Vec::new();
    for (id, should_migrate) in [
        ("legacy-unmodified", true),
        ("legacy-missing-admin-flag", true),
        ("legacy-admin-modified", false),
        ("custom-label", false),
        ("custom-order", false),
        ("custom-added", false),
        ("custom-removed", false),
        ("custom-only", false),
        ("other-slug", false),
        ("other-protocol", false),
    ] {
        let mut service = dummy_service();
        service.id = id.into();
        service.slug = "llm-openai".into();
        service.inference_admin_modified = id == "legacy-admin-modified";
        service.inference = inference::default_inference("llm-openai");
        let metadata = service.inference.as_mut().unwrap();
        // Existing backfill owns metrics and missing delegation. Use canonical
        // populated values so this test isolates the voices-only correction.
        metadata.model_list = false;
        metadata.realtime = false;
        let voice = metadata.voice.as_mut().unwrap();
        voice.voices = legacy.clone();
        voice.models.push(VoiceModel {
            id: "preserved-admin-model".into(),
            label: "Preserved model".into(),
            default: false,
        });
        match id {
            "custom-label" => voice.voices[3].label = "Custom Aube label".into(),
            "custom-order" => voice.voices.swap(0, 1),
            "custom-added" => voice.voices.push(VoiceChoice {
                id: "custom-added-voice".into(),
                label: "Custom added voice".into(),
            }),
            "custom-removed" => {
                voice.voices.remove(3);
            }
            "custom-only" => {
                voice.voices = vec![VoiceChoice {
                    id: "custom-only-voice".into(),
                    label: "Custom only voice".into(),
                }];
            }
            "other-protocol" => {
                voice.protocol = VoiceProtocol::XaiRealtime;
                voice.usage_source = VoiceUsageSource::ServerMeasured;
                voice.delegation_model = None;
            }
            _ => {}
        }
        if id == "other-slug" {
            service.slug = "custom-openai-catalog".into();
        }
        let mut before = bson::to_document(&service).unwrap();
        if id == "legacy-missing-admin-flag" {
            before.remove("inference_admin_modified");
        }
        // Raw, unknown fields also survive a targeted migration.
        before.insert(
            "preserved_migration_metadata",
            doc! {"note":"keep", "revision":7},
        );
        before
            .get_document_mut("inference")
            .unwrap()
            .get_document_mut("voice")
            .unwrap()
            .insert("preserved_voice_metadata", doc! {"enabled":true});
        rows.insert_one(&before).await.unwrap();
        fixtures.push((id, should_migrate, before));
    }

    inference::backfill(&db).await.unwrap();
    let mut after_first = Vec::new();
    for (id, should_migrate, before) in &fixtures {
        let after = rows.find_one(doc! {"_id":*id}).await.unwrap().unwrap();
        let mut expected = before.clone();
        if *should_migrate {
            expected
                .get_document_mut("inference")
                .unwrap()
                .get_document_mut("voice")
                .unwrap()
                .insert("voices", corrected.clone());
        }
        assert_eq!(
            after, expected,
            "migration changed unexpected fields for {id}"
        );

        let mut untouched_before = before.clone();
        let mut untouched_after = after.clone();
        for document in [&mut untouched_before, &mut untouched_after] {
            document
                .get_document_mut("inference")
                .unwrap()
                .get_document_mut("voice")
                .unwrap()
                .remove("voices");
        }
        assert_eq!(
            untouched_after, untouched_before,
            "only inference.voice.voices may change for {id}"
        );

        // Regression: read the updated row through the typed path used by call
        // admission/revalidation/recovery and re-run the live voice selector.
        // No previously stored choice may stop validating, on any fixture.
        let stored_before: Vec<VoiceChoice> = bson::from_bson(
            before
                .get_document("inference")
                .unwrap()
                .get_document("voice")
                .unwrap()
                .get("voices")
                .unwrap()
                .clone(),
        )
        .unwrap();
        let typed = db
            .collection::<DownstreamService>(COLLECTION_NAME)
            .find_one(doc! {"_id":*id})
            .await
            .unwrap()
            .unwrap();
        let metadata = typed.inference.unwrap().voice.unwrap();
        assert_eq!(
            metadata.voices[..stored_before.len()],
            stored_before[..],
            "stored choices must keep their IDs, labels and order for {id}"
        );
        for old in &stored_before {
            assert_eq!(
                super::voice::selected_voice(&metadata, &metadata.models[0].id, Some(&old.id))
                    .unwrap_or_else(|_| panic!("{id}: stored voice {} rejected", old.id)),
                old.id
            );
        }
        if *should_migrate {
            assert_eq!(metadata.voices, expected_openai_voice_choices());
            assert_legacy_choices_selectable(&metadata, id);
            assert_eq!(
                super::voice::selected_voice(&metadata, &metadata.models[0].id, Some("shida"))
                    .unwrap(),
                "shida"
            );
            assert_eq!(
                super::voice::selected_voice(&metadata, &metadata.models[0].id, None).unwrap(),
                "marin"
            );
        } else {
            assert_eq!(metadata.voices, stored_before, "{id} must stay untouched");
        }
        after_first.push((*id, after));
    }

    inference::backfill(&db).await.unwrap();
    for (id, expected) in after_first {
        assert_eq!(
            rows.find_one(doc! {"_id":id}).await.unwrap().unwrap(),
            expected,
            "repeated migration must be idempotent for {id}"
        );
    }
    db.drop().await.unwrap();
}

#[test]
fn voice_metadata_derivation_validation_and_rolling_compatibility() {
    for slug in ["llm-openai", "llm-xai"] {
        let mut service = dummy_service();
        service.inference = inference::default_inference(slug);
        service.inference.as_mut().unwrap().realtime = false;
        let view = inference::view(&service, None, false).unwrap();
        assert!(view.realtime);
        assert!(
            inference::capabilities(&service)
                .unwrap()
                .supports_realtime_voice
        );
        voice::validate(view.voice.as_ref().unwrap()).unwrap();
        // Old replicas did not deny unknown fields in ServiceInference.
        #[derive(serde::Deserialize)]
        struct OldInference {
            wire_protocol: InferenceWireProtocol,
            #[serde(default)]
            realtime: bool,
        }
        let wire = serde_json::to_value(inference::normalized(service.inference.as_ref().unwrap()))
            .unwrap();
        let old: OldInference = serde_json::from_value(wire.clone()).unwrap();
        assert!(old.realtime);
        assert_eq!(old.wire_protocol, view.wire_protocol);
        let mut future = wire;
        future["future_addition"] = serde_json::json!(true);
        future["voice"]["protocol"] = serde_json::json!("future_voice");
        let parsed: ServiceInference = serde_json::from_value(future).unwrap();
        assert!(!super::voice::supported_metadata(
            parsed.voice.as_ref().unwrap()
        ));
    }
    let mut metadata = voice::default_voice("llm-openai").unwrap();
    metadata.models.push(VoiceModel {
        id: "catalog-added-model".into(),
        label: "Future model".into(),
        default: false,
    });
    voice::validate(&metadata).unwrap();
    assert!(super::voice::selected_voice(&metadata, "catalog-added-model", None).is_ok());
    assert!(super::voice::selected_voice(&metadata, "unlisted-model", None).is_err());
    assert!(
        super::voice::selected_voice(&metadata, "catalog-added-model", Some("unlisted-voice"))
            .is_err()
    );
    metadata.models[1].default = true;
    assert!(voice::validate(&metadata).is_err());
    metadata.models.pop();
    metadata.voices[0].id = "https://attacker.example".into();
    assert!(voice::validate(&metadata).is_err());
    let mut legacy = dummy_service();
    legacy.inference = inference::default_inference("chrono-llm");
    legacy.inference.as_mut().unwrap().realtime = true;
    assert!(inference::capabilities(&legacy).is_none());
    let stored = bson::to_document(&legacy).unwrap();
    assert!(!stored.contains_key("supports_realtime_voice"));
}

#[tokio::test]
async fn voice_backfill_seeds_fresh_and_existing_but_preserves_admin_edits_and_clears() {
    Box::pin(async {
        let db =
            crate::test_utils::connect_transaction_test_database("voice_catalog_backfill").await;
        let rows = db.collection::<DownstreamService>(COLLECTION_NAME);
        for (id, inference_value, modified) in [
            ("fresh", None, false),
            (
                "existing",
                inference::default_inference("chrono-llm"),
                false,
            ),
            ("modified", inference::default_inference("chrono-llm"), true),
            ("cleared", None, true),
        ] {
            let mut service = dummy_service();
            service.id = id.into();
            service.slug = "llm-openai".into();
            service.inference = inference_value;
            service.inference_admin_modified = modified;
            rows.insert_one(service).await.unwrap();
        }
        inference::backfill(&db).await.unwrap();
        inference::backfill(&db).await.unwrap();
        for id in ["fresh", "existing", "modified", "cleared"] {
            let row = rows.find_one(doc! {"_id":id}).await.unwrap().unwrap();
            match id {
                "fresh" => assert_eq!(row.inference, inference::default_inference("llm-openai")),
                "existing" => {
                    let i = row.inference.unwrap();
                    assert_eq!(i.wire_protocol, InferenceWireProtocol::OpenaiCompletions);
                    assert!(
                        !i.realtime,
                        "voice-only backfill leaves other stored fields untouched"
                    );
                    assert!(i.voice.is_some());
                }
                "modified" => assert!(row.inference.unwrap().voice.is_none()),
                _ => assert!(row.inference.is_none()),
            }
        }
        db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn voice_responses_backfill_preserves_admin_model_choices() {
    let db = crate::test_utils::connect_transaction_test_database("voice_responses_backfill").await;
    let rows = db.collection::<DownstreamService>(COLLECTION_NAME);
    for (id, modified, model) in [
        ("old-default", false, None),
        ("old-admin", true, None),
        ("chosen", true, Some("admin-task-model")),
    ] {
        let mut service = dummy_service();
        service.id = id.into();
        service.slug = "llm-openai".into();
        service.inference_admin_modified = modified;
        service.inference = inference::default_inference("llm-openai");
        service
            .inference
            .as_mut()
            .unwrap()
            .voice
            .as_mut()
            .unwrap()
            .delegation_model = model.map(str::to_owned);
        rows.insert_one(service).await.unwrap();
    }
    // Reproduce the wire format from versions predating the new field.
    db.collection::<bson::Document>(COLLECTION_NAME)
        .update_many(
            doc! {"_id":{"$in":["old-default","old-admin"]}},
            doc! {"$unset":{"inference.voice.delegation_model":""}},
        )
        .await
        .unwrap();
    let mut cleared = rows
        .find_one(doc! {"_id":"old-admin"})
        .await
        .unwrap()
        .unwrap();
    cleared.id = "explicit-null".into();
    let metadata = cleared.inference.as_mut().unwrap().voice.as_mut().unwrap();
    metadata.models[0].label = "Admin voice label".into();
    metadata.billing_metrics = vec![crate::models::service_billing::BillingMetric::VoiceSeconds];
    rows.insert_one(&cleared).await.unwrap();
    let raw = db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one(doc! {"_id":"explicit-null"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        raw.get_document("inference")
            .unwrap()
            .get_document("voice")
            .unwrap()
            .get("delegation_model"),
        Some(&bson::Bson::Null)
    );
    inference::backfill(&db).await.unwrap();
    inference::backfill(&db).await.unwrap();
    let preserved = rows
        .find_one(doc! {"_id":"explicit-null"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(preserved.inference, cleared.inference);
    for (id, expected) in [
        ("old-default", Some("gpt-6-luna")),
        ("old-admin", Some("gpt-6-luna")),
        ("chosen", Some("admin-task-model")),
        ("explicit-null", None),
    ] {
        let row = rows.find_one(doc! {"_id":id}).await.unwrap().unwrap();
        assert_eq!(
            row.inference
                .unwrap()
                .voice
                .unwrap()
                .delegation_model
                .as_deref(),
            expected
        );
    }
    db.drop().await.unwrap();
}
