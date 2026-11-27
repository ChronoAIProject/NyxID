use super::*;
use crate::models::catalog_skill_revision::SkillReference;
use crate::services::{agent_skill_service::source_fixture, assistant_team_service as team};
use sha2::{Digest, Sha256};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

async fn bounded(response: Response, error: bool) -> Value {
    let raw = axum::body::to_bytes(response.into_body(), 20_000)
        .await
        .unwrap();
    let rpc: Value = serde_json::from_slice(&raw).unwrap();
    assert!(rpc["result"].to_string().len() <= 9200, "{}", raw.len());
    assert_eq!(rpc["result"]["isError"], error, "{rpc}");
    serde_json::from_str(rpc["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

async fn registry(f: &Fixture) -> (MockServer, SkillReference) {
    let server = MockServer::start().await;
    let bytes = source_fixture::archive();
    let pin = SkillReference {
        source: "ornn".into(),
        skill_id: uuid::Uuid::new_v4().to_string(),
        name: "source-recovery".into(),
        version: "1.0".into(),
        sha256: hex::encode(Sha256::digest(&bytes)),
        dependencies: vec![],
        disabled: false,
    };
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "ornn-api".into();
    catalog.base_url = server.uri();
    catalog.identity_propagation_mode = "jwt".into();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    let now = bson::DateTime::now();
    f.state
        .db
        .collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .insert_one(
            doc! {"_id":uuid::Uuid::new_v4().to_string(),"service_id":&catalog.id,
            "name":"getskilljson","method":"GET","path":"/synthetic/skill-json",
            "is_active":true,"created_at":now,"updated_at":now},
        )
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/synthetic/skill-json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(source_json()))
        .mount(&server)
        .await;
    let id = &pin.skill_id;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}")))
        .and(query_param("version", "1.0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{
            "guid":id,"name":pin.name,"version":"1.0","skillHash":pin.sha256,
            "description":"Private synthetic onboarding input","isPrivate":true,"createdBy":f.owner
        }})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/closure")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions/1.0/download")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(&server)
        .await;
    (server, pin)
}

fn source_json() -> String {
    json!({"files":source_fixture::files().into_iter().map(|(path,bytes)|
        json!({"path":path,"content":String::from_utf8(bytes).unwrap()})).collect::<Vec<_>>()})
    .to_string()
}

#[tokio::test]
async fn skill_read_recovery_private_derivative_preserves_created_ids() {
    use crate::services::{
        assistant_agent_learning_review as review, assistant_skill_authoring::tests as authored,
        feature_flag_service as flags,
    };
    let (f, service, ids) = operation_effect_fixture("source_private_derivative").await;
    let (server, _) = registry(&f).await;
    review::migrate_publication_targets(&f.state, false)
        .await
        .unwrap();
    flags::set_platform_override(
        &f.state.db,
        crate::services::assistant_agent_learning::FLAG_KEY,
        &flags::FlagTarget::User(f.owner.clone()),
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let owner = authenticate_id(&f, &f.nyxbot_thread).await;
    let created = result(
        call(
            &f,
            &owner,
            "nyxid__spawn_subagent",
            json!({"name":"desk","description":"Personal desk procedure","services":[service]}),
        )
        .await,
        false,
    )
    .await;
    let desk_id = created["subagent"]["id"].as_str().unwrap();
    let catalog = f
        .state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .find_one(doc! {"slug":"ornn-api"})
        .await
        .unwrap()
        .unwrap();
    let now = bson::DateTime::now();
    f.state
        .db
        .collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .insert_one(
            doc! {"_id":uuid::Uuid::new_v4().to_string(),"service_id":&catalog.id,
            "name":"create_workspace_fixture","method":"POST","path":"/synthetic/workspaces",
            "is_active":true,"created_at":now,"updated_at":now},
        )
        .await
        .unwrap();
    let workspace_id = "synthetic-workspace-2271";
    let original = json!({"workspace_id":workspace_id,"guide":source_json()}).to_string();
    Mock::given(method("POST"))
        .and(path("/synthetic/workspaces"))
        .respond_with(ResponseTemplate::new(200).set_body_string(original.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let first = bounded(
        call(&f, &owner, "ornn-api__create_workspace_fixture", json!({})).await,
        false,
    )
    .await;
    assert_eq!(first["complete"], false);
    let mut recovered = first["content"].as_str().unwrap().to_owned();
    let mut next = first["next_part"].as_u64();
    while let Some(part) = next {
        let page = bounded(
            call(
                &f,
                &owner,
                "nyxid__read_tool_result",
                json!({"ref":first["ref"],"part":part}),
            )
            .await,
            false,
        )
        .await;
        recovered.push_str(page["content"].as_str().unwrap());
        next = page["next_part"].as_u64();
        assert_eq!(page["complete"], next.is_none());
    }
    assert_eq!(recovered, original);
    let recovered: Value = serde_json::from_str(&recovered).unwrap();
    assert_eq!(recovered["workspace_id"], workspace_id);
    // This supported derivative is the complete requested desk procedure. It is
    // deliberately separate from copying the 49 KB input package verbatim.
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    let markdown = "# 个人工作台\n1. 读取已保存的工作台标识。\n2. 检查现有助手和允许的读取操作。\n3. 无法确认时报告未完成，不重复创建。";
    let facts = format!("Workspace: {workspace_id}\nAssistant: {desk_id}\n");
    let draft = json!({"agent":desk_id,"name":"personal-desk","description":"A complete personal desk procedure",
        "skill_md":markdown,"files":[{"path":"references/facts.txt","content":facts}],
        "scope":{"status":"complete","omissions":[]}});
    let card = result(
        call(&f, &owner, "nyxid__draft_agent_skill", draft).await,
        true,
    )
    .await;
    let proposal: crate::models::assistant_agent_learning::AssistantAgentLearningProposal = f
        .state
        .db
        .collection(crate::models::assistant_agent_learning::PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":card["proposal_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    let published_id = uuid::Uuid::new_v4().to_string();
    authored::mount_publication(&f, &server, &proposal, &published_id, "POST").await;
    let ack = card["acknowledgement_id"].as_str().unwrap().to_owned();
    crate::handlers::assistant_nyxagent::decide_acknowledgement(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((f.nyxbot_thread.clone(), ack.clone())),
        Json(serde_json::from_value(json!({"decision":"allow"})).unwrap()),
    )
    .await
    .unwrap();
    // Repeated decision exercises the publication saga's existing once-only path.
    let _ = crate::handlers::assistant_nyxagent::decide_acknowledgement(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((f.nyxbot_thread.clone(), ack)),
        Json(serde_json::from_value(json!({"decision":"allow"})).unwrap()),
    )
    .await;
    let saved = bounded(
        call(
            &f,
            &owner,
            "nyxid__get_agent_skills",
            json!({"agent":desk_id,"skill_id":published_id}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(saved["complete"], true);
    assert_eq!(saved["provenance"], "attached");
    assert_eq!(saved["pin"]["skill_id"], published_id);
    assert_eq!(
        saved["pin"]["sha256"],
        proposal.publication.as_ref().unwrap().sha256
    );
    result(call(&f,&owner,"nyxid__set_agent_operations",
        json!({"subagent":"desk","service_id":service,"selection":{"expected_revision":0,"read_only":true}})).await,false).await;
    let scopes = bounded(
        call(
            &f,
            &owner,
            "nyxid__get_agent_operations",
            json!({"subagent":"desk","service_id":service}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(scopes["complete"], true);
    assert_eq!(
        scopes["services"][0]["endpoint_ids"],
        json!([ids[0], ids[2]])
    );
    assert_eq!(scopes["services"][0]["revision"], 1);
    assert_eq!(scopes["services"][0]["all_operations"], false);
    let desk = team::agent(&f.state.db, &f.owner, desk_id).await.unwrap();
    assert_eq!(desk.id, desk_id);
    assert_eq!(desk.skills.len(), 1);
    assert_eq!(desk.skills[0].skill_id, published_id);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path() == "/synthetic/workspaces")
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path() == "/api/v1/skills" && r.method == "POST")
            .count(),
        1
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
            .count_documents(doc! {"user_id":&f.owner,"name":"desk"})
            .await
            .unwrap(),
        1
    );
    // Publication's ordinary private-owner verification rejects public/shared
    // metadata; the mock mounted by the saga fixture returns isPrivate=true,
    // createdBy=the owner, and empty user/org shares and grants.
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn skill_read_source_mcp_recovers_five_files_and_verifies_saved_setup() {
    let (f, service, ids) = operation_effect_fixture("source_mcp_flow").await;
    let (server, pin) = registry(&f).await;
    let owner = authenticate_id(&f, &f.nyxbot_thread).await;
    let first_result = bounded(
        direct_call(&f, &owner, "ornn-api__getskilljson", json!({})).await,
        false,
    )
    .await;
    assert_eq!(first_result["complete"], false);
    let mut recovered = first_result["content"].as_str().unwrap().to_owned();
    let mut next = first_result["next_part"].as_u64();
    while let Some(part) = next {
        let page = bounded(
            call(
                &f,
                &owner,
                "nyxid__read_tool_result",
                json!({"ref":first_result["ref"],"part":part}),
            )
            .await,
            false,
        )
        .await;
        recovered.push_str(page["content"].as_str().unwrap());
        next = page["next_part"].as_u64();
        assert_eq!(page["complete"], next.is_none());
    }
    assert_eq!(
        recovered.len(),
        first_result["total_bytes"].as_u64().unwrap() as usize
    );
    assert_eq!(recovered, source_json());
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.url.path() == "/synthetic/skill-json")
            .count(),
        1
    );
    let preview = result(
        call(
            &f,
            &owner,
            "nyxid__preview_agent_skill",
            json!({"skill":pin.skill_id,"version":"1.0"}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(preview["reference"], json!(pin));
    // One owner card attaches the existing private source. Recovery issues no
    // create, draft or upload request and never repeats this attachment write.
    let mut attach =
        json!({"agent":f.chat.agent_id,"selection":{"expected_revision":0,"skills":[pin]}});
    let card = result(
        call(&f, &owner, "nyxid__set_agent_skills", attach.clone()).await,
        true,
    )
    .await;
    let ack = card["acknowledgement_id"].as_str().unwrap();
    acks::decide(&f.state.db, &f.owner, &f.nyxbot_thread, ack, true)
        .await
        .unwrap();
    attach["acknowledgement_id"] = json!(ack);
    result(
        direct_call(&f, &owner, "nyxid__set_agent_skills", attach).await,
        false,
    )
    .await;
    let specialist = authenticate(&f).await;
    let first = bounded(
        direct_call(
            &f,
            &specialist,
            "nyxid__skill_read",
            json!({"skill":pin.skill_id}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(first["complete"], false);
    // An old caller receives a usable error, never a page from an unbound source.
    let restart = bounded(
        call(
            &f,
            &specialist,
            "nyxid__skill_read",
            json!({"skill":pin.skill_id,"offset":first["next_offset"]}),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(restart["details"]["reason"], "source_required");
    assert_eq!(restart["details"]["complete"], false);
    assert!(restart.get("content").is_none());
    let manifest = bounded(
        call(
            &f,
            &specialist,
            "nyxid__skill_read",
            json!({"skill":pin.skill_id,"path":"/"}),
        )
        .await,
        false,
    )
    .await;
    let fingerprint = &manifest["source_fingerprint"];
    let expected = source_fixture::expected();
    let files = source_fixture::files();
    assert_eq!(
        manifest["content"],
        files.iter().map(|(p, _)| *p).collect::<Vec<_>>().join("\n")
    );
    let mut total = 0;
    for (path, input) in files {
        let mut offset = 0;
        let mut bytes = Vec::new();
        loop {
            let args = json!({"skill":pin.skill_id,"path":path,"offset":offset,"source_fingerprint":fingerprint});
            let response = if offset == 0 {
                direct_call(&f, &specialist, "nyxid__skill_read", args).await
            } else {
                call(&f, &specialist, "nyxid__skill_read", args).await
            };
            let page = bounded(response, false).await;
            assert_eq!(page["source_fingerprint"], *fingerprint);
            assert_eq!(page["sha256"], expected[path]["sha256"]);
            assert_eq!(page["total_bytes"], expected[path]["total_bytes"]);
            bytes.extend_from_slice(page["content"].as_str().unwrap().as_bytes());
            if page["complete"] == true {
                assert!(page["next_offset"].is_null());
                break;
            }
            let next = page["next_offset"].as_u64().unwrap() as usize;
            assert!(next > offset);
            offset = next;
        }
        assert_eq!(bytes, input);
        assert_eq!(
            hex::encode(Sha256::digest(&bytes)),
            expected[path]["sha256"]
        );
        total += bytes.len();
    }
    assert_eq!(total, 49152);
    let saved_pin = bounded(
        call(
            &f,
            &owner,
            "nyxid__get_agent_skills",
            json!({"agent":f.chat.agent_id,"skill_id":pin.skill_id}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(saved_pin["pin"], json!(pin));
    assert_eq!(saved_pin["provenance"], "attached");
    assert_eq!(saved_pin["effective"], true);
    assert_eq!(saved_pin["complete"], true);
    result(call(&f, &owner, "nyxid__set_agent_operations",
        json!({"subagent":"worker","service_id":service,"selection":{"expected_revision":0,"read_only":true}})).await, false).await;
    let saved = bounded(
        direct_call(
            &f,
            &owner,
            "nyxid__get_agent_operations",
            json!({"subagent":"worker","service_id":service}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(saved["complete"], true);
    assert_eq!(saved["services"][0]["all_operations"], false);
    assert_eq!(
        saved["services"][0]["endpoint_ids"],
        json!([ids[0], ids[2]])
    );
    assert_eq!(saved["services"][0]["revision"], 1);
    let live = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert_eq!(live.skills_revision, 1);
    assert_eq!(live.skills, vec![pin.clone()]);
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.method == "GET")
    );
    // A changed source, even at a new path's offset zero, cannot reuse the manifest.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"skills.0.name":"renamed-source"}},
        )
        .await
        .unwrap();
    let changed = bounded(call(&f, &specialist, "nyxid__skill_read",
        json!({"skill":pin.skill_id,"path":"references/final.md","source_fingerprint":fingerprint})).await, true).await;
    assert_eq!(changed["details"]["reason"], "source_changed");
    assert!(changed.get("content").is_none());
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"skills.0.disabled":true}},
        )
        .await
        .unwrap();
    let disabled = bounded(
        call(
            &f,
            &specialist,
            "nyxid__skill_read",
            json!({"skill":pin.skill_id,"path":"/","source_fingerprint":fingerprint}),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(disabled["details"]["reason"], "skill_not_bound");
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"skills.0.disabled":false}},
        )
        .await
        .unwrap();
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/skills/{}/versions/1.0/download",
            pin.skill_id
        )))
        .respond_with(ResponseTemplate::new(403).set_body_string("private upstream detail"))
        .mount(&server)
        .await;
    let denied = bounded(
        call(
            &f,
            &specialist,
            "nyxid__skill_read",
            json!({"skill":pin.skill_id,"path":"/","source_fingerprint":fingerprint}),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(denied["details"]["reason"], "ornn_principal_denied");
    assert!(!denied.to_string().contains("private upstream detail"));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn skill_read_recovery_targeted_pins_are_live_bounded_and_retry_reads_only() {
    let (f, service, _) = operation_effect_fixture("source_targeted_readback").await;
    let (_, pin) = registry(&f).await;
    let owner = authenticate_id(&f, &f.nyxbot_thread).await;
    let mut pins = vec![pin.clone()];
    for n in 0..15 {
        let mut unrelated = pin.clone();
        unrelated.skill_id = uuid::Uuid::new_v4().to_string();
        unrelated.name = format!("unrelated-{n}");
        unrelated.dependencies = (0..16)
            .map(|d| crate::models::catalog_skill_revision::SkillPin {
                source: "ornn".into(),
                skill_id: uuid::Uuid::new_v4().to_string(),
                name: format!("dep-{d}"),
                version: "1.0".into(),
                sha256: pin.sha256.clone(),
            })
            .collect();
        pins.push(unrelated);
    }
    let agents = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME);
    agents
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"skills":bson::to_bson(&pins).unwrap(),"skills_revision":1_i64}},
        )
        .await
        .unwrap();
    let inventory = result(
        call(
            &f,
            &owner,
            "nyxid__get_agent_skills",
            json!({"agent":f.chat.agent_id}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(inventory["skills"].as_array().unwrap().len(), 16);
    assert!(crate::services::agent_skill_service::wire_len(&inventory) > 24_000);
    let args = json!({"agent":f.chat.agent_id,"skill_id":pin.skill_id});
    for wrapped in [false, true] {
        let params = if wrapped {
            json!({"name":"nyx__call_tool","arguments":{"tool_name":"nyxid__get_agent_skills","arguments_json":args.to_string()}})
        } else {
            json!({"name":"nyxid__get_agent_skills","arguments":args})
        };
        // Exact completed ancestry still permits a live native state read.
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.nyxbot_thread},
                doc! {"$set":{"active_turn.retry_digests":[chat_activity_digest(&params)]}},
            )
            .await
            .unwrap();
        let page = if wrapped {
            bounded(
                call(&f, &owner, "nyxid__get_agent_skills", args.clone()).await,
                false,
            )
            .await
        } else {
            bounded(
                direct_call(&f, &owner, "nyxid__get_agent_skills", args.clone()).await,
                false,
            )
            .await
        };
        assert_eq!(page["pin"], json!(pin));
        assert_eq!(page["complete"], true);
    }
    agents
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"skills.0.disabled":true}},
        )
        .await
        .unwrap();
    let disabled = bounded(
        call(&f, &owner, "nyxid__get_agent_skills", args.clone()).await,
        false,
    )
    .await;
    assert_eq!(disabled["pin"]["disabled"], true);
    assert_eq!(disabled["effective"], false);
    let absent = bounded(
        call(
            &f,
            &owner,
            "nyxid__get_agent_skills",
            json!({"agent":f.chat.agent_id,"skill_id":uuid::Uuid::new_v4().to_string()}),
        )
        .await,
        false,
    )
    .await;
    assert!(absent["pin"].is_null());
    assert_eq!(absent["complete"], true);
    // Only the two exact native state readers are exempt. Paid/approved bound
    // reads, refs, setup writes, downstream names and unknown outcomes stay fenced.
    for name in [
        "nyxid__skill_read",
        "nyxid__read_tool_result",
        "nyxid__set_agent_skills",
        "nyxid__set_agent_operations",
        "ornn-api__get_agent_skills",
    ] {
        assert!(!setup_recovery_read(&json!({"name":name,"arguments":{}})));
        assert!(!setup_recovery_read(
            &json!({"name":"nyx__call_tool","arguments":{"tool_name":name}})
        ));
    }
    let op_args = json!({"subagent":"worker","service_id":service});
    let params = json!({"name":"nyxid__get_agent_operations","arguments":op_args});
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.nyxbot_thread},
            doc! {"$set":{"active_turn.retry_digests":[chat_activity_digest(&params)]}},
        )
        .await
        .unwrap();
    bounded(
        direct_call(&f, &owner, "nyxid__get_agent_operations", op_args).await,
        false,
    )
    .await;
    let specialist = authenticate(&f).await;
    let refused = result(
        call(
            &f,
            &specialist,
            "nyxid__get_agent_skills",
            json!({"agent":"nyxbot","skill_id":pin.skill_id}),
        )
        .await,
        true,
    )
    .await;
    assert!(refused.get("pin").is_none());
    // Readback has no snapshot that can outlive the live target authority.
    agents
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"user_id":uuid::Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    let revoked = result(
        call(&f, &owner, "nyxid__get_agent_skills", args).await,
        true,
    )
    .await;
    assert!(revoked.get("pin").is_none());
    f.state.db.drop().await.unwrap();
}
