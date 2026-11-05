use super::*;
use crate::models::crm_record::CrmActor;
use crate::services::crm::{self, records};
use mongodb::bson::{Document, doc};
use serde_json::{Value, json};
async fn decoded(response: Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let rpc: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(rpc.get("error").is_none(), "{rpc}");
    serde_json::from_str(rpc["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
#[tokio::test]
async fn crm_direct_and_universal_wrappers_preserve_validation_conflicts_and_scope_refusals() {
    let f = crm::tools::tests::fixture().await;
    let mut auth = McpAuthContext::user(f.owner.clone(), AuthMethod::ApiKey);
    auth.chat = Some(f.chat.clone());
    auth.api_key_id = Some(f.chat.api_key_id.clone());
    let company = records::create(
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
    .unwrap();
    // The admitted agent observation must be newer than the human creation;
    // older work intentionally skips fields before evaluating their values.
    f.state
        .db
        .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.chat.conversation_id},
            doc! {"$set": {"active_turn.started_at": mongodb::bson::DateTime::now()}},
        )
        .await
        .unwrap();
    for universal in [false, true] {
        for (case, input) in [
            (
                "conflict",
                json!({"expected_revision":99,"values":{"name":"Change"}}),
            ),
            (
                "validation",
                json!({"expected_revision":1,"values":{"name":"   "}}),
            ),
        ] {
            let name = "nyxid__crm_patch_record";
            let args = json!({"record_id":company.id,"mutation_key":format!("{universal}-{case}"),"input":input});
            let response = if universal {
                Box::pin(handle_meta_call_tool(
                    &f.state,
                    &auth,
                    None,
                    &json!({"tool_name":name,"arguments_json":args.to_string()}),
                    Some(json!(1)),
                    false,
                    crate::services::billing::route_inventory::internal_node_dispatch_permit(),
                ))
                .await
            } else {
                handle_account_tool(&f.state, &auth, name, &args, Some(json!(1))).await
            };
            let body = decoded(response).await;
            if case == "conflict" {
                assert_eq!(body["error_code"], 13400, "universal={universal}: {body}");
                assert_eq!(body["current"]["id"], company.id);
                assert_eq!(body["current"]["revision"], 1);
            } else {
                assert_eq!(body["error_code"], 13401, "universal={universal}: {body}");
                assert_eq!(body["errors"][0]["path"], "values.name");
            }
        }
        let name = "nyxid__crm_query";
        let args = json!({"record_id":company.id,"input":{"object":"company"}});
        let response = if universal {
            Box::pin(handle_meta_call_tool(
                &f.state,
                &auth,
                None,
                &json!({"tool_name":name,"arguments":args}),
                Some(json!(2)),
                false,
                crate::services::billing::route_inventory::internal_node_dispatch_permit(),
            ))
            .await
        } else {
            handle_account_tool(&f.state, &auth, name, &args, Some(json!(2))).await
        };
        assert_eq!(decoded(response).await["error_code"], 13401);
    }
    let contact = records::create(
        &f.state.db,
        &f.owner,
        CrmActor::human(&f.owner),
        records::CreateRecordInput {
            object: "contact".into(),
            values: serde_json::from_value(json!({"name":"Contact","company":company.id})).unwrap(),
            protect: vec![],
        },
        None,
    )
    .await
    .unwrap();
    f.state.db.collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(doc! {"_id":&f.chat.agent_id},doc! {"$set":{"grants.crm":[{"owner_id":&f.owner,"object":"contact","capabilities":["read","write"]}]}}).await.unwrap();
    for universal in [false, true] {
        for (name, args, code) in [
            (
                "nyxid__crm_patch_record",
                json!({"record_id":contact.id,"mutation_key":format!("redacted-{universal}"),"input":{"expected_revision":99,"values":{"name":"Change"}}}),
                13400,
            ),
            (
                "nyxid__crm_query",
                json!({"input":{"object":"contact","filters":[{"field":"company","op":"eq","value":company.id}]}}),
                1003,
            ),
        ] {
            let response = if universal {
                Box::pin(handle_meta_call_tool(
                    &f.state,
                    &auth,
                    None,
                    &json!({"tool_name":name,"arguments_json":args.to_string()}),
                    Some(json!(3)),
                    false,
                    crate::services::billing::route_inventory::internal_node_dispatch_permit(),
                ))
                .await
            } else {
                handle_account_tool(&f.state, &auth, name, &args, Some(json!(3))).await
            };
            let body = decoded(response).await;
            if code == 13400 {
                assert_eq!(body["error_code"], 13400);
                assert_eq!(body["current"]["links"]["company"], json!([]));
                assert!(!body.to_string().contains(&company.id));
            } else {
                assert_eq!(body["error"], "forbidden");
            }
        }
    }
    for name in crm::tools::NAMES {
        assert!(native_activity_names().contains(&format!("nyxid__{name}")));
    }
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::crm_record::COLLECTION_NAME)
            .find_one(doc! {"_id":&company.id})
            .await
            .unwrap()
            .unwrap()
            .get_i64("revision")
            .unwrap(),
        1
    );
    f.state.db.drop().await.unwrap();
}
