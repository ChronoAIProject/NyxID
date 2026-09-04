use std::collections::HashMap;
use std::sync::LazyLock;

use axum::extract::Query;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};

use crate::errors::AppError;

pub const ASSISTANT_ACTIONS_SCHEMA_VERSION: u32 = 4;
pub const ASSISTANT_ACTIONS_REVISION: &str = "nyxid-assistant-actions.v8";

/// Maximum accepted `?revision=` length. Matches Aevatar's
/// `ReadRequiredString(..., 128)` bound on the revision field.
const MAX_REVISION_PARAM_CHARS: usize = 128;

/// Mirrors Aevatar `PinnedActionsByRevision` on `origin/feature/integrate`
/// (the superset of `origin/dev`'s `{v4…v7}`). Action-name order is Aevatar's
/// listed order; it is not a prefix of the current default array (v7 skips
/// `service.reauthorize` at index 1). `aevatar-nyxid-actions.v1` is Aevatar's
/// own composition namespace and is not a NyxID-published revision.
const PINNED_ACTIONS_BY_REVISION: &[(&str, &[&str])] = &[
    ("nyxid-assistant-actions.v4", &["service.connect"]),
    (
        "nyxid-assistant-actions.v5",
        &[
            "service.connect",
            "service.reauthorize",
            "key.create",
            "key.rotate",
        ],
    ),
    (
        "nyxid-assistant-actions.v6",
        &["service.connect", "key.create"],
    ),
    (
        "nyxid-assistant-actions.v7",
        &["service.connect", "key.create", "key.rotate"],
    ),
    (
        "nyxid-assistant-actions.v8",
        &[
            "service.connect",
            "service.reauthorize",
            "key.create",
            "key.rotate",
        ],
    ),
];

/// Historical `key.create` params_schema. Aevatar compiles this as
/// `KeyCreateParamsSchema` and selects it for v4 and v5 (`ValidatePinnedContract`).
/// v4 pins only `service.connect`, so this shape is served only on v5. The
/// live descriptor is the least-scope variant and must not be edited.
fn key_create_params_schema_pre_least_scope() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "platform", "allowedServiceIds"],
        "properties": {
            "name": { "type": "string" },
            "platform": { "type": "string" },
            "allowedServiceIds": {
                "type": "array",
                "items": { "type": "string" }
            }
        }
    })
}

/// The `(param name, schema factory)` table for a single revision.
type ParamsSchemaOverrides = &'static [(&'static str, fn() -> Value)];

/// Per-revision `params_schema` overrides. Absence means "use the current
/// live descriptor". A future schema split is a new row here, not a new
/// branch in `compose_revision_manifest`.
const PARAMS_SCHEMA_OVERRIDES_BY_REVISION: &[(&str, ParamsSchemaOverrides)] = &[(
    "nyxid-assistant-actions.v5",
    &[("key.create", key_create_params_schema_pre_least_scope)],
)];

fn params_schema_override(revision: &str, action: &str) -> Option<Value> {
    PARAMS_SCHEMA_OVERRIDES_BY_REVISION
        .iter()
        .find(|(rev, _)| *rev == revision)
        .and_then(|(_, overrides)| {
            overrides
                .iter()
                .find(|(name, _)| *name == action)
                .map(|(_, schema)| schema())
        })
}

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
const CONNECTION_REVOKE_DESCRIPTION: &str = "Ask the user's browser to revoke one exact legacy service connection and securely clear its stored credential. Destructive: NyxID confirms every time and reports only the safe connection reference. Never request, expose, or repeat credential material in chat.";
const PROVIDER_DISCONNECT_DESCRIPTION: &str = "Ask the user's browser to disconnect one exact legacy provider token and securely clear its stored token material. Destructive: NyxID confirms every time and reports only the safe provider reference. Never request, expose, or repeat token material in chat.";
const PROVIDER_SET_APP_CREDENTIALS_DESCRIPTION: &str = "Ask the user's browser to set the user's own OAuth app credentials for one exact provider. The client ID and secret are entered only inside NyxID's browser journey; NyxID reports only the safe provider reference. Never request, expose, or repeat credential material in chat.";

const NODE_REGISTER_TOKEN_DESCRIPTION: &str = "Ask the user's browser to mint a one-time NyxID node registration token. Use when the user needs to enrol a new on-premise credential node. NyxID displays the token once in the browser and reports only a safe node reference. Never request, expose, or repeat the token in chat.";
const NODE_ROTATE_TOKEN_DESCRIPTION: &str = "Ask the user's browser to rotate one exact node's auth token. Use when a node credential must be replaced. NyxID displays replacement material once in the browser and reports only the node reference. Never request, expose, or repeat token material in chat.";
const NODE_DELETE_DESCRIPTION: &str = "Ask the user's browser to delete one exact credential node. Destructive: the node loses access and its local credential store is orphaned. NyxID confirms every time and reports only the node reference.";
const NODE_TRANSFER_DESCRIPTION: &str = "Ask the user's browser to transfer one exact credential node to another owner. Destructive: the current owner loses control of the node. NyxID confirms every time and reports only the node reference.";
const NODE_INJECT_CREDENTIAL_DESCRIPTION: &str = "Ask the user's browser to inject a credential into one exact node's local encrypted store. NyxID owns credential entry entirely; the secret is encrypted to the node and never travels through chat. Reports only a safe pending-credential reference. Never ask the user for the secret in chat.";
const PENDING_CREDENTIAL_PUSH_DESCRIPTION: &str = "Ask the user's browser to queue a credential push to one exact node. NyxID owns credential entry and encryption; the assistant receives only a safe pending-credential reference. Never ask the user for the secret in chat.";
const PENDING_CREDENTIAL_CANCEL_DESCRIPTION: &str = "Ask the user's browser to cancel one exact queued pending credential. Destructive: the queued secret is discarded. NyxID confirms every time and reports only the pending-credential reference.";
const DEVICE_ONBOARD_DESCRIPTION: &str = "Ask the user's browser to onboard a headless device and issue its scoped provisioning credentials. NyxID owns the whole journey and shows provisioning material once in the browser, reporting only a safe device reference. Never request, expose, or repeat a device code or credential in chat.";

const ORG_CREATE_DESCRIPTION: &str = "Ask the user's browser to create a NyxID organization. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const ORG_UPDATE_DESCRIPTION: &str = "Ask the user's browser to update one exact organization's profile. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const ORG_DELETE_DESCRIPTION: &str = "Ask the user's browser to delete one exact organization and every membership it owns. Destructive: NyxID confirms every time and never remembers this action. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const ORG_MEMBER_ADD_DESCRIPTION: &str = "Ask the user's browser to add one exact member to an organization with an explicit role. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const ORG_MEMBER_REMOVE_DESCRIPTION: &str = "Ask the user's browser to remove one exact organization member. Destructive: NyxID confirms every time and never remembers this action. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const ORG_MEMBER_UPDATE_ROLE_DESCRIPTION: &str = "Ask the user's browser to change one exact organization member's role. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const ORG_INVITE_DESCRIPTION: &str = "Ask the user's browser to mint an organization invite for an explicit role. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const ORG_SET_PRIMARY_DESCRIPTION: &str = "Ask the user's browser to set one exact organization as the primary one. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const ACCOUNT_PROFILE_UPDATE_DESCRIPTION: &str = "Ask the user's browser to update the signed-in account profile. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const ACCOUNT_REVOKE_CONSENT_DESCRIPTION: &str = "Ask the user's browser to revoke one exact application's consent. Destructive: NyxID confirms every time and never remembers this action. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const ACCOUNT_DELETE_DESCRIPTION: &str = "Ask the user's browser to open the irreversible account deletion journey. Destructive: NyxID confirms every time and never remembers this action. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const ACCOUNT_MFA_SETUP_DESCRIPTION: &str = "Ask the user's browser to set up multi-factor authentication. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const APPROVAL_CONFIGURE_DESCRIPTION: &str = "Ask the user's browser to configure the approval policy for one exact service. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const APPROVAL_ENABLE_DESCRIPTION: &str = "Ask the user's browser to require approvals for one exact service. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const APPROVAL_DISABLE_DESCRIPTION: &str = "Ask the user's browser to stop requiring approvals for one exact service. Destructive: NyxID confirms every time and never remembers this action. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const APPROVAL_REVOKE_GRANT_DESCRIPTION: &str = "Ask the user's browser to revoke one exact standing approval grant. Destructive: NyxID confirms every time and never remembers this action. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const NOTIFICATIONS_UPDATE_DESCRIPTION: &str = "Ask the user's browser to open notification settings so the user can change them. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const NOTIFICATIONS_TELEGRAM_LINK_DESCRIPTION: &str = "Ask the user's browser to link a Telegram account for approval notifications. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const NOTIFICATIONS_TELEGRAM_DISCONNECT_DESCRIPTION: &str = "Ask the user's browser to disconnect the linked Telegram account. Destructive: NyxID confirms every time and never remembers this action. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const SERVICE_ACCOUNT_CREATE_DESCRIPTION: &str = "Ask the user's browser to create a service account and show its secret once. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const SERVICE_ACCOUNT_UPDATE_DESCRIPTION: &str = "Ask the user's browser to update one exact service account. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const SERVICE_ACCOUNT_DELETE_DESCRIPTION: &str = "Ask the user's browser to delete one exact service account. Destructive: NyxID confirms every time and never remembers this action. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const SERVICE_ACCOUNT_ROTATE_SECRET_DESCRIPTION: &str = "Ask the user's browser to rotate one exact service account's secret. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const SERVICE_ACCOUNT_REVOKE_TOKENS_DESCRIPTION: &str = "Ask the user's browser to revoke every live token for one exact service account. Destructive: NyxID confirms every time and never remembers this action. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const DEVELOPER_APP_CREATE_DESCRIPTION: &str = "Ask the user's browser to create a developer OAuth application and show its secret once. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const DEVELOPER_APP_UPDATE_DESCRIPTION: &str = "Ask the user's browser to update one exact developer OAuth application. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const DEVELOPER_APP_DELETE_DESCRIPTION: &str = "Ask the user's browser to delete one exact developer OAuth application. Destructive: NyxID confirms every time and never remembers this action. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const DEVELOPER_APP_ROTATE_SECRET_DESCRIPTION: &str = "Ask the user's browser to rotate one exact developer OAuth application's secret. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const EXTERNAL_KEY_ADD_GCP_SERVICE_ACCOUNT_DESCRIPTION: &str = "Ask the user's browser to add a GCP service-account key as an external credential. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";
const OPENCLAW_CONNECT_DESCRIPTION: &str = "Ask the user's browser to connect a self-hosted OpenClaw gateway. NyxID owns the journey in the user's own browser and reports only a safe typed resource reference. Never request, expose, or repeat credential material in chat.";

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
            AssistantActionDescriptor {
                action: "connection.revoke",
                description: CONNECTION_REVOKE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["serviceId"],
                    "properties": {
                        "serviceId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "provider.disconnect",
                description: PROVIDER_DISCONNECT_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["providerId"],
                    "properties": {
                        "providerId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "provider.set_app_credentials",
                description: PROVIDER_SET_APP_CREDENTIALS_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["providerId"],
                    "properties": {
                        "providerId": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "node.register_token",
                description: NODE_REGISTER_TOKEN_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name"],
                    "properties": {
                        "name": { "type": "string" },
                        "targetOrgId": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "node.rotate_token",
                description: NODE_ROTATE_TOKEN_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["nodeId"],
                    "properties": {
                        "nodeId": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "node.delete",
                description: NODE_DELETE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["nodeId"],
                    "properties": {
                        "nodeId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "node.transfer",
                description: NODE_TRANSFER_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["nodeId", "newOwnerUserId"],
                    "properties": {
                        "nodeId": { "type": "string" },
                        "newOwnerUserId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "node.inject_credential",
                description: NODE_INJECT_CREDENTIAL_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["nodeId", "serviceSlug", "injectionMethod", "fieldName"],
                    "properties": {
                        "nodeId": { "type": "string" },
                        "serviceSlug": { "type": "string" },
                        "injectionMethod": { "type": "string" },
                        "fieldName": { "type": "string" },
                        "targetUrl": { "type": "string" },
                        "label": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "pending_credential.push",
                description: PENDING_CREDENTIAL_PUSH_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["nodeId", "serviceSlug", "injectionMethod", "fieldName"],
                    "properties": {
                        "nodeId": { "type": "string" },
                        "serviceSlug": { "type": "string" },
                        "injectionMethod": { "type": "string" },
                        "fieldName": { "type": "string" },
                        "targetUrl": { "type": "string" },
                        "label": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "pending_credential.cancel",
                description: PENDING_CREDENTIAL_CANCEL_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["nodeId", "pendingCredentialId"],
                    "properties": {
                        "nodeId": { "type": "string" },
                        "pendingCredentialId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "device.onboard",
                description: DEVICE_ONBOARD_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["label"],
                    "properties": {
                        "label": { "type": "string" },
                        "targetOrgId": { "type": "string" },
                        "defaultServiceIds": { "type": "array", "items": { "type": "string" } }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "org.create",
                description: ORG_CREATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["displayName"],
                    "properties": {
                        "displayName": { "type": "string" },
                        "contactEmail": { "type": "string" },
                        "avatarUrl": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "org.update",
                description: ORG_UPDATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["orgId"],
                    "properties": {
                        "orgId": { "type": "string" },
                        "displayName": { "type": "string" },
                        "slug": { "type": "string" },
                        "contactEmail": { "type": "string" },
                        "avatarUrl": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "org.delete",
                description: ORG_DELETE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["orgId"],
                    "properties": {
                        "orgId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "org.member_add",
                description: ORG_MEMBER_ADD_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["orgId", "userId", "role"],
                    "properties": {
                        "orgId": { "type": "string" },
                        "userId": { "type": "string" },
                        "role": { "type": "string" },
                        "allowedServiceIds": { "type": "array", "items": { "type": "string" } }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "org.member_remove",
                description: ORG_MEMBER_REMOVE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["orgId", "memberId"],
                    "properties": {
                        "orgId": { "type": "string" },
                        "memberId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "org.member_update_role",
                description: ORG_MEMBER_UPDATE_ROLE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["orgId", "memberId", "role"],
                    "properties": {
                        "orgId": { "type": "string" },
                        "memberId": { "type": "string" },
                        "role": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "org.invite",
                description: ORG_INVITE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["orgId", "role"],
                    "properties": {
                        "orgId": { "type": "string" },
                        "role": { "type": "string" },
                        "allowedServiceIds": { "type": "array", "items": { "type": "string" } }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "org.set_primary",
                description: ORG_SET_PRIMARY_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["orgId"],
                    "properties": {
                        "orgId": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "account.profile_update",
                description: ACCOUNT_PROFILE_UPDATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "displayName": { "type": "string" },
                        "avatarUrl": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "account.revoke_consent",
                description: ACCOUNT_REVOKE_CONSENT_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["clientId"],
                    "properties": {
                        "clientId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "account.delete",
                description: ACCOUNT_DELETE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {}
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "account.mfa_setup",
                description: ACCOUNT_MFA_SETUP_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {}
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "approval.configure",
                description: APPROVAL_CONFIGURE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["serviceId"],
                    "properties": {
                        "serviceId": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "approval.enable",
                description: APPROVAL_ENABLE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["serviceId"],
                    "properties": {
                        "serviceId": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "approval.disable",
                description: APPROVAL_DISABLE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["serviceId"],
                    "properties": {
                        "serviceId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "approval.revoke_grant",
                description: APPROVAL_REVOKE_GRANT_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["grantId"],
                    "properties": {
                        "grantId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "notifications.update",
                description: NOTIFICATIONS_UPDATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {}
                }),
                risk: "low",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "notifications.telegram_link",
                description: NOTIFICATIONS_TELEGRAM_LINK_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {}
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "notifications.telegram_disconnect",
                description: NOTIFICATIONS_TELEGRAM_DISCONNECT_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {}
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "service_account.create",
                description: SERVICE_ACCOUNT_CREATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name"],
                    "properties": {
                        "name": { "type": "string" },
                        "description": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "service_account.update",
                description: SERVICE_ACCOUNT_UPDATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["serviceAccountId"],
                    "properties": {
                        "serviceAccountId": { "type": "string" },
                        "name": { "type": "string" },
                        "description": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "service_account.delete",
                description: SERVICE_ACCOUNT_DELETE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["serviceAccountId"],
                    "properties": {
                        "serviceAccountId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "service_account.rotate_secret",
                description: SERVICE_ACCOUNT_ROTATE_SECRET_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["serviceAccountId"],
                    "properties": {
                        "serviceAccountId": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "service_account.revoke_tokens",
                description: SERVICE_ACCOUNT_REVOKE_TOKENS_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["serviceAccountId"],
                    "properties": {
                        "serviceAccountId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "developer_app.create",
                description: DEVELOPER_APP_CREATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name", "redirectUris"],
                    "properties": {
                        "name": { "type": "string" },
                        "redirectUris": { "type": "array", "items": { "type": "string" } }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "developer_app.update",
                description: DEVELOPER_APP_UPDATE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["clientId"],
                    "properties": {
                        "clientId": { "type": "string" },
                        "name": { "type": "string" },
                        "redirectUris": { "type": "array", "items": { "type": "string" } }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "developer_app.delete",
                description: DEVELOPER_APP_DELETE_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["clientId"],
                    "properties": {
                        "clientId": { "type": "string" }
                    }
                }),
                risk: "destructive",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "developer_app.rotate_secret",
                description: DEVELOPER_APP_ROTATE_SECRET_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["clientId"],
                    "properties": {
                        "clientId": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "external_key.add_gcp_service_account",
                description: EXTERNAL_KEY_ADD_GCP_SERVICE_ACCOUNT_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "label": { "type": "string" }
                    }
                }),
                risk: "grant",
                tier: "v1",
                remember_eligible: false,
            },
            AssistantActionDescriptor {
                action: "openclaw.connect",
                description: OPENCLAW_CONNECT_DESCRIPTION,
                params_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["gatewayUrl"],
                    "properties": {
                        "gatewayUrl": { "type": "string" }
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

/// Per-revision bodies: that revision's action-name set, with per-revision
/// `params_schema` overlays from `PARAMS_SCHEMA_OVERRIDES_BY_REVISION`.
/// Aevatar's `ValidatePinnedContract` selects a compiled constant per
/// (revision, action) and `JsonNode.DeepEquals`s it; current bytes only
/// pass for revisions that pin the live descriptor.
static PINNED_REVISION_BODIES: LazyLock<HashMap<&'static str, String>> = LazyLock::new(|| {
    PINNED_ACTIONS_BY_REVISION
        .iter()
        .map(|(revision, action_names)| {
            (*revision, compose_revision_manifest(revision, action_names))
        })
        .collect()
});

#[derive(Debug, Default, Deserialize)]
pub(crate) struct AssistantActionsQuery {
    revision: Option<String>,
}

fn compose_revision_manifest(revision: &'static str, action_names: &[&str]) -> String {
    let latest: Value = serde_json::from_str(manifest_body())
        .expect("latest assistant actions manifest must parse");
    let actions = latest
        .get("actions")
        .and_then(Value::as_array)
        .expect("latest manifest must have an actions array");
    let filtered: Vec<Value> = action_names
        .iter()
        .map(|name| {
            let mut entry = actions
                .iter()
                .find(|entry| entry.get("action").and_then(Value::as_str) == Some(*name))
                .cloned()
                .unwrap_or_else(|| {
                    panic!("pinned action missing from current descriptors for {revision}: {name}")
                });
            if let Some(schema) = params_schema_override(revision, name) {
                entry["params_schema"] = schema;
            }
            entry
        })
        .collect();

    serde_json::to_string(&json!({
        "schema_version": latest["schema_version"],
        "revision": revision,
        "actions": filtered,
    }))
    .expect("revision assistant actions manifest must serialize")
}

fn revision_param_is_malformed(revision: &str) -> bool {
    revision.chars().count() > MAX_REVISION_PARAM_CHARS || revision.chars().any(char::is_control)
}

fn json_manifest_response(body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

pub fn manifest_body() -> &'static str {
    MANIFEST_BODY.as_str()
}

fn pinned_revision_body(revision: &str) -> Option<&'static str> {
    PINNED_REVISION_BODIES.get(revision).map(String::as_str)
}

fn resolve_assistant_actions_body(revision: Option<&str>) -> Result<&'static str, AppError> {
    let Some(revision) = revision else {
        return Ok(manifest_body());
    };
    if revision_param_is_malformed(revision) {
        return Err(AppError::ValidationError(
            "Invalid assistant actions revision".to_string(),
        ));
    }
    pinned_revision_body(revision)
        .ok_or_else(|| AppError::NotFound("Unknown assistant actions revision".to_string()))
}

/// GET /api/v1/assistant/actions
pub async fn get_assistant_actions(Query(query): Query<AssistantActionsQuery>) -> Response {
    match resolve_assistant_actions_body(query.revision.as_deref()) {
        Ok(body) => json_manifest_response(body),
        Err(error) => error.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap as StdHashMap, HashSet},
        sync::Arc,
    };

    use axum::{
        Extension,
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
        middleware,
        response::Response,
    };
    use serde::Deserialize;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::{
        ASSISTANT_ACTIONS_REVISION, ASSISTANT_ACTIONS_SCHEMA_VERSION, PINNED_ACTIONS_BY_REVISION,
        manifest_body, resolve_assistant_actions_body,
    };
    use crate::config::TrustedProxyRange;
    use crate::mw::rate_limit::{GlobalRateLimiter, PerIpRateLimiter, rate_limit_middleware};

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
        "node.delete",
        "node.transfer",
        "pending_credential.push",
        "pending_credential.cancel",
        "org.create",
        "org.update",
        "org.delete",
        "org.member_add",
        "org.member_remove",
        "org.member_update_role",
        "org.invite",
        "org.set_primary",
        "account.profile_update",
        "account.revoke_consent",
        "account.delete",
        "approval.configure",
        "approval.enable",
        "approval.disable",
        "approval.revoke_grant",
        "notifications.update",
        "notifications.telegram_link",
        "notifications.telegram_disconnect",
        "service_account.update",
        "service_account.delete",
        "service_account.revoke_tokens",
        "developer_app.update",
        "developer_app.delete",
        "external_key.add_gcp_service_account",
        "openclaw.connect",
        "device.onboard",
        "// Wave 2 (dormant; not pinned by any Aevatar revision yet)",
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
        "connection.revoke",
        "provider.disconnect",
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
                },
                {
                    "action": "connection.revoke",
                    "description": super::CONNECTION_REVOKE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["serviceId"],
                        "properties": { "serviceId": { "type": "string" } }
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "provider.disconnect",
                    "description": super::PROVIDER_DISCONNECT_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["providerId"],
                        "properties": { "providerId": { "type": "string" } }
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "provider.set_app_credentials",
                    "description": super::PROVIDER_SET_APP_CREDENTIALS_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["providerId"],
                        "properties": { "providerId": { "type": "string" } }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "node.register_token",
                    "description": super::NODE_REGISTER_TOKEN_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["name"],
                        "properties": {
                            "name": { "type": "string" },
                            "targetOrgId": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "node.rotate_token",
                    "description": super::NODE_ROTATE_TOKEN_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["nodeId"],
                        "properties": {
                            "nodeId": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "node.delete",
                    "description": super::NODE_DELETE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["nodeId"],
                        "properties": {
                            "nodeId": { "type": "string" }
                        }
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "node.transfer",
                    "description": super::NODE_TRANSFER_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["nodeId", "newOwnerUserId"],
                        "properties": {
                            "nodeId": { "type": "string" },
                            "newOwnerUserId": { "type": "string" }
                        }
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "node.inject_credential",
                    "description": super::NODE_INJECT_CREDENTIAL_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["nodeId", "serviceSlug", "injectionMethod", "fieldName"],
                        "properties": {
                            "nodeId": { "type": "string" },
                            "serviceSlug": { "type": "string" },
                            "injectionMethod": { "type": "string" },
                            "fieldName": { "type": "string" },
                            "targetUrl": { "type": "string" },
                            "label": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "pending_credential.push",
                    "description": super::PENDING_CREDENTIAL_PUSH_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["nodeId", "serviceSlug", "injectionMethod", "fieldName"],
                        "properties": {
                            "nodeId": { "type": "string" },
                            "serviceSlug": { "type": "string" },
                            "injectionMethod": { "type": "string" },
                            "fieldName": { "type": "string" },
                            "targetUrl": { "type": "string" },
                            "label": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "pending_credential.cancel",
                    "description": super::PENDING_CREDENTIAL_CANCEL_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["nodeId", "pendingCredentialId"],
                        "properties": {
                            "nodeId": { "type": "string" },
                            "pendingCredentialId": { "type": "string" }
                        }
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "device.onboard",
                    "description": super::DEVICE_ONBOARD_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["label"],
                        "properties": {
                            "label": { "type": "string" },
                            "targetOrgId": { "type": "string" },
                            "defaultServiceIds": { "type": "array", "items": { "type": "string" } }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "org.create",
                    "description": super::ORG_CREATE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["displayName"],
                        "properties": {
                            "displayName": { "type": "string" },
                            "contactEmail": { "type": "string" },
                            "avatarUrl": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "org.update",
                    "description": super::ORG_UPDATE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["orgId"],
                        "properties": {
                            "orgId": { "type": "string" },
                            "displayName": { "type": "string" },
                            "slug": { "type": "string" },
                            "contactEmail": { "type": "string" },
                            "avatarUrl": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "org.delete",
                    "description": super::ORG_DELETE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["orgId"],
                        "properties": {
                            "orgId": { "type": "string" }
                        }
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "org.member_add",
                    "description": super::ORG_MEMBER_ADD_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["orgId", "userId", "role"],
                        "properties": {
                            "orgId": { "type": "string" },
                            "userId": { "type": "string" },
                            "role": { "type": "string" },
                            "allowedServiceIds": { "type": "array", "items": { "type": "string" } }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "org.member_remove",
                    "description": super::ORG_MEMBER_REMOVE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["orgId", "memberId"],
                        "properties": {
                            "orgId": { "type": "string" },
                            "memberId": { "type": "string" }
                        }
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "org.member_update_role",
                    "description": super::ORG_MEMBER_UPDATE_ROLE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["orgId", "memberId", "role"],
                        "properties": {
                            "orgId": { "type": "string" },
                            "memberId": { "type": "string" },
                            "role": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "org.invite",
                    "description": super::ORG_INVITE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["orgId", "role"],
                        "properties": {
                            "orgId": { "type": "string" },
                            "role": { "type": "string" },
                            "allowedServiceIds": { "type": "array", "items": { "type": "string" } }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "org.set_primary",
                    "description": super::ORG_SET_PRIMARY_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["orgId"],
                        "properties": {
                            "orgId": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "account.profile_update",
                    "description": super::ACCOUNT_PROFILE_UPDATE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "displayName": { "type": "string" },
                            "avatarUrl": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "account.revoke_consent",
                    "description": super::ACCOUNT_REVOKE_CONSENT_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["clientId"],
                        "properties": {
                            "clientId": { "type": "string" }
                        }
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "account.delete",
                    "description": super::ACCOUNT_DELETE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {}
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "account.mfa_setup",
                    "description": super::ACCOUNT_MFA_SETUP_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {}
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "approval.configure",
                    "description": super::APPROVAL_CONFIGURE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["serviceId"],
                        "properties": {
                            "serviceId": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "approval.enable",
                    "description": super::APPROVAL_ENABLE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["serviceId"],
                        "properties": {
                            "serviceId": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "approval.disable",
                    "description": super::APPROVAL_DISABLE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["serviceId"],
                        "properties": {
                            "serviceId": { "type": "string" }
                        }
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "approval.revoke_grant",
                    "description": super::APPROVAL_REVOKE_GRANT_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["grantId"],
                        "properties": {
                            "grantId": { "type": "string" }
                        }
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "notifications.update",
                    "description": super::NOTIFICATIONS_UPDATE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {}
                    },
                    "risk": "low",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "notifications.telegram_link",
                    "description": super::NOTIFICATIONS_TELEGRAM_LINK_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {}
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "notifications.telegram_disconnect",
                    "description": super::NOTIFICATIONS_TELEGRAM_DISCONNECT_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {}
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "service_account.create",
                    "description": super::SERVICE_ACCOUNT_CREATE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["name"],
                        "properties": {
                            "name": { "type": "string" },
                            "description": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "service_account.update",
                    "description": super::SERVICE_ACCOUNT_UPDATE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["serviceAccountId"],
                        "properties": {
                            "serviceAccountId": { "type": "string" },
                            "name": { "type": "string" },
                            "description": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "service_account.delete",
                    "description": super::SERVICE_ACCOUNT_DELETE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["serviceAccountId"],
                        "properties": {
                            "serviceAccountId": { "type": "string" }
                        }
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "service_account.rotate_secret",
                    "description": super::SERVICE_ACCOUNT_ROTATE_SECRET_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["serviceAccountId"],
                        "properties": {
                            "serviceAccountId": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "service_account.revoke_tokens",
                    "description": super::SERVICE_ACCOUNT_REVOKE_TOKENS_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["serviceAccountId"],
                        "properties": {
                            "serviceAccountId": { "type": "string" }
                        }
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "developer_app.create",
                    "description": super::DEVELOPER_APP_CREATE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["name", "redirectUris"],
                        "properties": {
                            "name": { "type": "string" },
                            "redirectUris": { "type": "array", "items": { "type": "string" } }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "developer_app.update",
                    "description": super::DEVELOPER_APP_UPDATE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["clientId"],
                        "properties": {
                            "clientId": { "type": "string" },
                            "name": { "type": "string" },
                            "redirectUris": { "type": "array", "items": { "type": "string" } }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "developer_app.delete",
                    "description": super::DEVELOPER_APP_DELETE_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["clientId"],
                        "properties": {
                            "clientId": { "type": "string" }
                        }
                    },
                    "risk": "destructive",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "developer_app.rotate_secret",
                    "description": super::DEVELOPER_APP_ROTATE_SECRET_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["clientId"],
                        "properties": {
                            "clientId": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "external_key.add_gcp_service_account",
                    "description": super::EXTERNAL_KEY_ADD_GCP_SERVICE_ACCOUNT_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "label": { "type": "string" }
                        }
                    },
                    "risk": "grant",
                    "tier": "v1",
                    "remember_eligible": false
                },
                {
                    "action": "openclaw.connect",
                    "description": super::OPENCLAW_CONNECT_DESCRIPTION,
                    "params_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["gatewayUrl"],
                        "properties": {
                            "gatewayUrl": { "type": "string" }
                        }
                    },
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
        assert_manifest_conforms_revision(body, ASSISTANT_ACTIONS_REVISION, None);
    }

    fn assert_manifest_conforms_revision(
        body: &str,
        expected_revision: &str,
        expected_actions: Option<&[&str]>,
    ) {
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
        assert_eq!(revision, expected_revision);

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

        if let Some(expected) = expected_actions {
            let expected_set: HashSet<&str> = expected.iter().copied().collect();
            assert_eq!(seen_actions, expected_set);
        } else {
            assert!(seen_actions.contains("service.connect"));
            assert!(seen_actions.contains("service.reauthorize"));
            assert!(seen_actions.contains("key.create"));
            assert!(seen_actions.contains("key.rotate"));
        }
    }

    fn current_params_schema(action: &str) -> Value {
        match action {
            "service.connect" => service_connect_params_schema(),
            "service.reauthorize" => service_reauthorize_params_schema(),
            "key.create" => key_create_params_schema(),
            "key.rotate" => key_rotate_params_schema(),
            other => panic!("no current schema helper for {other}"),
        }
    }

    /// Aevatar `KeyCreateParamsSchema` (the pre-least-scope constant).
    /// Independent of NyxID's live descriptor so a serving regression that
    /// copies current `key.create` bytes into v5 cannot hide behind a shared
    /// helper.
    fn aevatar_key_create_params_schema_pre_least_scope() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["name", "platform", "allowedServiceIds"],
            "properties": {
                "name": { "type": "string" },
                "platform": { "type": "string" },
                "allowedServiceIds": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }
        })
    }

    /// Mirrors `ValidatePinnedContract`'s per-revision schema selection on
    /// both Aevatar lineages: v4/v5 → `KeyCreateParamsSchema`; v6/v7/v8 →
    /// `LeastScopeKeyCreateParamsSchema`. Other pinned verbs have a single
    /// compiled constant.
    fn aevatar_pinned_params_schema(revision: &str, action: &str) -> Value {
        match action {
            "key.create" => match revision {
                "nyxid-assistant-actions.v6"
                | "nyxid-assistant-actions.v7"
                | "nyxid-assistant-actions.v8" => key_create_params_schema(),
                _ => aevatar_key_create_params_schema_pre_least_scope(),
            },
            other => current_params_schema(other),
        }
    }

    fn aevatar_pinned_risk(action: &str) -> &'static str {
        match action {
            "service.connect" | "service.reauthorize" | "key.create" | "key.rotate" => "grant",
            other => panic!("no Aevatar pinned risk for {other}"),
        }
    }

    fn aevatar_pinned_remember_eligible(action: &str) -> bool {
        match action {
            "service.connect" => true,
            "service.reauthorize" | "key.create" | "key.rotate" => false,
            other => panic!("no Aevatar pinned remember_eligible for {other}"),
        }
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct AevatarRevisionMirror {
        lineages: StdHashMap<String, AevatarLineageMirror>,
    }

    #[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
    struct AevatarLineageMirror {
        supported_revision: String,
        revisions: StdHashMap<String, AevatarRevisionSets>,
    }

    #[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
    struct AevatarRevisionSets {
        pinned: Vec<String>,
        executable: Vec<String>,
    }

    fn load_aevatar_revision_mirror() -> AevatarRevisionMirror {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/assistant/aevatar-pinned-actions-by-revision.json"
        ))
        .expect("manually synced Aevatar revision-set mirror must parse")
    }

    fn integrate_lineage_sets(revision: &str) -> AevatarRevisionSets {
        load_aevatar_revision_mirror()
            .lineages
            .get("origin/feature/integrate")
            .expect("mirror must include origin/feature/integrate")
            .revisions
            .get(revision)
            .cloned()
            .unwrap_or_else(|| panic!("integrate lineage missing {revision}"))
    }

    /// Reproduces Aevatar `Load` + `ValidatePinnedContract` for one served
    /// composition: skip unknown/non-pinned names, DeepEquals the
    /// revision-selected schema (via `serde_json::Value`, the Rust analogue
    /// of `JsonNode.DeepEquals` on canonicalized JSON), check pinned
    /// risk/remember, require every pinned name, and require every
    /// executable name.
    fn assert_matches_aevatar_pinned_contract(body: &str, revision: &str) {
        let sets = integrate_lineage_sets(revision);
        let manifest: Value = serde_json::from_str(body).expect("served body must be JSON");
        let actions = manifest["actions"]
            .as_array()
            .expect("actions must be an array");

        let pinned: HashSet<&str> = sets.pinned.iter().map(String::as_str).collect();
        let mut seen = HashSet::new();
        for entry in actions {
            let action = entry["action"].as_str().expect("action must be a string");
            if !pinned.contains(action) {
                continue;
            }
            assert!(
                seen.insert(action),
                "duplicate pinned action {action} at {revision}"
            );
            assert_eq!(
                entry["params_schema"],
                aevatar_pinned_params_schema(revision, action),
                "Aevatar JsonNode.DeepEquals pin failed for {action} at {revision}"
            );
            assert_eq!(
                entry["risk"].as_str(),
                Some(aevatar_pinned_risk(action)),
                "pinned risk mismatch for {action} at {revision}"
            );
            assert_eq!(
                entry["remember_eligible"].as_bool(),
                Some(aevatar_pinned_remember_eligible(action)),
                "pinned remember_eligible mismatch for {action} at {revision}"
            );
        }

        let seen_owned: HashSet<String> = seen.iter().map(|name| (*name).to_string()).collect();
        let pinned_owned: HashSet<String> = sets.pinned.iter().cloned().collect();
        assert_eq!(
            seen_owned, pinned_owned,
            "served {revision} is missing a pinned action"
        );
        for executable in &sets.executable {
            assert!(
                seen_owned.contains(executable),
                "executable action {executable} missing from served {revision}"
            );
        }
        if revision == "nyxid-assistant-actions.v4" {
            assert!(
                !seen.contains("key.create"),
                "v4 pins only service.connect; key.create must not be served"
            );
        }
    }

    fn action_names(manifest: &Value) -> Vec<&str> {
        manifest["actions"]
            .as_array()
            .expect("actions must be an array")
            .iter()
            .map(|entry| entry["action"].as_str().expect("action must be a string"))
            .collect()
    }

    async fn request_assistant_actions(uri: &str) -> Response {
        let state = crate::test_utils::test_app_state_no_db().await;
        let (_, private_api) = crate::routes::build_router();
        private_api
            .with_state(state)
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn response_bytes(response: Response) -> (StatusCode, Vec<u8>) {
        let status = response.status();
        let body = to_bytes(response.into_body(), MAXIMUM_REGISTRY_BYTES)
            .await
            .unwrap();
        (status, body.to_vec())
    }

    #[test]
    fn assistant_actions_manifest_matches_golden_payload() {
        let manifest: Value = serde_json::from_str(manifest_body()).unwrap();
        let actions = manifest["actions"].as_array().unwrap();

        assert_eq!(manifest["schema_version"], 4);
        assert_eq!(manifest["revision"], "nyxid-assistant-actions.v8");
        assert_eq!(actions.len(), 57);
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
    fn dormant_descriptors_are_appended_and_never_remembered() {
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
            "connection.revoke",
            "provider.disconnect",
            "node.delete",
            "node.transfer",
            "pending_credential.cancel",
            "org.delete",
            "org.member_remove",
            "account.revoke_consent",
            "account.delete",
            "approval.disable",
            "approval.revoke_grant",
            "notifications.telegram_disconnect",
            "service_account.delete",
            "service_account.revoke_tokens",
            "developer_app.delete",
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
        assert_eq!(
            wave2.len(),
            53,
            "expected 15 Wave-2 + 8 Wave-3 + 30 Wave-4 dormant descriptors"
        );
        for name in wave2 {
            let entry = actions
                .iter()
                .find(|entry| entry["action"].as_str() == Some(name))
                .unwrap();
            assert_eq!(
                entry["remember_eligible"], false,
                "dormant descriptors must never be remembered: {name}"
            );
            // `low` is a legitimate third risk tier (execute immediately, show a
            // receipt) for verbs that only open a settings surface.
            let low = ["notifications.update"];
            let expected_risk = if destructive.contains(&name) {
                "destructive"
            } else if low.contains(&name) {
                "low"
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

    #[tokio::test]
    async fn assistant_actions_default_body_is_byte_identical_to_golden() {
        let response = request_assistant_actions("/api/v1/assistant/actions").await;
        let (status, body) = response_bytes(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_slice(), manifest_body().as_bytes());
        let golden_bytes = serde_json::to_vec(&golden_manifest()).expect("golden must serialize");
        assert_eq!(body.as_slice(), golden_bytes.as_slice());
    }

    #[tokio::test]
    async fn assistant_actions_revision_v7_serves_exact_action_set_with_current_schemas() {
        let response = request_assistant_actions(
            "/api/v1/assistant/actions?revision=nyxid-assistant-actions.v7",
        )
        .await;
        let (status, body) = response_bytes(response).await;
        assert_eq!(status, StatusCode::OK);
        let manifest: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(manifest["revision"], "nyxid-assistant-actions.v7");
        assert_eq!(
            action_names(&manifest),
            vec!["service.connect", "key.create", "key.rotate"]
        );
        for entry in manifest["actions"].as_array().unwrap() {
            let action = entry["action"].as_str().unwrap();
            assert_eq!(
                entry["params_schema"],
                current_params_schema(action),
                "current schema mismatch for {action}"
            );
        }
    }

    #[tokio::test]
    async fn assistant_actions_revision_v4_serves_service_connect_only() {
        let response = request_assistant_actions(
            "/api/v1/assistant/actions?revision=nyxid-assistant-actions.v4",
        )
        .await;
        let (status, body) = response_bytes(response).await;
        assert_eq!(status, StatusCode::OK);
        let manifest: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(manifest["revision"], "nyxid-assistant-actions.v4");
        assert_eq!(action_names(&manifest), vec!["service.connect"]);
        assert_eq!(
            manifest["actions"][0]["params_schema"],
            service_connect_params_schema()
        );
    }

    #[tokio::test]
    async fn assistant_actions_unknown_revision_returns_404_stable_error() {
        for revision in ["nyxid-assistant-actions.v3", "aevatar-nyxid-actions.v1"] {
            let response = request_assistant_actions(&format!(
                "/api/v1/assistant/actions?revision={revision}"
            ))
            .await;
            let (status, body) = response_bytes(response).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "revision {revision}");
            let error: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(error["error"], "not_found");
            assert_eq!(error["error_code"], 1003);
            assert!(error.get("actions").is_none());
            assert!(error.get("schema_version").is_none());
            assert!(
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .is_some_and(|message| message.contains("Unknown assistant actions revision")),
                "stable unknown-revision message for {revision}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn assistant_actions_malformed_revision_param_returns_400() {
        let oversized = "n".repeat(129);
        let oversized_response =
            request_assistant_actions(&format!("/api/v1/assistant/actions?revision={oversized}"))
                .await;
        let (status, body) = response_bytes(oversized_response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let error: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["error"], "validation_error");
        assert_eq!(error["error_code"], 1008);
        assert!(error.get("actions").is_none());

        let control_response = request_assistant_actions(
            "/api/v1/assistant/actions?revision=nyxid-assistant-actions.v7%0A",
        )
        .await;
        let (status, body) = response_bytes(control_response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let error: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["error"], "validation_error");
        assert_eq!(error["error_code"], 1008);
        assert!(error.get("actions").is_none());
    }

    #[tokio::test]
    async fn assistant_actions_revision_query_is_honored_for_every_published_revision() {
        let default_response = request_assistant_actions("/api/v1/assistant/actions").await;
        let (default_status, default_body) = response_bytes(default_response).await;
        assert_eq!(default_status, StatusCode::OK);

        for (revision, expected_names) in PINNED_ACTIONS_BY_REVISION {
            let response = request_assistant_actions(&format!(
                "/api/v1/assistant/actions?revision={revision}"
            ))
            .await;
            let (status, body) = response_bytes(response).await;
            assert_eq!(status, StatusCode::OK, "published revision {revision}");
            let manifest: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(
                manifest["revision"].as_str(),
                Some(*revision),
                "handler ignored ?revision={revision}"
            );
            assert_eq!(
                action_names(&manifest),
                expected_names.to_vec(),
                "handler served the wrong action set for {revision}"
            );
            assert_ne!(
                body.as_slice(),
                default_body.as_slice(),
                "negotiated {revision} must not be the bare latest body"
            );
        }
    }

    #[test]
    fn assistant_actions_every_published_revision_passes_parser_contract() {
        for (revision, action_names) in PINNED_ACTIONS_BY_REVISION {
            let body = resolve_assistant_actions_body(Some(revision))
                .unwrap_or_else(|error| panic!("missing composed body for {revision}: {error}"));
            assert_manifest_conforms_revision(body, revision, Some(action_names));
            assert_matches_aevatar_pinned_contract(body, revision);
        }
        assert_manifest_conforms(manifest_body());
    }

    #[test]
    fn assistant_actions_revision_sets_are_monotone_append_only() {
        // Going forward, each published set is a subset of the next. Aevatar's
        // frozen pin map has one historical reduction: v5 (WaveOneDraft)
        // listed reauthorize+rotate, then v6 (LeastScope) dropped them. That
        // pair is the only permitted non-subset consecutive step.
        let latest = PINNED_ACTIONS_BY_REVISION
            .last()
            .expect("at least one published revision");
        let latest_set: HashSet<&str> = latest.1.iter().copied().collect();

        for window in PINNED_ACTIONS_BY_REVISION.windows(2) {
            let (left_revision, left_actions) = window[0];
            let (right_revision, right_actions) = window[1];
            let left: HashSet<&str> = left_actions.iter().copied().collect();
            let right: HashSet<&str> = right_actions.iter().copied().collect();
            let is_historical_v5_to_v6 = left_revision == "nyxid-assistant-actions.v5"
                && right_revision == "nyxid-assistant-actions.v6";
            if is_historical_v5_to_v6 {
                assert!(
                    !left.is_subset(&right),
                    "v5→v6 is the frozen historical reduction; if it became monotone, drop the exception"
                );
                continue;
            }
            assert!(
                left.is_subset(&right),
                "{left_revision} must be a subset of {right_revision}"
            );
        }

        for (revision, actions) in PINNED_ACTIONS_BY_REVISION {
            let set: HashSet<&str> = actions.iter().copied().collect();
            assert!(
                set.is_subset(&latest_set),
                "{revision} must be a subset of the latest pin {}",
                latest.0
            );
        }
    }

    #[test]
    fn assistant_actions_revision_sets_match_manually_synced_aevatar_mirror() {
        let mirror = load_aevatar_revision_mirror();
        let dev = mirror
            .lineages
            .get("origin/dev")
            .expect("mirror must include origin/dev");
        let integrate = mirror
            .lineages
            .get("origin/feature/integrate")
            .expect("mirror must include origin/feature/integrate");

        assert_eq!(dev.supported_revision, "nyxid-assistant-actions.v7");
        assert_eq!(integrate.supported_revision, "nyxid-assistant-actions.v8");
        assert!(
            !dev.revisions.contains_key("nyxid-assistant-actions.v8"),
            "origin/dev does not publish v8"
        );

        assert_eq!(
            integrate.revisions.len(),
            PINNED_ACTIONS_BY_REVISION.len(),
            "NyxID publishes the integrate superset of revisions"
        );
        for (revision, action_names) in PINNED_ACTIONS_BY_REVISION {
            let sets = integrate
                .revisions
                .get(*revision)
                .unwrap_or_else(|| panic!("integrate mirror missing {revision}"));
            let expected: Vec<String> = action_names
                .iter()
                .map(|name| (*name).to_string())
                .collect();
            assert_eq!(
                sets.pinned, expected,
                "manually synced pinned set mismatch for {revision}"
            );
            for executable in &sets.executable {
                assert!(
                    sets.pinned.iter().any(|name| name == executable),
                    "{executable} is executable on {revision} but not pinned"
                );
            }
        }

        for (revision, sets) in &dev.revisions {
            assert_eq!(
                integrate.revisions.get(revision),
                Some(sets),
                "{revision} pinned/executable sets must match across Aevatar lineages"
            );
        }

        let v5 = integrate
            .revisions
            .get("nyxid-assistant-actions.v5")
            .expect("v5");
        assert_eq!(
            v5.executable,
            vec!["service.connect".to_string()],
            "both lineages make only service.connect executable in v5"
        );
        let v8 = integrate
            .revisions
            .get("nyxid-assistant-actions.v8")
            .expect("v8");
        assert!(
            v8.pinned.iter().any(|name| name == "service.reauthorize")
                && !v8
                    .executable
                    .iter()
                    .any(|name| name == "service.reauthorize"),
            "v8 pins service.reauthorize dormant (not executable)"
        );
    }

    #[tokio::test]
    async fn assistant_actions_route_with_query_remains_rate_limit_exempt() {
        let state = crate::test_utils::test_app_state_no_db().await;
        let (_, private_api) = crate::routes::build_router();
        let per_ip = Arc::new(PerIpRateLimiter::new(1, 60));
        let global = Arc::new(GlobalRateLimiter::new_local(1, 1));
        let app = private_api
            .with_state(state)
            .layer(middleware::from_fn(rate_limit_middleware))
            .layer(Extension(per_ip))
            .layer(Extension(global))
            .layer(Extension(Arc::new(Vec::<TrustedProxyRange>::new())));

        let query_uri = "/api/v1/assistant/actions?revision=nyxid-assistant-actions.v7";
        for _ in 0..3 {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(query_uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "query-string fetch must stay rate-limit exempt"
            );
        }

        let first_counted = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/runtime-config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first_counted.status(), StatusCode::OK);

        let limited = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/runtime-config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);

        let still_exempt = app
            .oneshot(
                Request::builder()
                    .uri(query_uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(still_exempt.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn assistant_actions_dormant_descriptor_is_served_but_absent_from_all_pinned_sets() {
        let default_manifest: Value = serde_json::from_str(manifest_body()).unwrap();
        let default_names = action_names(&default_manifest);
        let latest_pin: HashSet<&str> = PINNED_ACTIONS_BY_REVISION
            .last()
            .expect("latest pin")
            .1
            .iter()
            .copied()
            .collect();
        let dormant: Vec<&str> = default_names
            .iter()
            .copied()
            .filter(|name| !latest_pin.contains(name))
            .collect();
        assert_eq!(
            dormant.len(),
            53,
            "expected 15 Wave-2 + 8 Wave-3 + 30 Wave-4 dormant descriptors"
        );

        for (revision, action_names) in PINNED_ACTIONS_BY_REVISION {
            for name in &dormant {
                assert!(
                    !action_names.contains(name),
                    "{name} must stay absent from pinned set {revision}"
                );
            }
        }

        let response = request_assistant_actions("/api/v1/assistant/actions").await;
        let (status, body) = response_bytes(response).await;
        assert_eq!(status, StatusCode::OK);
        let served: Value = serde_json::from_slice(&body).unwrap();
        for name in dormant {
            assert!(
                action_names(&served).contains(&name),
                "default body must still serve dormant descriptor {name}"
            );
        }
    }

    #[test]
    fn default_manifest_matches_aevatar_chat_contract_pin() {
        let pin: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/assistant/aevatar-chat-contract-pin.json"
        ))
        .expect("AC-0 pin must parse");
        let registry = pin
            .get("action_registry")
            .expect("pin must include action_registry");
        assert_eq!(
            registry.get("schema_version").and_then(Value::as_u64),
            Some(u64::from(ASSISTANT_ACTIONS_SCHEMA_VERSION))
        );
        assert_eq!(
            registry
                .get("revision_is_observability_label")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            registry
                .get("unknown_or_divergent_descriptors")
                .and_then(Value::as_str),
            Some("degrade_per_action")
        );

        let default_manifest: Value = serde_json::from_str(manifest_body()).unwrap();
        let names = action_names(&default_manifest);
        for action in [
            "service.connect",
            "service.reauthorize",
            "key.create",
            "key.rotate",
        ] {
            assert!(
                names.contains(&action),
                "{action} must remain in the additive default manifest"
            );
        }
    }
}
