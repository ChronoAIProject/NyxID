//! Registry of NyxID-hosted curated OpenAPI overlays for seeded catalog
//! services.
//!
//! Each overlay is a small, hand-curated OpenAPI 3.1 document embedded at
//! compile time from `backend/specs/catalog/`, annotated with
//! `x-aevatar-tool` markers, and served publicly at
//! `/api/v1/catalog-specs/{spec_key}/openapi.json`. The overlays are the
//! source of truth for the `ServiceEndpoint` rows that
//! `catalog_spec_sync` materializes at startup, which in turn drive the
//! `service_id + endpoint_id` operation catalog consumed by Aevatar
//! workflow admission via `GET /api/v1/mcp/config` (issue #1290).
//!
//! Several catalog slugs can share one spec key when the underlying API
//! surface is identical (e.g. `api-github` / `api-github-pat`, or the
//! Lark / Feishu domain pairs).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

const SPEC_PATH_PREFIX: &str = "/api/v1/catalog-specs/";
const SPEC_PATH_SUFFIX: &str = "/openapi.json";

/// Embedded overlay documents, keyed by the spec key used in the hosted
/// URL path.
const HOSTED_SPEC_SOURCES: &[(&str, &str)] = &[
    (
        "ifttt-mcp",
        include_str!("../../specs/catalog/ifttt-mcp.openapi.json"),
    ),
    (
        "ifttt",
        include_str!("../../specs/catalog/ifttt.openapi.json"),
    ),
    (
        "notion",
        include_str!("../../specs/catalog/notion.openapi.json"),
    ),
    (
        "aurinko",
        include_str!("../../specs/catalog/aurinko.openapi.json"),
    ),
    (
        "anthropic",
        include_str!("../../specs/catalog/anthropic.openapi.json"),
    ),
    (
        "cohere",
        include_str!("../../specs/catalog/cohere.openapi.json"),
    ),
    (
        "deepseek",
        include_str!("../../specs/catalog/deepseek.openapi.json"),
    ),
    (
        "discord",
        include_str!("../../specs/catalog/discord.openapi.json"),
    ),
    (
        "discord-bot",
        include_str!("../../specs/catalog/discord-bot.openapi.json"),
    ),
    (
        "elevenlabs",
        include_str!("../../specs/catalog/elevenlabs.openapi.json"),
    ),
    (
        "facebook",
        include_str!("../../specs/catalog/facebook.openapi.json"),
    ),
    (
        "firecrawl",
        include_str!("../../specs/catalog/firecrawl.openapi.json"),
    ),
    (
        "github",
        include_str!("../../specs/catalog/github.openapi.json"),
    ),
    (
        "google",
        include_str!("../../specs/catalog/google.openapi.json"),
    ),
    (
        "google-ai",
        include_str!("../../specs/catalog/google-ai.openapi.json"),
    ),
    (
        "google-calendar",
        include_str!("../../specs/catalog/google-calendar.openapi.json"),
    ),
    (
        "google-gmail",
        include_str!("../../specs/catalog/google-gmail.openapi.json"),
    ),
    (
        "google-drive",
        include_str!("../../specs/catalog/google-drive.openapi.json"),
    ),
    (
        "google-docs",
        include_str!("../../specs/catalog/google-docs.openapi.json"),
    ),
    (
        "google-sheets",
        include_str!("../../specs/catalog/google-sheets.openapi.json"),
    ),
    (
        "google-slides",
        include_str!("../../specs/catalog/google-slides.openapi.json"),
    ),
    (
        "lark",
        include_str!("../../specs/catalog/lark.openapi.json"),
    ),
    (
        "lark-bot",
        include_str!("../../specs/catalog/lark-bot.openapi.json"),
    ),
    (
        "microsoft-graph",
        include_str!("../../specs/catalog/microsoft-graph.openapi.json"),
    ),
    (
        "mistral",
        include_str!("../../specs/catalog/mistral.openapi.json"),
    ),
    (
        "openai",
        include_str!("../../specs/catalog/openai.openapi.json"),
    ),
    (
        "openrouter",
        include_str!("../../specs/catalog/openrouter.openapi.json"),
    ),
    (
        "reddit",
        include_str!("../../specs/catalog/reddit.openapi.json"),
    ),
    (
        "slack",
        include_str!("../../specs/catalog/slack.openapi.json"),
    ),
    (
        "spotify",
        include_str!("../../specs/catalog/spotify.openapi.json"),
    ),
    (
        "telegram-bot",
        include_str!("../../specs/catalog/telegram-bot.openapi.json"),
    ),
    (
        "telnyx",
        include_str!("../../specs/catalog/telnyx.openapi.json"),
    ),
    (
        "twitch",
        include_str!("../../specs/catalog/twitch.openapi.json"),
    ),
    (
        "twilio",
        include_str!("../../specs/catalog/twilio.openapi.json"),
    ),
    (
        "twitter",
        include_str!("../../specs/catalog/twitter.openapi.json"),
    ),
];

/// Catalog service slug -> spec key.
const SLUG_TO_SPEC_KEY: &[(&str, &str)] = &[
    ("api-ifttt", "ifttt"),
    ("api-ifttt-mcp", "ifttt-mcp"),
    ("api-notion", "notion"),
    ("api-aurinko", "aurinko"),
    ("api-discord", "discord"),
    ("api-discord-bot", "discord-bot"),
    ("api-elevenlabs", "elevenlabs"),
    ("api-facebook", "facebook"),
    ("api-feishu", "lark"),
    ("api-feishu-bot", "lark-bot"),
    ("api-firecrawl", "firecrawl"),
    ("api-github", "github"),
    ("api-github-pat", "github"),
    ("api-google", "google"),
    ("api-google-workspace", "google-workspace"),
    ("api-google-calendar", "google-calendar"),
    ("api-google-drive", "google-drive"),
    ("api-google-gmail", "google-gmail"),
    ("api-google-docs", "google-docs"),
    ("api-google-sheets", "google-sheets"),
    ("api-google-slides", "google-slides"),
    ("api-lark", "lark"),
    ("api-lark-bot", "lark-bot"),
    ("api-microsoft", "microsoft-graph"),
    ("api-reddit", "reddit"),
    ("api-slack", "slack"),
    ("api-slack-bot", "slack"),
    ("api-spotify", "spotify"),
    ("api-telegram-bot", "telegram-bot"),
    ("api-telnyx", "telnyx"),
    ("api-twitch", "twitch"),
    ("api-twilio", "twilio"),
    ("api-twitter", "twitter"),
    ("llm-anthropic", "anthropic"),
    ("llm-cohere", "cohere"),
    ("llm-deepseek", "deepseek"),
    ("llm-google-ai", "google-ai"),
    ("llm-mistral", "mistral"),
    ("llm-openai", "openai"),
    ("llm-openrouter", "openrouter"),
];

static PARSED_SPECS: LazyLock<HashMap<&'static str, Arc<serde_json::Value>>> = LazyLock::new(
    || {
        let mut specs: HashMap<_, _> = HOSTED_SPEC_SOURCES
            .iter()
            .map(|(key, source)| {
                let parsed =
                    serde_json::from_str::<serde_json::Value>(source).unwrap_or_else(|error| {
                        panic!("embedded catalog spec '{key}' is not valid JSON: {error}")
                    });
                (*key, Arc::new(parsed))
            })
            .collect();
        // Drive owns the editor bundle; Workspace adds Calendar and Gmail.
        let mut drive = (*specs["google-drive"]).clone();
        drive["info"]["description"] =
            "Google Drive file operations and Docs, Sheets, and Slides editing through one Google OAuth connection. Editor paths declare their Google API servers and accept the full Drive scope, subject to file permissions. During the upgrade window, editor requests return workspace_destinations_not_activated (12300) until the operator enables GOOGLE_WORKSPACE_MULTI_ORIGIN_ENABLED after upgrading readers and node agents."
                .into();
        for key in ["google-docs", "google-sheets", "google-slides"] {
            let servers = specs[key]["servers"].clone();
            for (path, item) in specs[key]["paths"].as_object().expect("Product paths") {
                let mut item = item.clone();
                item["servers"] = servers.clone();
                assert!(
                    drive["paths"]
                        .as_object_mut()
                        .expect("Drive paths")
                        .insert(path.clone(), item)
                        .is_none(),
                    "Duplicate Drive path"
                );
            }
        }
        specs.insert("google-drive", Arc::new(drive.clone()));
        let mut workspace = drive;
        workspace["info"]["title"] = "Google Workspace".into();
        workspace["info"]["description"] =
            "Google Workspace uses one Google OAuth connection for Drive, Calendar, Gmail, Docs, Sheets, and Slides. The root server https://www.googleapis.com serves Drive, Calendar, and Gmail; Docs, Sheets, and Slides paths declare their respective https://docs.googleapis.com, https://sheets.googleapis.com, and https://slides.googleapis.com servers. Standard OpenAPI server precedence applies: operation servers override path servers, which override the root server. During the operator-controlled upgrade window, editor requests return workspace_destinations_not_activated (12300) until the operator enables GOOGLE_WORKSPACE_MULTI_ORIGIN_ENABLED after upgrading readers and node agents."
                .into();
        for key in ["google-calendar", "google-gmail"] {
            for (path, item) in specs[key]["paths"].as_object().expect("Product paths") {
                assert!(
                    workspace["paths"]
                        .as_object_mut()
                        .expect("Workspace paths")
                        .insert(path.clone(), item.clone())
                        .is_none(),
                    "Duplicate Workspace path"
                );
            }
        }
        specs.insert("google-workspace", Arc::new(workspace));
        specs
    },
);

/// Parsed overlay document for a spec key (the `{spec_key}` URL segment).
pub fn spec_for_key(spec_key: &str) -> Option<Arc<serde_json::Value>> {
    PARSED_SPECS.get(spec_key).cloned()
}

/// Spec key registered for a catalog service slug, if the slug is hydrated.
pub fn spec_key_for_slug(slug: &str) -> Option<&'static str> {
    SLUG_TO_SPEC_KEY
        .iter()
        .find(|(candidate, _)| *candidate == slug)
        .map(|(_, key)| *key)
}

/// Parsed overlay document for a catalog service slug.
pub fn spec_for_slug(slug: &str) -> Option<Arc<serde_json::Value>> {
    spec_key_for_slug(slug).and_then(spec_for_key)
}

/// Relative hosted path (`/api/v1/catalog-specs/{spec_key}/openapi.json`)
/// for a catalog service slug.
pub fn spec_path_for_slug(slug: &str) -> Option<String> {
    spec_key_for_slug(slug).map(|key| format!("{SPEC_PATH_PREFIX}{key}{SPEC_PATH_SUFFIX}"))
}

/// Parsed overlay document for a hosted URL path, used to short-circuit
/// spec fetches that point back at this deployment.
pub fn spec_for_url_path(path: &str) -> Option<Arc<serde_json::Value>> {
    let spec_key = path
        .strip_prefix(SPEC_PATH_PREFIX)?
        .strip_suffix(SPEC_PATH_SUFFIX)?;
    if spec_key.is_empty() || spec_key.contains('/') {
        return None;
    }
    spec_for_key(spec_key)
}

/// Catalog service slugs that have a hosted overlay.
pub fn hydrated_slugs() -> impl Iterator<Item = &'static str> {
    SLUG_TO_SPEC_KEY.iter().map(|(slug, _)| *slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frozen from 28fd2c44, including the original eight api-google operations.
    /// Additions are allowed; moving or editing any existing operation is not.
    #[test]
    fn existing_google_operation_contracts_are_additive() {
        use sha2::{Digest, Sha256};
        let frozen: serde_json::Value = serde_json::from_str(include_str!(
            "../../specs/fixtures/google-existing-operations.json"
        ))
        .unwrap();
        for (slug, operations) in frozen.as_object().unwrap() {
            let spec = spec_for_slug(slug).unwrap();
            for pinned in operations.as_array().unwrap() {
                let path = pinned["path"].as_str().unwrap();
                let method = pinned["method"].as_str().unwrap().to_ascii_lowercase();
                let operation = &spec["paths"][path][&method];
                assert_eq!(
                    operation["operationId"], pinned["operation_id"],
                    "{slug} {method} {path}"
                );
                let mut canonical = operation.clone();
                canonical.sort_all_objects();
                let digest = hex::encode(Sha256::digest(serde_json::to_vec(&canonical).unwrap()));
                assert_eq!(
                    digest, pinned["operation_sha256"],
                    "changed existing contract: {slug} {method} {path}"
                );
            }
        }
    }

    #[test]
    fn google_editor_operations_have_verified_origins_scopes_and_policies() {
        use crate::services::google_workspace::{DRIVE, GoogleProduct};
        let evidence: serde_json::Value = serde_json::from_str(include_str!(
            "../../specs/fixtures/google-editor-scope-acceptance.json"
        ))
        .unwrap();
        for (slug, proof) in evidence.as_object().unwrap() {
            let product = GoogleProduct::from_slug(slug).unwrap();
            let spec = spec_for_slug(slug).unwrap();
            assert_eq!(
                spec["servers"],
                serde_json::json!([{ "url": proof["origin"] }])
            );
            let policy = product.operation_policy().unwrap();
            let operations = proof["operations"].as_object().unwrap();
            assert_eq!(policy.rules.len(), operations.len());
            let defaults = product.default_scopes();
            let allowed = product.allowed_scopes();
            assert!(defaults.iter().any(|scope| scope == DRIVE));
            for (id, operation) in operations {
                let path = operation["path"].as_str().unwrap();
                let method = operation["method"].as_str().unwrap();
                assert_eq!(
                    spec["paths"][path][method.to_ascii_lowercase()]["operationId"],
                    *id
                );
                let scopes = operation["accepted_scopes"].as_array().unwrap();
                assert!(
                    scopes.iter().any(|scope| scope == DRIVE),
                    "{id} does not accept Drive"
                );
                assert!(
                    scopes
                        .iter()
                        .any(|scope| defaults.iter().any(|s| scope == s))
                );
                assert!(
                    scopes
                        .iter()
                        .any(|scope| allowed.iter().any(|s| scope == s))
                );
                let rule = policy
                    .rules
                    .iter()
                    .find(|rule| rule.method == method && rule.path_template == path)
                    .unwrap();
                if path.contains("{range}") {
                    assert_eq!(
                        rule.path_parameter_constraints.get("range"),
                        Some(
                            &crate::models::downstream_service::ProxyPathConstraint::SheetsA1Range
                        )
                    );
                } else {
                    assert!(rule.path_parameter_constraints.is_empty());
                }
            }
        }
    }
    use std::collections::HashSet;

    use crate::services::openapi_parser;

    #[test]
    fn every_embedded_spec_parses_as_openapi_with_operations() {
        for key in PARSED_SPECS.keys() {
            let spec = spec_for_key(key).expect("registered spec");
            assert!(
                spec.get("openapi").is_some(),
                "spec '{key}' missing openapi version"
            );
            let endpoints = openapi_parser::parse_openapi_spec_value(&spec)
                .unwrap_or_else(|error| panic!("spec '{key}' failed to parse: {error:?}"));
            assert!(!endpoints.is_empty(), "spec '{key}' has no operations");
        }
    }

    #[test]
    fn google_workspace_is_the_union_of_all_six_products() {
        let workspace = spec_for_slug("api-google-workspace").unwrap();
        let drive = spec_for_slug("api-google-drive").unwrap();
        let calendar = spec_for_slug("api-google-calendar").unwrap();
        let gmail = spec_for_slug("api-google-gmail").unwrap();
        let paths = workspace["paths"].as_object().unwrap();
        assert_eq!(
            paths.len(),
            drive["paths"].as_object().unwrap().len()
                + calendar["paths"].as_object().unwrap().len()
                + gmail["paths"].as_object().unwrap().len()
        );
        assert_eq!(workspace["servers"][0]["url"], "https://www.googleapis.com");
        assert_eq!(
            openapi_parser::parse_openapi_spec_value(&drive)
                .unwrap()
                .len(),
            22
        );
        assert_eq!(
            crate::services::openapi_parser::parse_openapi_spec_value(&workspace)
                .unwrap()
                .len(),
            38
        );
        for key in ["google-docs", "google-sheets", "google-slides"] {
            let product = spec_for_key(key).unwrap();
            for (path, item) in product["paths"].as_object().unwrap() {
                let mut expected = item.clone();
                expected["servers"] = product["servers"].clone();
                assert_eq!(paths[path], expected);
                assert_eq!(drive["paths"][path], expected);
            }
        }
        for spec in [drive, calendar, gmail] {
            for (path, item) in spec["paths"].as_object().unwrap() {
                assert_eq!(&paths[path], item);
            }
        }
    }

    /// Aevatar's workflow admission proof builder
    /// (`NyxIdOperationAdmissionProofBuilder`) accepts only this fixed
    /// schema-keyword whitelist and drops the whole endpoint on any other
    /// keyword, an unresolved `$ref`, or a union `type` array. Every
    /// overlay schema must stay inside the subset or the operation becomes
    /// invisible to workflow binding (see NyxID#1296).
    const AEVATAR_SCHEMA_KEYWORDS: &[&str] = &[
        "type",
        "enum",
        "properties",
        "required",
        "items",
        "additionalProperties",
        "title",
        "description",
        "default",
        "example",
        "examples",
        "deprecated",
    ];

    fn assert_admissible_schema(spec_key: &str, context: &str, schema: &serde_json::Value) {
        let Some(object) = schema.as_object() else {
            return;
        };
        for (key, value) in object {
            assert!(
                AEVATAR_SCHEMA_KEYWORDS.contains(&key.as_str()),
                "spec '{spec_key}' {context}: keyword '{key}' is outside the aevatar admission subset"
            );
            match key.as_str() {
                "type" => assert!(
                    value.is_string(),
                    "spec '{spec_key}' {context}: union type arrays are inadmissible"
                ),
                "properties" | "additionalProperties" | "items" => {
                    if let Some(children) = value.as_object() {
                        if key == "properties" {
                            for (name, child) in children {
                                assert_admissible_schema(
                                    spec_key,
                                    &format!("{context}.{name}"),
                                    child,
                                );
                            }
                        } else {
                            assert_admissible_schema(spec_key, context, value);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    #[test]
    fn every_schema_stays_inside_aevatar_admission_subset() {
        for (key, _) in HOSTED_SPEC_SOURCES {
            let spec = spec_for_key(key).expect("registered spec");
            assert!(
                spec.pointer("/components/schemas").is_none(),
                "spec '{key}' must inline schemas; aevatar rejects unresolved $refs"
            );
            let paths = spec
                .get("paths")
                .and_then(|paths| paths.as_object())
                .unwrap_or_else(|| panic!("spec '{key}' missing paths"));
            for (path, item) in paths {
                let Some(item) = item.as_object() else {
                    continue;
                };
                for method in ["get", "post", "put", "patch", "delete"] {
                    let Some(operation) = item.get(method) else {
                        continue;
                    };
                    assert!(
                        !serde_json::to_string(operation)
                            .expect("serialize")
                            .contains("$ref"),
                        "spec '{key}' {method} {path} contains a $ref"
                    );
                    for param in operation
                        .get("parameters")
                        .and_then(|params| params.as_array())
                        .into_iter()
                        .flatten()
                    {
                        if let Some(schema) = param.get("schema") {
                            let name = param.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                            assert_admissible_schema(
                                key,
                                &format!("{method} {path} param {name}"),
                                schema,
                            );
                        }
                    }
                    if let Some(content) = operation
                        .pointer("/requestBody/content")
                        .and_then(|content| content.as_object())
                    {
                        for media in content.values() {
                            if let Some(schema) = media.get("schema") {
                                assert_admissible_schema(
                                    key,
                                    &format!("{method} {path} body"),
                                    schema,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_operation_has_unique_operation_id_and_aevatar_marker() {
        for (key, _) in HOSTED_SPEC_SOURCES {
            let spec = spec_for_key(key).expect("registered spec");
            let paths = spec
                .get("paths")
                .and_then(|paths| paths.as_object())
                .unwrap_or_else(|| panic!("spec '{key}' missing paths"));

            let mut operation_ids = HashSet::new();
            for (path, item) in paths {
                let Some(item) = item.as_object() else {
                    continue;
                };
                for method in ["get", "post", "put", "patch", "delete"] {
                    let Some(operation) = item.get(method) else {
                        continue;
                    };
                    let operation_id = operation
                        .get("operationId")
                        .and_then(|id| id.as_str())
                        .unwrap_or_else(|| {
                            panic!("spec '{key}' {method} {path} missing operationId")
                        });
                    assert!(
                        operation_ids.insert(operation_id.to_string()),
                        "spec '{key}' duplicate operationId '{operation_id}'"
                    );
                    let marker = operation.get("x-aevatar-tool").unwrap_or_else(|| {
                        panic!("spec '{key}' {method} {path} missing x-aevatar-tool")
                    });
                    assert!(
                        marker.get("readOnly").is_some_and(|v| v.is_boolean()),
                        "spec '{key}' {method} {path} marker missing readOnly"
                    );
                }
            }
        }
    }

    #[test]
    fn every_slug_mapping_targets_a_registered_spec() {
        for (slug, spec_key) in SLUG_TO_SPEC_KEY {
            assert!(
                spec_for_key(spec_key).is_some(),
                "slug '{slug}' maps to unregistered spec key '{spec_key}'"
            );
        }
    }

    #[test]
    fn spec_for_url_path_resolves_hosted_paths_only() {
        assert!(spec_for_url_path("/api/v1/catalog-specs/firecrawl/openapi.json").is_some());
        assert!(spec_for_url_path("/api/v1/catalog-specs/elevenlabs/openapi.json").is_some());
        assert!(spec_for_url_path("/api/v1/catalog-specs/twilio/openapi.json").is_some());
        assert!(spec_for_url_path("/api/v1/catalog-specs/lark-bot/openapi.json").is_some());
        assert!(spec_for_url_path("/api/v1/catalog-specs/unknown/openapi.json").is_none());
        assert!(spec_for_url_path("/api/v1/catalog-specs//openapi.json").is_none());
        assert!(spec_for_url_path("/api/v1/catalog-specs/a/b/openapi.json").is_none());
        assert!(spec_for_url_path("/other/firecrawl/openapi.json").is_none());
    }

    #[test]
    fn spec_path_for_slug_builds_hosted_path() {
        assert_eq!(
            spec_path_for_slug("api-firecrawl").as_deref(),
            Some("/api/v1/catalog-specs/firecrawl/openapi.json")
        );
        assert_eq!(
            spec_path_for_slug("api-github-pat").as_deref(),
            Some("/api/v1/catalog-specs/github/openapi.json")
        );
        assert_eq!(
            spec_path_for_slug("api-elevenlabs").as_deref(),
            Some("/api/v1/catalog-specs/elevenlabs/openapi.json")
        );
        assert_eq!(
            spec_path_for_slug("api-twilio").as_deref(),
            Some("/api/v1/catalog-specs/twilio/openapi.json")
        );
        assert!(spec_path_for_slug("llm-openclaw").is_none());
    }
}
