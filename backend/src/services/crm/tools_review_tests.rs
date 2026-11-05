use super::*;

#[tokio::test]
async fn crm_matched_revision_one_is_not_created_and_committed_result_survives_turn_end() {
    use crate::models::assistant_crm_artifact::{CrmArtifactRole, TurnCrmArtifact};
    let f = fixture().await;
    company(&f).await;
    let args = json!({"mutation_key":"unchanged-existing","input":{"object":"company","match_field":"name","values":{"name":"Company"},"expected_revision":1}});
    let first = call(&f, "crm_upsert_record", args.clone(), 1)
        .await
        .unwrap();
    assert_eq!(first["result"]["created"], false);
    assert_eq!(first["result"]["revision"], 1);
    let live = f
        .state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":&f.row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(live.active_turn.unwrap().crm_artifacts.is_empty());
    let args = json!({"mutation_key":"committed-new-record","input":{"object":"company","match_field":"name","values":{"name":"Committed"}}});
    let first = call(&f, "crm_upsert_record", args.clone(), 2)
        .await
        .unwrap();
    assert_eq!(first["result"]["created"], true);
    f.state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn":bson::Bson::Null}},
        )
        .await
        .unwrap();
    let result = super::super::artifacts::committed_result(
        &f.state.db,
        &f.chat,
        Ok(TurnCrmArtifact::Record {
            id: first["result"]["id"].as_str().unwrap().to_owned(),
            owner_id: f.owner.clone(),
            object_key: "company".into(),
            title: "Committed".into(),
            role: CrmArtifactRole::Created,
        }),
        first.clone(),
    )
    .await;
    assert_eq!(result, first);
    f.state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn":bson::to_bson(&f.row.active_turn).unwrap()}},
        )
        .await
        .unwrap();
    assert_eq!(call(&f, "crm_upsert_record", args, 2).await.unwrap(), first);
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::crm_record::COLLECTION_NAME)
            .count_documents(
                doc! {"owner_id":&f.owner,"object_key":"company","values.name":"Committed"}
            )
            .await
            .unwrap(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_empty_delivery_never_reads_the_database() {
    let f = fixture().await;
    let client =
        mongodb::Client::with_uri_str("mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=5000")
            .await
            .unwrap();
    let db = client.database("unreachable");
    assert!(
        !tokio::time::timeout(
            std::time::Duration::from_millis(100),
            super::super::artifacts::enabled_for(&db, "owner", &[])
        )
        .await
        .unwrap()
        .unwrap()
    );
    let message = bson::from_document(doc! {
        "_id":"message", "group_id":"group", "user_id":&f.owner, "seq":1_i64,
        "role":"agent", "text":"No artifacts", "created_at":bson::DateTime::now(),
    })
    .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        super::super::artifacts::publish_group(&db, &f.row, &message),
    )
    .await
    .unwrap()
    .unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_custom_object_same_key_race_and_prospective_grants_are_exact() {
    let f = fixture().await;
    for capabilities in [
        vec![CrmCapability::Read],
        vec![CrmCapability::Read, CrmCapability::Write],
    ] {
        assert!(
            validate_grants(
                &f.state.db,
                &f.owner,
                &[CrmGrant {
                    owner_id: f.owner.clone(),
                    object: "research_list".into(),
                    capabilities
                }]
            )
            .await
            .is_err()
        );
    }
    set_grants(
        &f,
        vec![(
            "research_list",
            vec![
                CrmCapability::Read,
                CrmCapability::Write,
                CrmCapability::Schema,
            ],
        )],
    )
    .await;
    let agent = f
        .state
        .db
        .collection::<AssistantAgent>(AGENTS)
        .find_one(doc! {"_id":&f.chat.agent_id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        require_object(
            &f.chat,
            &agent,
            &f.owner,
            "different_list",
            CrmCapability::Schema
        )
        .is_err()
    );
    assert!(
        require_object(
            &f.chat,
            &agent,
            &f.owner,
            "different_list",
            CrmCapability::Read
        )
        .is_err()
    );
    assert!(
        require_object(
            &f.chat,
            &agent,
            &f.owner,
            "different_list",
            CrmCapability::Write
        )
        .is_err()
    );
    let input: objects::CreateObjectInput = serde_json::from_value(json!({"key":"research_list","label":"Research","plural_label":"Research lists","fields":[{"key":"name","label":"Name","type":"text","max_length":200,"required":true,"archived":false}]})).unwrap();
    let (a, b) = tokio::join!(
        objects::create(&f.state.db, &f.owner, input.clone(), None),
        objects::create(&f.state.db, &f.owner, input, None)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert!(matches!(
        a.err().or_else(|| b.err()).unwrap(),
        AppError::CrmValidationFailed(_)
    ));
    assert_eq!(objects::list(&f.state.db, &f.owner).await.unwrap().len(), 5);
    call(&f,"crm_upsert_record",json!({"mutation_key":"research-row","input":{"object":"research_list","match_field":"name","values":{"name":"Research row"}}}),1).await.unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_seed_lengths_are_insert_only_and_present_summaries_are_bounded() {
    use crate::models::crm_object::{COLLECTION_NAME as OBJECTS, CrmFieldKind};
    let f = fixture().await;
    for (key, fields) in [
        ("company", vec![("name", 200), ("domain", 253)]),
        (
            "contact",
            vec![("name", 200), ("email", 254), ("title", 200)],
        ),
    ] {
        let object = objects::get(&f.state.db, &f.owner, key).await.unwrap();
        for (key, expected) in fields {
            assert!(
                matches!(object.fields.iter().find(|field|field.key==key).unwrap().kind, CrmFieldKind::Text {max_length} if max_length==expected)
            );
        }
    }
    f.state
        .db
        .collection::<Document>(OBJECTS)
        .update_one(
            doc! {"owner_id":&f.owner,"key":"company"},
            doc! {"$set":{"fields.0.max_length":1000}},
        )
        .await
        .unwrap();
    objects::ensure_seeded(&f.state.db, &f.owner).await.unwrap();
    let mut object = objects::get(&f.state.db, &f.owner, "company")
        .await
        .unwrap();
    assert!(matches!(
        object.fields[0].kind,
        CrmFieldKind::Text { max_length: 1000 }
    ));
    object.fields = (0..7)
        .map(|n| crate::models::crm_object::CrmField {
            key: format!("field_{n}"),
            label: format!("Field {n}"),
            required: n == 0,
            archived: false,
            kind: CrmFieldKind::Text { max_length: 1000 },
        })
        .collect();
    f.state
        .db
        .collection::<crate::models::crm_object::CrmObject>(OBJECTS)
        .replace_one(doc! {"_id":&object.id}, &object)
        .await
        .unwrap();
    let record = records::create(
        &f.state.db,
        &f.owner,
        CrmActor::human(&f.owner),
        records::CreateRecordInput {
            object: "company".into(),
            values: (0..7)
                .map(|n| (format!("field_{n}"), json!("x".repeat(250))))
                .collect(),
            protect: vec![],
        },
        None,
    )
    .await
    .unwrap();
    let result = call(
        &f,
        "crm_present",
        json!({"kind":"record","id":record.id}),
        1,
    )
    .await
    .unwrap();
    let summary = &result["result"];
    assert_eq!(summary["title"], "x".repeat(200));
    assert_eq!(summary["fields"].as_array().unwrap().len(), 5);
    assert_eq!(
        summary["fields"][4],
        json!({"key":"field_4","label":"Field 4","value":"x".repeat(200)})
    );
    assert_eq!(summary["revision"], 1);
    assert!(summary.get("values").is_none());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_async_chat_and_trigger_links_are_exact_once_and_flag_gated() {
    use crate::models::assistant_crm_artifact::{CrmArtifactRole, TurnCrmArtifact};
    let f = fixture().await;
    let mut row = f.row.clone();
    row.active_turn.as_mut().unwrap().crm_artifacts = vec![TurnCrmArtifact::Object {
        id: "object-id".into(),
        owner_id: f.owner.clone(),
        object_key: "asset".into(),
        title: "Private dataset title".into(),
        role: CrmArtifactRole::Created,
    }];
    let expected = format!(
        "Answer\n\n{}/assistant/crm?owner={}&object=asset",
        f.state.config.frontend_url.trim_end_matches('/'),
        f.owner
    );
    let rendered =
        crate::handlers::assistant_team::channel_artifact_text(&f.state, &row, "Answer").await;
    assert_eq!(rendered, expected);
    assert_eq!(
        crate::handlers::assistant_team::channel_artifact_text(&f.state, &row, &rendered).await,
        expected
    );
    assert_eq!(
        crate::handlers::assistant_team::channel_artifact_text(
            &f.state,
            &row,
            "I couldn't finish answering that question. Please ask again."
        )
        .await,
        expected.replace(
            "Answer",
            "I couldn't finish answering that question. Please ask again."
        )
    );
    feature_flag_service::set_platform_override(
        &f.state.db,
        super::super::FLAG_KEY,
        &FlagTarget::User(f.owner.clone()),
        false,
        &f.owner,
    )
    .await
    .unwrap();
    assert_eq!(
        crate::handlers::assistant_team::channel_artifact_text(&f.state, &row, "Answer").await,
        "Answer"
    );
    f.state.db.drop().await.unwrap();
}
