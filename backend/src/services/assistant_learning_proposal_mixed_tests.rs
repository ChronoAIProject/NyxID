//! Shared by the two binaries in the record-field compatibility driver.
use super::*;
use crate::{
    crypto::aes::EncryptionKeys, services::assistant_skill_authoring::tests::contextualize,
};
use bson::Document;
use std::sync::Arc;

async fn written(keys: Arc<EncryptionKeys>) -> Document {
    let mut bundle = Box::pin(
        crate::services::assistant_skill_authoring::tests::recovery::mixed_authored_row(
            keys.clone(),
        ),
    )
    .await;
    bundle.insert(
        "learned",
        Box::pin(learning::tests::mixed_learned_row(keys.clone())).await,
    );
    for kind in ["authored", "learned"] {
        let row = bundle.get_document(kind).unwrap();
        assert!(!row.contains_key("body_context_version"));
        assert!(!row.contains_key("scope_context_version"));
        keys.decrypt(row.get_binary_generic("body_encrypted").unwrap())
            .await
            .unwrap();
        if kind == "authored" {
            let p = row.get_document("publication").unwrap();
            assert!(!p.contains_key("interface_context_version"));
            keys.decrypt(row.get_binary_generic("scope_encrypted").unwrap())
                .await
                .unwrap();
            keys.decrypt(p.get_binary_generic("interface_encrypted").unwrap())
                .await
                .unwrap();
        }
    }
    bundle
}

async fn check_read(state: &AppState, bundle: &Document, reject: bool) {
    let learned = bson::from_document(bundle.get_document("learned").unwrap().clone()).unwrap();
    let generated = draft(state, &learned).await;
    let target = original_target_from_body(state, &learned).await;
    if reject {
        assert!(generated.is_err() && target.is_err());
    } else {
        assert_eq!(generated.unwrap().name, "checklist");
        assert_eq!(target.unwrap(), Some(None));
    }
    let row: AssistantAgentLearningProposal =
        bson::from_document(bundle.get_document("authored").unwrap().clone()).unwrap();
    if row.status == "rejected" {
        assert!(row.body_encrypted.is_empty());
        assert!(draft(state, &row).await.is_err());
        assert_eq!(original_target_from_body(state, &row).await.unwrap(), None);
        return;
    }
    let decoded = draft(state, &row).await;
    if row.publication.is_none() {
        assert_eq!(decoded.unwrap().description, "Edited weekly review");
        return;
    }
    let agent = bson::from_document(bundle.get_document("agent").unwrap().clone()).unwrap();
    let preview = preview_value(state, &agent, &row).await.unwrap().0;
    assert_eq!(preview["scope_unavailable"], reject);
    assert_eq!(preview["failure_code"] == "draft_unavailable", reject);
    let target = original_target_from_body(state, &row).await;
    if reject {
        assert!(decoded.is_err() && target.is_err());
        assert!(preview["scope"].is_null());
        assert!(preview["files"].as_array().unwrap().is_empty());
    } else {
        let base = decoded.unwrap().base_skill.unwrap();
        assert_eq!(target.unwrap(), Some(Some(base.skill_id)));
        assert_eq!(preview["scope"]["status"], "reduced");
        assert!(
            preview["files"][0]["content"]
                .as_str()
                .unwrap()
                .contains("search_web")
        );
    }
}

async fn change(state: &AppState, bundle: &mut Document, clear: bool) {
    let raw = bundle.get_document("authored").unwrap();
    let row: AssistantAgentLearningProposal = bson::from_document(raw.clone()).unwrap();
    let db = &state.db;
    db.collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .insert_one(bundle.get_document("agent").unwrap())
        .await
        .unwrap();
    feature_flag_service::set_platform_override(
        db,
        learning::FLAG_KEY,
        &FlagTarget::Global,
        true,
        &row.owner_id,
    )
    .await
    .unwrap();
    let proposals = db.collection::<Document>(PROPOSALS_COLLECTION_NAME);
    proposals.insert_one(raw).await.unwrap();
    if clear {
        super::reject(
            state,
            &row.owner_id,
            &row.agent_id,
            &row.id,
            row.revision,
            "rejected",
        )
        .await
        .unwrap();
    } else {
        let mut edited = draft(state, &row).await.unwrap();
        edited.description = "Edited weekly review".into();
        edit(
            state,
            &row.owner_id,
            &row.agent_id,
            &row.id,
            row.revision,
            &json!(edited),
        )
        .await
        .unwrap();
    }
    let next = proposals
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!next.contains_key("body_context_version"));
    for sibling in ["scope_encrypted", "scope_context_version", "scope_digest"] {
        assert_eq!(next.get(sibling), raw.get(sibling), "{sibling}");
    }
    if clear {
        assert!(!next.contains_key("body_encrypted"));
    } else {
        state
            .encryption_keys
            .decrypt(next.get_binary_generic("body_encrypted").unwrap())
            .await
            .unwrap();
    }
    bundle.insert("authored", next);
}

#[tokio::test]
async fn learning_proposal_mixed_records() {
    let step = std::env::var("NYXID_RECORD_FIXTURE_STEP").unwrap_or_else(|_| "roundtrip".into());
    let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
    let db = crate::test_utils::connect_transaction_test_database("mixed_learning_readers").await;
    let mut state = crate::test_utils::test_app_state(db.clone());
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(
        step == "roundtrip" || std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true"),
    );
    keys.set_allow_legacy_record_reads(
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false"),
    );
    state.encryption_keys = Arc::new(keys);
    let mut bundle: Document = match step.as_str() {
        "roundtrip" | "write" | "contextual-fixture" => {
            Box::pin(written(state.encryption_keys.clone())).await
        }
        _ => bson::from_slice(&std::fs::read(file.as_ref().unwrap()).unwrap()).unwrap(),
    };
    match step.as_str() {
        "roundtrip" | "contextual-fixture" => {
            for kind in ["authored", "learned"] {
                contextualize(
                    &state.encryption_keys,
                    bundle.get_document_mut(kind).unwrap(),
                )
                .await;
            }
            check_read(&state, &bundle, false).await;
        }
        "read" | "reject" | "write" => check_read(&state, &bundle, step == "reject").await,
        "replace" | "clear" | "replace-contextual" | "clear-contextual" => {
            change(&state, &mut bundle, step.starts_with("clear")).await;
        }
        _ => panic!("unknown learning fixture step"),
    }
    if let Some(file) = file
        && matches!(
            step.as_str(),
            "write" | "contextual-fixture" | "replace" | "clear"
        )
    {
        std::fs::write(file, bson::to_vec(&bundle).unwrap()).unwrap();
    }
    db.drop().await.unwrap();
}
