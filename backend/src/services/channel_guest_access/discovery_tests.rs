use super::{
    AnswerAudience, policy,
    test_support::{mcp_request, start_guest_with_audience},
};
use crate::models::channel_guest_policy::GuestDataPolicy;
use bson::{Document, doc};
use serde_json::{Value, json};

#[tokio::test]
async fn enrolled_sheets_discovery_guides_both_catalogs_and_preserves_ordinary_tools() {
    use crate::{
        models::{
            assistant_agent::{AgentGrants, GuestAccess},
            channel_bot,
            feature_flag_override::FlagTargetKind,
        },
        services::{
            assistant_agent_credential_service as credentials,
            assistant_authority_tests::{connected, direct_fixture},
            assistant_team_service as team,
            channel_guest_access::{self as guest, google_sheets},
            feature_flag_admin_service as flags,
        },
    };

    fn tool<'a>(list: &'a Value, name: &str) -> &'a Value {
        list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing {name}: {list}"))
    }

    async fn search(router: &axum::Router, key: &str, name: &str, compact: bool) -> Value {
        let result = mcp_request(
            router,
            key,
            "tools/call",
            json!({
                "name":"nyx__search_tools", "arguments":{"query":name}
            }),
        )
        .await;
        assert_eq!(result["isError"], false, "{result}");
        assert!(result.to_string().len() <= 10_000);
        let search: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(search["matches"][0]["name"], name, "{search}");
        assert_eq!(
            search["matches"][0]["input_schema_omitted"] == true,
            compact,
            "{search}"
        );
        for hidden in ["private_workbook_2704", "Hidden_Config", "Hidden_Questions"] {
            assert!(!search.to_string().contains(hidden));
        }
        search["matches"][0].clone()
    }

    let f = Box::pin(direct_fixture("enrolled_sheets_discovery")).await;
    let db = &f.state.db;
    let mut connections = Vec::new();
    let mut rules = Vec::new();
    for slug in ["api-google-workspace", "api-google-sheets"] {
        let connection = connected(db, &f.owner, slug, "https://sheets.googleapis.com").await;
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        catalog.slug = slug.into();
        catalog.base_url = "https://sheets.googleapis.com".into();
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
        db.collection::<Document>(crate::models::user_service::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&connection},
                doc! {"$set":{"catalog_service_id":&catalog.id}},
            )
            .await
            .unwrap();
        let spec = crate::services::catalog_spec_registry::spec_for_slug(slug).unwrap();
        for (method, path) in [
            ("get", "/v4/spreadsheets/{spreadsheetId}/values/{range}"),
            (
                "post",
                "/v4/spreadsheets/{spreadsheetId}/values/{range}:append",
            ),
            ("put", "/v4/spreadsheets/{spreadsheetId}/values/{range}"),
        ] {
            let operation = &spec["paths"][path][method];
            let mut endpoint = crate::test_utils::test_service_endpoint(
                &catalog.id,
                operation["operationId"].as_str().unwrap(),
                &method.to_ascii_uppercase(),
                path,
            );
            endpoint.description = Some(
                operation["x-aevatar-tool"]["description"]
                    .as_str()
                    .unwrap()
                    .into(),
            );
            let mut parameters = operation["parameters"].clone();
            // Force the real bounded-search path to omit even a single schema.
            parameters[0]["description"] =
                json!("Synthetic spreadsheet identifier help. ".repeat(500));
            endpoint.parameters = Some(parameters);
            if method != "get" {
                endpoint.request_body_schema =
                    Some(operation["requestBody"]["content"]["application/json"]["schema"].clone());
                endpoint.request_content_type = Some("application/json".into());
                endpoint.request_body_required = true;
            }
            db.collection::<crate::models::service_endpoint::ServiceEndpoint>(
                crate::models::service_endpoint::COLLECTION_NAME,
            )
            .insert_one(endpoint)
            .await
            .unwrap();
        }
        for (action, selector) in [
            ("read", "Hidden_Config!B2:D5"),
            ("create", "Hidden_Questions"),
        ] {
            rules.push(json!({
                "source_id":"google_sheets", "collection_id":"values",
                "audience":{"kind":"all_guests"}, "delivery":"shared",
                "scope":{"kind":"resources","record_ids":[format!("{connection}/private_workbook_2704/{selector}")]},
                "operations":{action:{}}
            }));
        }
        connections.push(connection);
    }
    team::set_grants(
        db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(AgentGrants {
            service_ids: connections.clone(),
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    let (_, router) = crate::routes::build_router_with_state(f.state.clone());
    let router = router.with_state(f.state.clone());
    let owner_key =
        credentials::load_for_conversation(db, &f.state.encryption_keys, &f.owner, &f.row.id)
            .await
            .unwrap()
            .unwrap();
    // This personal specialist provides ordinary owner and non-enrolled guest
    // controls; it does not establish organization membership or live acceptance.
    for ordinary_guest in [false, true] {
        db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.row.id},
                doc! {"$set":{"guest_turn":ordinary_guest}},
            )
            .await
            .unwrap();
        let list = mcp_request(&router, owner_key.raw_key.as_str(), "tools/list", json!({})).await;
        for slug in ["api-google-workspace", "api-google-sheets"] {
            let name = format!("{slug}__sheets_append_values");
            let append = tool(&list, &name);
            let properties = &append["inputSchema"]["properties"];
            assert_eq!(
                properties["valueInputOption"]["enum"],
                json!(["RAW", "USER_ENTERED"])
            );
            assert_eq!(
                properties["insertDataOption"]["enum"],
                json!(["OVERWRITE", "INSERT_ROWS"])
            );
            assert!(properties["body"]["properties"]["range"].is_object());
            assert!(
                properties["body"]["properties"]["values"]
                    .get("maxItems")
                    .is_none()
            );
            assert!(
                !append["description"]
                    .as_str()
                    .unwrap()
                    .contains("Channel guest append")
            );
            let found = search(&router, owner_key.raw_key.as_str(), &name, true).await;
            assert!(
                !found["description"]
                    .as_str()
                    .unwrap()
                    .contains("Channel guest append")
            );
            let read = tool(&list, &format!("{slug}__sheets_get_values"));
            assert_eq!(
                read["inputSchema"]["properties"]["valueRenderOption"]["enum"],
                json!(["FORMATTED_VALUE", "UNFORMATTED_VALUE", "FORMULA"])
            );
        }
    }

    guest::ensure_indexes(db).await.unwrap();
    db.collection::<Document>(channel_bot::COLLECTION_NAME)
        .insert_one(doc! {"_id":"bot-1","is_active":true,"guest_access_v1":true,"guest_access_v1_revision":1_i64})
        .await.unwrap();
    let actor = crate::services::audit_service::AuditActor::from_auth_user(&f.auth);
    guest::set_pilot(
        db,
        guest::PilotSelection {
            bot_ids: None,
            fleet: true,
        },
        &actor,
    )
    .await
    .unwrap();
    flags::mutate(
        db,
        guest::FLAG,
        flags::Mutation::Override {
            kind: FlagTargetKind::Global,
            target_id: None,
            enabled: Some(true),
        },
        &actor,
    )
    .await
    .unwrap();
    let policy: GuestDataPolicy = serde_json::from_value(json!({
        "_id":"bot-1", "bot_id":"bot-1", "revision":1, "rules":rules
    }))
    .unwrap();
    policy::validate_rules(&policy.rules).unwrap();
    db.collection::<GuestDataPolicy>(crate::models::channel_guest_policy::COLLECTION_NAME)
        .insert_one(policy)
        .await
        .unwrap();

    for audience in [AnswerAudience::Shared, AnswerAudience::Private] {
        let source = if audience == AnswerAudience::Shared {
            "shared"
        } else {
            "private"
        };
        let chat = Box::pin(start_guest_with_audience(&f, source, source, audience)).await;
        let key = credentials::load_for_conversation(
            db,
            &f.state.encryption_keys,
            &f.owner,
            &chat.conversation_id,
        )
        .await
        .unwrap()
        .unwrap();
        let list = mcp_request(&router, key.raw_key.as_str(), "tools/list", json!({})).await;
        for slug in ["api-google-workspace", "api-google-sheets"] {
            let read_name = format!("{slug}__sheets_get_values");
            let read = tool(&list, &read_name);
            let properties = &read["inputSchema"]["properties"];
            assert_eq!(
                properties["valueRenderOption"]["enum"],
                json!(["FORMATTED_VALUE", "UNFORMATTED_VALUE"])
            );
            assert!(
                properties["range"]["description"]
                    .as_str()
                    .unwrap()
                    .contains("named tab")
            );
            let found = search(&router, key.raw_key.as_str(), &read_name, true).await;
            for guidance in [
                "bounded A1",
                "named tab",
                "32 columns",
                "2000 cells",
                "approval policy",
                "discovery is not approval",
            ] {
                assert!(
                    found["description"].as_str().unwrap().contains(guidance),
                    "{found}"
                );
            }
            assert!(
                !list["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["name"] == format!("{slug}__sheets_update_values"))
            );
            let append_name = format!("{slug}__sheets_append_values");
            if audience == AnswerAudience::Private {
                assert!(
                    !list["tools"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|tool| tool["name"] == append_name)
                );
                continue;
            }
            let append = tool(&list, &append_name);
            let schema = &append["inputSchema"];
            let properties = &schema["properties"];
            assert_eq!(properties["valueInputOption"]["enum"], json!(["RAW"]));
            assert_eq!(
                properties["insertDataOption"]["enum"],
                json!(["INSERT_ROWS"])
            );
            assert_eq!(
                properties["includeValuesInResponse"]["enum"],
                json!([false])
            );
            assert!(properties.get("responseValueRenderOption").is_none());
            assert!(properties.get("responseDateTimeRenderOption").is_none());
            for name in ["valueInputOption", "insertDataOption", "body"] {
                assert!(
                    schema["required"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(name)),
                    "{schema}"
                );
            }
            let body = &properties["body"];
            assert_eq!(body["additionalProperties"], false);
            assert!(body["properties"].get("range").is_none());
            assert_eq!(
                body["properties"]["majorDimension"]["enum"],
                json!(["ROWS"])
            );
            let values = &body["properties"]["values"];
            assert_eq!(values["minItems"], 1);
            assert_eq!(values["maxItems"], 1);
            assert_eq!(values["items"]["maxItems"], google_sheets::MAX_COLUMNS);
            assert_eq!(
                values["items"]["items"]["anyOf"],
                json!([{"type":"string"},{"type":"number"},{"type":"boolean"}])
            );
            let guidance = values["description"].as_str().unwrap();
            assert!(guidance.contains(&google_sheets::MAX_APPEND_BYTES.to_string()));
            assert!(guidance.contains("no more cells than the selected column range"));
            let found = search(&router, key.raw_key.as_str(), &append_name, true).await;
            for guidance in [
                "valueInputOption=RAW",
                "insertDataOption=INSERT_ROWS",
                "once each",
                "shared room",
                "named tab",
                "column range",
                "exactly one nonempty row",
                "approval policy",
                "discovery is not approval",
            ] {
                assert!(
                    found["description"].as_str().unwrap().contains(guidance),
                    "{found}"
                );
            }
            assert_eq!(found["chat_access"], "granted");
        }
        for hidden in ["private_workbook_2704", "Hidden_Config", "Hidden_Questions"] {
            assert!(!list.to_string().contains(hidden));
        }
    }
    // The v1 contract still applies at `all`. With normal-sized schemas, search
    // publishes the same projected schema as list, including the body wrapper.
    team::set_grants(
        db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Guests(
            connections
                .into_iter()
                .map(|id| (id, GuestAccess::All))
                .collect(),
        ),
    )
    .await
    .unwrap();
    db.collection::<Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .update_many(doc! {}, doc! {"$unset":{"parameters.0.description":""}})
        .await
        .unwrap();
    let chat = Box::pin(start_guest_with_audience(
        &f,
        "shared-all",
        "shared-all",
        AnswerAudience::Shared,
    ))
    .await;
    let key = credentials::load_for_conversation(
        db,
        &f.state.encryption_keys,
        &f.owner,
        &chat.conversation_id,
    )
    .await
    .unwrap()
    .unwrap();
    let list = mcp_request(&router, key.raw_key.as_str(), "tools/list", json!({})).await;
    for slug in ["api-google-workspace", "api-google-sheets"] {
        for operation in ["sheets_get_values", "sheets_append_values"] {
            let name = format!("{slug}__{operation}");
            let found = search(&router, key.raw_key.as_str(), &name, false).await;
            assert_eq!(found["inputSchema"], tool(&list, &name)["inputSchema"]);
            if operation == "sheets_append_values" {
                assert_eq!(
                    found["inputSchema"]["properties"]["valueInputOption"]["enum"],
                    json!(["RAW"])
                );
                assert_eq!(
                    found["inputSchema"]["properties"]["insertDataOption"]["enum"],
                    json!(["INSERT_ROWS"])
                );
                assert!(
                    found["description"]
                        .as_str()
                        .unwrap()
                        .contains("discovery is not approval")
                );
            }
        }
    }
    db.drop().await.unwrap();
}
