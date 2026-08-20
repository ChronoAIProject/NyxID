use std::sync::LazyLock;

use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Value, json};

pub const ASSISTANT_ACTIONS_SCHEMA_VERSION: u32 = 4;
pub const ASSISTANT_ACTIONS_REVISION: &str = "nyxid-assistant-actions.v8";

const SERVICE_CONNECT_DESCRIPTION: &str = "Ask the user's browser to connect a service through NyxID. Use when a task needs a catalog service (by slug) or a custom HTTPS endpoint that the user has not connected yet. NyxID owns the entire journey - auth modality, consent copy, and credential storage - and reports back only completion or decline with a safe resource reference. Never ask the user for keys, tokens, or passwords in chat.";
const SERVICE_REAUTHORIZE_DESCRIPTION: &str = "Ask the user's browser to re-authorize an existing connected service and review its requested scopes. Use when a task needs permissions that the referenced user service does not currently grant. NyxID owns the authorization journey and credential storage, and reports only a safe user-service reference. Never ask the user for keys, tokens, passwords, or authorization codes in chat.";
const KEY_CREATE_DESCRIPTION: &str = "Ask the user's browser to create a scoped NyxID API key for the named platform and allowed services. Use when the user wants a new agent identity bounded to specific user-service IDs. NyxID owns key creation and one-time key display, and reports only a safe key reference. Never request, expose, or repeat key material in chat.";
const KEY_ROTATE_DESCRIPTION: &str = "Ask the user's browser to rotate one exact NyxID API key. Use when the user needs a replacement credential for the identified key. NyxID commits an authoritative predecessor-successor relation, displays replacement key material once in the browser, and reports only the replacement key reference. Never request, expose, or repeat key material in chat.";

// Wave-2 descriptors below are published DORMANT: they are deliberately absent
// from every Aevatar revision pin, so every current composition skips them at
// registry load (upstream `Load_ShouldIgnoreUnknownActionWhenExecutableActionsArePresent`).
// They become executable only when a future revision pins them; until that
// revision bump they may still be amended. Do not edit the four shipped
// descriptors above - their schemas are pinned byte-for-byte by
// `JsonNode.DeepEquals` in deployed compositions.
const KEY_UPDATE_DESCRIPTION: &str = "Ask the user's browser to update the display metadata of one exact NyxID API key - its name, platform, or description. Use when the user wants to relabel or reclassify an existing agent key without changing what it can reach. NyxID owns the journey and reports only the safe key reference. Never request, expose, or repeat key material in chat.";
const KEY_DELETE_DESCRIPTION: &str = "Ask the user's browser to permanently delete one exact NyxID API key. Use when the user wants to retire an agent identity entirely. Deletion is destructive and confirmed in the browser every time; NyxID reports only the safe key reference. Never request, expose, or repeat key material in chat.";
const KEY_EXTEND_SCOPE_DESCRIPTION: &str = "Ask the user's browser to widen one exact NyxID API key's allowed services by the listed user-service IDs. Use when a task needs the key to reach a service it cannot reach today. Widening is confirmed in the browser and never remembered; NyxID reports only the safe key reference. Never request, expose, or repeat key material in chat.";
const KEY_BIND_CREDENTIAL_DESCRIPTION: &str = "Ask the user's browser to bind one exact NyxID API key to a specific stored external credential for one user service. Use when an agent key must use a dedicated credential instead of the service default. Binding is confirmed in the browser and never remembered; NyxID reports only the safe key reference. Never request, expose, or repeat credential material in chat.";
const SERVICE_UPDATE_DESCRIPTION: &str = "Ask the user's browser to update one exact connected service's configuration - display name, endpoint URL, or auth-method metadata. Use when the user wants to correct or rename an existing connected service. NyxID owns the journey and credential storage and reports only the safe user-service reference. Never ask the user for keys, tokens, or passwords in chat.";
const SERVICE_DELETE_DESCRIPTION: &str = "Ask the user's browser to permanently disconnect and delete one exact connected service. Use when the user wants to remove a service connection entirely. Deletion is destructive and confirmed in the browser every time; NyxID reports only the safe user-service reference. Never ask the user for keys, tokens, or passwords in chat.";
const SERVICE_ROUTE_DESCRIPTION: &str = "Ask the user's browser to change how one exact connected service is routed - through a named credential node, or directly when viaNodeId is omitted. Use when the user wants requests for the referenced service to run via a specific node or to clear that routing. NyxID reports only the safe user-service reference. Never ask the user for keys, tokens, or passwords in chat.";
const SERVICE_ROTATE_CREDENTIAL_DESCRIPTION: &str = "Ask the user's browser to replace the stored credential of one exact connected service. Use when the user has minted a new upstream key and wants NyxID to store it. The new credential is entered only inside NyxID's browser journey; NyxID reports only the safe user-service reference. Never ask the user for keys, tokens, or passwords in chat.";
const ENDPOINT_UPDATE_DESCRIPTION: &str = "Ask the user's browser to update one exact user endpoint - its label, target URL, or OpenAPI spec URL. Use when the user wants to correct where a custom endpoint points. NyxID reports only the safe user-service reference for the affected service. Never ask the user for keys, tokens, or passwords in chat.";
const ENDPOINT_DELETE_DESCRIPTION: &str = "Ask the user's browser to permanently delete one exact user endpoint. Use when the user wants to remove a custom endpoint definition. Deletion is destructive and confirmed in the browser every time; NyxID reports only the safe user-service reference for the affected service. Never ask the user for keys, tokens, or passwords in chat.";
const EXTERNAL_KEY_ROTATE_DESCRIPTION: &str = "Ask the user's browser to replace the secret of one exact stored external credential. Use when the user has a replacement API key for a connected provider. The replacement is entered only inside NyxID's browser journey; NyxID reports only the safe key reference. Never request, expose, or repeat credential material in chat.";
const EXTERNAL_KEY_DELETE_DESCRIPTION: &str = "Ask the user's browser to permanently delete one exact stored external credential. Use when the user wants a stored provider key removed from NyxID. Deletion is destructive, confirmed in the browser every time, and may cascade an approval-grant review inside the journey; NyxID reports only the safe key reference. Never request, expose, or repeat credential material in chat.";

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
                action: "service.reauthorize",
                description: SERVICE_REAUTHORIZE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["userServiceId", "requestedScopes"],
                    "properties": {
                        "userServiceId": { "type": "string" },
                        "requestedScopes": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
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
            // ---- Wave 2 (dormant until a future revision pins them) ----
            AssistantActionDescriptor {
                action: "key.update",
                description: KEY_UPDATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["keyId"],
                    "properties": {
                        "keyId": { "type": "string" },
                        "name": { "type": "string" },
                        "platform": { "type": "string" },
                        "description": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "key.delete",
                description: KEY_DELETE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["keyId"],
                    "properties": {
                        "keyId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "key.extend_scope",
                description: KEY_EXTEND_SCOPE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["keyId", "addServiceIds"],
                    "properties": {
                        "keyId": { "type": "string" },
                        "addServiceIds": {
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
                action: "key.bind_credential",
                description: KEY_BIND_CREDENTIAL_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["keyId", "userServiceId", "externalKeyId"],
                    "properties": {
                        "keyId": { "type": "string" },
                        "userServiceId": { "type": "string" },
                        "externalKeyId": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "service.update",
                description: SERVICE_UPDATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["userServiceId"],
                    "properties": {
                        "userServiceId": { "type": "string" },
                        "name": { "type": "string" },
                        "endpointUrl": { "type": "string" },
                        "authMethod": { "type": "string" },
                        "authKeyName": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "service.delete",
                description: SERVICE_DELETE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["userServiceId"],
                    "properties": {
                        "userServiceId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "service.route",
                description: SERVICE_ROUTE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["userServiceId"],
                    "properties": {
                        "userServiceId": { "type": "string" },
                        "viaNodeId": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "service.rotate_credential",
                description: SERVICE_ROTATE_CREDENTIAL_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["userServiceId"],
                    "properties": {
                        "userServiceId": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "endpoint.update",
                description: ENDPOINT_UPDATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["endpointId"],
                    "properties": {
                        "endpointId": { "type": "string" },
                        "label": { "type": "string" },
                        "endpointUrl": { "type": "string" },
                        "openapiSpecUrl": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "endpoint.delete",
                description: ENDPOINT_DELETE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["endpointId"],
                    "properties": {
                        "endpointId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "external_key.rotate",
                description: EXTERNAL_KEY_ROTATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["externalKeyId"],
                    "properties": {
                        "externalKeyId": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "external_key.delete",
                description: EXTERNAL_KEY_DELETE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["externalKeyId"],
                    "properties": {
                        "externalKeyId": { "type": "string" }
                    }
                }),
                risk: "destructive",
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
    /// Names the manifest may contain. The first 14 mirror the Aevatar
    /// parser's `SupportedActions` contract; the Wave-2 block extends the
    /// rail with dormant verbs the upstream loader provably skips until a
    /// future revision pins them (it validates only that `action` is a
    /// string of at most 128 chars before skipping). Names outside this
    /// list still fail the conformance test below.
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
        // Wave 2 (dormant; not pinned by any Aevatar revision yet)
        "key.update",
        "key.delete",
        "key.extend_scope",
        "key.bind_credential",
        "service.update",
        "service.delete",
        "service.route",
        "service.rotate_credential",
        "endpoint.update",
        "endpoint.delete",
        "external_key.rotate",
        "external_key.delete",
    ];

    /// The composition the deployed v8 pin executes. Everything else in the
    /// manifest is dormant and must stay non-remembered until pinned.
    const V8_PINNED_ACTIONS: &[&str] = &[
        "service.connect",
        "service.reauthorize",
        "key.create",
        "key.rotate",
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

    fn service_reauthorize_params_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["userServiceId", "requestedScopes"],
            "properties": {
                "userServiceId": { "type": "string" },
                "requestedScopes": {
                    "type": "array",
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

    fn key_update_params_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["keyId"],
            "properties": {
                "keyId": { "type": "string" },
                "name": { "type": "string" },
                "platform": { "type": "string" },
                "description": { "type": "string" }
            }
        })
    }

    fn key_delete_params_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["keyId"],
            "properties": {
                "keyId": { "type": "string" }
            }
        })
    }

    fn key_extend_scope_params_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["keyId", "addServiceIds"],
            "properties": {
                "keyId": { "type": "string" },
                "addServiceIds": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 64,
                    "uniqueItems": true,
                    "items": { "type": "string" }
                }
            }
        })
    }

    fn key_bind_credential_params_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["keyId", "userServiceId", "externalKeyId"],
            "properties": {
                "keyId": { "type": "string" },
                "userServiceId": { "type": "string" },
                "externalKeyId": { "type": "string" }
            }
        })
    }

    fn service_update_params_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["userServiceId"],
            "properties": {
                "userServiceId": { "type": "string" },
                "name": { "type": "string" },
                "endpointUrl": { "type": "string" },
                "authMethod": { "type": "string" },
                "authKeyName": { "type": "string" }
            }
        })
    }

    fn user_service_only_params_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["userServiceId"],
            "properties": {
                "userServiceId": { "type": "string" }
            }
        })
    }

    fn service_route_params_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["userServiceId"],
            "properties": {
                "userServiceId": { "type": "string" },
                "viaNodeId": { "type": "string" }
            }
        })
    }

    fn endpoint_update_params_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["endpointId"],
            "properties": {
                "endpointId": { "type": "string" },
                "label": { "type": "string" },
                "endpointUrl": { "type": "string" },
                "openapiSpecUrl": { "type": "string" }
            }
        })
    }

    fn endpoint_delete_params_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["endpointId"],
            "properties": {
                "endpointId": { "type": "string" }
            }
        })
    }

    fn external_key_only_params_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["externalKeyId"],
            "properties": {
                "externalKeyId": { "type": "string" }
            }
        })
    }

    fn golden_manifest() -> Value {
        json!({
            "schema_version": 4,
            "revision": "nyxid-assistant-actions.v8",
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
                    "action": "service.reauthorize",
                    "description": super::SERVICE_REAUTHORIZE_DESCRIPTION,
                    "params_schema": service_reauthorize_params_schema(),
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
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
                },
                {
                    "action": "key.update",
                    "description": super::KEY_UPDATE_DESCRIPTION,
                    "params_schema": key_update_params_schema(),
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "key.delete",
                    "description": super::KEY_DELETE_DESCRIPTION,
                    "params_schema": key_delete_params_schema(),
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "key.extend_scope",
                    "description": super::KEY_EXTEND_SCOPE_DESCRIPTION,
                    "params_schema": key_extend_scope_params_schema(),
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "key.bind_credential",
                    "description": super::KEY_BIND_CREDENTIAL_DESCRIPTION,
                    "params_schema": key_bind_credential_params_schema(),
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "service.update",
                    "description": super::SERVICE_UPDATE_DESCRIPTION,
                    "params_schema": service_update_params_schema(),
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "service.delete",
                    "description": super::SERVICE_DELETE_DESCRIPTION,
                    "params_schema": user_service_only_params_schema(),
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "service.route",
                    "description": super::SERVICE_ROUTE_DESCRIPTION,
                    "params_schema": service_route_params_schema(),
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "service.rotate_credential",
                    "description": super::SERVICE_ROTATE_CREDENTIAL_DESCRIPTION,
                    "params_schema": user_service_only_params_schema(),
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "endpoint.update",
                    "description": super::ENDPOINT_UPDATE_DESCRIPTION,
                    "params_schema": endpoint_update_params_schema(),
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "endpoint.delete",
                    "description": super::ENDPOINT_DELETE_DESCRIPTION,
                    "params_schema": endpoint_delete_params_schema(),
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "external_key.rotate",
                    "description": super::EXTERNAL_KEY_ROTATE_DESCRIPTION,
                    "params_schema": external_key_only_params_schema(),
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "external_key.delete",
                    "description": super::EXTERNAL_KEY_DELETE_DESCRIPTION,
                    "params_schema": external_key_only_params_schema(),
                    "risk": "destructive",
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
        assert!(seen_actions.contains("service.reauthorize"));
        assert!(seen_actions.contains("key.create"));
        assert!(seen_actions.contains("key.rotate"));
    }

    #[test]
    fn assistant_actions_manifest_matches_golden_payload() {
        let manifest: Value = serde_json::from_str(manifest_body()).unwrap();
        let actions = manifest["actions"].as_array().unwrap();

        assert_eq!(manifest["schema_version"], 4);
        assert_eq!(manifest["revision"], "nyxid-assistant-actions.v8");
        assert_eq!(actions.len(), 16);
        assert_eq!(actions[0]["action"], "service.connect");
        assert_eq!(actions[0]["risk"], "grant");
        assert_eq!(actions[0]["tier"], "v1");
        assert_eq!(actions[0]["remember_eligible"], true);
        assert_eq!(actions[0]["params_schema"], service_connect_params_schema());
        assert_eq!(actions[1]["action"], "service.reauthorize");
        assert_eq!(actions[1]["risk"], "grant");
        assert_eq!(actions[1]["tier"], "v1");
        assert_eq!(actions[1]["remember_eligible"], false);
        assert_eq!(
            actions[1]["params_schema"],
            service_reauthorize_params_schema()
        );
        assert_eq!(actions[2]["action"], "key.create");
        assert_eq!(actions[2]["risk"], "grant");
        assert_eq!(actions[2]["tier"], "v1");
        assert_eq!(actions[2]["remember_eligible"], false);
        assert_eq!(actions[2]["params_schema"], key_create_params_schema());
        assert_eq!(
            actions[2]["params_schema"]["properties"]["allowedServiceIds"]["minItems"],
            1
        );
        assert_eq!(
            actions[2]["params_schema"]["properties"]["allowedServiceIds"]["uniqueItems"],
            true
        );
        assert_eq!(actions[3]["action"], "key.rotate");
        assert_eq!(actions[3]["risk"], "grant");
        assert_eq!(actions[3]["tier"], "v1");
        assert_eq!(actions[3]["remember_eligible"], false);
        assert_eq!(actions[3]["params_schema"], key_rotate_params_schema());
        assert_eq!(manifest, golden_manifest());
    }

    #[test]
    fn assistant_actions_manifest_conforms_to_aevatar_parser_contract() {
        assert_manifest_conforms(manifest_body());
    }

    /// Guards the dormant-merge protocol: the revision stays v8, the four
    /// pinned descriptors keep their positions (deployed compositions
    /// deep-equal their schemas), and every Wave-2 descriptor is present,
    /// never remember-eligible, and correctly marked destructive where the
    /// wave says so. A revision bump or a shipped-schema edit fails here
    /// before it can fail an Aevatar startup.
    #[test]
    fn wave2_descriptors_are_dormant_appended_and_never_remembered() {
        let manifest: Value = serde_json::from_str(manifest_body()).unwrap();
        assert_eq!(manifest["revision"], "nyxid-assistant-actions.v8");

        let actions = manifest["actions"].as_array().unwrap();
        for (index, pinned) in V8_PINNED_ACTIONS.iter().enumerate() {
            assert_eq!(
                actions[index]["action"], *pinned,
                "pinned descriptor moved: {pinned}"
            );
        }

        let destructive = [
            "key.delete",
            "service.delete",
            "endpoint.delete",
            "external_key.delete",
        ];
        let wave2: Vec<&str> = SUPPORTED_ACTIONS
            .iter()
            .copied()
            .filter(|name| !V8_PINNED_ACTIONS.contains(name))
            .filter(|name| {
                actions
                    .iter()
                    .any(|entry| entry["action"].as_str() == Some(name))
            })
            .collect();
        assert_eq!(wave2.len(), 12, "expected all 12 Wave-2 descriptors");
        for name in wave2 {
            let entry = actions
                .iter()
                .find(|entry| entry["action"].as_str() == Some(name))
                .unwrap();
            assert_eq!(
                entry["remember_eligible"], false,
                "dormant descriptors must never be remembered: {name}"
            );
            let expected_risk = if destructive.contains(&name) {
                "destructive"
            } else {
                "grant"
            };
            assert_eq!(entry["risk"], expected_risk, "wrong risk: {name}");
        }
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
        let (_, private_api) = crate::routes::build_router();

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
