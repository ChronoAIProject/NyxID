use super::*;
use crate::services::{
    assistant_authority_tests::{self, Fixture},
    feature_flag_service::{self, FlagTarget},
};
use chrono::{Duration, Utc};
use mongodb::bson::{self, Document};

pub(crate) async fn fixture() -> Fixture {
    let f = assistant_authority_tests::direct_fixture("crm_native").await;
    super::super::ensure_indexes(&f.state.db).await.unwrap();
    objects::ensure_seeded(&f.state.db, &f.owner).await.unwrap();
    feature_flag_service::set_platform_override(
        &f.state.db,
        super::super::FLAG_KEY,
        &FlagTarget::User(f.owner.clone()),
        true,
        &f.owner,
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"access_mode_override":"autopilot"}},
        )
        .await
        .unwrap();
    set_grants(
        &f,
        vec![
            ("contact", vec![CrmCapability::Read, CrmCapability::Write]),
            (
                "company",
                vec![
                    CrmCapability::Read,
                    CrmCapability::Write,
                    CrmCapability::Schema,
                ],
            ),
        ],
    )
    .await;
    f
}
async fn set_grants(f: &Fixture, scopes: Vec<(&str, Vec<CrmCapability>)>) {
    let grants: Vec<_> = scopes
        .into_iter()
        .map(|(object, capabilities)| CrmGrant {
            owner_id: f.owner.clone(),
            object: object.into(),
            capabilities,
        })
        .collect();
    validate_grants(&f.state.db, &f.owner, &grants)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"grants.crm":bson::to_bson(&grants).unwrap()}},
        )
        .await
        .unwrap();
}
async fn call(f: &Fixture, name: &str, args: Value, rpc: i32) -> AppResult<Value> {
    execute(
        &f.state,
        &f.chat,
        &format!("nyxid__{name}"),
        &args,
        &json!(rpc),
    )
    .await
}
async fn company(f: &Fixture) -> records::RecordResponse {
    records::create(
        &f.state.db,
        &f.owner,
        CrmActor::human(&f.owner),
        records::CreateRecordInput {
            object: "company".into(),
            values: serde_json::from_value(json!({"name":"Company"})).unwrap(),
            protect: vec![],
        },
        None,
    )
    .await
    .unwrap()
}

#[test]
fn crm_native_discovery_and_admission_agree_on_closed_shapes() {
    for name in NAMES {
        let schema = schema(name);
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(action_class(name).is_some(), true);
    }
    let read = json!({"input":{"object":"company"}});
    assert!(validate_shape(&read, &schema("crm_query")));
    for bad in [
        json!({"record_id":uuid::Uuid::new_v4(),"input":{"object":"company"}}),
        json!({"owner_id":5,"input":{"object":"company"}}),
        json!({"owner_id":"bad","input":{"object":"company"}}),
        json!({"input":{"object":"BAD"}}),
    ] {
        assert!(!validate_shape(&bad, &schema("crm_query")), "{bad}");
    }
    let options: Vec<_> = (0..100)
        .map(|n| json!({"value":format!("{n}_option"),"label":format!("Option {n}")}))
        .collect();
    let args = json!({"mutation_key":"schema-1","object":"company","field_key":"industry","input":{"expected_schema_revision":1,"options":options}});
    assert!(validate_shape(&args, &schema("crm_update_field")));
    let mut invalid = args.clone();
    invalid["input"]["options"][0]["value"] = json!("bad-key");
    assert!(!validate_shape(&invalid, &schema("crm_update_field")));
    assert_ne!(
        mutation_identity("chat", "first"),
        mutation_identity("chat", "second")
    );
    assert_eq!(
        mutation_identity("chat", "first"),
        mutation_identity("chat", "first")
    );
}

#[tokio::test]
async fn crm_native_annotations_follow_declared_action_class() {
    let f = fixture().await;
    let service =
        crate::services::assistant_account_tools::virtual_service_for(&f.state.db, &f.chat).await;
    for (tool, class, hints) in [
        (
            "nyxid__crm_query",
            acks::OwnerActionClass::Read,
            (true, false, false),
        ),
        (
            "nyxid__crm_patch_record",
            acks::OwnerActionClass::Change,
            (false, false, false),
        ),
        (
            "nyxid__crm_delete_view",
            acks::OwnerActionClass::Destructive,
            (false, true, false),
        ),
    ] {
        assert_eq!(acks::known_action_class(tool, &json!({})), Some(class));
        let annotations =
            crate::services::mcp_service::tool_annotations(tool, std::slice::from_ref(&service))
                .unwrap();
        assert_eq!(
            (
                annotations["readOnlyHint"].as_bool().unwrap(),
                annotations["destructiveHint"].as_bool().unwrap(),
                annotations["openWorldHint"].as_bool().unwrap(),
            ),
            hints,
            "{tool}"
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_native_scopes_reference_targets_and_write_receipt_identity() {
    let f = fixture().await;
    let record = company(&f).await;
    let args = json!({"record_id":record.id,"mutation_key":"patch-1","input":{"expected_revision":1,"values":{"domain":"new.test"}}});
    let first = call(&f, "crm_patch_record", args.clone(), 1).await.unwrap();
    let replay = call(&f, "crm_patch_record", args.clone(), 99)
        .await
        .unwrap();
    assert_eq!(first, replay);
    let audit = f
        .state
        .db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"event_type":"crm_tool_mutation","event_data.tool":"crm_patch_record"})
        .await
        .unwrap()
        .unwrap();
    let metadata = audit.get_document("event_data").unwrap();
    assert_eq!(metadata.get_str("owner_id").unwrap(), f.owner);
    assert_eq!(
        metadata
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        ["owner_id", "tool", "conversation_id", "turn_id"]
            .map(str::to_owned)
            .into_iter()
            .collect()
    );
    assert!(!format!("{metadata:?}").contains("new.test"));
    let second=call(&f,"crm_patch_record",json!({"record_id":record.id,"mutation_key":"patch-2","input":{"expected_revision":2,"values":{"domain":"second.test"}}}),1).await.unwrap();
    assert_eq!(second["result"]["revision"], 3);
    let agent = f
        .state
        .db
        .collection::<AssistantAgent>(AGENTS)
        .find_one(doc! {"_id":&f.chat.agent_id})
        .await
        .unwrap()
        .unwrap();
    let detail = records::get(&f.state.db, &f.owner, &record.id)
        .await
        .unwrap();
    let actor = detail.updated_by.unwrap();
    assert_eq!(actor.agent_name.as_deref(), Some(agent.name.as_str()));
    let history = activities::list(&f.state.db, &f.owner, &record.id, None)
        .await
        .unwrap();
    assert!(
        history
            .activities
            .iter()
            .any(|row| row.actor.agent_name.as_deref() == Some(agent.name.as_str()))
    );
    let roster = crate::services::assistant_team_service::roster_note(&f.state.db, &f.owner)
        .await
        .unwrap();
    assert!(roster.contains("company [read,write,schema]"));
    assert!(matches!(call(&f,"crm_patch_record",json!({"record_id":record.id,"mutation_key":"patch-1","input":{"expected_revision":3,"values":{}}}),7).await,Err(AppError::CrmIdempotencyMismatch)));
    set_grants(
        &f,
        vec![("contact", vec![CrmCapability::Read, CrmCapability::Write])],
    )
    .await;
    assert!(
        call(
            &f,
            "crm_query",
            json!({"record_id":record.id,"input":{"object":"company"}}),
            2
        )
        .await
        .is_err()
    );
    assert!(
        call(&f, "crm_query", json!({"input":{"object":"company"}}), 2)
            .await
            .is_err()
    );
    assert!(call(&f,"crm_query",json!({"input":{"object":"contact","filters":[{"field":"company","op":"eq","value":record.id}]}}),2).await.is_err());
    assert!(call(&f,"crm_upsert_record",json!({"mutation_key":"relation","input":{"object":"contact","match_field":"email","values":{"name":"Contact","email":"a@test","company":record.id}}}),3).await.is_err());
    let errors = crate::services::assistant_account_tools::error_result(
        super::super::validation_error("values.name", "is required"),
    );
    assert_eq!(errors.value["errors"][0]["path"], "values.name");
    let mut guest = f.chat.clone();
    guest.guest = true;
    assert!(!available(&f.state.db, &guest).await.unwrap());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_native_schema_dto_replay_and_concrete_confirmation() {
    let f = fixture().await;
    let mut args = json!({"mutation_key":"add-score","object":"company","input":{"expected_schema_revision":1,"field":{"key":"score","label":"Score","type":"number","format":"percent","required":false,"archived":false}}});
    // Honor an owner card if the configured access mode requests one.
    let refusal = call(&f, "crm_add_field", args.clone(), 1).await.unwrap();
    if let Some(ack) = refusal["acknowledgement_id"].as_str() {
        acks::decide(&f.state.db, &f.owner, &f.chat.conversation_id, ack, true)
            .await
            .unwrap();
        args["acknowledgement_id"] = json!(ack);
    }
    let first = call(&f, "crm_add_field", args.clone(), 2).await.unwrap();
    let replay = call(&f, "crm_add_field", args.clone(), 3).await.unwrap();
    assert_eq!(first, replay);
    assert!(first["result"]["id"].is_string());
    assert!(first["result"].get("_id").is_none());
    assert!(
        first["result"]["created_at"]
            .as_str()
            .unwrap()
            .contains('T')
    );
    let change = json!({"object":"company","field_key":"score","input":{"expected_schema_revision":2,"required":true,"archived":true,"options":[{"value":"1_option","label":"One","archived":true}]}});
    let summary = confirmation_summary(
        &f.state.db,
        &f.owner,
        "crm_update_field",
        &change,
        Some("company"),
    )
    .await
    .unwrap();
    assert!(summary.contains("required=true"));
    assert!(summary.contains("archived=true"));
    assert!(summary.contains("1_option=One [archived]"));
    let record = company(&f).await;
    let archive = confirmation_summary(
        &f.state.db,
        &f.owner,
        "crm_archive_record",
        &json!({"record_id":record.id}),
        Some("company"),
    )
    .await
    .unwrap();
    assert!(archive.contains("archive record"));
    assert!(archive.contains(&record.id));
    assert!(archive.contains("/assistant/crm?owner="));
    assert_ne!(archive, summary);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_native_transaction_fence_checks_turn_key_session_and_live_grants() {
    for failure in ["turn", "key", "session", "grant"] {
        let f = fixture().await;
        let agent = f
            .state
            .db
            .collection::<AssistantAgent>(AGENTS)
            .find_one(doc! {"_id":&f.chat.agent_id})
            .await
            .unwrap()
            .unwrap();
        let admission = MutationAdmission {
            user_id: f.owner.clone(),
            agent_id: agent.id,
            agent_owner_id: f.owner.clone(),
            session_id: f.row.nyxagent_session_id.clone(),
            conversation_id: f.row.id.clone(),
            turn_id: f.chat.turn_id.clone().unwrap(),
            api_key_id: f.chat.api_key_id.clone(),
            grants: bson::to_bson(&agent.grants.crm).unwrap(),
            owner: f.owner.clone(),
            schema: false,
        };
        match failure {
            "turn" => {
                f.state.db.collection::<Document>(CONVERSATIONS).update_one(doc!{"_id":&f.row.id},doc!{"$set":{"active_turn.lease_expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
            }
            "key" => {
                f.state
                    .db
                    .collection::<Document>(crate::models::api_key::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&f.chat.api_key_id},
                        doc! {"$set":{"is_active":false}},
                    )
                    .await
                    .unwrap();
            }
            "session" => {
                f.state
                    .db
                    .collection::<Document>(CONVERSATIONS)
                    .update_one(
                        doc! {"_id":&f.row.id},
                        doc! {"$set":{"nyxagent_session_id":"rebound"}},
                    )
                    .await
                    .unwrap();
            }
            _ => {
                set_grants(&f, vec![("contact", vec![CrmCapability::Read])]).await;
            }
        }
        let result = MUTATION_ADMISSION
            .scope(
                admission,
                records::create(
                    &f.state.db,
                    &f.owner,
                    CrmActor::human(&f.owner),
                    records::CreateRecordInput {
                        object: "company".into(),
                        values: serde_json::from_value(json!({"name":"Must not commit"})).unwrap(),
                        protect: vec![],
                    },
                    None,
                ),
            )
            .await;
        assert!(
            matches!(result, Err(AppError::Forbidden(_))),
            "{failure}: {result:?}"
        );
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::crm_record::COLLECTION_NAME)
                .count_documents(doc! {"owner_id":&f.owner})
                .await
                .unwrap(),
            0
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn crm_viewer_reads_do_not_take_mutation_admission_and_revoked_members_cannot_write() {
    let f = crate::services::org_agent_tests::Fixture::new("crm_viewer_reads").await;
    super::super::ensure_indexes(&f.state.db).await.unwrap();
    for actor in [&f.viewer, &f.member] {
        feature_flag_service::set_platform_override(
            &f.state.db,
            super::super::FLAG_KEY,
            &FlagTarget::User(actor.clone()),
            true,
            &f.admin,
        )
        .await
        .unwrap();
        let row = assistant_authority_tests::new_orchestrator(&f.state, actor, "Inspect CRM").await;
        let chat = acks::for_key(&f.state.db, actor, Some(&row.credential_api_key_id))
            .await
            .unwrap()
            .unwrap();
        let read = execute(
            &f.state,
            &chat,
            "nyxid__crm_schema",
            &json!({"owner_id":f.org}),
            &json!(1),
        )
        .await
        .unwrap();
        assert_eq!(read["result"].as_array().unwrap().len(), 4);
        let agent = f
            .state
            .db
            .collection::<Document>(AGENTS)
            .find_one(doc! {"_id":&chat.agent_id})
            .await
            .unwrap()
            .unwrap();
        assert!(!agent.contains_key("crm_write_fence"));
        let args = json!({"owner_id":f.org,"mutation_key":"org-write","input":{"object":"company","match_field":"name","values":{"name":"Org company"}}});
        if actor == &f.member {
            f.revoke(actor).await;
        }
        assert!(
            execute(
                &f.state,
                &chat,
                "nyxid__crm_upsert_record",
                &args,
                &json!(2)
            )
            .await
            .is_err()
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_confirmation_does_not_restore_a_revoked_related_object_grant() {
    let f = fixture().await;
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"access_mode_override":"ask_every_time"}},
        )
        .await
        .unwrap();
    let record = company(&f).await;
    let mut args = json!({"mutation_key":"related-write","input":{"object":"contact","match_field":"email","values":{"name":"Contact","email":"person@test","company":record.id}}});
    let refusal = call(&f, "crm_upsert_record", args.clone(), 1)
        .await
        .unwrap();
    let id = refusal["acknowledgement_id"].as_str().unwrap();
    acks::decide(&f.state.db, &f.owner, &f.chat.conversation_id, id, true)
        .await
        .unwrap();
    args["acknowledgement_id"] = json!(id);
    set_grants(
        &f,
        vec![("contact", vec![CrmCapability::Read, CrmCapability::Write])],
    )
    .await;
    assert!(matches!(
        call(&f, "crm_upsert_record", args, 2).await,
        Err(AppError::Forbidden(_))
    ));
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::crm_record::COLLECTION_NAME)
            .count_documents(doc! {"owner_id":&f.owner,"object_key":"contact"})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_native_deleted_view_replays_after_one_approval_with_live_authority() {
    let f = fixture().await;
    let envelope = json!({"formatVersion":1,"renderer":"json-render","rendererSpecVersion":"nyxid-crm-v1","queries":{"companies":{"kind":"table","query":{"object":"company"}}},"spec":{"root":"table","elements":{"table":{"type":"ObjectTable","props":{"queryId":"companies"},"children":[]}}}});
    let view = views::create(
        &f.state.db,
        &f.owner,
        CrmActor::human(&f.owner),
        views::CreateViewInput {
            title: "Company directory".into(),
            envelope,
        },
        None,
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"access_mode_override":"ask_every_time"}},
        )
        .await
        .unwrap();
    let mut args = json!({"view_id":view.id,"mutation_key":"delete-directory"});
    let card = call(&f, "crm_delete_view", args.clone(), 1).await.unwrap();
    let ack = card["acknowledgement_id"].as_str().unwrap();
    acks::decide(&f.state.db, &f.owner, &f.chat.conversation_id, ack, true)
        .await
        .unwrap();
    args["acknowledgement_id"] = json!(ack);
    let first = call(&f, "crm_delete_view", args.clone(), 2).await.unwrap();
    let retry = call(&f, "crm_delete_view", args.clone(), 99).await.unwrap();
    assert_eq!(first, retry);
    assert_eq!(first["result"]["deleted"], true);
    assert_eq!(
        views::for_delete(&f.state.db, &f.owner, &view.id)
            .await
            .unwrap()
            .revision,
        view.revision + 1
    );
    assert!(views::get(&f.state.db, &f.owner, &view.id).await.is_err());
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"access_mode_override":"autopilot"}},
        )
        .await
        .unwrap();
    let mut changed = args.clone();
    changed["mutation_key"] = json!("new-delete");
    changed
        .as_object_mut()
        .unwrap()
        .remove("acknowledgement_id");
    assert!(matches!(
        call(&f, "crm_delete_view", changed, 100).await,
        Err(AppError::NotFound(_))
    ));
    let other = company(&f).await;
    assert!(matches!(call(&f,"crm_patch_record",json!({"record_id":other.id,"mutation_key":"delete-directory","input":{"expected_revision":1,"values":{"name":"Wrong operation"}}}),101).await,Err(AppError::CrmIdempotencyMismatch)));
    set_grants(
        &f,
        vec![("contact", vec![CrmCapability::Read, CrmCapability::Write])],
    )
    .await;
    assert!(matches!(
        call(&f, "crm_delete_view", args, 102).await,
        Err(AppError::Forbidden(_))
    ));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_flag_off_hides_native_inventory_and_team_schema_and_refuses_dispatch() {
    let f = fixture().await;
    feature_flag_service::set_platform_override(
        &f.state.db,
        super::super::FLAG_KEY,
        &FlagTarget::User(f.owner.clone()),
        false,
        &f.owner,
    )
    .await
    .unwrap();
    assert!(!available(&f.state.db, &f.chat).await.unwrap());
    let service =
        crate::services::assistant_account_tools::virtual_service_for(&f.state.db, &f.chat).await;
    for endpoint in &service.endpoints {
        assert!(!endpoint.name.starts_with("crm_"));
        assert!(
            endpoint
                .request_body_schema
                .as_ref()
                .and_then(|s| s.get("properties"))
                .and_then(|p| p.get("crm"))
                .is_none()
        );
    }
    for name in NAMES {
        assert!(call(&f, name, json!({}), 1).await.is_err(), "{name}");
    }
    let summaries =
        crate::services::assistant_team_service::summaries(&f.state.db, &f.owner, true, false, 0)
            .await
            .unwrap();
    for summary in summaries {
        assert!(serde_json::to_value(summary).unwrap().get("crm").is_none());
    }
    f.state.db.drop().await.unwrap();
}
