use super::{
    assistant_account_tools as tools, assistant_acknowledgement_service as acks,
    assistant_nyxagent as engine, key_service,
};
use crate::{
    models::{
        assistant_acknowledgement::{AssistantAcknowledgement, COLLECTION_NAME as ACKS},
        assistant_conversation::AssistantConversation,
        user::UserType,
    },
    mw::auth::{ASSISTANT_ACCOUNT_SCOPE, AuthMethod, AuthUser},
    test_utils::{
        connect_transaction_test_database, test_app_state, test_auth_user, test_user,
        test_user_endpoint, test_user_service,
    },
};
use chrono::{Duration, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) struct Fixture {
    pub state: crate::AppState,
    pub owner: String,
    pub row: AssistantConversation,
    pub chat: acks::ChatAuthority,
    pub auth: AuthUser,
}
impl Fixture {
    pub fn tools(&self) -> tools::AccountTools<'_> {
        tools::AccountTools {
            db: &self.state.db,
            keys: &self.state.encryption_keys,
            http: &self.state.http_client,
            config: &self.state.config,
            token_exchange_cache: &self.state.token_exchange_cache,
            node_manager: &self.state.node_ws_manager,
        }
    }
    pub async fn allow_account(&self) {
        let refusal = acks::account_gate(&self.state.db, &self.chat)
            .await
            .unwrap()
            .unwrap();
        acks::decide(
            &self.state.db,
            &self.owner,
            &self.row.id,
            refusal["acknowledgement_id"].as_str().unwrap(),
            true,
        )
        .await
        .unwrap();
    }
}

pub(crate) async fn fixture(name: &str) -> Fixture {
    let db = connect_transaction_test_database(name).await;
    engine::ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db);
    let row = engine::begin_turn(
        &state.db,
        &owner,
        &engine::TurnRequest {
            conversation_id: None,
            text: "Please manage my account".into(),
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let chat = acks::for_key(&state.db, &owner, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let mut auth = test_auth_user(&owner);
    auth.auth_method = AuthMethod::ApiKey;
    auth.scope = "proxy".into();
    auth.api_key_id = Some(row.credential_api_key_id.clone());
    auth.allow_all_services = false;
    auth.allow_all_nodes = true;
    Fixture {
        state,
        owner,
        row,
        chat,
        auth,
    }
}

pub(crate) async fn connected(db: &Database, owner: &str, slug: &str, url: &str) -> String {
    let endpoint = Uuid::new_v4().to_string();
    let service = Uuid::new_v4().to_string();
    db.collection(crate::models::user_endpoint::COLLECTION_NAME)
        .insert_one(test_user_endpoint(
            &endpoint, owner, "GitHub", url, None, None,
        ))
        .await
        .unwrap();
    db.collection(crate::models::user_service::COLLECTION_NAME)
        .insert_one(test_user_service(
            &service, owner, slug, &endpoint, None, None,
        ))
        .await
        .unwrap();
    service
}

#[test]
fn digest_is_canonical_and_excludes_only_the_top_level_acknowledgement_id() {
    let a: Value = serde_json::from_str(r#"{"z":[{"b":2,"a":1}],"a":true}"#).unwrap();
    let b = json!({"acknowledgement_id": "ignored", "a": true, "z": [{"a": 1, "b": 2}]});
    assert_eq!(acks::arguments_digest(&a), acks::arguments_digest(&b));
    assert_ne!(
        acks::arguments_digest(&a),
        acks::arguments_digest(&json!({"a": false}))
    );
    assert_ne!(
        acks::arguments_digest(&json!({"nested": {"acknowledgement_id": "a"}})),
        acks::arguments_digest(&json!({"nested": {"acknowledgement_id": "b"}}))
    );
}

#[tokio::test]
async fn acknowledgement_service_allow_is_atomic_scoped_and_versions_the_key() {
    let f = fixture("ack_service_allow").await;
    let db = &f.state.db;
    let service = connected(db, &f.owner, "github", "https://api.github.com").await;
    let (a, b) = tokio::join!(
        acks::service_gate(db, &f.chat, &service, "github", "GitHub", false),
        acks::service_gate(db, &f.chat, &service, "github", "GitHub", false),
    );
    let a = a.unwrap().unwrap();
    assert_eq!(a, b.unwrap().unwrap());
    assert_eq!(a["error"], "acknowledgement_required");
    assert_eq!(a["kind"], "service");
    assert_eq!(
        a["instructions"],
        "Ask the user to approve access to GitHub for this chat (a card is shown in the chat), then retry."
    );
    let id = a["acknowledgement_id"].as_str().unwrap();
    for (user, conversation) in [
        ("other", f.row.id.as_str()),
        (&f.owner, "nyxa-00000000000000000000000000000000"),
    ] {
        assert!(matches!(
            acks::decide(db, user, conversation, id, true).await,
            Err(crate::errors::AppError::NotFound(_))
        ));
    }
    let before = key_service::get_api_key(db, &f.owner, &f.chat.api_key_id)
        .await
        .unwrap();
    let (allow, deny) = tokio::join!(
        acks::decide(db, &f.owner, &f.row.id, id, true),
        acks::decide(db, &f.owner, &f.row.id, id, true),
    );
    assert_eq!(usize::from(allow.is_ok()) + usize::from(deny.is_ok()), 1);
    let key = key_service::get_api_key(db, &f.owner, &f.chat.api_key_id)
        .await
        .unwrap();
    assert_eq!(key.allowed_service_ids, vec![service.clone()]);
    assert!(key.state_version > before.state_version);
    assert!(
        acks::service_gate(db, &f.chat, &service, "github", "GitHub", false)
            .await
            .unwrap()
            .is_none()
    );
    let rows = acks::history(db, &f.owner, &f.row.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "allowed");
    let doc = bson::to_document(&rows[0]).unwrap();
    assert!(matches!(
        doc.get("decided_at"),
        Some(bson::Bson::DateTime(_))
    ));
    assert!(!format!("{:?}", rows[0]).contains(&f.chat.api_key_id));
}

#[tokio::test]
async fn service_decisions_reject_platform_missing_disabled_and_other_owner_ids() {
    let f = fixture("ack_visible_user_service_only").await;
    let db = &f.state.db;
    let platform = crate::test_utils::test_auto_connected_catalog_service();
    db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    )
    .insert_one(&platform)
    .await
    .unwrap();
    let other = connected(db, "other", "other", "https://example.com").await;
    let disabled = connected(db, &f.owner, "disabled", "https://example.com").await;
    db.collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(doc! {"_id": &disabled}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    let before = key_service::get_api_key(db, &f.owner, &f.chat.api_key_id)
        .await
        .unwrap();
    for id in [platform.id, Uuid::new_v4().to_string(), other, disabled] {
        // Simulate a legacy acknowledgement that bypassed the MCP source gate.
        let row = acks::request(
            db,
            &f.chat,
            acks::Request {
                kind: "service",
                service: Some((&id, "example", "Example")),
                tool: None,
                arguments: None,
                summary: "Allow Example?",
                platform: false,
            },
        )
        .await
        .unwrap();
        for allow in [true, false] {
            assert!(matches!(
                acks::decide(db, &f.owner, &f.row.id, &row.id, allow).await,
                Err(crate::errors::AppError::NotFound(_))
            ));
        }
    }
    let key = key_service::get_api_key(db, &f.owner, &f.chat.api_key_id)
        .await
        .unwrap();
    assert_eq!(key.allowed_service_ids, before.allowed_service_ids);
    assert_eq!(key.state_version, before.state_version);
    assert!(
        acks::history(db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .iter()
            .all(|row| row.status == "pending")
    );
}

#[tokio::test]
async fn acknowledgements_deny_expire_and_reask_only_after_a_new_user_message() {
    let f = fixture("ack_deny_expire").await;
    let db = &f.state.db;
    let service = connected(db, &f.owner, "github", "https://api.github.com").await;
    let request = acks::service_gate(db, &f.chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    let id = request["acknowledgement_id"].as_str().unwrap();
    acks::decide(db, &f.owner, &f.row.id, id, false)
        .await
        .unwrap();
    // Even past its former pending deadline, denial is sticky for this user turn.
    db.collection::<AssistantAcknowledgement>(ACKS)
        .update_one(
            doc! {"_id": id},
            doc! {"$set": {"expires_at": bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let denial = acks::service_gate(db, &f.chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(denial["error"], "acknowledgement_denied");
    assert_eq!(denial["acknowledgement_id"], id);
    engine::finish_turn(
        db,
        &f.row,
        &f.chat.api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "Access denied".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        acks::service_gate(db, &f.chat, &service, "github", "GitHub", false)
            .await
            .unwrap()
            .unwrap()["error"],
        "acknowledgement_denied"
    );
    engine::begin_turn(
        db,
        &f.owner,
        &engine::TurnRequest {
            conversation_id: Some(f.row.id.clone()),
            text: "Ask for access again".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let again = acks::service_gate(db, &f.chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    let next = again["acknowledgement_id"].as_str().unwrap();
    assert_ne!(next, id);
    db.collection::<AssistantAcknowledgement>(ACKS)
        .update_one(
            doc! {"_id": next},
            doc! {"$set": {"expires_at": bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    assert!(matches!(
        acks::decide(db, &f.owner, &f.row.id, next, true).await,
        Err(crate::errors::AppError::Conflict(_))
    ));
    let history = acks::history(db, &f.owner, &f.row.id).await.unwrap();
    assert_eq!(
        history.iter().find(|a| a.id == next).unwrap().status,
        "expired"
    );
}

#[tokio::test]
async fn action_acknowledgements_bind_arguments_key_conversation_and_are_single_use() {
    let f = fixture("ack_actions").await;
    let db = &f.state.db;
    let args = json!({"api_key_id": Uuid::new_v4().to_string()});
    let tool = "nyxid__delete_agent_key";
    let row = acks::request(
        db,
        &f.chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some(tool),
            arguments: Some(&args),
            summary: "Delete agent key 'ci-bot'",
            platform: false,
        },
    )
    .await
    .unwrap();
    assert!(
        !acks::consume_action(db, &f.chat, &row.id, tool, &args)
            .await
            .unwrap()
    );
    let allowed = acks::decide(db, &f.owner, &f.row.id, &row.id, true)
        .await
        .unwrap();
    assert_eq!(
        (allowed.expires_at - allowed.decided_at.unwrap()).num_seconds(),
        acks::ACTION_SECONDS
    );
    assert!(
        !acks::consume_action(db, &f.chat, &row.id, "nyxid__delete_node", &args)
            .await
            .unwrap()
    );
    assert!(
        !acks::consume_action(
            db,
            &f.chat,
            &row.id,
            tool,
            &json!({"api_key_id": "different"})
        )
        .await
        .unwrap()
    );
    let other = fixture("ack_other").await;
    assert!(
        acks::consume_action(db, &other.chat, &row.id, tool, &args)
            .await
            .is_err()
    );
    let second = engine::begin_turn(
        db,
        &f.owner,
        &engine::TurnRequest {
            conversation_id: None,
            text: "other chat".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let second = acks::for_key(db, &f.owner, Some(&second.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert!(
        !acks::consume_action(db, &second, &row.id, tool, &args)
            .await
            .unwrap()
    );
    let (a, b) = tokio::join!(
        acks::consume_action(db, &f.chat, &row.id, tool, &args),
        acks::consume_action(db, &f.chat, &row.id, tool, &args)
    );
    assert_eq!(usize::from(a.unwrap()) + usize::from(b.unwrap()), 1);
    assert_eq!(
        acks::history(db, &f.owner, &f.row.id).await.unwrap()[0].status,
        "used"
    );
    let row = acks::request(
        db,
        &f.chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some(tool),
            arguments: Some(&args),
            summary: "Delete agent key",
            platform: false,
        },
    )
    .await
    .unwrap();
    acks::decide(db, &f.owner, &f.row.id, &row.id, true)
        .await
        .unwrap();
    db.collection::<AssistantAcknowledgement>(ACKS).update_one(doc! {"_id": &row.id},
        doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
    assert!(
        !acks::consume_action(db, &f.chat, &row.id, tool, &args)
            .await
            .unwrap()
    );
    assert_eq!(
        acks::history(db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .iter()
            .find(|a| a.id == row.id)
            .unwrap()
            .status,
        "expired"
    );
}

#[tokio::test]
async fn rotation_invalidates_account_and_action_acknowledgements() {
    let f = fixture("ack_rotate").await;
    let db = &f.state.db;
    f.allow_account().await;
    let row = acks::request(
        db,
        &f.chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("nyxid__delete_node"),
            arguments: Some(&json!({"node_id": "node"})),
            summary: "Delete node",
            platform: false,
        },
    )
    .await
    .unwrap();
    acks::decide(db, &f.owner, &f.row.id, &row.id, true)
        .await
        .unwrap();
    let rotated =
        key_service::rotate_api_key(db, &f.state.encryption_keys, &f.owner, &f.chat.api_key_id)
            .await
            .unwrap();
    assert!(
        acks::consume_action(
            db,
            &f.chat,
            &row.id,
            "nyxid__delete_node",
            &json!({"node_id": "node"})
        )
        .await
        .is_err()
    );
    let key = key_service::get_api_key(db, &f.owner, &rotated.id)
        .await
        .unwrap();
    assert!(
        !key.scopes
            .split_whitespace()
            .any(|s| s == ASSISTANT_ACCOUNT_SCOPE)
    );
    assert!(
        acks::history(db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .iter()
            .all(|row| row.status == "expired")
    );
    let chat = acks::for_key(db, &f.owner, Some(&rotated.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        acks::account_gate(db, &chat).await.unwrap().unwrap()["error"],
        "acknowledgement_required"
    );
    // Rotation does not rewrite the last turn's credential id. Deletion must
    // follow the live credential row and revoke its successor nonetheless.
    db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.row.id},
            doc! {"$set": {"active_turn": bson::Bson::Null}},
        )
        .await
        .unwrap();
    engine::delete(db, &f.owner, &f.row.id).await.unwrap();
    assert!(
        key_service::get_api_key(db, &f.owner, &rotated.id)
            .await
            .is_err()
    );
    assert_eq!(
        db.collection::<bson::Document>(ACKS)
            .count_documents(doc! {"conversation_id": &f.row.id})
            .await
            .unwrap(),
        0
    );
}

fn minimal_args(name: &str, id: &str) -> Value {
    let schema = tools::schema(name);
    let mut args = serde_json::Map::new();
    for required in schema["required"].as_array().unwrap() {
        let field = required.as_str().unwrap();
        let property = &schema["properties"][field];
        let value = property["enum"]
            .as_array()
            .map(|values| values[0].clone())
            .unwrap_or_else(|| {
                if property["type"] == "boolean" {
                    json!(true)
                } else {
                    json!(id)
                }
            });
        args.insert(field.into(), value);
    }
    Value::Object(args)
}

#[test]
fn native_inventory_is_closed_and_schemas_exclude_secret_inputs() {
    assert_eq!(tools::TOOL_NAMES.len(), 22);
    let service = tools::virtual_service();
    assert_eq!(service.service_slug, "nyxid");
    assert_eq!(service.service_name, "NyxID account");
    assert!(matches!(
        service.source,
        super::mcp_service::McpToolSource::Internal
    ));
    assert!(service.executable);
    for name in tools::TOOL_NAMES {
        let args = minimal_args(name, &Uuid::new_v4().to_string());
        tools::validate_arguments(name, &args).unwrap();
        for forbidden in [
            "token",
            "bot_token",
            "secret",
            "app_secret",
            "credential",
            "full_key",
            "scopes",
            "owner_id",
        ] {
            let mut invalid = args.clone();
            invalid[forbidden] = json!("must never be accepted");
            assert!(
                tools::validate_arguments(name, &invalid).is_err(),
                "{name} {forbidden}"
            );
        }
        let endpoint = service.endpoints.iter().find(|e| e.name == *name).unwrap();
        assert!(
            endpoint
                .description
                .as_ref()
                .unwrap()
                .contains(if tools::destructive(name) {
                    "Destructive:"
                } else {
                    "Non-destructive."
                })
        );
    }
    for excluded in [
        "create_agent_key",
        "rotate_agent_key",
        "decide_approval",
        "manage_org",
        "billing",
    ] {
        assert!(tools::validate_arguments(excluded, &json!({})).is_err());
    }
    assert!(
        tools::validate_arguments(
            "update_agent_key",
            &json!({"api_key_id": "id", "rate_limit_burst": -1})
        )
        .is_err()
    );
}

#[tokio::test]
async fn every_native_tool_requires_a_real_conversation_key_and_audits_refusals() {
    let f = fixture("native_key_gate").await;
    let mut auth = f.auth.clone();
    auth.api_key_id = Some(Uuid::new_v4().to_string());
    // Platform metadata alone is not a conversation credential.
    let key = key_service::get_api_key(&f.state.db, &f.owner, &f.chat.api_key_id)
        .await
        .unwrap();
    let mut impostor = key.clone();
    impostor.id = auth.api_key_id.clone().unwrap();
    f.state
        .db
        .collection(crate::models::api_key::COLLECTION_NAME)
        .insert_one(impostor)
        .await
        .unwrap();
    for name in tools::TOOL_NAMES {
        let result = f
            .tools()
            .execute(
                &auth,
                &format!("nyxid__{name}"),
                &minimal_args(name, &key.id),
            )
            .await;
        assert!(result.is_error, "{name}");
        assert_eq!(result.value["error"], "forbidden", "{name}");
    }
    let count = f
        .state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .count_documents(doc! {"event_type": "assistant_account_tool_call"})
        .await
        .unwrap();
    assert_eq!(count, tools::TOOL_NAMES.len() as u64);
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(ACKS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn every_native_tool_requests_account_acknowledgement_and_preserves_owner_scope() {
    let f = fixture("native_account_gate").await;
    let missing = Uuid::new_v4().to_string();
    for name in tools::TOOL_NAMES {
        let result = f
            .tools()
            .execute(
                &f.auth,
                &format!("nyxid__{name}"),
                &minimal_args(name, &missing),
            )
            .await;
        assert!(result.is_error, "{name}");
        assert_eq!(result.value["error"], "acknowledgement_required", "{name}");
        assert_eq!(result.value["kind"], "account");
    }
    let requests = acks::history(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    acks::decide(&f.state.db, &f.owner, &f.row.id, &requests[0].id, true)
        .await
        .unwrap();
    for name in tools::TOOL_NAMES {
        let result = f
            .tools()
            .execute(
                &f.auth,
                &format!("nyxid__{name}"),
                &minimal_args(name, &missing),
            )
            .await;
        if name.starts_with("list_") && *name != "list_agent_key_bindings" {
            assert!(!result.is_error, "{name}: unexpected error result");
        } else {
            assert!(result.is_error, "{name}");
            assert!(
                result.value["error"]
                    .as_str()
                    .unwrap()
                    .ends_with("not_found"),
                "{name}: unexpected error code"
            );
        }
    }
    let logs: Vec<bson::Document> = {
        use futures::TryStreamExt;
        f.state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .find(doc! {"event_type": "assistant_account_tool_call"})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap()
    };
    assert_eq!(logs.len(), tools::TOOL_NAMES.len() * 2);
    for log in logs {
        assert_eq!(log.get_str("api_key_id").unwrap(), f.chat.api_key_id);
        let event = log.get_document("event_data").unwrap();
        assert_eq!(event.get_str("conversation_id").unwrap(), f.row.id);
        assert!(event.contains_key("tool_name") && event.contains_key("outcome"));
        assert!(!event.contains_key("arguments") && !event.contains_key("result"));
    }
}

pub(crate) async fn ordinary_key(f: &Fixture) -> String {
    key_service::create_api_key(
        &f.state.db,
        &f.owner,
        "ci-bot",
        "proxy",
        None,
        None,
        None,
        None,
        Some(true),
        Some(false),
        Some(true),
        None,
        None,
        Some("codex"),
        Some("https://agent.example.com/callback"),
    )
    .await
    .unwrap()
    .id
}

async fn call(f: &Fixture, name: &str, mut args: Value) -> Value {
    let name = format!("nyxid__{name}");
    let mut result = f.tools().execute(&f.auth, &name, &args).await;
    if result.value["error"] == "acknowledgement_required" && result.value["kind"] == "action" {
        let id = result.value["acknowledgement_id"].as_str().unwrap();
        acks::decide(&f.state.db, &f.owner, &f.row.id, id, true)
            .await
            .unwrap();
        args["acknowledgement_id"] = json!(id);
        result = f.tools().execute(&f.auth, &name, &args).await;
    }
    assert!(!result.is_error, "{name}: unexpected error result");
    let serialized = result.value.to_string();
    for secret_field in [
        "key_hash",
        "full_key",
        "key_ciphertext",
        "bot_token_encrypted",
        "auth_token_hash",
    ] {
        assert!(!serialized.contains(secret_field), "{name}");
    }
    result.value
}

#[tokio::test]
async fn native_account_inventory_executes_existing_services_and_audits_every_tool() {
    native_inventory(false).await;
}

#[tokio::test]
async fn native_account_full_access_executes_every_tool_without_cards_and_audits_the_mode() {
    native_inventory(true).await;
}

async fn native_inventory(full: bool) {
    let f = fixture("native_success_inventory").await;
    if full {
        idle(&f).await;
        super::assistant_access_mode_service::change(
            &f.state.db,
            &f.owner,
            &f.row.id,
            crate::models::assistant_conversation::AccessMode::Full,
        )
        .await
        .unwrap();
    } else {
        f.allow_account().await;
    }
    let db = &f.state.db;
    let key = ordinary_key(&f).await;
    let service = connected(db, &f.owner, "example", "https://service.example.com").await;
    let credential = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    db.collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME).insert_one(doc! {
        "_id": &credential, "user_id": &f.owner, "label": "Stored credential", "credential_type": "api_key",
        "status": "active", "created_at": now, "updated_at": now,
    }).await.unwrap();
    call(&f, "list_agent_keys", json!({})).await;
    assert_eq!(
        call(&f, "get_agent_key", json!({"api_key_id": key})).await["name"],
        "ci-bot"
    );
    let updated = call(&f, "update_agent_key", json!({"api_key_id": key, "name": "release-bot",
        "description": "CI deploys", "platform": "generic", "callback_url": "https://agent.example.com/new",
        "rate_limit_per_second": 10, "rate_limit_burst": 20, "allow_all_services": false,
        "allow_auto_connected_services": true, "allow_all_nodes": false,
        "allowed_service_slugs": ["example"], "allowed_node_ids": []})).await;
    assert_eq!(updated["allowed_service_ids"], json!([service]));
    assert_eq!(updated["allow_auto_connected_services"], true);
    let binding = call(
        &f,
        "bind_agent_key_credential",
        json!({"api_key_id": key,
        "user_service_id": service, "user_api_key_id": credential}),
    )
    .await;
    assert_eq!(
        call(&f, "list_agent_key_bindings", json!({"api_key_id": key})).await["total"],
        1
    );
    call(
        &f,
        "unbind_agent_key_credential",
        json!({"api_key_id": key, "binding_id": binding["id"]}),
    )
    .await;
    let bot = Uuid::new_v4().to_string();
    db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).insert_one(doc! {
        "_id": &bot, "user_id": &f.owner, "platform": "telegram", "label": "Build bot",
        "bot_token_encrypted": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1,2,3]},
        "platform_bot_id": "123", "platform_bot_username": "build_bot", "webhook_registered": false,
        "webhook_secret_hash": "not-returned", "status": "active", "is_active": true,
        "created_at": now, "updated_at": now,
    }).await.unwrap();
    call(&f, "list_channel_bots", json!({})).await;
    assert_eq!(
        call(&f, "get_channel_bot", json!({"bot_id": bot})).await["label"],
        "Build bot"
    );
    call(
        &f,
        "update_channel_bot",
        json!({"bot_id": bot, "label": "Deploy bot"}),
    )
    .await;
    let route = call(&f, "set_channel_route", json!({"bot_id": bot, "agent_api_key_id": key,
        "platform_conversation_id": "chat123", "allow_agent_initiated": true, "default_agent": true})).await;
    assert_eq!(
        call(&f, "list_channel_routes", json!({"bot_id": bot})).await["total"],
        1
    );
    call(
        &f,
        "set_channel_route",
        json!({"route_id": route["id"], "agent_api_key_id": key,
        "allow_agent_initiated": false, "default_agent": false}),
    )
    .await;
    call(&f, "delete_channel_route", json!({"route_id": route["id"]})).await;
    call(&f, "delete_channel_bot", json!({"bot_id": bot})).await;
    call(
        &f,
        "set_service_enabled",
        json!({"service_id": service, "enabled": false}),
    )
    .await;
    let services = call(&f, "list_my_services", json!({})).await;
    assert_eq!(services["items"][0]["is_active"], false);
    call(
        &f,
        "set_service_enabled",
        json!({"service_id": service, "enabled": true}),
    )
    .await;
    call(
        &f,
        "set_approval_mode",
        json!({"service_id": service, "approval_mode": "per_request", "approval_required": true}),
    )
    .await;
    assert_eq!(
        call(&f, "list_approval_configs", json!({})).await["items"][0]["approval_required"],
        true
    );
    call(&f, "list_pending_approvals", json!({})).await;
    call(&f, "delete_service", json!({"service_id": service})).await;
    let node = Uuid::new_v4().to_string();
    db.collection::<bson::Document>(crate::models::node::COLLECTION_NAME).insert_one(doc! {
        "_id": &node, "user_id": &f.owner, "name": "Workstation", "status": "offline",
        "auth_token_hash": "not-returned", "is_active": true, "created_at": now, "updated_at": now,
    }).await.unwrap();
    assert_eq!(
        call(&f, "list_nodes", json!({})).await["items"][0]["name"],
        "Workstation"
    );
    call(&f, "delete_node", json!({"node_id": node})).await;
    call(&f, "delete_agent_key", json!({"api_key_id": key})).await;
    assert!(key_service::get_api_key(db, &f.owner, &key).await.is_err());
    for name in tools::TOOL_NAMES {
        let count = db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {
                "event_type": "assistant_account_tool_call",
                "event_data.tool_name": format!("nyxid__{name}"),
                "event_data.outcome": "success",
            })
            .await
            .unwrap();
        assert!(count >= 1, "{name}");
        if tools::destructive(name) && !full {
            let count = db
                .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {
                    "event_type": "assistant_account_tool_call",
                    "event_data.tool_name": format!("nyxid__{name}"),
                    "event_data.outcome": "success",
                    "event_data.acknowledgement_id": {"$type": "string"},
                })
                .await
                .unwrap();
            assert_eq!(count, 1, "{name}");
        }
    }
    if full {
        assert_eq!(
            db.collection::<bson::Document>(ACKS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"event_type": "assistant_account_tool_call",
                "event_data.access_mode": {"$ne": "full"}})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn assistant_keys_cannot_self_widen_bind_or_become_route_agents() {
    let f = fixture("native_self_widen").await;
    f.allow_account().await;
    assert_assistant_key_boundaries(&f).await;
}

#[tokio::test]
async fn full_access_cannot_modify_chat_keys_or_use_them_as_route_agents() {
    use crate::models::assistant_conversation::AccessMode::Full;
    let f = fixture("native_full_self_widen").await;
    idle(&f).await;
    super::assistant_access_mode_service::change(&f.state.db, &f.owner, &f.row.id, Full)
        .await
        .unwrap();
    assert_assistant_key_boundaries(&f).await;
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(ACKS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn native_key_tools_hide_another_owners_existing_key_even_with_full_access() {
    use crate::models::{api_key::COLLECTION_NAME as KEYS, assistant_conversation::AccessMode};
    let f = fixture("native_other_owner").await;
    idle(&f).await;
    super::assistant_access_mode_service::change(
        &f.state.db,
        &f.owner,
        &f.row.id,
        AccessMode::Full,
    )
    .await
    .unwrap();
    let mut other = key_service::get_api_key(&f.state.db, &f.owner, &f.chat.api_key_id)
        .await
        .unwrap();
    other.id = Uuid::new_v4().to_string();
    other.user_id = Uuid::new_v4().to_string();
    other.key_hash = Uuid::new_v4().to_string();
    other.name = "another owner's private key".into();
    f.state
        .db
        .collection::<crate::models::api_key::ApiKey>(KEYS)
        .insert_one(&other)
        .await
        .unwrap();
    for name in [
        "get_agent_key",
        "update_agent_key",
        "delete_agent_key",
        "list_agent_key_bindings",
        "bind_agent_key_credential",
        "unbind_agent_key_credential",
    ] {
        let result = f
            .tools()
            .execute(
                &f.auth,
                &format!("nyxid__{name}"),
                &minimal_args(name, &other.id),
            )
            .await;
        assert!(result.is_error, "{name}");
        assert_eq!(result.value["error"], "not_found", "{name}");
        assert!(!result.value.to_string().contains(&other.name));
    }
    assert!(
        key_service::get_api_key(&f.state.db, &other.user_id, &other.id)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn legacy_conversation_defaults_to_ask_and_owner_unique_credentials_migrate() {
    use crate::models::{
        assistant_agent_credential::COLLECTION_NAME as CREDENTIALS,
        assistant_conversation::{AccessMode, COLLECTION_NAME as CONVERSATIONS},
    };
    let f = fixture("native_legacy_migration").await;
    let mut legacy = bson::to_document(&f.row).unwrap();
    legacy.remove("access_mode");
    let row: AssistantConversation = bson::from_document(legacy.clone()).unwrap();
    assert_eq!(row.access_mode, AccessMode::Ask);
    assert!(matches!(
        legacy.get("created_at"),
        Some(bson::Bson::DateTime(_))
    ));
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .replace_one(doc! {"_id": &f.row.id}, legacy)
        .await
        .unwrap();
    let credentials = f.state.db.collection::<bson::Document>(CREDENTIALS);
    credentials
        .update_one(
            doc! {"conversation_id": &f.row.id},
            doc! {"$unset": {"conversation_id": ""}},
        )
        .await
        .unwrap();
    credentials
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"user_id": 1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    engine::ensure_indexes(&f.state.db).await.unwrap();
    engine::ensure_indexes(&f.state.db).await.unwrap();
    assert!(
        key_service::get_api_key(&f.state.db, &f.owner, &f.chat.api_key_id)
            .await
            .is_err()
    );
    assert_eq!(credentials.count_documents(doc! {}).await.unwrap(), 0);
    for _ in 0..2 {
        engine::begin_turn(
            &f.state.db,
            &f.owner,
            &engine::TurnRequest {
                conversation_id: None,
                text: "New chat".into(),
                model: None,
                access_mode: None,
            },
            &f.state.encryption_keys,
        )
        .await
        .unwrap();
    }
    assert_eq!(
        credentials
            .count_documents(doc! {"user_id": &f.owner})
            .await
            .unwrap(),
        2
    );
}

async fn assert_assistant_key_boundaries(f: &Fixture) {
    let before = key_service::get_api_key(&f.state.db, &f.owner, &f.chat.api_key_id)
        .await
        .unwrap();
    let other = engine::begin_turn(
        &f.state.db,
        &f.owner,
        &engine::TurnRequest {
            conversation_id: None,
            text: "Another conversation".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    for id in [&f.chat.api_key_id, &other.credential_api_key_id] {
        let result = f
            .tools()
            .execute(
                &f.auth,
                "nyxid__delete_agent_key",
                &json!({"api_key_id": id}),
            )
            .await;
        assert!(result.is_error);
        assert_eq!(result.value["error"], "validation_error");
        assert!(
            key_service::get_api_key(&f.state.db, &f.owner, id)
                .await
                .is_ok()
        );
    }
    assert_eq!(
        f.state
            .db
            .collection::<AssistantAcknowledgement>(ACKS)
            .count_documents(doc! {"tool_name": "nyxid__delete_agent_key"})
            .await
            .unwrap(),
        0
    );
    for args in [
        json!({"api_key_id": f.chat.api_key_id, "allow_all_services": true}),
        json!({"api_key_id": f.chat.api_key_id, "name": "hide assistant identity", "platform": "codex"}),
    ] {
        let result = f
            .tools()
            .execute(&f.auth, "nyxid__update_agent_key", &args)
            .await;
        assert!(result.is_error);
        assert_eq!(result.value["error"], "validation_error");
    }
    let result = f.tools().execute(&f.auth, "nyxid__bind_agent_key_credential", &json!({
        "api_key_id": f.chat.api_key_id, "user_service_id": "service", "user_api_key_id": "credential",
    })).await;
    assert!(result.is_error);
    let key = key_service::get_api_key(&f.state.db, &f.owner, &f.chat.api_key_id)
        .await
        .unwrap();
    assert_eq!(key.allow_all_services, before.allow_all_services);
    assert_eq!(key.state_version, before.state_version);
    assert_eq!(key.platform, before.platform);
    assert!(key.allowed_service_ids.is_empty());
    let result = super::channel_routing_service::create_conversation(
        &f.state.db,
        &f.owner,
        Some("bot"),
        "telegram",
        "chat",
        "private",
        None,
        &key.id,
        false,
        false,
    )
    .await;
    assert!(
        matches!(result, Err(crate::errors::AppError::ValidationError(message)) if message.contains("Assistant chat keys"))
    );
    let normal = ordinary_key(f).await;
    let route = super::channel_routing_service::create_conversation(
        &f.state.db,
        &f.owner,
        Some("bot"),
        "telegram",
        "chat",
        "private",
        None,
        &normal,
        false,
        false,
    )
    .await
    .unwrap();
    let result = super::channel_routing_service::update_conversation(
        &f.state.db,
        &route.id,
        &f.owner,
        Some(&key.id),
        None,
        None,
        None,
    )
    .await;
    assert!(
        matches!(result, Err(crate::errors::AppError::ValidationError(message)) if message.contains("Assistant chat keys"))
    );
    let result = f
        .tools()
        .execute(
            &f.auth,
            "nyxid__set_channel_route",
            &json!({
                "route_id": route.id, "agent_api_key_id": key.id,
            }),
        )
        .await;
    assert!(result.is_error);
}

async fn idle(f: &Fixture) {
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.row.id},
            doc! {"$set": {"active_turn": bson::Bson::Null}},
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn access_mode_switch_is_owner_scoped_fenced_and_preserves_acknowledged_services() {
    use super::assistant_access_mode_service::change;
    use crate::models::assistant_conversation::AccessMode::{Ask, Full};
    let f = fixture("mode_switch").await;
    let db = &f.state.db;
    assert!(matches!(
        change(db, &f.owner, &f.row.id, Full).await,
        Err(crate::errors::AppError::AssistantTurnActive)
    ));
    assert!(matches!(
        change(db, &Uuid::new_v4().to_string(), &f.row.id, Full).await,
        Err(crate::errors::AppError::NotFound(_))
    ));
    let service = connected(db, &f.owner, "approved", "https://service.example.com").await;
    let refusal = acks::service_gate(db, &f.chat, &service, "approved", "Approved", false)
        .await
        .unwrap()
        .unwrap();
    acks::decide(
        db,
        &f.owner,
        &f.row.id,
        refusal["acknowledgement_id"].as_str().unwrap(),
        true,
    )
    .await
    .unwrap();
    f.allow_account().await;
    acks::request(
        db,
        &f.chat,
        acks::Request {
            kind: "action",
            tool: Some("nyxid__delete_node"),
            service: None,
            arguments: Some(&json!({"node_id": "example"})),
            summary: "Delete node example",
            platform: false,
        },
    )
    .await
    .unwrap();
    idle(&f).await;
    let before = key_service::get_api_key(db, &f.owner, &f.chat.api_key_id)
        .await
        .unwrap();
    let (old, row) = change(db, &f.owner, &f.row.id, Full).await.unwrap();
    assert_eq!(old, Ask);
    assert_eq!(row.access_mode, Full);
    let full = key_service::get_api_key(db, &f.owner, &f.chat.api_key_id)
        .await
        .unwrap();
    assert!(full.allow_all_services && full.allow_all_nodes && full.allow_auto_connected_services);
    assert!(full.scopes.contains(ASSISTANT_ACCOUNT_SCOPE));
    assert!(full.state_version > before.state_version);
    let (old, row) = change(db, &f.owner, &f.row.id, Ask).await.unwrap();
    assert_eq!(old, Full);
    assert_eq!(row.access_mode, Ask);
    let ask = key_service::get_api_key(db, &f.owner, &f.chat.api_key_id)
        .await
        .unwrap();
    assert!(!ask.allow_all_services && ask.allow_all_nodes);
    assert!(ask.allow_auto_connected_services);
    assert_eq!(ask.allowed_service_ids, vec![service]);
    assert_eq!(ask.scopes, "proxy");
    assert!(ask.state_version > full.state_version);
    assert_eq!(
        acks::history(db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .iter()
            .filter(|row| row.status == "pending")
            .count(),
        0
    );
}

#[tokio::test]
async fn full_draft_provisions_full_authority_and_rotation_and_replacement_preserve_mode() {
    use super::assistant_agent_credential_service as credentials;
    use crate::models::assistant_conversation::AccessMode::Full;
    let f = fixture("mode_full_draft").await;
    let row = engine::begin_turn(
        &f.state.db,
        &f.owner,
        &engine::TurnRequest {
            conversation_id: None,
            text: "Manage everything".into(),
            model: None,
            access_mode: Some(Full),
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let mut key = key_service::get_api_key(&f.state.db, &f.owner, &row.credential_api_key_id)
        .await
        .unwrap();
    assert!(key.allow_all_services && key.allow_all_nodes);
    assert!(key.scopes.contains(ASSISTANT_ACCOUNT_SCOPE));
    let rotated =
        key_service::rotate_api_key(&f.state.db, &f.state.encryption_keys, &f.owner, &key.id)
            .await
            .unwrap();
    key = key_service::get_api_key(&f.state.db, &f.owner, &rotated.id)
        .await
        .unwrap();
    assert!(key.allow_all_services && key.allow_all_nodes);
    assert!(key.scopes.contains(ASSISTANT_ACCOUNT_SCOPE));
    key_service::delete_api_key(&f.state.db, &f.owner, &key.id)
        .await
        .unwrap();
    let replacement =
        credentials::load_or_provision(&f.state.db, &f.state.encryption_keys, &f.owner, &row.id)
            .await
            .unwrap();
    key = key_service::get_api_key(&f.state.db, &f.owner, &replacement.api_key_id)
        .await
        .unwrap();
    assert!(key.allow_all_services && key.allow_all_nodes);
    assert!(key.scopes.contains(ASSISTANT_ACCOUNT_SCOPE));
    let chat = acks::for_key(&f.state.db, &f.owner, Some(&key.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(chat.access_mode, Full);
    assert!(
        acks::account_gate(&f.state.db, &chat)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        acks::service_gate(&f.state.db, &chat, "service", "example", "Example", false)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(ACKS)
            .count_documents(doc! {"conversation_id": &row.id})
            .await
            .unwrap(),
        0
    );
    let event = f
        .state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"event_type": "assistant_access_mode_changed",
        "event_data.conversation_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event
            .get_document("event_data")
            .unwrap()
            .get_str("new_mode")
            .unwrap(),
        "full"
    );
}

#[tokio::test]
async fn acknowledgement_history_keeps_pending_and_only_twenty_decided_without_arguments() {
    let f = fixture("ack_history_bound").await;
    let pending = acks::request(
        &f.state.db,
        &f.chat,
        acks::Request {
            kind: "account",
            service: None,
            tool: None,
            arguments: None,
            summary: "Account",
            platform: false,
        },
    )
    .await
    .unwrap();
    let mut decided = Vec::new();
    for index in 0..25 {
        let mut row = pending.clone();
        row.id = Uuid::new_v4().to_string();
        row.status = "denied".into();
        row.created_at -= Duration::seconds(index + 1);
        row.decided_at = Some(row.created_at);
        decided.push(row);
    }
    f.state
        .db
        .collection::<AssistantAcknowledgement>(ACKS)
        .insert_many(decided)
        .await
        .unwrap();
    let rows = acks::history(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 21);
    assert_eq!(rows.iter().filter(|row| row.status == "pending").count(), 1);
    assert!(
        rows.iter()
            .all(|row| row.created_at >= pending.created_at - Duration::seconds(21))
    );
}

#[tokio::test]
async fn conversation_provisioning_rolls_back_its_key_when_the_first_message_cannot_commit() {
    let f = fixture("chat_provision_rollback").await;
    let db = &f.state.db;
    let keys = db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME);
    let before = keys.count_documents(doc! {}).await.unwrap();
    // A deterministic validation failure happens after the key has been inserted
    // in begin_turn's transaction; no key, credential or conversation may escape.
    db.run_command(
        doc! {"collMod": crate::models::assistant_message::COLLECTION_NAME,
        "validator": {"text": {"$ne": "reject this message"}}, "validationLevel": "strict"},
    )
    .await
    .unwrap();
    let result = engine::begin_turn(
        db,
        &f.owner,
        &engine::TurnRequest {
            conversation_id: None,
            text: "reject this message".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await;
    assert!(result.is_err());
    assert_eq!(keys.count_documents(doc! {}).await.unwrap(), before);
    assert_eq!(
        db.collection::<bson::Document>(crate::models::assistant_agent_credential::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn deleting_a_conversation_revokes_children_clears_bindings_and_audits_after_commit() {
    use crate::models::api_key_credential::{ApiKeyCredential, COLLECTION_NAME as CHILDREN};
    let f = fixture("chat_delete_children").await;
    let child = Uuid::new_v4().to_string();
    let key = &f.chat.api_key_id;
    let db = &f.state.db;
    db.collection::<bson::Document>(CHILDREN)
        .insert_one(doc! {
            "_id": &child, "api_key_id": key, "user_id": &f.owner,
            "secret_hash": "private-hash", "secret_prefix": "nyxid_ag_child",
            "label": "Test child", "login_request_id": "test-login", "is_active": true,
            "created_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::agent_service_binding::COLLECTION_NAME)
        .insert_one(doc! {"_id": Uuid::new_v4().to_string(), "api_key_id": key,
        "user_id": &f.owner, "user_service_id": "service", "user_api_key_id": "credential",
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now()})
        .await
        .unwrap();
    idle(&f).await;
    engine::delete(db, &f.owner, &f.row.id).await.unwrap();
    let row = db
        .collection::<ApiKeyCredential>(CHILDREN)
        .find_one(doc! {"_id": &child})
        .await
        .unwrap()
        .unwrap();
    assert!(!row.is_active && row.revoked_at.is_some());
    assert_eq!(
        db.collection::<bson::Document>(crate::models::agent_service_binding::COLLECTION_NAME)
            .count_documents(doc! {"api_key_id": key})
            .await
            .unwrap(),
        0
    );
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let audit = db
                .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .find_one(doc! {"event_type": "agent_key_credential_revoked",
                "event_data.credential_id": &child})
                .await
                .unwrap();
            if audit.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
