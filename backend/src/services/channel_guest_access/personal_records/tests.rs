use super::*;
use crate::{
    models::channel_guest_policy::GuestDataPolicy,
    services::{
        channel_guest_access::{AnswerAudience, GuestActor, GuestAssurance, GuestTurn, policy},
        transaction_service,
    },
};

fn test_policy(bot: &str) -> GuestDataPolicy {
    let policy = serde_json::from_value::<GuestDataPolicy>(json!({
        "_id":bot,
        "bot_id":bot,
        "revision":1,
        "rules":[{
            "source_id":SOURCE_ID,
            "collection_id":SOURCE_ID,
            "scope":{"kind":"own_records"},
            "audience":{"kind":"all_guests"},
            "delivery":"private",
            "operations":{
                "create":{"write":["title","body"]},
                "read":{"read":["title"]},
                "update":{"write":["title"]},
                "delete":{}
            }
        }]
    }))
    .unwrap();
    policy::validate_rules(&policy.rules).unwrap();
    policy
}

fn permit(
    request: &PersonalRecordRequest,
    bot: &str,
    actor: &str,
    audience: AnswerAudience,
) -> AppResult<GuestDataPermit> {
    let turn = GuestTurn {
        bot_id: bot.into(),
        binding_id: "binding".into(),
        platform: "discord".into(),
        conversation_id: "room".into(),
        source_event_id: "event".into(),
        actor: GuestActor {
            id: actor.into(),
            assurance: GuestAssurance::ProviderVerified,
        },
        audience,
        policy_revision: 1,
        activation_epoch: 1,
        bot_revision: 1,
        meta_grant_id: None,
    };
    policy::authorize(
        &test_policy(bot),
        &turn,
        &policy::GuestDataRequest {
            source_id: SOURCE_ID,
            collection_id: SOURCE_ID,
            action: request.action(),
            record_id: request.record_id(),
            record_owner_id: None,
            fields: &request.fields().collect::<Vec<_>>(),
        },
    )
}

async fn run(
    db: &Database,
    bot: &str,
    actor: &str,
    tool: &'static str,
    arguments: Value,
) -> AppResult<Value> {
    let request = parse(tool, &arguments)?;
    let permit = permit(&request, bot, actor, AnswerAudience::Private)?;
    run_with_permit(db, permit, tool, arguments).await
}

async fn run_with_permit(
    db: &Database,
    permit: GuestDataPermit,
    tool: &'static str,
    arguments: Value,
) -> AppResult<Value> {
    transaction_service::run_db_only(db, &(db.clone(), permit, arguments), |session, ctx| {
        Box::pin(async move {
            let (db, permit, arguments) = ctx;
            execute(db, session, permit, None, parse(tool, arguments)?).await
        })
    })
    .await
}

async fn database(prefix: &str) -> Database {
    let db = crate::test_utils::connect_transaction_test_database(prefix).await;
    ensure_indexes(&db).await.unwrap();
    db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_many(["bot-1", "bot-2"].map(|bot| doc! {"_id":bot,"is_active":true}))
        .await
        .unwrap();
    db
}

#[test]
fn parser_rejects_untrusted_selectors_and_invalid_fields() {
    assert!(parse(CREATE, &json!({"title":"x","owner_id":"guest-b"})).is_err());
    assert!(parse(CREATE, &json!({"title":"x","bot_id":"bot-2"})).is_err());
    assert!(parse(CREATE, &json!({"title":null})).is_err());
    assert!(parse(CREATE, &json!({})).is_err());
    assert!(parse(CREATE, &json!(["title", "body"])).is_err());
    assert!(parse(CREATE, &json!({"body":"a".repeat(BODY_LIMIT + 1)})).is_err());
    assert!(parse(LIST, &json!({"title_prefix":""})).is_err());
    assert!(parse(LIST, &json!({"cursor":{"$gt":"record"}})).is_err());
    assert!(
        parse(
            READ,
            &json!({"record_id":"00000000-0000-0000-0000-000000000000"})
        )
        .is_err()
    );
    assert_eq!(tool_definitions().len(), 5);
    assert!(tool_definitions().iter().all(|tool| is_tool(&tool.name)));
}

#[tokio::test]
async fn mongodb_crud_search_and_pagination_use_guest_and_bot_predicates() {
    let db = database("guest_records_crud").await;
    let created = run(
        &db,
        "bot-1",
        "guest-a",
        CREATE,
        json!({"title":"[work] alpha","body":"private"}),
    )
    .await
    .unwrap();
    let id = created["record_id"].as_str().unwrap();
    assert!(owns_record(&db, "bot-1", "guest-a", id).await.unwrap());
    assert!(!owns_record(&db, "bot-1", "guest-b", id).await.unwrap());
    let got = run(&db, "bot-1", "guest-a", READ, json!({"record_id":id}))
        .await
        .unwrap();
    assert_eq!(got, json!({"record_id":id,"title":"[work] alpha"}));
    for (bot, guest) in [("bot-1", "guest-b"), ("bot-2", "guest-a")] {
        assert!(matches!(
            run(&db, bot, guest, READ, json!({"record_id":id})).await,
            Err(AppError::NotFound(_))
        ));
        let search = run(&db, bot, guest, LIST, json!({"title_prefix":"[work]"}))
            .await
            .unwrap();
        assert_eq!(search, json!({"records":[],"next_cursor":null}));
        assert!(matches!(
            run(
                &db,
                bot,
                guest,
                UPDATE,
                json!({"record_id":id,"title":"changed"})
            )
            .await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            run(&db, bot, guest, DELETE, json!({"record_id":id})).await,
            Err(AppError::NotFound(_))
        ));
    }
    assert!(
        permit(
            &parse(READ, &json!({"record_id":id})).unwrap(),
            "bot-1",
            "guest-a",
            AnswerAudience::Shared
        )
        .is_err()
    );
    assert!(matches!(
        run(
            &db,
            "bot-1",
            "guest-a",
            UPDATE,
            json!({"record_id":id,"body":"changed"})
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    run(
        &db,
        "bot-1",
        "guest-a",
        UPDATE,
        json!({"record_id":id,"title":"[work] updated"}),
    )
    .await
    .unwrap();
    for item in 0..PAGE_SIZE {
        run(
            &db,
            "bot-1",
            "guest-a",
            CREATE,
            json!({"title":format!("[work] {item}")}),
        )
        .await
        .unwrap();
    }
    run(
        &db,
        "bot-1",
        "guest-a",
        CREATE,
        json!({"title":"work other"}),
    )
    .await
    .unwrap();
    let first = run(
        &db,
        "bot-1",
        "guest-a",
        LIST,
        json!({"title_prefix":"[work]"}),
    )
    .await
    .unwrap();
    assert_eq!(first["records"].as_array().unwrap().len(), PAGE_SIZE);
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = run(
        &db,
        "bot-1",
        "guest-a",
        LIST,
        json!({"title_prefix":"[work]","cursor":cursor}),
    )
    .await
    .unwrap();
    let last = &second["records"].as_array().unwrap()[0];
    assert!(
        first["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["record_id"] != last["record_id"])
    );
    assert_eq!(second["records"].as_array().unwrap().len(), 1);
    assert_eq!(second["next_cursor"], Value::Null);
    assert_eq!(
        run(&db, "bot-1", "guest-a", DELETE, json!({"record_id":id}))
            .await
            .unwrap(),
        json!({"record_id":id,"deleted":true})
    );
    assert!(matches!(
        run(&db, "bot-1", "guest-a", READ, json!({"record_id":id})).await,
        Err(AppError::NotFound(_))
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn mongodb_write_uses_current_owner_even_with_an_earlier_permit() {
    let db = database("guest_records_stale_owner").await;
    let created = run(&db, "bot-1", "guest-a", CREATE, json!({"title":"original"}))
        .await
        .unwrap();
    let id = created["record_id"].as_str().unwrap();
    let update = json!({"record_id":id,"title":"stale"});
    let delete = json!({"record_id":id});
    let stale_update = permit(
        &parse(UPDATE, &update).unwrap(),
        "bot-1",
        "guest-a",
        AnswerAudience::Private,
    )
    .unwrap();
    let stale_delete = permit(
        &parse(DELETE, &delete).unwrap(),
        "bot-1",
        "guest-a",
        AnswerAudience::Private,
    )
    .unwrap();
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(doc! {"_id":id}, doc! {"$set":{"owner_id":"guest-b"}})
        .await
        .unwrap();
    assert!(matches!(
        run_with_permit(&db, stale_update, UPDATE, update).await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        run_with_permit(&db, stale_delete, DELETE, delete).await,
        Err(AppError::NotFound(_))
    ));
    assert_eq!(
        run(&db, "bot-1", "guest-b", READ, json!({"record_id":id}))
            .await
            .unwrap(),
        json!({"record_id":id,"title":"original"})
    );
    let other = run(&db, "bot-2", "guest-b", CREATE, json!({"title":"keep"}))
        .await
        .unwrap();
    let bots = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME);
    bots.update_one(
        doc! {"_id":"bot-1"},
        doc! {"$inc":{"ownership_version":1_i64}},
    )
    .await
    .unwrap();
    assert!(!owns_record(&db, "bot-1", "guest-b", id).await.unwrap());
    assert_eq!(
        run(&db, "bot-1", "guest-b", LIST, json!({})).await.unwrap(),
        json!({"records":[],"next_cursor":null})
    );
    assert!(matches!(
        run(&db, "bot-1", "guest-b", DELETE, json!({"record_id":id})).await,
        Err(AppError::NotFound(_))
    ));
    let fresh = run(
        &db,
        "bot-1",
        "guest-b",
        CREATE,
        json!({"title":"new dataset"}),
    )
    .await
    .unwrap();
    let fresh_id = fresh["record_id"].as_str().unwrap();
    assert!(
        owns_record(&db, "bot-1", "guest-b", fresh_id)
            .await
            .unwrap()
    );
    bots.update_one(doc! {"_id":"bot-1"}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    purge_bot(&db, "bot-1").await.unwrap();
    assert!(matches!(
        run(&db, "bot-1", "guest-b", CREATE, json!({"title":"late"})).await,
        Err(AppError::Forbidden(_))
    ));
    assert_eq!(
        db.collection::<Document>(COLLECTION_NAME)
            .count_documents(doc! {"bot_id":"bot-1"})
            .await
            .unwrap(),
        0
    );
    assert!(
        owns_record(
            &db,
            "bot-2",
            "guest-b",
            other["record_id"].as_str().unwrap()
        )
        .await
        .unwrap()
    );
    db.drop().await.unwrap();
}

async fn start_guest(
    f: &crate::services::assistant_authority_tests::Fixture,
    sender: &str,
    event: &str,
) -> crate::services::assistant_acknowledgement_service::ChatAuthority {
    use crate::models::{
        assistant_conversation::{
            COLLECTION_NAME as CONVERSATIONS, ChannelOrigin, ChannelTurnContext, TurnOrigin,
        },
        channel_thread::ThreadSenderKind,
    };
    use crate::services::{
        assistant_authority_tests::chat_for_row,
        assistant_nyxagent::{self as engine, TurnRequest, TurnStart},
        channel_guest_access::{self as guest, GuestAdmission, VerifiedGuestInput},
    };
    let db = &f.state.db;
    let mode = guest::select_mode(db, "bot-1").await.unwrap();
    let GuestAdmission::AddOn(turn) = guest::classify(
        db,
        mode,
        VerifiedGuestInput {
            bot_id: "bot-1",
            binding_id: "binding",
            platform: "discord",
            conversation_id: sender,
            source_event_id: event,
            sender_id: Some(sender),
            audience: AnswerAudience::Private,
            meta_grant_id: None,
        },
    )
    .await
    .unwrap() else {
        panic!("expected the guest add-on admission");
    };
    let id = turn.hosted_conversation_id();
    let existing = db
        .collection::<Document>(CONVERSATIONS)
        .find_one(doc! {"_id":&id})
        .projection(doc! {"_id":1})
        .await
        .unwrap()
        .is_some();
    let mut start = TurnStart::from(&TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        conversation_id: existing.then(|| id.clone()),
        agent_id: f.row.agent_id.clone(),
        text: "Show my personal records".into(),
        model: None,
        access_mode: None,
    });
    start.guest = true;
    start.origin = TurnOrigin::Channel;
    start.channel_event_id = Some(event.into());
    start.new_id = Some(id);
    start.channel = Some(ChannelOrigin {
        org_execution: None,
        native_event_id: None,
        thread: None,
        nyxbot_channel_id: "binding".into(),
        partition: sender.into(),
        platform: "discord".into(),
    });
    start.channel_context = Some(ChannelTurnContext {
        channel_guest: Some(turn),
        meta_grant: None,
        text_completeness: None,
        org_origin: None,
        display_name: None,
        conversation_label: None,
        followed: false,
        source_event_id: event.into(),
        channel_id: "binding".into(),
        channel_bot_id: "bot-1".into(),
        conversation_kind: "private".into(),
        sender_kind: ThreadSenderKind::Human,
        guest: true,
        original_message: true,
    });
    db.collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
        .insert_one(doc! {
            "_id":event, "user_id":&f.owner, "status":"running",
            "delivery":{"version":1},
        })
        .await
        .unwrap();
    let row = Box::pin(engine::begin_turn(
        db,
        &f.owner,
        start,
        &f.state.encryption_keys,
    ))
    .await
    .unwrap();
    chat_for_row(&f.state, &f.owner, &row).await
}

async fn mcp_response(
    router: &axum::Router,
    key: &str,
    method: &str,
    params: Value,
) -> (axum::http::StatusCode, Value) {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("x-api-key", key)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    if !status.is_success() {
        return (status, Value::Null);
    }
    let bytes = axum::body::to_bytes(response.into_body(), 256 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn mcp_request(router: &axum::Router, key: &str, method: &str, params: Value) -> Value {
    let (status, body) = mcp_response(router, key, method, params).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert!(body.get("error").is_none(), "{body}");
    body["result"].clone()
}

#[tokio::test]
async fn live_guest_tool_discovery_and_execution_obey_policy_and_approval() {
    use crate::{
        models::{channel_bot, channel_guest_policy, feature_flag_override, notification_channel},
        services::{
            assistant_agent_credential_service as credentials,
            assistant_authority_tests::direct_fixture,
            channel_guest_access::{self as guest, guest_tools},
        },
    };

    let f = Box::pin(direct_fixture("guest_records_hosted")).await;
    let db = &f.state.db;
    guest::ensure_indexes(db).await.unwrap();
    db.collection::<Document>(channel_bot::COLLECTION_NAME)
        .insert_one(doc! {"_id":"bot-1","is_active":true,"guest_access_v1":true,"guest_access_v1_revision":1_i64})
        .await
        .unwrap();
    crate::test_utils::init_test_audit_chain_hmac_key();
    guest::set_pilot(
        db,
        guest::PilotSelection {
            bot_ids: None,
            fleet: true,
        },
        &crate::services::audit_service::AuditActor::from_auth_user(&f.auth),
    )
    .await
    .unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    guest::update_activation_in_session(db, &mut session, true)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    db.collection::<Document>(feature_flag_override::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(), "flag_key":guest::FLAG,
            "target_kind":"global","enabled":true,"created_at":bson::DateTime::now(),
            "updated_at":bson::DateTime::now(),"updated_by":&f.owner,
        })
        .await
        .unwrap();
    let no_policy = Box::pin(start_guest(&f, "guest-a", "event-no-policy")).await;
    assert!(
        guest_tools::definitions(db, &no_policy, &f.owner)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        guest_tools::execute(db, &no_policy, &f.owner, CREATE, &json!({"title":"denied"}))
            .await
            .is_err()
    );

    let policies = db.collection::<GuestDataPolicy>(channel_guest_policy::COLLECTION_NAME);
    policies.insert_one(test_policy("bot-1")).await.unwrap();
    let a = Box::pin(start_guest(&f, "guest-a", "event-a")).await;
    let b = Box::pin(start_guest(&f, "guest-b", "event-b")).await;

    let (_, private) = crate::routes::build_router_with_state(f.state.clone());
    let router = private.with_state(f.state.clone());
    let credential = credentials::load_for_conversation(
        db,
        &f.state.encryption_keys,
        &f.owner,
        &a.conversation_id,
    )
    .await
    .unwrap()
    .unwrap();
    let mcp_tools = mcp_request(
        &router,
        credential.raw_key.as_str(),
        "tools/list",
        json!({}),
    )
    .await;
    assert!(
        mcp_tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == CREATE)
    );
    for args in [
        json!({"name":"nyxid__list_agent_keys","arguments":{}}),
        json!({"name":"nyx__call_tool","arguments":{
            "tool_name":"nyxid__list_agent_keys","arguments_json":"{}"
        }}),
    ] {
        let refused = mcp_request(&router, credential.raw_key.as_str(), "tools/call", args).await;
        assert_eq!(refused["isError"], true, "{refused}");
        let refusal: Value =
            serde_json::from_str(refused["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(refusal["error"], "owner_only");
    }
    let tools = guest_tools::definitions(db, &a, &f.owner).await.unwrap();
    assert_eq!(tools.len(), 5);
    let update = tools.iter().find(|tool| tool.name == UPDATE).unwrap();
    assert!(update.input_schema["properties"].get("body").is_none());
    let result = mcp_request(
        &router,
        credential.raw_key.as_str(),
        "tools/call",
        json!({"name":CREATE,"arguments":{"title":"My note","body":"private"}}),
    )
    .await;
    assert_eq!(result["isError"], false, "{result}");
    let created: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    let id = created["record_id"].as_str().unwrap();
    let arguments = json!({"record_id":id});
    assert_eq!(
        guest_tools::execute(db, &a, &f.owner, READ, &arguments)
            .await
            .unwrap(),
        json!({"record_id":id,"title":"My note"})
    );
    assert!(matches!(
        guest_tools::execute(db, &b, &f.owner, READ, &arguments).await,
        Err(AppError::NotFound(_))
    ));
    let edited = json!({"record_id":id,"title":"Revised note"});
    guest_tools::execute(db, &a, &f.owner, UPDATE, &edited)
        .await
        .unwrap();
    assert_eq!(
        guest_tools::execute(db, &a, &f.owner, LIST, &json!({"title_prefix":"Revised"}))
            .await
            .unwrap(),
        json!({"records":[{"record_id":id,"title":"Revised note"}],"next_cursor":null})
    );
    assert_eq!(
        guest_tools::execute(db, &b, &f.owner, LIST, &json!({}))
            .await
            .unwrap(),
        json!({"records":[],"next_cursor":null})
    );
    db.collection::<Document>(notification_channel::COLLECTION_NAME)
        .update_one(
            doc! {"user_id":&f.owner},
            doc! {"$set":{"approval_required":true}},
        )
        .await
        .unwrap();
    assert!(
        guest_tools::definitions(db, &a, &f.owner)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        guest_tools::execute(db, &a, &f.owner, DELETE, &arguments).await,
        Err(AppError::Forbidden(_))
    ));
    db.collection::<Document>(notification_channel::COLLECTION_NAME)
        .update_one(
            doc! {"user_id":&f.owner},
            doc! {"$set":{"approval_required":false}},
        )
        .await
        .unwrap();
    assert_eq!(
        guest_tools::execute(db, &a, &f.owner, DELETE, &arguments)
            .await
            .unwrap(),
        json!({"record_id":id,"deleted":true})
    );

    db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&a.conversation_id},
            doc! {"$set":{"active_turn":null}},
        )
        .await
        .unwrap();
    let next = Box::pin(start_guest(&f, "guest-a", "event-a-next")).await;
    assert_eq!(a.conversation_id, next.conversation_id);
    assert_ne!(a.api_key_id, next.api_key_id);
    let next_key = credentials::load_for_conversation(
        db,
        &f.state.encryption_keys,
        &f.owner,
        &next.conversation_id,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(credential.raw_key.as_str() != next_key.raw_key.as_str());
    let key_collection = db.collection::<Document>(crate::models::api_key::COLLECTION_NAME);
    assert!(
        key_collection
            .find_one(doc! {"_id":&a.api_key_id,"is_active":true})
            .await
            .unwrap()
            .is_none()
    );
    let (status, reply) = mcp_response(
        &router,
        credential.raw_key.as_str(),
        "tools/call",
        json!({"name":CREATE,"arguments":{"title":"Stale source"}}),
    )
    .await;
    assert!(
        matches!(
            status,
            axum::http::StatusCode::UNAUTHORIZED | axum::http::StatusCode::FORBIDDEN
        ) || reply.get("error").is_some_and(|error| !error.is_null())
            || reply["result"]["isError"] == true,
        "{status} {reply}"
    );
    assert_eq!(
        db.collection::<Document>(COLLECTION_NAME)
            .count_documents(doc! {"bot_id":"bot-1"})
            .await
            .unwrap(),
        0
    );
    assert!(
        mcp_request(&router, next_key.raw_key.as_str(), "tools/list", json!({})).await["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == CREATE)
    );
    policies
        .update_one(doc! {"bot_id":"bot-1"}, doc! {"$inc":{"revision":1_i64}})
        .await
        .unwrap();
    assert!(matches!(
        guest_tools::execute(db, &next, &f.owner, READ, &arguments).await,
        Err(AppError::Forbidden(_))
    ));
    db.drop().await.unwrap();
}
