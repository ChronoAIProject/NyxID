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
        "twitch",
        include_str!("../../specs/catalog/twitch.openapi.json"),
    ),
    (
        "twitter",
        include_str!("../../specs/catalog/twitter.openapi.json"),
    ),
];

/// Catalog service slug -> spec key.
const SLUG_TO_SPEC_KEY: &[(&str, &str)] = &[
    ("api-discord", "discord"),
    ("api-discord-bot", "discord-bot"),
    ("api-facebook", "facebook"),
    ("api-feishu", "lark"),
    ("api-feishu-bot", "lark-bot"),
    ("api-firecrawl", "firecrawl"),
    ("api-github", "github"),
    ("api-github-pat", "github"),
    ("api-google", "google"),
    ("api-lark", "lark"),
    ("api-lark-bot", "lark-bot"),
    ("api-microsoft", "microsoft-graph"),
    ("api-reddit", "reddit"),
    ("api-slack", "slack"),
    ("api-slack-bot", "slack"),
    ("api-spotify", "spotify"),
    ("api-telegram-bot", "telegram-bot"),
    ("api-twitch", "twitch"),
    ("api-twitter", "twitter"),
    ("llm-anthropic", "anthropic"),
    ("llm-cohere", "cohere"),
    ("llm-deepseek", "deepseek"),
    ("llm-google-ai", "google-ai"),
    ("llm-mistral", "mistral"),
    ("llm-openai", "openai"),
    ("llm-openrouter", "openrouter"),
];

static PARSED_SPECS: LazyLock<HashMap<&'static str, Arc<serde_json::Value>>> =
    LazyLock::new(|| {
        HOSTED_SPEC_SOURCES
            .iter()
            .map(|(key, source)| {
                let parsed =
                    serde_json::from_str::<serde_json::Value>(source).unwrap_or_else(|error| {
                        panic!("embedded catalog spec '{key}' is not valid JSON: {error}")
                    });
                (*key, Arc::new(parsed))
            })
            .collect()
    });

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
    use std::collections::HashSet;

    use super::*;
    use crate::services::openapi_parser;

    #[test]
    fn every_embedded_spec_parses_as_openapi_with_operations() {
        for (key, _) in HOSTED_SPEC_SOURCES {
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
        assert!(spec_path_for_slug("llm-openclaw").is_none());
    }
}
