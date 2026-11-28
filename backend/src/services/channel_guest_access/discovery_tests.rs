use super::{
    AnswerAudience, policy,
    test_support::{
        legacy_shared_evidence, mcp_request, seeded_tools_list, start_guest_with_audience,
        start_guest_with_evidence,
    },
};
use crate::{
    models::channel_guest_policy::GuestDataPolicy, services::channel_guest_access::google_sheets,
};
use bson::{Document, doc};
use serde_json::{Value, json};

fn catalog_token(slug: &str) -> &'static str {
    match slug {
        "api-google-workspace" => "ws6workspace",
        "api-google-sheets" => "ws6sheets",
        _ => panic!("unexpected fixture catalog {slug}"),
    }
}

fn catalog_label(slug: &str) -> String {
    let token = catalog_token(slug);
    format!("{token} {}", "L".repeat(199 - token.len()))
}

fn tool<'a>(list: &'a Value, name: &str) -> &'a Value {
    list["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == name)
        .unwrap_or_else(|| panic!("missing {name}: {list}"))
}

async fn search(router: &axum::Router, key: &str, name: &str, compact: bool) -> Value {
    // Qualified Google tool names share search terms. Use the connection label
    // and operation to distinguish both catalogs before bounded search compacts
    // the best match; partial matches can still appear in the full response.
    let (slug, operation) = name.split_once("__").unwrap();
    let query = format!("{} {operation}", catalog_token(slug));
    let result = mcp_request(
        router,
        key,
        "tools/call",
        json!({
            "name":"nyx__search_tools", "arguments":{"query":&query}
        }),
    )
    .await;
    assert_eq!(result["isError"], false, "{result}");
    assert!(result.to_string().len() <= 10_000);
    let search: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        search["matches"][0]["name"], name,
        "query {query}: {search}"
    );
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

fn assert_read_schema(read: &Value) {
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
}

fn assert_append_schema(append: &Value) {
    let schema = &append["inputSchema"];
    assert_eq!(schema["additionalProperties"], false);
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
}

fn assert_guidance(tool: &Value, operation: &str) {
    let description = tool["description"].as_str().unwrap();
    let required: &[&str] = if operation == "sheets_get_values" {
        &[
            "exact authorized A1",
            "named tab",
            "32 columns",
            "2000 cells",
            "effective approval policy must permit the read",
        ]
    } else {
        &[
            "valueInputOption=RAW",
            "insertDataOption=INSERT_ROWS",
            "once each",
            "shared room",
            "authorized named tab",
            "column range",
            "32 columns",
            "exactly one nonempty row",
            "body.values",
            "effective approval policy that permits it",
        ]
    };
    for guidance in required
        .iter()
        .copied()
        .chain(["live grants", "Google access"])
    {
        assert!(
            description
                .to_ascii_lowercase()
                .contains(&guidance.to_ascii_lowercase()),
            "{tool}"
        );
    }
    assert!(
        description.starts_with(if operation == "sheets_get_values" {
            "Read the exact"
        } else {
            "Append exactly"
        })
    );
}

async fn assert_effective_approval(db: &mongodb::Database, owner: &str, catalogs: &[String]) {
    use crate::{
        models::service_approval_config::ApprovalEffect,
        services::{approval_service, operation_descriptor},
    };
    for catalog in catalogs {
        for (method, path, effect) in [
            (
                "GET",
                "/v4/spreadsheets/private_workbook_2704/values/Hidden_Config%21B2%3AD5",
                ApprovalEffect::AutoAllow,
            ),
            (
                "POST",
                "/v4/spreadsheets/private_workbook_2704/values/Hidden_Questions%21A%3AD:append",
                ApprovalEffect::RequireApproval,
            ),
        ] {
            let operation = operation_descriptor::build_http_descriptor(method, path, None);
            let decision = approval_service::resolve_org_aware_approval(
                db, owner, owner, catalog, &operation, false,
            )
            .await
            .unwrap();
            assert_eq!(decision.required, effect == ApprovalEffect::RequireApproval);
            assert_eq!(decision.effect, effect);
        }
    }
}

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
            assistant_authority_tests::direct_fixture,
            assistant_team_service as team, channel_guest_access as guest,
            destination_routing::{self, tests as google},
            feature_flag_admin_service as flags,
        },
    };

    let f = Box::pin(direct_fixture("enrolled_sheets_discovery")).await;
    let db = &f.state.db;
    google::seed(db).await;
    let mut connections = Vec::new();
    let mut catalog_ids = Vec::new();
    let mut oversized_endpoint_ids = Vec::new();
    let mut rules = Vec::new();
    for slug in ["api-google-workspace", "api-google-sheets"] {
        let long_label = catalog_label(slug);
        assert_eq!(long_label.chars().count(), 200);
        let connection = google::connect(db, &f.owner, slug).await;
        let catalog_id = connection.catalog_service_id.as_deref().unwrap();
        let catalog = db
            .collection::<crate::models::downstream_service::DownstreamService>(
                crate::models::downstream_service::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":catalog_id})
            .await
            .unwrap()
            .unwrap();
        assert!(!destination_routing::workspace_destinations_pending(
            &catalog
        ));
        if slug == "api-google-workspace" {
            assert_eq!(
                catalog.destination_targets["sheets"],
                "https://sheets.googleapis.com"
            );
        }
        db.collection::<Document>(crate::models::user_endpoint::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&connection.endpoint_id},
                doc! {"$set":{"label":&long_label}},
            )
            .await
            .unwrap();
        let endpoints = crate::services::service_endpoint_service::list_endpoints(db, catalog_id)
            .await
            .unwrap();
        for name in ["sheets_get_values", "sheets_append_values"] {
            let endpoint = endpoints
                .iter()
                .find(|endpoint| endpoint.name == name)
                .unwrap();
            assert_eq!(
                endpoint.target_id.as_deref(),
                (slug == "api-google-workspace").then_some("sheets")
            );
            assert_eq!(
                endpoint.parameters.as_ref().unwrap()[0]["name"],
                "spreadsheetId"
            );
            // Force schema omission only for the two catalogs' target operations.
            db.collection::<Document>(crate::models::service_endpoint::COLLECTION_NAME)
                .update_one(doc! {"_id":&endpoint.id,"service_id":catalog_id},
                    doc! {"$set":{"parameters.0.description":"Synthetic spreadsheet identifier help. ".repeat(500)}})
                .await.unwrap();
            oversized_endpoint_ids.push(endpoint.id.clone());
        }
        catalog_ids.push(catalog_id.to_owned());
        for (action, selector) in [
            ("read", "Hidden_Config!B2:D5"),
            ("create", "Hidden_Questions"),
        ] {
            rules.push(json!({
                "source_id":"google_sheets", "collection_id":"values",
                "audience":{"kind":"all_guests"}, "delivery":"shared",
                "scope":{"kind":"resources","record_ids":[format!("{}/private_workbook_2704/{selector}", connection.id)]},
                "operations":{action:{}}
            }));
        }
        connections.push(connection.id);
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
        let list = seeded_tools_list(&router, owner_key.raw_key.as_str()).await;
        for slug in ["api-google-workspace", "api-google-sheets"] {
            let long_label = catalog_label(slug);
            let name = format!("{slug}__sheets_append_values");
            let append = tool(&list, &name);
            assert!(append["inputSchema"].get("additionalProperties").is_none());
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
                append["description"]
                    .as_str()
                    .unwrap()
                    .starts_with(&format!("[{long_label}] "))
            );
            let found = search(&router, owner_key.raw_key.as_str(), &name, true).await;
            assert!(
                found["description"]
                    .as_str()
                    .unwrap()
                    .starts_with(&format!("[{long_label}] "))
            );
            let read = tool(&list, &format!("{slug}__sheets_get_values"));
            assert_eq!(
                read["inputSchema"]["properties"]["valueRenderOption"]["enum"],
                json!(["FORMATTED_VALUE", "UNFORMATTED_VALUE", "FORMULA"])
            );
        }
    }

    // Discovery remains available while the effective default requires approval
    // for writes. The fixture performs no provider operation or approval request.
    db.collection::<Document>(crate::models::notification_channel::COLLECTION_NAME)
        .update_one(
            doc! {"user_id":&f.owner},
            doc! {"$unset":{"approval_required":""}},
        )
        .await
        .unwrap();
    assert_eq!(
        crate::services::approval_service::user_approval_mode(db, &f.owner)
            .await
            .unwrap(),
        crate::services::approval_policy::GlobalApprovalMode::WritesOnly
    );
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

    for (source, audience, proof) in [
        ("shared", AnswerAudience::Shared, true),
        ("private", AnswerAudience::Private, false),
        ("ambiguous", AnswerAudience::Shared, false),
    ] {
        let chat = Box::pin(start_guest_with_evidence(
            &f, source, source, audience, proof,
        ))
        .await;
        if source == "ambiguous" {
            legacy_shared_evidence(db, &chat).await;
        }
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
            let long_label = catalog_label(slug);
            let read_name = format!("{slug}__sheets_get_values");
            let read = tool(&list, &read_name);
            assert_read_schema(read);
            let found = search(&router, key.raw_key.as_str(), &read_name, true).await;
            assert_guidance(&found, "sheets_get_values");
            assert!(
                !list["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["name"] == format!("{slug}__sheets_update_values"))
            );
            let append_name = format!("{slug}__sheets_append_values");
            if !proof {
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
            assert_append_schema(append);
            let found = search(&router, key.raw_key.as_str(), &append_name, true).await;
            assert_guidance(&found, "sheets_append_values");
            let description = append["description"].as_str().unwrap();
            assert!(description.ends_with(&format!("[{long_label}]")));
            assert!(description.chars().count() > 512);
            assert_eq!(
                found["description"],
                description.chars().take(512).collect::<String>()
            );
            assert_eq!(found["description"].as_str().unwrap().chars().count(), 512);
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
        .update_many(
            doc! {"_id":{"$in":oversized_endpoint_ids},"service_id":{"$in":&catalog_ids}},
            doc! {"$unset":{"parameters.0.description":""}},
        )
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
            assert_guidance(&found, operation);
        }
    }
    assert_effective_approval(db, &f.owner, &catalog_ids).await;
    Box::pin(super::sheets_runtime_tests::exercise(
        &f,
        &chat.conversation_id,
    ))
    .await;
    db.drop().await.unwrap();
}
