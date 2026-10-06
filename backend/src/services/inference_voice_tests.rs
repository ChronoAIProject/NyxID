use super::{inference_service as inference, inference_voice as voice};
use crate::models::downstream_service::{test_helpers::dummy_service, *};
use mongodb::bson::{self, doc};

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
