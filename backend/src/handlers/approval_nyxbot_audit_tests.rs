use super::*;
use crate::{
    models::{assistant_settings::AccessMode, user::UserType, user_service},
    services::{
        assistant_account_tools::{AccountTools, ToolResult},
        assistant_authority_tests::{chat_for_row, new_orchestrator},
        assistant_nyxagent, assistant_settings_service, key_service, security_readiness_service,
    },
    test_utils::{test_user, test_user_service},
};

const EVENT: &str = "service_approval_config_set";
const TOOL: &str = "nyxid__set_approval_mode";

struct Call {
    auth: AuthUser,
    user_service_id: String,
}

impl Call {
    async fn prepare(f: &Fixture, catalog: bool) -> Self {
        let db = &f.state.db;
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&f.owner, UserType::Person))
            .await
            .unwrap();
        security_readiness_service::ready_test_replica(db).await;
        assistant_nyxagent::ensure_indexes(db).await.unwrap();
        let conversation = new_orchestrator(&f.state, &f.owner, "Manage approval settings").await;
        let key = key_service::get_api_key(db, &f.owner, &conversation.credential_api_key_id)
            .await
            .unwrap();
        let mut auth = f.auth();
        auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
        auth.api_key_id = Some(key.id);
        auth.api_key_name = Some(key.name);
        auth.assistant_chat = Some(Arc::new(
            chat_for_row(&f.state, &f.owner, &conversation).await,
        ));
        assistant_settings_service::update(
            db,
            &f.owner,
            assistant_settings_service::Update {
                access_mode: Some(AccessMode::Autopilot),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let user_service_id = if catalog {
            Uuid::new_v4().to_string()
        } else {
            f.service.clone()
        };
        db.collection(user_service::COLLECTION_NAME)
            .insert_one(test_user_service(
                &user_service_id,
                &f.owner,
                "fixture-service",
                "endpoint",
                catalog.then_some(f.service.as_str()),
                None,
            ))
            .await
            .unwrap();
        let now = bson::DateTime::now();
        db.collection::<Document>(CONFIGS)
            .insert_one(doc! {
                "_id":Uuid::new_v4().to_string(),"user_id":&f.owner,"service_id":&f.service,
                "service_name":"original","approval_required":true,"approval_mode":"grant",
                "created_at":now,"updated_at":now
            })
            .await
            .unwrap();
        Self {
            auth,
            user_service_id,
        }
    }

    async fn execute(&self, f: &Fixture) -> ToolResult {
        let state = &f.state;
        let tools = AccountTools {
            db: &state.db,
            keys: &state.encryption_keys,
            http: &state.http_client,
            config: &state.config,
            jwt_keys: &state.jwt_keys,
            token_exchange_cache: &state.token_exchange_cache,
            node_manager: &state.node_ws_manager,
        };
        tools
            .execute(
                &self.auth,
                TOOL,
                &json!({"service_id":self.user_service_id,
            "approval_mode":"per_request","approval_required":false}),
            )
            .await
    }
}

#[tokio::test]
async fn nyxbot_policy_death_replay_and_parity() {
    let f = Fixture::new().await;
    let call = Call::prepare(&f, true).await;
    let context = crate::turn_trace::TurnContext::for_turn(&Uuid::new_v4().to_string());
    let db = &f.state.db;
    let before = f.rows(AUDIT).await;
    f.monitor.lock().unwrap().stop_after_source = true;
    let stopped = AssertUnwindSafe(REQUEST.scope(
        (),
        crate::turn_trace::scope(context, Box::pin(call.execute(&f))),
    ))
    .catch_unwind()
    .await;
    assert!(stopped.unwrap_err().is::<ProcessStopped>());
    let policies = f.rows(CONFIGS).await;
    assert_eq!(policies.len(), 1);
    assert_eq!(policies[0].get_bool("approval_required"), Ok(false));
    assert_eq!(policies[0].get_str("approval_mode"), Ok("per_request"));
    assert_eq!(f.rows(AUDIT).await, before);
    let intents = f.rows(INTENTS).await;
    println!(
        "APPROVAL_DEATH change=NyxBot durable_intents={}",
        intents.len()
    );
    assert_eq!(intents.len(), 1);

    let actual = relay_twice(&f, &intents[0], EVENT).await;
    assert_event_content(&f, actual, Some(&call.auth), context, EVENT, json!({
        "service_id":f.service, "service_name":"fixture-service", "user_service_id":call.user_service_id,
        "policy_owner_user_id":f.owner, "approval_required":false, "approval_mode":"per_request",
        "approval_rule_count":0, "default_effect":null
    })).await;
    crate::test_utils::drop_test_database(db).await;
}

async fn tool_outcome(f: &Fixture) -> String {
    let row = f
        .state
        .db
        .collection::<Document>(AUDIT)
        .find_one(doc! {"event_type":"assistant_account_tool_call"})
        .await
        .unwrap()
        .unwrap();
    let data = row.get_document("event_data").unwrap();
    assert_eq!(data.get_str("tool_name"), Ok(TOOL));
    data.get_str("outcome").unwrap().to_owned()
}

#[tokio::test]
async fn nyxbot_policy_rejected_intent() {
    let f = Fixture::new().await;
    let call = Call::prepare(&f, true).await;
    let before = f.rows(CONFIGS).await;
    let grants = f.rows(GRANTS).await;
    reject_writes(&f, INTENTS).await;
    let result = Box::pin(call.execute(&f)).await;
    assert!(result.is_error, "expected rejected policy change");
    assert_eq!(result.value["error_code"], 1007);
    assert_eq!(f.rows(CONFIGS).await, before);
    assert_eq!(f.rows(GRANTS).await, grants);
    assert!(f.rows(INTENTS).await.is_empty());
    assert_eq!(tool_outcome(&f).await, "refused");
    crate::test_utils::drop_test_database(&f.state.db).await;
}

#[tokio::test]
async fn nyxbot_policy_request_commands() {
    let f = Fixture::new().await;
    let call = Call::prepare(&f, false).await;
    let receipt =
        audit_service::notify_on_audit_write_for_user("assistant_action_auto_allowed", &f.owner);
    let result = REQUEST.scope((), Box::pin(call.execute(&f))).await;
    assert!(!result.is_error, "{}", result.value);
    assert_eq!(
        result.value,
        json!({"service_id":f.service,
        "approval_required":false,"approval_mode":"per_request"})
    );
    assert_eq!(tool_outcome(&f).await, "success");
    receipt.await.unwrap();
    let commands = f.monitor.lock().unwrap().commands.clone();
    println!(
        "APPROVAL_REQUEST change=NyxBot count={} {commands:?}",
        commands.len()
    );
    assert_eq!(commands.len(), 17);
    let upsert = commands
        .iter()
        .position(|(cmd, coll)| cmd == "findAndModify" && coll == CONFIGS)
        .unwrap();
    assert_eq!(
        &commands[upsert + 1..upsert + 5],
        &[
            ("insert".into(), INTENTS.into()),
            ("commitTransaction".into(), "".into()),
            ("update".into(), GRANTS.into()),
            ("update".into(), REQUESTS.into()),
        ]
    );
    assert_eq!(f.rows(INTENTS).await.len(), 1);
    crate::test_utils::drop_test_database(&f.state.db).await;
}
