use std::sync::LazyLock;

use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Value, json};

pub const ASSISTANT_ACTIONS_SCHEMA_VERSION: u32 = 4;
pub const ASSISTANT_ACTIONS_REVISION: &str = "nyxid-assistant-actions.v7";

const SERVICE_CONNECT_DESCRIPTION: &str = "Ask the user's browser to connect a service through NyxID. Use when a task needs a catalog service (by slug) or a custom HTTPS endpoint that the user has not connected yet. NyxID owns the entire journey - auth modality, consent copy, and credential storage - and reports back only completion or decline with a safe resource reference. Never ask the user for keys, tokens, or passwords in chat.";
const KEY_CREATE_DESCRIPTION: &str = "Ask the user's browser to create a scoped NyxID API key for the named platform and allowed services. Use when the user wants a new agent identity bounded to specific user-service IDs. NyxID owns key creation and one-time key display, and reports only a safe key reference. Never request, expose, or repeat key material in chat.";
const KEY_ROTATE_DESCRIPTION: &str = "Ask the user's browser to rotate one exact NyxID API key. Use when the user needs a replacement credential for the identified key. NyxID commits an authoritative predecessor-successor relation, displays replacement key material once in the browser, and reports only the replacement key reference. Never request, expose, or repeat key material in chat.";

#[derive(Serialize)]
struct AssistantActionsManifest {
    schema_version: u32,
    revision: &'static str,
    actions: Vec<AssistantActionDescriptor>,
}

#[derive(Serialize)]
struct AssistantActionDescriptor {
    action: &'static str,
    description: &'static str,
    params_schema: Value,
    risk: &'static str,
    tier: &'static str,
    remember_eligible: bool,
}

// This static metadata intentionally has no service or model layer, mirroring llms_txt.
static MANIFEST_BODY: LazyLock<String> = LazyLock::new(|| {
    let manifest = AssistantActionsManifest {
        schema_version: ASSISTANT_ACTIONS_SCHEMA_VERSION,
        revision: ASSISTANT_ACTIONS_REVISION,
        actions: vec![
            AssistantActionDescriptor {
                action: "service.connect",
                description: SERVICE_CONNECT_DESCRIPTION,
                params_schema: json!({
                    "oneOf": [
                        {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["catalogService"],
                            "properties": {
                                "catalogService": {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "required": ["serviceSlug"],
                                    "properties": {
                                        "serviceSlug": { "type": "string" },
                                        "requestedScopes": {
                                            "type": "array",
                                            "items": { "type": "string" }
                                        },
                                        "viaNodeId": { "type": "string" },
                                        "targetOrgId": { "type": "string" }
                                    }
                                }
                            }
                        },
                        {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["customService"],
                            "properties": {
                                "customService": {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "required": ["name", "endpointUrl", "authMethod"],
                                    "properties": {
                                        "name": { "type": "string" },
                                        "endpointUrl": { "type": "string" },
                                        "authMethod": { "type": "string" },
                                        "authKeyName": { "type": "string" },
                                        "viaNodeId": { "type": "string" },
                                        "targetOrgId": { "type": "string" }
                                    }
                                }
                            }
                        }
                    ]
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: true,
            },
            AssistantActionDescriptor {
                action: "key.create",
                description: KEY_CREATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name", "platform", "allowedServiceIds"],
                    "properties": {
                        "name": { "type": "string" },
                        "platform": { "type": "string" },
                        "allowedServiceIds": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 64,
                            "uniqueItems": true,
                            "items": { "type": "string" }
                        }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "key.rotate",
                description: KEY_ROTATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["keyId"],
                    "properties": {
                        "keyId": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
        ],
    };

    serde_json::to_string(&manifest).expect("assistant actions manifest must serialize")
});

pub fn manifest_body() -> &'static str {
    MANIFEST_BODY.as_str()
}

/// GET /api/v1/assistant/actions
pub async fn get_assistant_actions() -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        manifest_body(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::{ASSISTANT_ACTIONS_REVISION, ASSISTANT_ACTIONS_SCHEMA_VERSION, manifest_body};

    const MAXIMUM_REGISTRY_BYTES: usize = 1_048_576;
    const SUPPORTED_ACTIONS: &[&str] = &[
        "service.connect",
        "service.reauthorize",
        "provider.set_app_credentials",
        "key.create",
        "key.rotate",
        "node.register_token",
        "node.rotate_token",
        "node.inject_credential",
        "service_account.create",
        "service_account.rotate_secret",
        "developer_app.create",
        "developer_app.rotate_secret",
        "account.mfa_setup",
        "device.onboard",
    ];
    const FORBIDDEN_SECRET_NAMES: &[&str] = &[
        "token",
        "tokens",
        "accesstoken",
        "refreshtoken",
        "authorization",
        "cookie",
        "cookies",
        "secret",
        "secrets",
        "clientsecret",
        "password",
        "passphrase",
        "usercode",
        "devicecode",
        "rawbody",
        "rawupstreambody",
        "credential",
        "credentials",
    ];

    fn service_connect_params_schema() -> Value {
        json!({
            "oneOf": [
                {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["catalogService"],
                    "properties": {
                        "catalogService": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["serviceSlug"],
                            "properties": {
                                "serviceSlug": { "type": "string" },
                                "requestedScopes": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                },
                                "viaNodeId": { "type": "string" },
                                "targetOrgId": { "type": "string" }
                            }
                        }
                    }
                },
                {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["customService"],
                    "properties": {
                        "customService": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["name", "endpointUrl", "authMethod"],
                            "properties": {
                                "name": { "type": "string" },
                                "endpointUrl": { "type": "string" },
                                "authMethod": { "type": "string" },
                                "authKeyName": { "type": "string" },
                                "viaNodeId": { "type": "string" },
                                "targetOrgId": { "type": "string" }
                            }
                        }
                    }
                }
            ]
        })
    }

    fn key_create_params_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["name", "platform", "allowedServiceIds"],
            "properties": {
                "name": { "type": "string" },
                "platform": { "type": "string" },
                "allowedServiceIds": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 64,
                    "uniqueItems": true,
                    "items": { "type": "string" }
                }
            }
        })
    }

    fn key_rotate_params_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["keyId"],
            "properties": {
                "keyId": { "type": "string" }
            }
        })
    }

    fn golden_manifest() -> Value {
        json!({
            "schema_version": 4,
            "revision": "nyxid-assistant-actions.v7",
            "actions": [
                {
                    "action": "service.connect",
                    "description": super::SERVICE_CONNECT_DESCRIPTION,
                    "params_schema": service_connect_params_schema(),
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": true
                },
                {
                    "action": "key.create",
                    "description": super::KEY_CREATE_DESCRIPTION,
                    "params_schema": key_create_params_schema(),
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "key.rotate",
                    "description": super::KEY_ROTATE_DESCRIPTION,
                    "params_schema": key_rotate_params_schema(),
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                }
            ]
        })
    }

    fn normalize_secret_name(name: &str) -> String {
        name.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|character| character.to_ascii_lowercase())
            .collect()
    }

    fn validate_schema_node(node: &Value) {
        let object = node.as_object().expect("schema node must be an object");

        if let Some(one_of) = object.get("oneOf") {
            let branches = one_of.as_array().expect("oneOf must be an array");
            assert!(!branches.is_empty(), "oneOf must not be empty");
            for branch in branches {
                validate_schema_node(branch);
            }
            return;
        }

        let schema_type = object
            .get("type")
            .and_then(Value::as_str)
            .expect("schema type must be a string");
        assert!(schema_type.chars().count() <= 32, "schema type is too long");

        match schema_type {
            "object" => {
                assert_eq!(
                    object.get("additionalProperties"),
                    Some(&Value::Bool(false)),
                    "object schemas must reject additional properties"
                );
                let properties = object
                    .get("properties")
                    .and_then(Value::as_object)
                    .expect("object schemas must have a properties object");
                if let Some(required) = object.get("required") {
                    for name in required.as_array().expect("required must be an array") {
                        let name = name.as_str().expect("required names must be strings");
                        assert!(
                            properties.contains_key(name),
                            "required property is not declared: {name}"
                        );
                    }
                }
                for (name, property_schema) in properties {
                    let normalized = normalize_secret_name(name);
                    assert!(
                        !FORBIDDEN_SECRET_NAMES.contains(&normalized.as_str()),
                        "forbidden secret-like property name: {name}"
                    );
                    validate_schema_node(property_schema);
                }
            }
            "array" => {
                if let Some(min_items) = object.get("minItems") {
                    assert!(min_items.as_u64().is_some(), "minItems must be an integer");
                }
                if let Some(max_items) = object.get("maxItems") {
                    assert!(max_items.as_u64().is_some(), "maxItems must be an integer");
                }
                if let Some(unique_items) = object.get("uniqueItems") {
                    assert!(
                        unique_items.as_bool().is_some(),
                        "uniqueItems must be a boolean"
                    );
                }
                validate_schema_node(
                    object
                        .get("items")
                        .expect("array schemas must have an items schema"),
                );
            }
            "string" => {}
            other => panic!("unsupported schema type: {other}"),
        }
    }

    fn assert_manifest_conforms(body: &str) {
        assert!(body.len() <= MAXIMUM_REGISTRY_BYTES);

        let manifest: Value = serde_json::from_str(body).expect("manifest must be valid JSON");
        let root = manifest
            .as_object()
            .expect("manifest root must be an object");
        assert_eq!(
            root.get("schema_version").and_then(Value::as_u64),
            Some(u64::from(ASSISTANT_ACTIONS_SCHEMA_VERSION))
        );
        let revision = root
            .get("revision")
            .and_then(Value::as_str)
            .expect("revision must be a string");
        assert!(revision.chars().count() <= 128);
        assert_eq!(revision, ASSISTANT_ACTIONS_REVISION);

        let actions = root
            .get("actions")
            .and_then(Value::as_array)
            .expect("actions must be an array");
        let mut seen_actions = HashSet::new();

        for entry in actions {
            let entry = entry.as_object().expect("action entry must be an object");
            let action = entry
                .get("action")
                .and_then(Value::as_str)
                .expect("action must be a string");
            assert!(action.chars().count() <= 128);
            assert!(SUPPORTED_ACTIONS.contains(&action));
            assert!(seen_actions.insert(action), "duplicate action: {action}");

            let description = entry
                .get("description")
                .and_then(Value::as_str)
                .expect("description must be a string");
            assert!(!description.trim().is_empty());
            assert!(description.chars().count() <= 2048);
            assert!(!description.chars().any(char::is_control));

            let params_schema = entry
                .get("params_schema")
                .expect("params_schema is required");
            assert!(params_schema.is_object());
            validate_schema_node(params_schema);

            let risk = entry
                .get("risk")
                .and_then(Value::as_str)
                .expect("risk must be a string");
            assert!(["low", "grant", "destructive"].contains(&risk));
            assert_eq!(
                entry.get("tier").and_then(Value::as_str),
                Some("v1"),
                "tier must be v1"
            );
            let remember_eligible = entry
                .get("remember_eligible")
                .and_then(Value::as_bool)
                .expect("remember_eligible must be a boolean");
            if risk == "destructive" {
                assert!(!remember_eligible);
            }
        }

        assert!(seen_actions.contains("service.connect"));
        assert!(seen_actions.contains("key.create"));
        assert!(seen_actions.contains("key.rotate"));
    }

    #[test]
    fn assistant_actions_manifest_matches_golden_payload() {
        let manifest: Value = serde_json::from_str(manifest_body()).unwrap();
        let actions = manifest["actions"].as_array().unwrap();

        assert_eq!(manifest["schema_version"], 4);
        assert_eq!(manifest["revision"], "nyxid-assistant-actions.v7");
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0]["action"], "service.connect");
        assert_eq!(actions[0]["risk"], "grant");
        assert_eq!(actions[0]["tier"], "v1");
        assert_eq!(actions[0]["remember_eligible"], true);
        assert_eq!(actions[0]["params_schema"], service_connect_params_schema());
        assert_eq!(actions[1]["action"], "key.create");
        assert_eq!(actions[1]["risk"], "grant");
        assert_eq!(actions[1]["tier"], "v1");
        assert_eq!(actions[1]["remember_eligible"], false);
        assert_eq!(actions[1]["params_schema"], key_create_params_schema());
        assert_eq!(
            actions[1]["params_schema"]["properties"]["allowedServiceIds"]["minItems"],
            1
        );
        assert_eq!(
            actions[1]["params_schema"]["properties"]["allowedServiceIds"]["uniqueItems"],
            true
        );
        assert_eq!(actions[2]["action"], "key.rotate");
        assert_eq!(actions[2]["risk"], "grant");
        assert_eq!(actions[2]["tier"], "v1");
        assert_eq!(actions[2]["remember_eligible"], false);
        assert_eq!(actions[2]["params_schema"], key_rotate_params_schema());
        assert_eq!(manifest, golden_manifest());
    }

    #[test]
    fn assistant_actions_manifest_conforms_to_aevatar_parser_contract() {
        assert_manifest_conforms(manifest_body());
    }

    #[test]
    fn secret_name_normalization_is_ascii_alphanumeric_only() {
        assert_eq!(normalize_secret_name("Client-Secret"), "clientsecret");
        assert!(FORBIDDEN_SECRET_NAMES.contains(&normalize_secret_name("client_secret").as_str()));
        assert_eq!(normalize_secret_name("t\u{00f6}ken"), "tken");
    }

    #[tokio::test]
    async fn assistant_actions_route_is_public_json_and_matches_static_body() {
        let state = crate::test_utils::test_app_state_no_db().await;
        let (_, private_api) = crate::routes::build_router(
            1024 * 1024,
            crate::services::anonymous_endpoint_service::DEFAULT_PUBLIC_PROXY_MAX_BODY_SIZE,
        );

        let response = private_api
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/assistant/actions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
        let body = to_bytes(response.into_body(), MAXIMUM_REGISTRY_BYTES)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), manifest_body().as_bytes());
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            golden_manifest()
        );
    }
}
