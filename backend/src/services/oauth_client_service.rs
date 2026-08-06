use chrono::{NaiveDate, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Binary, Bson, Document, doc, spec::BinarySubtype};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use url::Url;
use uuid::Uuid;

use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::authorization_code::COLLECTION_NAME as AUTH_CODES;
use crate::models::consent::COLLECTION_NAME as CONSENTS;
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::models::refresh_token::COLLECTION_NAME as REFRESH_TOKENS;

/// Standard OIDC scope that requests refresh-token issuance.
pub const OFFLINE_ACCESS_SCOPE: &str = "offline_access";

/// Known scopes supported by NyxID. Used for validation of
/// `allowed_scopes` on OAuth clients.
///
/// The list mixes OIDC-standard scopes (openid, profile, email, roles,
/// groups, offline_access) with NyxID-specific extensions (proxy,
/// urn:nyxid:scope:*).
/// `urn:nyxid:scope:broker_binding` opts a client into the OAuth broker
/// pattern when present in their allowed_scopes.
pub const KNOWN_OIDC_SCOPES: &[&str] = &[
    "openid",
    "profile",
    "email",
    "roles",
    "groups",
    OFFLINE_ACCESS_SCOPE,
    "proxy",
    "urn:nyxid:scope:broker_binding",
];

/// Default allowed scopes for new OAuth clients.
pub const DEFAULT_ALLOWED_SCOPES: &str = "openid profile email";

/// Delegation scopes OAuth clients may request through RFC 8693 token
/// exchange. `account:read` is intentionally absent: that capability is
/// confined to admin-configured downstream/user service rows.
pub const OAUTH_CLIENT_DELEGATION_SCOPES: &[&str] = &["llm:proxy", "proxy:*", "llm:status"];

pub fn validate_oauth_client_delegation_scopes(scopes: &str) -> AppResult<()> {
    for scope in scopes.split_whitespace() {
        if !OAUTH_CLIENT_DELEGATION_SCOPES.contains(&scope) {
            return Err(AppError::ValidationError(format!(
                "Invalid delegation scope '{}'. Must be one of: {}",
                scope,
                OAUTH_CLIENT_DELEGATION_SCOPES.join(", ")
            )));
        }
    }
    Ok(())
}

/// Default scopes for the built-in MCP OAuth client and dynamic registrations.
///
/// Includes `roles` and `groups` so MCP clients (Cursor, Claude Code, Codex,
/// etc.) that request RBAC claims pass scope validation. Token issuance is
/// still gated by what the client requests at `/oauth/authorize` and what the
/// user consents to.
pub const DEFAULT_MCP_ALLOWED_SCOPES: &str =
    "openid profile email roles groups proxy offline_access";

pub const ADMIN_CLIENT_TYPE_FILTERS: &[&str] = &["public", "confidential", "other"];
pub const ADMIN_CREATOR_TYPE_FILTERS: &[&str] =
    &["dynamic_registration", "system", "owned", "ownerless"];
pub const ADMIN_BROKER_FILTERS: &[&str] = &["enabled", "disabled", "flag", "scope"];

/// Filters that accept free-text values alongside their fixed options, mapped to
/// the stored field the text is matched against with a case-insensitive
/// `contains`. Only filters backed by a single string column can appear here:
/// `is_active` is a boolean, `broker` is derived from several fields, and
/// `created_at` is a date, so none of them has a column a substring could match.
pub const ADMIN_CUSTOM_TEXT_FILTERS: &[(&str, &str)] = &[
    ("client_type", "client_type"),
    ("creator_type", "created_by"),
    ("scope", "allowed_scopes"),
];

/// Stored field a custom-text filter searches, or `None` when the filter takes
/// no free text.
pub fn admin_custom_text_field(filter: &str) -> Option<&'static str> {
    ADMIN_CUSTOM_TEXT_FILTERS
        .iter()
        .find(|(key, _)| *key == filter)
        .map(|(_, stored)| *stored)
}
pub const ADMIN_SEARCH_FIELDS: &[(&str, &str)] = &[
    ("client", "Client"),
    ("client_type", "Client type"),
    ("created_by", "Created by"),
    ("allowed_scopes", "Allowed scopes"),
];
const ADMIN_STATUS_FILTERS: &[&str] = &["true", "false"];
pub const ADMIN_SORT_OPTIONS: &[&str] = &[
    "-created_at",
    "created_at",
    "client_name",
    "-client_name",
    "client_type",
    "-client_type",
    "created_by",
    "-created_by",
    "broker",
    "-broker",
    "-is_active",
    "is_active",
    "allowed_scopes",
    "-allowed_scopes",
];

const ADMIN_SEARCH_MAX_CHARS: usize = 256;
const ADMIN_SEARCH_FILTER_MAX_GROUPS: usize = 5;
const ADMIN_SEARCH_FILTER_MAX_VALUES_PER_GROUP: usize = 8;
const ADMIN_SEARCH_FILTER_MAX_TOTAL_VALUES: usize = 32;
const ADMIN_CREATED_DATES_MAX_VALUES: usize = 32;
const ADMIN_BROKER_SORT_FIELD: &str = "__nyxid_admin_broker_sort";

/// Validate and canonicalize `allowed_scopes`.
///
/// - Every scope must be in [`KNOWN_OIDC_SCOPES`].
/// - `openid` is always required (auto-prepended if missing).
/// - Duplicates are removed.
/// - Returns a deduplicated, space-separated string.
pub fn validate_allowed_scopes(scopes: &str) -> AppResult<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    for s in scopes.split_whitespace() {
        if !KNOWN_OIDC_SCOPES.contains(&s) {
            return Err(AppError::ValidationError(format!(
                "Unknown OIDC scope '{s}'. Must be one of: {}",
                KNOWN_OIDC_SCOPES.join(", ")
            )));
        }
        if seen.insert(s) {
            out.push(s);
        }
    }

    // openid is mandatory per OIDC spec
    if !seen.contains("openid") {
        out.insert(0, "openid");
    }

    Ok(out.join(" "))
}

/// Validate and canonicalize `allowed_scopes` supplied as an API list.
///
/// An explicit empty list is normalized to `openid`, while omission should be
/// handled by the caller when the endpoint wants to apply
/// [`DEFAULT_ALLOWED_SCOPES`].
pub fn validate_allowed_scopes_list(scopes: &[String]) -> AppResult<String> {
    validate_allowed_scopes(&scopes.join(" "))
}

/// Validate, trim, and deduplicate registered OAuth redirect URIs.
///
/// Allows HTTPS, localhost HTTP, loopback/custom app schemes, and other
/// schemes historically accepted by NyxID developer apps, but rejects obvious
/// browser execution/local-file schemes and fragments. Returns the exact
/// trimmed URI strings rather than `url`-serialized strings because OAuth
/// redirect URI matching is exact.
pub fn validate_redirect_uris(redirect_uris: &[String]) -> AppResult<Vec<String>> {
    if redirect_uris.is_empty() {
        return Err(AppError::ValidationError(
            "At least one redirect_uri is required".to_string(),
        ));
    }

    let mut unique = HashSet::new();
    let mut validated = Vec::new();

    for raw_uri in redirect_uris {
        let uri = raw_uri.trim();
        if uri.is_empty() {
            return Err(AppError::ValidationError(
                "redirect_uri cannot be empty".to_string(),
            ));
        }

        let parsed = Url::parse(uri).map_err(|_| {
            AppError::ValidationError(format!("Invalid redirect_uri format: {uri}"))
        })?;

        if matches!(parsed.scheme(), "javascript" | "data" | "file") {
            return Err(AppError::ValidationError(format!(
                "Unsupported redirect_uri scheme: {uri}"
            )));
        }

        if parsed.fragment().is_some() {
            return Err(AppError::ValidationError(format!(
                "redirect_uri must not contain fragment: {uri}"
            )));
        }

        let trimmed = uri.to_string();
        if unique.insert(trimmed.clone()) {
            validated.push(trimmed);
        }
    }

    Ok(validated)
}

/// Well-known client ID for native MCP clients (Cursor, Claude Code, etc.).
const MCP_CLIENT_ID: &str = "nyx-mcp";

/// Seed default OAuth clients at startup (idempotent).
///
/// Creates the `nyx-mcp` public client used by MCP desktop apps. The client
/// has no registered redirect URIs because loopback URIs are validated
/// dynamically per RFC 8252 section 7.3.
pub async fn seed_default_clients(db: &mongodb::Database) -> AppResult<()> {
    let collection = db.collection::<OauthClient>(OAUTH_CLIENTS);

    if let Some(existing) = collection.find_one(doc! { "_id": MCP_CLIENT_ID }).await? {
        if let Some(updated_scopes) = merge_missing_default_mcp_scopes(&existing.allowed_scopes)? {
            collection
                .update_one(
                    doc! { "_id": MCP_CLIENT_ID },
                    doc! { "$set": {
                        "allowed_scopes": &updated_scopes,
                        "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    }},
                )
                .await?;

            tracing::info!(
                allowed_scopes = %updated_scopes,
                "Upgraded default MCP OAuth client to include latest default scopes"
            );
        }

        return Ok(());
    }

    let now = Utc::now();
    let client = OauthClient {
        id: MCP_CLIENT_ID.to_string(),
        client_name: "NyxID MCP Client".to_string(),
        client_secret_hash: "NONE".to_string(),
        redirect_uris: vec![],
        allowed_scopes: DEFAULT_MCP_ALLOWED_SCOPES.to_string(),
        scope_provenance: crate::models::oauth_client::ScopeProvenance::Defaulted,
        grant_types: "authorization_code".to_string(),
        client_type: "public".to_string(),
        is_active: true,
        delegation_scopes: String::new(),
        default_service_catalog_slugs: Vec::new(),
        broker_capability_enabled: false,
        revocation_webhook_url: None,
        revocation_webhook_secret_encrypted: None,
        connection_webhook_url: None,
        connection_webhook_secret_encrypted: None,
        connection_webhook_enabled: false,
        created_by: Some("system".to_string()),
        created_at: now,
        updated_at: now,
    };

    collection.insert_one(&client).await?;
    tracing::info!("Seeded default MCP OAuth client (id={MCP_CLIENT_ID})");

    Ok(())
}

/// If `existing` is missing any scope from [`DEFAULT_MCP_ALLOWED_SCOPES`],
/// returns the merged, validated, canonical scope string. Returns `None` when
/// the existing scopes already cover the defaults (so callers can skip the
/// write).
fn merge_missing_default_mcp_scopes(existing: &str) -> AppResult<Option<String>> {
    let existing_set: std::collections::HashSet<&str> = existing.split_whitespace().collect();
    let missing: Vec<&str> = DEFAULT_MCP_ALLOWED_SCOPES
        .split_whitespace()
        .filter(|scope| !existing_set.contains(scope))
        .collect();

    if missing.is_empty() {
        return Ok(None);
    }

    let merged = format!("{existing} {}", missing.join(" "));
    Ok(Some(validate_allowed_scopes(&merged)?))
}

/// Backfill default MCP scopes onto OAuth clients created via Dynamic Client
/// Registration before the current scope set landed.
///
/// DCR is used by MCP clients (Cursor, Claude Code, Codex, etc.). Whenever
/// [`DEFAULT_MCP_ALLOWED_SCOPES`] grows, older DCR records would otherwise
/// fail authorization with `invalid_scope` (issue #434 was triggered by Codex
/// requesting `roles`/`groups`). This sweep upgrades them in place so existing
/// client_id caches keep working without re-registration.
///
/// Idempotent: clients that already cover the default set are skipped.
pub async fn migrate_dynamic_clients_grant_default_mcp_scopes(
    db: &mongodb::Database,
) -> AppResult<()> {
    let collection = db.collection::<OauthClient>(OAUTH_CLIENTS);

    let candidates: Vec<OauthClient> = collection
        .find(doc! { "created_by": "dynamic_registration" })
        .await?
        .try_collect()
        .await?;

    if candidates.is_empty() {
        return Ok(());
    }

    let now = bson::DateTime::from_chrono(Utc::now());
    let mut upgraded = 0_usize;

    for client in &candidates {
        let Some(updated_scopes) = merge_missing_default_mcp_scopes(&client.allowed_scopes)? else {
            continue;
        };

        collection
            .update_one(
                doc! { "_id": &client.id },
                doc! { "$set": {
                    "allowed_scopes": &updated_scopes,
                    "updated_at": now,
                }},
            )
            .await?;

        upgraded += 1;
    }

    if upgraded > 0 {
        tracing::info!(
            upgraded,
            "Backfilled default MCP scopes on dynamic-registration OAuth clients"
        );
    }

    Ok(())
}

/// Create a new OAuth client.
///
/// Returns the persisted client and, for confidential clients, the raw client
/// secret (which is only available at creation time -- only the hash is stored).
///
/// `allowed_scopes` must contain only known OIDC scopes (validated by the
/// caller). Pass [`DEFAULT_ALLOWED_SCOPES`] for the standard set.
#[allow(clippy::too_many_arguments)]
pub async fn create_client(
    db: &mongodb::Database,
    name: &str,
    redirect_uris: &[String],
    client_type: &str,
    created_by: &str,
    delegation_scopes: &str,
    allowed_scopes: &str,
    scope_provenance: crate::models::oauth_client::ScopeProvenance,
    broker_capability_enabled: bool,
    revocation_webhook_url: Option<&str>,
    revocation_webhook_secret_encrypted: Option<Vec<u8>>,
    default_service_catalog_slugs: &[String],
) -> AppResult<(OauthClient, Option<String>)> {
    let client_id = Uuid::new_v4().to_string();
    let now = Utc::now();

    let (secret_hash, raw_secret) = if client_type == "confidential" {
        let secret = generate_random_token();
        let hash = hash_token(&secret);
        (hash, Some(secret))
    } else {
        ("NONE".to_string(), None)
    };

    let client = OauthClient {
        id: client_id,
        client_name: name.to_string(),
        client_secret_hash: secret_hash,
        redirect_uris: redirect_uris.to_vec(),
        allowed_scopes: allowed_scopes.to_string(),
        scope_provenance,
        grant_types: "authorization_code".to_string(),
        client_type: client_type.to_string(),
        is_active: true,
        delegation_scopes: delegation_scopes.to_string(),
        default_service_catalog_slugs: default_service_catalog_slugs.to_vec(),
        broker_capability_enabled,
        revocation_webhook_url: revocation_webhook_url.map(str::to_string),
        revocation_webhook_secret_encrypted,
        connection_webhook_url: None,
        connection_webhook_secret_encrypted: None,
        connection_webhook_enabled: false,
        created_by: Some(created_by.to_string()),
        created_at: now,
        updated_at: now,
    };

    db.collection::<OauthClient>(OAUTH_CLIENTS)
        .insert_one(&client)
        .await?;

    Ok((client, raw_secret))
}

/// Validated filters for the platform-admin OAuth-client list.
pub struct AdminOAuthClientListParams<'a> {
    pub page: u64,
    pub per_page: u64,
    pub search: Option<&'a str>,
    pub search_filters: Option<&'a str>,
    pub custom_filters: Option<&'a str>,
    pub client_type: Option<&'a str>,
    pub creator_type: Option<&'a str>,
    pub broker: Option<&'a str>,
    pub is_active: Option<&'a str>,
    pub scope: Option<&'a str>,
    pub created_dates: Option<&'a str>,
    pub created_from: Option<&'a str>,
    pub created_to: Option<&'a str>,
    pub sort: &'a str,
    pub broker_require_admin_capability: bool,
}

/// List every OAuth client using the legacy newest-first ordering.
pub async fn list_clients_legacy(db: &mongodb::Database) -> AppResult<Vec<OauthClient>> {
    Ok(db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find(doc! {})
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?)
}

/// List OAuth clients using bounded, server-side admin-table controls.
pub async fn list_clients(
    db: &mongodb::Database,
    params: AdminOAuthClientListParams<'_>,
) -> AppResult<(Vec<OauthClient>, u64)> {
    if params.page == 0 || !(1..=100).contains(&params.per_page) {
        return Err(AppError::ValidationError(
            "page must be at least 1 and per_page must be between 1 and 100".to_string(),
        ));
    }

    let offset = params
        .page
        .checked_sub(1)
        .and_then(|page| page.checked_mul(params.per_page))
        .ok_or_else(|| AppError::ValidationError("page is too large".to_string()))?;
    let filter = admin_oauth_client_filter(&params)?;
    let sort = admin_oauth_client_sort(params.sort)?;
    let collection = db.collection::<OauthClient>(OAUTH_CLIENTS);

    let total = collection.count_documents(filter.clone()).await?;
    if offset >= total {
        return Ok((Vec::new(), total));
    }
    if i64::try_from(offset).is_err() {
        return Err(AppError::ValidationError("page is too large".to_string()));
    }

    let clients: Vec<OauthClient> = if sort.contains_key(ADMIN_BROKER_SORT_FIELD) {
        let offset = i64::try_from(offset)
            .map_err(|_| AppError::ValidationError("page is too large".to_string()))?;
        let pipeline = admin_broker_sort_pipeline(
            filter,
            sort,
            offset,
            params.per_page as i64,
            params.broker_require_admin_capability,
        );
        collection
            .aggregate(pipeline)
            .with_type::<OauthClient>()
            .allow_disk_use(true)
            .await?
            .try_collect()
            .await?
    } else {
        collection
            .find(filter)
            .sort(sort)
            .skip(offset)
            .limit(params.per_page as i64)
            .await?
            .try_collect()
            .await?
    };

    Ok((clients, total))
}

fn admin_oauth_client_filter(params: &AdminOAuthClientListParams<'_>) -> AppResult<Document> {
    let mut clauses = Vec::new();

    if let Some(search) = params
        .search
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if search.chars().count() > ADMIN_SEARCH_MAX_CHARS {
            return Err(AppError::ValidationError(format!(
                "search must be {ADMIN_SEARCH_MAX_CHARS} characters or less"
            )));
        }
        let escaped = regex::escape(search);
        clauses.push(doc! {
            "$or": [
                { "client_name": { "$regex": &escaped, "$options": "i" } },
                { "_id": { "$regex": &escaped, "$options": "i" } },
                { "client_type": { "$regex": &escaped, "$options": "i" } },
                { "created_by": { "$regex": &escaped, "$options": "i" } },
                { "allowed_scopes": { "$regex": &escaped, "$options": "i" } },
            ]
        });
    }

    if let Some(search_filters) = params.search_filters {
        clauses.extend(admin_search_filter_clauses(search_filters)?);
    }

    // Free text typed into a filter widens that filter rather than narrowing it:
    // the custom `contains` branches are OR'd into the same clause as that
    // filter's checked options, so `Type is public or contains "acme"` works.
    // A filter carrying only custom text still emits its clause.
    let mut custom = admin_custom_filter_branches(params.custom_filters)?;
    let mut push_filter = |clauses: &mut Vec<Document>, key: &str, options: Vec<Document>| {
        let mut branches = options;
        branches.extend(custom.remove(key).unwrap_or_default());
        if !branches.is_empty() {
            clauses.push(one_or_many(branches));
        }
    };

    push_filter(
        &mut clauses,
        "client_type",
        match params.client_type {
            None => Vec::new(),
            Some(client_type) => {
                admin_csv_filter_values("client_type", client_type, ADMIN_CLIENT_TYPE_FILTERS)?
                    .into_iter()
                    .map(|value| match value {
                        "public" | "confidential" => doc! { "client_type": value },
                        "other" => {
                            doc! { "client_type": { "$nin": ["public", "confidential"] } }
                        }
                        _ => unreachable!("client_type was validated against its fixed domain"),
                    })
                    .collect()
            }
        },
    );

    push_filter(
        &mut clauses,
        "creator_type",
        match params.creator_type {
            None => Vec::new(),
            Some(creator_type) => {
                admin_csv_filter_values("creator_type", creator_type, ADMIN_CREATOR_TYPE_FILTERS)?
                    .into_iter()
                    .map(|value| match value {
                        "dynamic_registration" | "system" => doc! { "created_by": value },
                        "owned" => doc! {
                            "created_by": {
                                "$exists": true,
                                "$nin": [
                                    Bson::Null,
                                    Bson::String("dynamic_registration".to_string()),
                                    Bson::String("system".to_string()),
                                ]
                            }
                        },
                        "ownerless" => doc! { "created_by": Bson::Null },
                        _ => unreachable!("creator_type was validated against its fixed domain"),
                    })
                    .collect()
            }
        },
    );

    if let Some(broker) = params.broker {
        let values = admin_csv_filter_values("broker", broker, ADMIN_BROKER_FILTERS)?;
        let branches = values
            .into_iter()
            .map(|value| admin_broker_filter(value, params.broker_require_admin_capability))
            .collect::<AppResult<Vec<_>>>()?;
        clauses.push(one_or_many(branches));
    }

    if let Some(is_active) = params.is_active {
        let values = admin_csv_filter_values("is_active", is_active, ADMIN_STATUS_FILTERS)?;
        if values.len() == 1 {
            clauses.push(doc! { "is_active": values[0] == "true" });
        }
    }

    push_filter(
        &mut clauses,
        "scope",
        match params.scope {
            None => Vec::new(),
            Some(scope) => admin_csv_filter_values("scope", scope, KNOWN_OIDC_SCOPES)?
                .into_iter()
                .map(|value| scope_token_filter(value, true))
                .collect(),
        },
    );

    if let Some(created_at) =
        admin_created_at_filter(params.created_dates, params.created_from, params.created_to)?
    {
        clauses.push(created_at);
    }

    Ok(match clauses.len() {
        0 => doc! {},
        1 => clauses.remove(0),
        _ => doc! { "$and": clauses },
    })
}

/// Parse the `custom_filters` query param into per-filter `contains` branches.
///
/// Shape: `{"client_type":["acme"],"creator_type":["d0d7b72a"]}`. Values are
/// regex-escaped and matched case-insensitively against the stored field the
/// filter maps to ([`ADMIN_CUSTOM_TEXT_FILTERS`]). Bounds mirror `search_filters`.
fn admin_custom_filter_branches(
    raw: Option<&str>,
) -> AppResult<HashMap<&'static str, Vec<Document>>> {
    let mut branches: HashMap<&'static str, Vec<Document>> = HashMap::new();
    let Some(raw) = raw else {
        return Ok(branches);
    };

    let groups: HashMap<String, Vec<String>> = serde_json::from_str(raw).map_err(|_| {
        AppError::ValidationError(
            "custom_filters must be a JSON object of filter keys to value arrays".to_string(),
        )
    })?;

    if groups.len() > ADMIN_SEARCH_FILTER_MAX_GROUPS {
        return Err(AppError::ValidationError(format!(
            "custom_filters must contain at most {ADMIN_SEARCH_FILTER_MAX_GROUPS} filters"
        )));
    }

    let mut total_values = 0usize;
    for (filter, values) in groups {
        let Some(stored_field) = admin_custom_text_field(&filter) else {
            return Err(AppError::ValidationError(format!(
                "custom_filters filter must be one of: {}",
                ADMIN_CUSTOM_TEXT_FILTERS
                    .iter()
                    .map(|(key, _)| *key)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };
        if values.is_empty() {
            return Err(AppError::ValidationError(format!(
                "custom_filters values for {filter} must not be empty"
            )));
        }
        if values.len() > ADMIN_SEARCH_FILTER_MAX_VALUES_PER_GROUP {
            return Err(AppError::ValidationError(format!(
                "custom_filters values for {filter} must contain at most {ADMIN_SEARCH_FILTER_MAX_VALUES_PER_GROUP} entries"
            )));
        }
        total_values = total_values.checked_add(values.len()).ok_or_else(|| {
            AppError::ValidationError("custom_filters contains too many values".to_string())
        })?;
        if total_values > ADMIN_SEARCH_FILTER_MAX_TOTAL_VALUES {
            return Err(AppError::ValidationError(format!(
                "custom_filters must contain at most {ADMIN_SEARCH_FILTER_MAX_TOTAL_VALUES} values"
            )));
        }

        let entry = branches.entry(stored_field_key(&filter)).or_default();
        for raw_value in values {
            let value = raw_value.trim();
            if value.is_empty() {
                return Err(AppError::ValidationError(format!(
                    "custom_filters values for {filter} must not be empty"
                )));
            }
            if value.chars().count() > ADMIN_SEARCH_MAX_CHARS {
                return Err(AppError::ValidationError(format!(
                    "custom_filters values must be {ADMIN_SEARCH_MAX_CHARS} characters or less"
                )));
            }
            entry.push(doc! {
                stored_field: {
                    "$regex": regex::escape(value),
                    "$options": "i",
                }
            });
        }
    }

    Ok(branches)
}

/// Borrow the `'static` filter key so the branch map outlives the parsed JSON.
fn stored_field_key(filter: &str) -> &'static str {
    ADMIN_CUSTOM_TEXT_FILTERS
        .iter()
        .find(|(key, _)| *key == filter)
        .map(|(key, _)| *key)
        .expect("filter was validated against ADMIN_CUSTOM_TEXT_FILTERS")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdminSearchFilterInput {
    field: String,
    values: Vec<String>,
}

fn admin_search_filter_clauses(raw: &str) -> AppResult<Vec<Document>> {
    let groups: Vec<AdminSearchFilterInput> = serde_json::from_str(raw).map_err(|_| {
        AppError::ValidationError(
            "search_filters must be a JSON array of field and values objects".to_string(),
        )
    })?;

    if groups.is_empty() {
        return Err(AppError::ValidationError(
            "search_filters must contain at least one field group".to_string(),
        ));
    }
    if groups.len() > ADMIN_SEARCH_FILTER_MAX_GROUPS {
        return Err(AppError::ValidationError(format!(
            "search_filters must contain at most {ADMIN_SEARCH_FILTER_MAX_GROUPS} field groups"
        )));
    }

    let mut seen_fields = HashSet::new();
    let mut total_values = 0usize;
    let mut clauses = Vec::with_capacity(groups.len());

    for group in groups {
        let stored_field = match group.field.as_str() {
            "client" => None,
            "client_type" => Some("client_type"),
            "created_by" => Some("created_by"),
            "allowed_scopes" => Some("allowed_scopes"),
            _ => {
                return Err(AppError::ValidationError(format!(
                    "search_filters field must be one of: {}",
                    ADMIN_SEARCH_FIELDS
                        .iter()
                        .map(|(key, _)| *key)
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        };

        if !seen_fields.insert(group.field.clone()) {
            return Err(AppError::ValidationError(format!(
                "search_filters must not repeat field {}",
                group.field
            )));
        }
        if group.values.is_empty() {
            return Err(AppError::ValidationError(format!(
                "search_filters values for {} must not be empty",
                group.field
            )));
        }
        if group.values.len() > ADMIN_SEARCH_FILTER_MAX_VALUES_PER_GROUP {
            return Err(AppError::ValidationError(format!(
                "search_filters values for {} must contain at most {ADMIN_SEARCH_FILTER_MAX_VALUES_PER_GROUP} entries",
                group.field
            )));
        }

        total_values = total_values
            .checked_add(group.values.len())
            .ok_or_else(|| {
                AppError::ValidationError("search_filters contains too many values".to_string())
            })?;
        if total_values > ADMIN_SEARCH_FILTER_MAX_TOTAL_VALUES {
            return Err(AppError::ValidationError(format!(
                "search_filters must contain at most {ADMIN_SEARCH_FILTER_MAX_TOTAL_VALUES} values"
            )));
        }

        let mut value_clauses = Vec::with_capacity(group.values.len());
        for raw_value in group.values {
            let value = raw_value.trim();
            if value.is_empty() {
                return Err(AppError::ValidationError(format!(
                    "search_filters values for {} must not be empty",
                    group.field
                )));
            }
            if value.chars().count() > ADMIN_SEARCH_MAX_CHARS {
                return Err(AppError::ValidationError(format!(
                    "search_filters values must be {ADMIN_SEARCH_MAX_CHARS} characters or less"
                )));
            }

            let regex = doc! {
                "$regex": regex::escape(value),
                "$options": "i",
            };
            value_clauses.push(match stored_field {
                Some(field) => doc! { field: regex },
                None => doc! {
                    "$or": [
                        { "client_name": regex.clone() },
                        { "_id": regex },
                    ]
                },
            });
        }
        clauses.push(one_or_many(value_clauses));
    }

    Ok(clauses)
}

fn admin_created_at_filter(
    created_dates: Option<&str>,
    created_from: Option<&str>,
    created_to: Option<&str>,
) -> AppResult<Option<Document>> {
    if let Some(created_dates) = created_dates {
        if created_from.is_some() || created_to.is_some() {
            return Err(AppError::ValidationError(
                "created_dates cannot be combined with created_from or created_to".to_string(),
            ));
        }

        let raw_dates = created_dates.split(',').collect::<Vec<_>>();
        if raw_dates.len() > ADMIN_CREATED_DATES_MAX_VALUES {
            return Err(AppError::ValidationError(format!(
                "created_dates must contain at most {ADMIN_CREATED_DATES_MAX_VALUES} values"
            )));
        }

        let mut seen = HashSet::new();
        let mut date_filters = Vec::new();
        for raw_date in raw_dates {
            let value = raw_date.trim();
            if value.is_empty() {
                return Err(AppError::ValidationError(
                    "created_dates must not contain empty values".to_string(),
                ));
            }
            let date = parse_admin_calendar_date("created_dates", value)?;
            if !seen.insert(date) {
                continue;
            }
            let exclusive_to = date.succ_opt().ok_or_else(|| {
                AppError::ValidationError(
                    "created_dates contains a date outside the supported range".to_string(),
                )
            })?;
            date_filters.push(doc! {
                "created_at": {
                    "$gte": midnight_utc(date),
                    "$lt": midnight_utc(exclusive_to),
                }
            });
        }

        return Ok(Some(one_or_many(date_filters)));
    }

    let created_from = created_from
        .map(|value| parse_admin_calendar_date("created_from", value))
        .transpose()?;
    let created_to = created_to
        .map(|value| parse_admin_calendar_date("created_to", value))
        .transpose()?;

    if created_from.is_some_and(|from| created_to.is_some_and(|to| from > to)) {
        return Err(AppError::ValidationError(
            "created_from must be on or before created_to".to_string(),
        ));
    }

    let mut bounds = Document::new();
    if let Some(from) = created_from {
        bounds.insert("$gte", midnight_utc(from));
    }
    if let Some(to) = created_to {
        let exclusive_to = to.succ_opt().ok_or_else(|| {
            AppError::ValidationError("created_to is outside the supported date range".to_string())
        })?;
        bounds.insert("$lt", midnight_utc(exclusive_to));
    }

    Ok((!bounds.is_empty()).then(|| doc! { "created_at": bounds }))
}

fn parse_admin_calendar_date(field: &str, value: &str) -> AppResult<NaiveDate> {
    let bytes = value.as_bytes();
    let has_exact_format = bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        });
    if !has_exact_format {
        return Err(AppError::ValidationError(format!(
            "{field} must be a valid date in YYYY-MM-DD format"
        )));
    }

    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        AppError::ValidationError(format!("{field} must be a valid date in YYYY-MM-DD format"))
    })
}

fn midnight_utc(date: NaiveDate) -> bson::DateTime {
    bson::DateTime::from_chrono(
        date.and_hms_opt(0, 0, 0)
            .expect("midnight is valid for every calendar date")
            .and_utc(),
    )
}

fn admin_csv_filter_values<'a>(
    field: &str,
    raw: &'a str,
    allowed: &[&str],
) -> AppResult<Vec<&'a str>> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();

    for raw_value in raw.split(',') {
        let value = raw_value.trim();
        if value.is_empty() {
            return Err(AppError::ValidationError(format!(
                "{field} must not contain empty values"
            )));
        }
        if !allowed.contains(&value) {
            return Err(AppError::ValidationError(format!(
                "{field} must contain only: {}",
                allowed.join(", ")
            )));
        }
        if seen.insert(value) {
            values.push(value);
        }
    }

    Ok(values)
}

fn one_or_many(mut filters: Vec<Document>) -> Document {
    if filters.len() == 1 {
        filters.remove(0)
    } else {
        doc! { "$or": filters }
    }
}

fn admin_broker_filter(broker: &str, broker_require_admin_capability: bool) -> AppResult<Document> {
    let flag_enabled = doc! { "broker_capability_enabled": true };
    let flag_not_enabled = doc! { "broker_capability_enabled": { "$ne": true } };
    let has_scope = scope_token_filter(
        crate::services::oauth_broker_service::BROKER_BINDING_SCOPE,
        true,
    );
    let lacks_scope = scope_token_filter(
        crate::services::oauth_broker_service::BROKER_BINDING_SCOPE,
        false,
    );

    let filter = match broker {
        "flag" => flag_enabled,
        "scope" if broker_require_admin_capability => doc! { "_id": { "$exists": false } },
        "scope" => doc! { "$and": [flag_not_enabled, has_scope] },
        "enabled" if broker_require_admin_capability => flag_enabled,
        "enabled" => doc! { "$or": [flag_enabled, has_scope] },
        "disabled" if broker_require_admin_capability => flag_not_enabled,
        "disabled" => doc! { "$and": [flag_not_enabled, lacks_scope] },
        _ => {
            return Err(AppError::ValidationError(format!(
                "broker must be one of: {}",
                ADMIN_BROKER_FILTERS.join(", ")
            )));
        }
    };

    Ok(filter)
}

fn scope_token_filter(scope: &str, present: bool) -> Document {
    let pattern = scope_token_pattern(scope);
    if present {
        doc! { "allowed_scopes": { "$regex": pattern } }
    } else {
        doc! { "allowed_scopes": { "$not": { "$regex": pattern } } }
    }
}

fn scope_token_pattern(scope: &str) -> String {
    format!(r"(^|\s){}(\s|$)", regex::escape(scope))
}

fn admin_oauth_client_sort(sort: &str) -> AppResult<Document> {
    let (field, direction) = match sort.strip_prefix('-') {
        Some(field) => (field, -1),
        None => (sort, 1),
    };
    if !matches!(
        field,
        "created_at"
            | "client_name"
            | "client_type"
            | "created_by"
            | "broker"
            | "is_active"
            | "allowed_scopes"
    ) {
        return Err(AppError::ValidationError(
            "sort must target created_at, client_name, client_type, created_by, broker, is_active, or allowed_scopes".to_string(),
        ));
    }

    let mut sort_doc = Document::new();
    let stored_field = if field == "broker" {
        ADMIN_BROKER_SORT_FIELD
    } else {
        field
    };
    sort_doc.insert(stored_field, direction);
    if field != "created_at" {
        sort_doc.insert("created_at", direction);
    }
    sort_doc.insert("_id", direction);
    Ok(sort_doc)
}

fn admin_broker_sort_pipeline(
    filter: Document,
    sort: Document,
    offset: i64,
    per_page: i64,
    broker_require_admin_capability: bool,
) -> Vec<Document> {
    let mut computed_fields = Document::new();
    computed_fields.insert(
        ADMIN_BROKER_SORT_FIELD,
        admin_broker_sort_expression(broker_require_admin_capability),
    );

    vec![
        doc! { "$match": filter },
        doc! { "$set": computed_fields },
        doc! { "$sort": sort },
        doc! { "$skip": offset },
        doc! { "$limit": per_page },
        doc! { "$unset": ADMIN_BROKER_SORT_FIELD },
    ]
}

/// Match the response's broker source ordering: none, legacy scope, explicit flag.
fn admin_broker_sort_expression(broker_require_admin_capability: bool) -> Document {
    let mut branches = vec![doc! {
        "case": {
            "$eq": [
                { "$ifNull": ["$broker_capability_enabled", false] },
                true,
            ]
        },
        "then": 2,
    }];

    if !broker_require_admin_capability {
        branches.push(doc! {
            "case": {
                "$regexMatch": {
                    "input": { "$ifNull": ["$allowed_scopes", ""] },
                    "regex": scope_token_pattern(
                        crate::services::oauth_broker_service::BROKER_BINDING_SCOPE,
                    ),
                }
            },
            "then": 1,
        });
    }

    doc! {
        "$switch": {
            "branches": branches,
            "default": 0,
        }
    }
}

/// List OAuth clients created by a specific user.
pub async fn list_clients_by_creator(
    db: &mongodb::Database,
    created_by: &str,
) -> AppResult<Vec<OauthClient>> {
    let clients: Vec<OauthClient> = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find(doc! { "created_by": created_by })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;

    Ok(clients)
}

/// Fetch a single OAuth client by ID.
pub async fn get_client(db: &mongodb::Database, client_id: &str) -> AppResult<OauthClient> {
    db.collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! { "_id": client_id })
        .await?
        .ok_or_else(|| AppError::NotFound("OAuth client not found".to_string()))
}

/// Fetch a single OAuth client by ID and owner.
pub async fn get_client_for_creator(
    db: &mongodb::Database,
    client_id: &str,
    created_by: &str,
) -> AppResult<OauthClient> {
    db.collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! { "_id": client_id, "created_by": created_by })
        .await?
        .ok_or_else(|| AppError::NotFound("OAuth client not found".to_string()))
}

/// Update the redirect URIs on an OAuth client.
pub async fn update_redirect_uris(
    db: &mongodb::Database,
    client_id: &str,
    redirect_uris: &[String],
) -> AppResult<()> {
    let now = Utc::now();
    let result = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(
            doc! { "_id": client_id, "is_active": true },
            doc! { "$set": {
                "redirect_uris": bson::to_bson(redirect_uris).map_err(|e| {
                    AppError::Internal(format!("Failed to convert redirect_uris to bson: {e}"))
                })?,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("OAuth client not found".to_string()));
    }

    Ok(())
}

/// Update mutable fields on an OAuth client owned by a specific user.
#[allow(clippy::too_many_arguments)]
pub async fn update_client_for_creator(
    db: &mongodb::Database,
    client_id: &str,
    created_by: &str,
    client_name: Option<&str>,
    redirect_uris: Option<&[String]>,
    delegation_scopes: Option<&str>,
    allowed_scopes: Option<&str>,
    broker_capability_enabled: Option<bool>,
    revocation_webhook_url: Option<&str>,
    revocation_webhook_secret_encrypted: Option<Vec<u8>>,
    default_service_catalog_slugs: Option<&[String]>,
) -> AppResult<OauthClient> {
    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };

    if let Some(name) = client_name {
        set_doc.insert("client_name", name);
    }

    if let Some(uris) = redirect_uris {
        set_doc.insert(
            "redirect_uris",
            bson::to_bson(uris).map_err(|e| {
                AppError::Internal(format!("Failed to convert redirect_uris to bson: {e}"))
            })?,
        );
    }

    if let Some(scopes) = delegation_scopes {
        set_doc.insert("delegation_scopes", scopes);
    }

    if let Some(scopes) = allowed_scopes {
        set_doc.insert("allowed_scopes", scopes);
    }

    if let Some(enabled) = broker_capability_enabled {
        set_doc.insert("broker_capability_enabled", enabled);
    }

    if let Some(url) = revocation_webhook_url {
        set_doc.insert("revocation_webhook_url", url);
    }

    if let Some(secret) = revocation_webhook_secret_encrypted {
        set_doc.insert(
            "revocation_webhook_secret_encrypted",
            Binary {
                subtype: BinarySubtype::Generic,
                bytes: secret,
            },
        );
    }

    if let Some(slugs) = default_service_catalog_slugs {
        set_doc.insert(
            "default_service_catalog_slugs",
            bson::to_bson(slugs).map_err(|e| {
                AppError::Internal(format!(
                    "Failed to convert default_service_catalog_slugs to bson: {e}"
                ))
            })?,
        );
    }

    let result = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(
            doc! { "_id": client_id, "created_by": created_by, "is_active": true },
            doc! { "$set": set_doc },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("OAuth client not found".to_string()));
    }

    get_client_for_creator(db, client_id, created_by).await
}

#[derive(Debug, Default)]
pub struct AdminUpdateClient<'a> {
    pub client_name: Option<&'a str>,
    pub redirect_uris: Option<&'a [String]>,
    pub allowed_scopes: Option<&'a str>,
    pub broker_capability_enabled: Option<bool>,
    pub is_active: Option<bool>,
}

/// Update mutable fields on any OAuth client by `_id`.
///
/// Unlike [`update_client_for_creator`], this deliberately does not filter by
/// `created_by` or `is_active`: platform admins must be able to provision
/// ownerless Dynamic Client Registration rows (`created_by =
/// "dynamic_registration"`) and reactivate/deactivate clients operationally.
pub async fn admin_update_client(
    db: &mongodb::Database,
    client_id: &str,
    update: AdminUpdateClient<'_>,
) -> AppResult<OauthClient> {
    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };

    if let Some(name) = update.client_name {
        set_doc.insert("client_name", name);
    }

    if let Some(uris) = update.redirect_uris {
        set_doc.insert(
            "redirect_uris",
            bson::to_bson(uris).map_err(|e| {
                AppError::Internal(format!("Failed to convert redirect_uris to bson: {e}"))
            })?,
        );
    }

    if let Some(scopes) = update.allowed_scopes {
        set_doc.insert("allowed_scopes", scopes);
    }

    if let Some(enabled) = update.broker_capability_enabled {
        set_doc.insert("broker_capability_enabled", enabled);
    }

    if let Some(active) = update.is_active {
        set_doc.insert("is_active", active);
    }

    let result = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(doc! { "_id": client_id }, doc! { "$set": set_doc })
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("OAuth client not found".to_string()));
    }

    let clears_pending_auth_codes = update.redirect_uris.is_some()
        || update.allowed_scopes.is_some()
        || update.broker_capability_enabled.is_some();

    if update.is_active == Some(false) {
        cascade_client_deactivation(db, client_id).await?;
    } else if clears_pending_auth_codes {
        delete_unused_authorization_codes(db, client_id).await?;
    }

    get_client(db, client_id).await
}

/// Soft-delete an OAuth client by marking it inactive.
pub async fn delete_client(db: &mongodb::Database, client_id: &str) -> AppResult<()> {
    let now = Utc::now();

    let result = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(
            doc! { "_id": client_id },
            doc! { "$set": {
                "is_active": false,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("OAuth client not found".to_string()));
    }

    cascade_client_deactivation(db, client_id).await?;

    Ok(())
}

/// Soft-delete an OAuth client owned by a specific user.
pub async fn delete_client_for_creator(
    db: &mongodb::Database,
    client_id: &str,
    created_by: &str,
) -> AppResult<()> {
    let now = Utc::now();
    let result = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(
            doc! { "_id": client_id, "created_by": created_by },
            doc! { "$set": {
                "is_active": false,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("OAuth client not found".to_string()));
    }

    cascade_client_deactivation(db, client_id).await?;

    Ok(())
}

// Mirrors org-delete cascade in org_service.rs so stale consents/refresh tokens
// do not linger after a single client is deactivated (issue #498).
async fn cascade_client_deactivation(db: &mongodb::Database, client_id: &str) -> AppResult<()> {
    db.collection::<bson::Document>(CONSENTS)
        .delete_many(doc! { "client_id": client_id })
        .await?;
    db.collection::<bson::Document>(REFRESH_TOKENS)
        .delete_many(doc! { "client_id": client_id })
        .await?;
    delete_unused_authorization_codes(db, client_id).await?;
    Ok(())
}

async fn delete_unused_authorization_codes(
    db: &mongodb::Database,
    client_id: &str,
) -> AppResult<()> {
    db.collection::<bson::Document>(AUTH_CODES)
        .delete_many(doc! { "client_id": client_id, "used": false })
        .await?;
    Ok(())
}

/// Rotate client secret for a confidential OAuth client owned by a specific user.
pub async fn rotate_client_secret_for_creator(
    db: &mongodb::Database,
    client_id: &str,
    created_by: &str,
) -> AppResult<(OauthClient, String)> {
    let client = get_client_for_creator(db, client_id, created_by).await?;

    if client.client_type != "confidential" {
        return Err(AppError::BadRequest(
            "Only confidential clients can rotate secret".to_string(),
        ));
    }

    let new_secret = generate_random_token();
    let new_hash = hash_token(&new_secret);

    db.collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(
            doc! { "_id": client_id, "created_by": created_by, "is_active": true },
            doc! { "$set": {
                "client_secret_hash": new_hash,
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            }},
        )
        .await?;

    let updated = get_client_for_creator(db, client_id, created_by).await?;
    Ok((updated, new_secret))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admin_list_params() -> AdminOAuthClientListParams<'static> {
        AdminOAuthClientListParams {
            page: 1,
            per_page: 25,
            search: None,
            search_filters: None,
            custom_filters: None,
            client_type: None,
            creator_type: None,
            broker: None,
            is_active: None,
            scope: None,
            created_dates: None,
            created_from: None,
            created_to: None,
            sort: "-created_at",
            broker_require_admin_capability: false,
        }
    }

    #[test]
    fn oauth_client_delegation_scopes_exclude_account_read() {
        for scope in OAUTH_CLIENT_DELEGATION_SCOPES {
            validate_oauth_client_delegation_scopes(scope).unwrap_or_else(|error| {
                panic!("OAuth-client scope {scope} should be valid: {error}")
            });
        }

        for scopes in ["account:read", "proxy:* account:read"] {
            assert!(matches!(
                validate_oauth_client_delegation_scopes(scopes),
                Err(AppError::ValidationError(_))
            ));
        }
    }

    #[test]
    fn admin_list_filter_composes_search_and_broker_or_clauses() {
        let params = AdminOAuthClientListParams {
            search: Some("aevatar.*"),
            broker: Some("enabled"),
            is_active: Some("true"),
            ..admin_list_params()
        };

        let filter = admin_oauth_client_filter(&params).expect("valid filters");
        let clauses = filter.get_array("$and").expect("combined filter");
        assert_eq!(clauses.len(), 3);

        let search = clauses[0].as_document().expect("search clause");
        let search_branches = search.get_array("$or").expect("search branches");
        assert_eq!(search_branches.len(), 5);
        let client_name = search_branches[0]
            .as_document()
            .and_then(|branch| branch.get_document("client_name").ok())
            .expect("client-name regex");
        assert_eq!(client_name.get_str("$regex").unwrap(), r"aevatar\.\*");
        assert!(
            search_branches[1]
                .as_document()
                .is_some_and(|branch| branch.contains_key("_id"))
        );
        assert!(
            search_branches[2]
                .as_document()
                .is_some_and(|branch| branch.contains_key("client_type"))
        );
        assert!(
            search_branches[3]
                .as_document()
                .is_some_and(|branch| branch.contains_key("created_by"))
        );
        assert!(
            search_branches[4]
                .as_document()
                .is_some_and(|branch| branch.contains_key("allowed_scopes"))
        );

        let broker = clauses[1].as_document().expect("broker clause");
        assert_eq!(broker.get_array("$or").unwrap().len(), 2);
        assert_eq!(clauses[2], Bson::Document(doc! { "is_active": true }));
    }

    #[test]
    fn admin_field_search_ors_values_and_ands_fields_and_legacy_search() {
        let params = AdminOAuthClientListParams {
            search: Some("legacy"),
            search_filters: Some(
                r#"[
                    {"field":"client","values":[" console.* ","Portal"]},
                    {"field":"client_type","values":["public"]}
                ]"#,
            ),
            ..admin_list_params()
        };

        let filter = admin_oauth_client_filter(&params).expect("valid field search");
        let clauses = filter.get_array("$and").expect("search groups use AND");
        assert_eq!(clauses.len(), 3);

        let global_search = clauses[0]
            .as_document()
            .and_then(|clause| clause.get_array("$or").ok())
            .expect("legacy global search remains its own OR group");
        assert_eq!(global_search.len(), 5);

        let names = clauses[1]
            .as_document()
            .and_then(|clause| clause.get_array("$or").ok())
            .expect("values in one search field use OR");
        assert_eq!(names.len(), 2);
        let first_client_branches = names[0]
            .as_document()
            .and_then(|branch| branch.get_array("$or").ok())
            .expect("each Client value matches name or ID");
        let first_name_regex = first_client_branches[0]
            .as_document()
            .and_then(|branch| branch.get_document("client_name").ok())
            .expect("client-name condition");
        assert_eq!(first_name_regex.get_str("$regex").unwrap(), r"console\.\*");
        assert_eq!(first_name_regex.get_str("$options").unwrap(), "i");
        let first_id_regex = first_client_branches[1]
            .as_document()
            .and_then(|branch| branch.get_document("_id").ok())
            .expect("client-ID condition");
        assert_eq!(first_id_regex, first_name_regex);

        assert_eq!(
            clauses[2],
            Bson::Document(doc! {
                "client_type": { "$regex": "public", "$options": "i" }
            })
        );
    }

    #[test]
    fn admin_client_field_searches_name_and_id_with_escaped_literals() {
        let clauses =
            admin_search_filter_clauses(r#"[{"field":"client","values":["client[01]+$"]}]"#)
                .expect("valid Client-column search");

        let client_branches = clauses[0].get_array("$or").expect("name-or-ID search");
        assert_eq!(client_branches.len(), 2);
        for field in ["client_name", "_id"] {
            let condition = client_branches
                .iter()
                .find_map(|branch| branch.as_document()?.get_document(field).ok())
                .expect("Client search field");
            assert_eq!(condition.get_str("$regex").unwrap(), r"client\[01\]\+\$");
            assert_eq!(condition.get_str("$options").unwrap(), "i");
        }
    }

    #[test]
    fn admin_field_search_rejects_malformed_shape_fields_and_values() {
        let overlong_value = "x".repeat(ADMIN_SEARCH_MAX_CHARS + 1);
        let too_many_values = serde_json::json!([{
            "field": "client",
            "values": (0..=ADMIN_SEARCH_FILTER_MAX_VALUES_PER_GROUP)
                .map(|index| format!("value-{index}"))
                .collect::<Vec<_>>(),
        }])
        .to_string();
        let overlong_json = serde_json::json!([{
            "field": "client",
            "values": [overlong_value],
        }])
        .to_string();

        let invalid = [
            "{".to_string(),
            "{}".to_string(),
            "[]".to_string(),
            r#"[{"field":"unknown","values":["value"]}]"#.to_string(),
            r#"[{"field":"client_name","values":["value"]}]"#.to_string(),
            r#"[{"field":"client","values":["one"]},{"field":"client","values":["two"]}]"#
                .to_string(),
            r#"[{"field":"client","values":[]}]"#.to_string(),
            r#"[{"field":"client","values":["   "]}]"#.to_string(),
            r#"[{"field":"client","values":["value"],"extra":true}]"#.to_string(),
            too_many_values,
            overlong_json,
        ];

        for raw in invalid {
            assert!(
                matches!(
                    admin_search_filter_clauses(&raw),
                    Err(AppError::ValidationError(_))
                ),
                "expected invalid search_filters to fail: {raw}"
            );
        }
    }

    #[test]
    fn admin_list_scalar_filters_keep_single_value_semantics() {
        assert_eq!(
            admin_oauth_client_filter(&AdminOAuthClientListParams {
                client_type: Some("public"),
                ..admin_list_params()
            })
            .unwrap(),
            doc! { "client_type": "public" }
        );
        assert_eq!(
            admin_oauth_client_filter(&AdminOAuthClientListParams {
                creator_type: Some("system"),
                ..admin_list_params()
            })
            .unwrap(),
            doc! { "created_by": "system" }
        );
        assert_eq!(
            admin_oauth_client_filter(&AdminOAuthClientListParams {
                is_active: Some("false"),
                ..admin_list_params()
            })
            .unwrap(),
            doc! { "is_active": false }
        );
    }

    #[test]
    fn admin_list_multi_value_filters_or_within_fields_and_and_across_fields() {
        let filter = admin_oauth_client_filter(&AdminOAuthClientListParams {
            client_type: Some(" public, confidential,public "),
            creator_type: Some("system, ownerless"),
            is_active: Some("true,false"),
            scope: Some("profile, email"),
            ..admin_list_params()
        })
        .unwrap();

        let field_clauses = filter.get_array("$and").expect("different fields use AND");
        assert_eq!(field_clauses.len(), 3, "both statuses add no restriction");

        let client_types = field_clauses[0]
            .as_document()
            .and_then(|condition| condition.get_array("$or").ok())
            .expect("client types use OR");
        assert_eq!(client_types.len(), 2, "duplicates are canonicalized");
        assert_eq!(
            client_types,
            &vec![
                Bson::Document(doc! { "client_type": "public" }),
                Bson::Document(doc! { "client_type": "confidential" }),
            ]
        );

        let creator_types = field_clauses[1]
            .as_document()
            .and_then(|condition| condition.get_array("$or").ok())
            .expect("creator types use OR");
        assert_eq!(creator_types.len(), 2);

        let scopes = field_clauses[2]
            .as_document()
            .and_then(|condition| condition.get_array("$or").ok())
            .expect("scopes use OR");
        assert_eq!(scopes.len(), 2);
        assert_eq!(
            scopes[0]
                .as_document()
                .and_then(|condition| condition.get_document("allowed_scopes").ok())
                .and_then(|condition| condition.get_str("$regex").ok()),
            Some(r"(^|\s)profile(\s|$)")
        );
        assert_eq!(
            scopes[1]
                .as_document()
                .and_then(|condition| condition.get_document("allowed_scopes").ok())
                .and_then(|condition| condition.get_str("$regex").ok()),
            Some(r"(^|\s)email(\s|$)")
        );
    }

    #[test]
    fn admin_custom_filter_text_ors_a_contains_branch_into_its_own_filter() {
        let filter = admin_oauth_client_filter(&AdminOAuthClientListParams {
            client_type: Some("public"),
            custom_filters: Some(r#"{"client_type":[" acme.* "],"creator_type":["d0d7b72a"]}"#),
            ..admin_list_params()
        })
        .unwrap();

        let field_clauses = filter.get_array("$and").expect("different fields use AND");
        assert_eq!(
            field_clauses.len(),
            2,
            "one clause per filter, not per value"
        );

        let client_types = field_clauses[0]
            .as_document()
            .and_then(|condition| condition.get_array("$or").ok())
            .expect("the checked option and the custom text OR together");
        assert_eq!(
            client_types,
            &vec![
                Bson::Document(doc! { "client_type": "public" }),
                Bson::Document(doc! {
                    "client_type": { "$regex": r"acme\.\*", "$options": "i" }
                }),
            ],
            "custom text is trimmed, regex-escaped, and matched case-insensitively"
        );

        // creator_type has no checked options here, so its custom text stands alone,
        // and it searches created_by -- the column that filter is derived from.
        assert_eq!(
            field_clauses[1].as_document(),
            Some(&doc! {
                "created_by": { "$regex": "d0d7b72a", "$options": "i" }
            }),
        );
    }

    #[test]
    fn admin_custom_filters_reject_unsupported_filters_and_out_of_bounds_values() {
        // Boolean, derived, and date filters have no column to run a contains on.
        for filter in ["is_active", "broker", "created_at", "nonsense"] {
            let error = admin_oauth_client_filter(&AdminOAuthClientListParams {
                custom_filters: Some(&format!(r#"{{"{filter}":["x"]}}"#)),
                ..admin_list_params()
            })
            .expect_err("filter does not accept custom text");
            assert!(
                matches!(error, AppError::ValidationError(message) if message.contains("custom_filters filter must be one of")),
                "{filter} must be rejected"
            );
        }

        let too_long = "a".repeat(ADMIN_SEARCH_MAX_CHARS + 1);
        for (raw, expected) in [
            ("not json", "custom_filters must be a JSON object"),
            (r#"{"scope":[]}"#, "must not be empty"),
            (r#"{"scope":["  "]}"#, "must not be empty"),
            (
                &format!(r#"{{"scope":["{too_long}"]}}"#),
                "characters or less",
            ),
            (
                r#"{"scope":["a","b","c","d","e","f","g","h","i"]}"#,
                "at most 8 entries",
            ),
        ] {
            let error = admin_oauth_client_filter(&AdminOAuthClientListParams {
                custom_filters: Some(raw),
                ..admin_list_params()
            })
            .expect_err("invalid custom_filters payload");
            assert!(
                matches!(error, AppError::ValidationError(message) if message.contains(expected)),
                "expected {expected:?} for {raw:?}"
            );
        }
    }

    #[test]
    fn admin_custom_text_filters_cover_only_string_backed_columns() {
        assert_eq!(admin_custom_text_field("client_type"), Some("client_type"));
        assert_eq!(admin_custom_text_field("creator_type"), Some("created_by"));
        assert_eq!(admin_custom_text_field("scope"), Some("allowed_scopes"));
        assert_eq!(admin_custom_text_field("is_active"), None);
        assert_eq!(admin_custom_text_field("broker"), None);
        assert_eq!(admin_custom_text_field("created_at"), None);
    }

    #[test]
    fn admin_csv_filters_trim_dedupe_and_reject_invalid_values() {
        assert_eq!(
            admin_csv_filter_values(
                "client_type",
                " public, confidential,public ",
                ADMIN_CLIENT_TYPE_FILTERS,
            )
            .unwrap(),
            ["public", "confidential"]
        );

        for raw in ["", ",", "public,", ",public", "public,,confidential"] {
            assert!(matches!(
                admin_csv_filter_values("client_type", raw, ADMIN_CLIENT_TYPE_FILTERS),
                Err(AppError::ValidationError(_))
            ));
        }
        assert!(matches!(
            admin_csv_filter_values("client_type", "public,native", ADMIN_CLIENT_TYPE_FILTERS,),
            Err(AppError::ValidationError(_))
        ));
    }

    #[test]
    fn admin_broker_filters_match_runtime_policy_semantics() {
        assert_eq!(
            admin_broker_filter("flag", false).unwrap(),
            doc! { "broker_capability_enabled": true }
        );
        assert_eq!(
            admin_broker_filter("scope", true).unwrap(),
            doc! { "_id": { "$exists": false } }
        );
        assert_eq!(
            admin_broker_filter("enabled", true).unwrap(),
            doc! { "broker_capability_enabled": true }
        );
        assert_eq!(
            admin_broker_filter("disabled", true).unwrap(),
            doc! { "broker_capability_enabled": { "$ne": true } }
        );

        let scope = admin_broker_filter("scope", false).unwrap();
        let scope_clauses = scope.get_array("$and").unwrap();
        assert_eq!(
            scope_clauses[0],
            Bson::Document(doc! { "broker_capability_enabled": { "$ne": true } })
        );
        let scope_regex = scope_clauses[1]
            .as_document()
            .and_then(|clause| clause.get_document("allowed_scopes").ok())
            .and_then(|condition| condition.get_str("$regex").ok())
            .expect("scope regex");
        assert!(scope_regex.starts_with(r"(^|\s)"));
        assert!(scope_regex.ends_with(r"(\s|$)"));

        let disabled = admin_broker_filter("disabled", false).unwrap();
        let disabled_clauses = disabled.get_array("$and").unwrap();
        let absent_scope = disabled_clauses[1]
            .as_document()
            .and_then(|clause| clause.get_document("allowed_scopes").ok())
            .and_then(|condition| condition.get_document("$not").ok())
            .expect("negative scope condition");
        assert!(absent_scope.contains_key("$regex"));
    }

    #[test]
    fn admin_multi_broker_filters_union_policy_aware_branches() {
        let legacy = admin_oauth_client_filter(&AdminOAuthClientListParams {
            broker: Some("scope, flag,scope"),
            broker_require_admin_capability: false,
            ..admin_list_params()
        })
        .unwrap();
        let legacy_branches = legacy.get_array("$or").expect("broker values use OR");
        assert_eq!(legacy_branches.len(), 2, "duplicates are canonicalized");
        assert!(
            legacy_branches[0]
                .as_document()
                .is_some_and(|branch| branch.contains_key("$and"))
        );
        assert_eq!(
            legacy_branches[1],
            Bson::Document(doc! { "broker_capability_enabled": true })
        );

        let strict = admin_oauth_client_filter(&AdminOAuthClientListParams {
            broker: Some("scope,flag"),
            broker_require_admin_capability: true,
            ..admin_list_params()
        })
        .unwrap();
        let strict_branches = strict.get_array("$or").expect("broker values use OR");
        assert_eq!(
            strict_branches[0],
            Bson::Document(doc! { "_id": { "$exists": false } })
        );
        assert_eq!(
            strict_branches[1],
            Bson::Document(doc! { "broker_capability_enabled": true })
        );
    }

    #[test]
    fn admin_scope_filter_is_exact_and_validated() {
        let params = AdminOAuthClientListParams {
            scope: Some("profile"),
            ..admin_list_params()
        };
        let filter = admin_oauth_client_filter(&params).unwrap();
        let condition = filter.get_document("allowed_scopes").unwrap();
        assert_eq!(condition.get_str("$regex").unwrap(), r"(^|\s)profile(\s|$)");

        let invalid = AdminOAuthClientListParams {
            scope: Some("profile:write"),
            ..admin_list_params()
        };
        assert!(matches!(
            admin_oauth_client_filter(&invalid),
            Err(AppError::ValidationError(_))
        ));

        let multi = admin_oauth_client_filter(&AdminOAuthClientListParams {
            scope: Some("profile,email"),
            ..admin_list_params()
        })
        .unwrap();
        assert_eq!(multi.get_array("$or").unwrap().len(), 2);
    }

    #[test]
    fn admin_created_at_filter_uses_inclusive_utc_calendar_days() {
        let range = admin_oauth_client_filter(&AdminOAuthClientListParams {
            created_from: Some("2026-07-03"),
            created_to: Some("2026-07-10"),
            ..admin_list_params()
        })
        .expect("valid date range");
        let bounds = range.get_document("created_at").expect("created-at bounds");
        assert_eq!(
            bounds.get_datetime("$gte").unwrap().to_chrono(),
            chrono::DateTime::parse_from_rfc3339("2026-07-03T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
        assert_eq!(
            bounds.get_datetime("$lt").unwrap().to_chrono(),
            chrono::DateTime::parse_from_rfc3339("2026-07-11T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );

        let from_only = admin_oauth_client_filter(&AdminOAuthClientListParams {
            created_from: Some("2026-07-03"),
            ..admin_list_params()
        })
        .expect("valid lower bound");
        let from_bounds = from_only.get_document("created_at").unwrap();
        assert!(from_bounds.contains_key("$gte"));
        assert!(!from_bounds.contains_key("$lt"));

        let to_only = admin_oauth_client_filter(&AdminOAuthClientListParams {
            created_to: Some("2026-07-10"),
            ..admin_list_params()
        })
        .expect("valid upper bound");
        let to_bounds = to_only.get_document("created_at").unwrap();
        assert!(!to_bounds.contains_key("$gte"));
        assert!(to_bounds.contains_key("$lt"));
    }

    #[test]
    fn admin_created_dates_filter_ors_trimmed_deduplicated_utc_days() {
        let filter = admin_oauth_client_filter(&AdminOAuthClientListParams {
            search: Some("console"),
            created_dates: Some("2026-07-03, 2026-07-10,2026-07-03"),
            ..admin_list_params()
        })
        .expect("valid exact dates");
        let clauses = filter
            .get_array("$and")
            .expect("dates are ANDed with search");
        assert_eq!(clauses.len(), 2);

        let dates = clauses[1]
            .as_document()
            .and_then(|clause| clause.get_array("$or").ok())
            .expect("exact dates use OR");
        assert_eq!(dates.len(), 2, "duplicate dates are removed");

        for (date_filter, expected_from, expected_to) in [
            (&dates[0], "2026-07-03T00:00:00Z", "2026-07-04T00:00:00Z"),
            (&dates[1], "2026-07-10T00:00:00Z", "2026-07-11T00:00:00Z"),
        ] {
            let bounds = date_filter
                .as_document()
                .and_then(|filter| filter.get_document("created_at").ok())
                .expect("full-day bounds");
            assert_eq!(
                bounds.get_datetime("$gte").unwrap().to_chrono(),
                chrono::DateTime::parse_from_rfc3339(expected_from)
                    .unwrap()
                    .with_timezone(&Utc)
            );
            assert_eq!(
                bounds.get_datetime("$lt").unwrap().to_chrono(),
                chrono::DateTime::parse_from_rfc3339(expected_to)
                    .unwrap()
                    .with_timezone(&Utc)
            );
        }
    }

    #[test]
    fn admin_created_dates_filter_rejects_conflicts_invalid_values_and_excess() {
        for params in [
            AdminOAuthClientListParams {
                created_dates: Some("2026-07-03"),
                created_from: Some("2026-07-01"),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                created_dates: Some("2026-07-03"),
                created_to: Some("2026-07-10"),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                created_dates: Some(""),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                created_dates: Some("2026-07-03,"),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                created_dates: Some("2026-02-30"),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                created_dates: Some("07/03/2026"),
                ..admin_list_params()
            },
        ] {
            assert!(matches!(
                admin_oauth_client_filter(&params),
                Err(AppError::ValidationError(_))
            ));
        }

        let too_many = vec!["2026-07-03"; ADMIN_CREATED_DATES_MAX_VALUES + 1].join(",");
        assert!(matches!(
            admin_oauth_client_filter(&AdminOAuthClientListParams {
                created_dates: Some(&too_many),
                ..admin_list_params()
            }),
            Err(AppError::ValidationError(_))
        ));
    }

    #[test]
    fn admin_created_at_filter_rejects_invalid_and_inverted_dates() {
        for params in [
            AdminOAuthClientListParams {
                created_from: Some("2026-02-30"),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                created_to: Some("07/10/2026"),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                created_from: Some(" 2026-07-10"),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                created_to: Some("2026-０7-10"),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                created_from: Some("2026-07-11"),
                created_to: Some("2026-07-10"),
                ..admin_list_params()
            },
        ] {
            assert!(matches!(
                admin_oauth_client_filter(&params),
                Err(AppError::ValidationError(_))
            ));
        }
    }

    #[test]
    fn admin_sort_is_allowlisted_and_stable() {
        assert_eq!(
            admin_oauth_client_sort("-created_at").unwrap(),
            doc! { "created_at": -1, "_id": -1 }
        );
        assert_eq!(
            admin_oauth_client_sort("client_name").unwrap(),
            doc! { "client_name": 1, "created_at": 1, "_id": 1 }
        );
        assert_eq!(
            admin_oauth_client_sort("allowed_scopes").unwrap(),
            doc! { "allowed_scopes": 1, "created_at": 1, "_id": 1 }
        );
        let broker_sort = admin_oauth_client_sort("-broker").unwrap();
        assert_eq!(broker_sort.get_i32(ADMIN_BROKER_SORT_FIELD).unwrap(), -1);
        assert_eq!(broker_sort.get_i32("created_at").unwrap(), -1);
        assert_eq!(broker_sort.get_i32("_id").unwrap(), -1);
        assert!(matches!(
            admin_oauth_client_sort("client_secret_hash"),
            Err(AppError::ValidationError(_))
        ));
        assert!(matches!(
            admin_oauth_client_sort("created_at,-client_name"),
            Err(AppError::ValidationError(_))
        ));
    }

    #[test]
    fn admin_broker_sort_expression_tracks_runtime_policy() {
        let legacy = admin_broker_sort_expression(false);
        let legacy_branches = legacy
            .get_document("$switch")
            .and_then(|switch| switch.get_array("branches"))
            .expect("legacy broker sort branches");
        assert_eq!(legacy_branches.len(), 2);
        let scope_regex = legacy_branches[1]
            .as_document()
            .and_then(|branch| branch.get_document("case").ok())
            .and_then(|case| case.get_document("$regexMatch").ok())
            .and_then(|regex_match| regex_match.get_str("regex").ok())
            .expect("exact broker-scope expression");
        assert_eq!(
            scope_regex,
            scope_token_pattern(crate::services::oauth_broker_service::BROKER_BINDING_SCOPE)
        );

        let strict = admin_broker_sort_expression(true);
        let strict_branches = strict
            .get_document("$switch")
            .and_then(|switch| switch.get_array("branches"))
            .expect("strict broker sort branches");
        assert_eq!(strict_branches.len(), 1);
    }

    #[test]
    fn admin_list_filter_rejects_unknown_domains_and_long_search() {
        for params in [
            AdminOAuthClientListParams {
                client_type: Some("native"),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                creator_type: Some("robot"),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                broker: Some("maybe"),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                is_active: Some("yes"),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                scope: Some("openid,"),
                ..admin_list_params()
            },
            AdminOAuthClientListParams {
                search: Some(
                    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
                ),
                ..admin_list_params()
            },
        ] {
            assert!(matches!(
                admin_oauth_client_filter(&params),
                Err(AppError::ValidationError(_))
            ));
        }
    }

    #[test]
    fn valid_default_scopes() {
        let result = validate_allowed_scopes("openid profile email").unwrap();
        assert_eq!(result, "openid profile email");
    }

    #[test]
    fn valid_with_roles_and_groups() {
        let result = validate_allowed_scopes("openid profile email roles groups").unwrap();
        assert_eq!(result, "openid profile email roles groups");
    }

    #[test]
    fn valid_minimal_openid_only() {
        let result = validate_allowed_scopes("openid").unwrap();
        assert_eq!(result, "openid");
    }

    #[test]
    fn valid_roles_without_profile() {
        let result = validate_allowed_scopes("openid roles").unwrap();
        assert_eq!(result, "openid roles");
    }

    #[test]
    fn auto_prepends_openid_when_missing() {
        let result = validate_allowed_scopes("profile email").unwrap();
        assert!(result.starts_with("openid"));
        assert!(result.contains("profile"));
        assert!(result.contains("email"));
    }

    #[test]
    fn deduplicates_scopes() {
        let result = validate_allowed_scopes("openid openid profile profile").unwrap();
        assert_eq!(result, "openid profile");
    }

    #[test]
    fn valid_with_proxy_scope() {
        let result = validate_allowed_scopes("openid profile email proxy").unwrap();
        assert_eq!(result, "openid profile email proxy");
    }

    #[test]
    fn valid_with_offline_access_scope() {
        let result = validate_allowed_scopes("openid offline_access").unwrap();
        assert_eq!(result, "openid offline_access");
    }

    #[test]
    fn rejects_unknown_scope() {
        let result = validate_allowed_scopes("openid admin");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("admin"));
    }

    #[test]
    fn rejects_arbitrary_scope() {
        let result = validate_allowed_scopes("openid read:users");
        assert!(result.is_err());
    }

    #[test]
    fn empty_string_gets_openid() {
        let result = validate_allowed_scopes("").unwrap();
        assert_eq!(result, "openid");
    }

    #[test]
    fn empty_list_gets_openid() {
        let result = validate_allowed_scopes_list(&[]).unwrap();
        assert_eq!(result, "openid");
    }

    #[test]
    fn default_mcp_scopes_include_roles_and_groups() {
        // Issue #434: Codex requests `roles` and `groups`; the DCR default
        // must allow both or scope validation rejects authorization.
        let scopes: Vec<&str> = DEFAULT_MCP_ALLOWED_SCOPES.split_whitespace().collect();
        assert!(scopes.contains(&"openid"));
        assert!(scopes.contains(&"profile"));
        assert!(scopes.contains(&"email"));
        assert!(scopes.contains(&"roles"));
        assert!(scopes.contains(&"groups"));
        assert!(scopes.contains(&"proxy"));
    }

    #[test]
    fn default_mcp_scopes_validate() {
        // Guard against typos / unknown scopes ever entering the constant.
        validate_allowed_scopes(DEFAULT_MCP_ALLOWED_SCOPES).unwrap();
    }

    #[test]
    fn default_mcp_scopes_include_offline_access() {
        assert!(
            DEFAULT_MCP_ALLOWED_SCOPES
                .split_whitespace()
                .any(|s| s == OFFLINE_ACCESS_SCOPE),
            "NyxID#1222: defaulted MCP clients need offline_access for refresh tokens"
        );
    }

    #[test]
    fn discovery_scopes_are_subset_of_known_scopes() {
        // Supported/registered coherence (NyxID#1222): everything discovery
        // advertises must be a scope validation accepts, so the layers can
        // never silently diverge again.
        for scope in crate::handlers::oidc_discovery::OPENID_CONFIGURATION_SCOPES_SUPPORTED {
            assert!(
                KNOWN_OIDC_SCOPES.contains(scope),
                "discovery advertises unknown scope: {scope}"
            );
        }
        for scope in crate::handlers::oidc_discovery::OAUTH_AUTHORIZATION_SERVER_SCOPES_SUPPORTED {
            assert!(
                KNOWN_OIDC_SCOPES.contains(scope),
                "discovery advertises unknown scope: {scope}"
            );
        }
        for scope in DEFAULT_MCP_ALLOWED_SCOPES.split_whitespace() {
            assert!(
                KNOWN_OIDC_SCOPES.contains(&scope),
                "MCP default contains unknown scope: {scope}"
            );
        }
    }

    #[tokio::test]
    async fn legacy_client_rows_deserialize_as_unknown_legacy_with_scopes_untouched() {
        use crate::test_utils::connect_test_database;
        // NyxID#1222: rows that predate provenance tracking must come back
        // as UnknownLegacy with allowed_scopes byte-identical — proving the
        // schema change alone never widens or rewrites legacy rows.
        let Some(db) = connect_test_database("oauth_legacy_provenance").await else {
            return;
        };
        let raw = bson::doc! {
            "_id": "legacy-provenance-client",
            "client_name": "Legacy",
            "client_secret_hash": "NONE",
            "redirect_uris": ["http://localhost/cb"],
            "allowed_scopes": "openid email",
            "grant_types": "authorization_code",
            "client_type": "public",
            "is_active": true,
            "created_at": bson::DateTime::now(),
            "updated_at": bson::DateTime::now(),
        };
        db.collection::<bson::Document>(OAUTH_CLIENTS)
            .insert_one(raw)
            .await
            .expect("insert raw legacy row");

        let client = db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find_one(bson::doc! { "_id": "legacy-provenance-client" })
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(
            client.scope_provenance,
            crate::models::oauth_client::ScopeProvenance::UnknownLegacy
        );
        assert_eq!(client.allowed_scopes, "openid email");
    }

    #[test]
    fn merge_returns_none_when_defaults_already_present() {
        let merged = merge_missing_default_mcp_scopes(DEFAULT_MCP_ALLOWED_SCOPES).unwrap();
        assert!(merged.is_none(), "no-op when nothing is missing");
    }

    #[test]
    fn merge_adds_only_missing_scopes() {
        // Pre-issue-#434 DCR records had `openid profile email proxy` but no
        // `roles`/`groups`. The merge must add exactly the missing pieces and
        // remain stable thereafter.
        let merged = merge_missing_default_mcp_scopes("openid profile email proxy")
            .unwrap()
            .expect("missing scopes should be merged in");

        let merged_set: std::collections::HashSet<&str> = merged.split_whitespace().collect();
        for scope in DEFAULT_MCP_ALLOWED_SCOPES.split_whitespace() {
            assert!(merged_set.contains(scope), "missing {scope} after merge");
        }
        // Idempotent: a second pass produces no change.
        assert!(merge_missing_default_mcp_scopes(&merged).unwrap().is_none());
    }

    #[test]
    fn merge_preserves_existing_extras_and_dedupes() {
        // A client with everything already plus a duplicate should stay valid
        // and not regress.
        let merged =
            merge_missing_default_mcp_scopes("openid profile profile email roles").unwrap();
        let final_scopes = merged.expect("groups + proxy should be added");
        let parts: Vec<&str> = final_scopes.split_whitespace().collect();
        let unique: std::collections::HashSet<&str> = parts.iter().copied().collect();
        assert_eq!(parts.len(), unique.len(), "merge must dedupe");
    }

    #[test]
    fn validate_redirect_uris_preserves_exact_trimmed_strings() {
        let uris = validate_redirect_uris(&[
            " https://app.example ".to_string(),
            "https://app.example".to_string(),
            "https://app.example/callback".to_string(),
        ])
        .unwrap();

        assert_eq!(
            uris,
            vec![
                "https://app.example".to_string(),
                "https://app.example/callback".to_string(),
            ]
        );
    }

    mod mongo {
        use super::*;
        use crate::models::authorization_code::AuthorizationCode;
        use crate::models::consent::Consent;
        use crate::models::refresh_token::RefreshToken;
        use crate::test_utils::connect_test_database;

        fn list_fixture(
            id: &str,
            name: &str,
            created_by: &str,
            allowed_scopes: &str,
            broker_capability_enabled: bool,
            is_active: bool,
            created_at: chrono::DateTime<Utc>,
        ) -> OauthClient {
            OauthClient {
                id: id.to_string(),
                client_name: name.to_string(),
                client_secret_hash: "NONE".to_string(),
                redirect_uris: vec![],
                allowed_scopes: allowed_scopes.to_string(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code".to_string(),
                client_type: "public".to_string(),
                is_active,
                delegation_scopes: String::new(),
                default_service_catalog_slugs: Vec::new(),
                broker_capability_enabled,
                revocation_webhook_url: None,
                revocation_webhook_secret_encrypted: None,
                connection_webhook_url: None,
                connection_webhook_secret_encrypted: None,
                connection_webhook_enabled: false,
                created_by: Some(created_by.to_string()),
                created_at,
                updated_at: created_at,
            }
        }

        #[tokio::test]
        async fn admin_legacy_list_returns_complete_collection_newest_first() {
            let Some(db) = connect_test_database("oauth_admin_legacy_list").await else {
                eprintln!("skipping oauth_admin_legacy_list test: no local MongoDB available");
                return;
            };
            let now = Utc::now();
            let clients = (0..30)
                .map(|index| {
                    list_fixture(
                        &format!("legacy-{index:02}"),
                        &format!("Legacy {index:02}"),
                        "system",
                        "openid",
                        false,
                        true,
                        now + chrono::Duration::seconds(index),
                    )
                })
                .collect::<Vec<_>>();
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_many(clients)
                .await
                .expect("insert legacy list fixtures");

            let listed = list_clients_legacy(&db).await.expect("list legacy clients");
            assert_eq!(listed.len(), 30, "legacy reads are not page-limited");
            assert_eq!(
                listed
                    .iter()
                    .map(|client| client.id.clone())
                    .collect::<Vec<_>>(),
                (0..30)
                    .rev()
                    .map(|index| format!("legacy-{index:02}"))
                    .collect::<Vec<_>>(),
                "legacy reads preserve the original newest-first order"
            );
        }

        async fn insert_dcr_client(
            db: &mongodb::Database,
            id: &str,
            allowed_scopes: &str,
        ) -> OauthClient {
            let now = Utc::now();
            let client = OauthClient {
                id: id.to_string(),
                client_name: "DCR Test Client".to_string(),
                client_secret_hash: "NONE".to_string(),
                redirect_uris: vec![],
                allowed_scopes: allowed_scopes.to_string(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code".to_string(),
                client_type: "public".to_string(),
                is_active: true,
                delegation_scopes: String::new(),
                default_service_catalog_slugs: Vec::new(),
                broker_capability_enabled: false,
                revocation_webhook_url: None,
                revocation_webhook_secret_encrypted: None,
                connection_webhook_url: None,
                connection_webhook_secret_encrypted: None,
                connection_webhook_enabled: false,
                created_by: Some("dynamic_registration".to_string()),
                created_at: now,
                updated_at: now,
            };
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_one(&client)
                .await
                .expect("insert dcr fixture");
            client
        }

        #[tokio::test]
        async fn admin_list_combines_filters_and_paginates_stably() {
            let Some(db) = connect_test_database("oauth_admin_list").await else {
                eprintln!("skipping oauth_admin_list test: no local MongoDB available");
                return;
            };
            let now = Utc::now();
            let broker_scope = crate::services::oauth_broker_service::BROKER_BINDING_SCOPE;
            let clients = vec![
                list_fixture(
                    "aevatar-new",
                    "Aevatar Console",
                    "dynamic_registration",
                    &format!("openid offline_access {broker_scope}"),
                    false,
                    true,
                    now,
                ),
                list_fixture(
                    "aevatar-old",
                    "Aevatar Console",
                    "dynamic_registration",
                    &format!("openid offline_access {broker_scope}"),
                    false,
                    true,
                    now,
                ),
                list_fixture(
                    "aevatar-flag",
                    "Aevatar Console",
                    "dynamic_registration",
                    "openid offline_access",
                    true,
                    true,
                    now - chrono::Duration::minutes(2),
                ),
                list_fixture(
                    "system-inactive",
                    "NyxID MCP Client",
                    "system",
                    DEFAULT_MCP_ALLOWED_SCOPES,
                    false,
                    false,
                    now - chrono::Duration::minutes(3),
                ),
            ];
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_many(clients)
                .await
                .expect("insert list fixtures");

            let list_page = |page| AdminOAuthClientListParams {
                page,
                per_page: 1,
                search: Some("aevatar"),
                search_filters: None,
                custom_filters: None,
                client_type: Some("public"),
                creator_type: Some("dynamic_registration"),
                broker: Some("scope"),
                is_active: Some("true"),
                scope: Some("offline_access"),
                created_dates: None,
                created_from: None,
                created_to: None,
                sort: "-created_at",
                broker_require_admin_capability: false,
            };

            let (first, first_total) = list_clients(&db, list_page(1)).await.unwrap();
            let (second, second_total) = list_clients(&db, list_page(2)).await.unwrap();
            assert_eq!(first_total, 2);
            assert_eq!(second_total, 2);
            assert_eq!(
                first
                    .iter()
                    .map(|client| client.id.as_str())
                    .collect::<Vec<_>>(),
                ["aevatar-old"]
            );
            assert_eq!(
                second
                    .iter()
                    .map(|client| client.id.as_str())
                    .collect::<Vec<_>>(),
                ["aevatar-new"]
            );

            let (past_end, past_end_total) = list_clients(&db, list_page(u64::MAX))
                .await
                .expect("out-of-range pages should not reach MongoDB skip serialization");
            assert!(past_end.is_empty());
            assert_eq!(past_end_total, 2);
        }

        #[tokio::test]
        async fn admin_list_multi_filters_use_or_with_exact_scopes_and_stable_pages() {
            let Some(db) = connect_test_database("oauth_admin_multi_list").await else {
                eprintln!("skipping oauth_admin_multi_list test: no local MongoDB available");
                return;
            };
            let now = Utc::now();

            let alpha = list_fixture(
                "alpha-profile",
                "Alpha",
                "dynamic_registration",
                "openid profile",
                false,
                true,
                now,
            );
            let mut beta = list_fixture(
                "beta-email",
                "Beta",
                "system",
                "openid email",
                false,
                false,
                now,
            );
            beta.client_type = "confidential".to_string();
            let lookalike = list_fixture(
                "profile-lookalike",
                "Lookalike",
                "system",
                "openid profile_extra",
                false,
                true,
                now,
            );
            let mut no_selected_scope = list_fixture(
                "no-selected-scope",
                "No selected scope",
                "dynamic_registration",
                "openid groups",
                false,
                true,
                now,
            );
            no_selected_scope.client_type = "confidential".to_string();
            let owned = list_fixture(
                "owned-email",
                "Owned",
                "user-id",
                "openid email",
                false,
                true,
                now,
            );

            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_many([alpha, beta, lookalike, no_selected_scope, owned])
                .await
                .expect("insert multi-filter fixtures");

            let list_page = |page| AdminOAuthClientListParams {
                page,
                per_page: 1,
                search: None,
                search_filters: None,
                custom_filters: None,
                client_type: Some("public, confidential"),
                creator_type: Some("dynamic_registration, system"),
                broker: None,
                is_active: Some("true,false"),
                scope: Some("profile,email"),
                created_dates: None,
                created_from: None,
                created_to: None,
                sort: "client_name",
                broker_require_admin_capability: false,
            };

            let (first, first_total) = list_clients(&db, list_page(1)).await.unwrap();
            let (second, second_total) = list_clients(&db, list_page(2)).await.unwrap();
            assert_eq!(first_total, 2);
            assert_eq!(second_total, 2);
            assert_eq!(first[0].id, "alpha-profile");
            assert_eq!(second[0].id, "beta-email");
        }

        #[tokio::test]
        async fn admin_list_filters_inclusive_dates_and_searches_all_visible_text() {
            let Some(db) = connect_test_database("oauth_admin_date_search").await else {
                eprintln!("skipping oauth_admin_date_search test: no local MongoDB available");
                return;
            };
            let at = |value: &str| {
                chrono::DateTime::parse_from_rfc3339(value)
                    .unwrap()
                    .with_timezone(&Utc)
            };
            let mut confidential = list_fixture(
                "date-start",
                "First boundary",
                "system",
                "openid",
                false,
                true,
                at("2026-07-03T00:00:00Z"),
            );
            confidential.client_type = "confidential".to_string();
            let scope_match = list_fixture(
                "date-end",
                "Last boundary",
                "system",
                "openid needle-scope",
                false,
                true,
                at("2026-07-10T23:59:59Z"),
            );
            let outside = list_fixture(
                "date-outside",
                "Outside",
                "system",
                "openid needle-scope",
                false,
                true,
                at("2026-07-11T00:00:00Z"),
            );
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_many([confidential, scope_match, outside])
                .await
                .expect("insert date-search fixtures");

            let range = AdminOAuthClientListParams {
                per_page: 100,
                created_from: Some("2026-07-03"),
                created_to: Some("2026-07-10"),
                sort: "created_at",
                ..admin_list_params()
            };
            let (clients, total) = list_clients(&db, range).await.expect("filter date range");
            assert_eq!(total, 2);
            assert_eq!(
                clients
                    .iter()
                    .map(|client| client.id.as_str())
                    .collect::<Vec<_>>(),
                ["date-start", "date-end"]
            );

            for (search, expected_id) in
                [("CONFIDENTIAL", "date-start"), ("NEEDLE-SCOPE", "date-end")]
            {
                let (clients, total) = list_clients(
                    &db,
                    AdminOAuthClientListParams {
                        per_page: 100,
                        search: Some(search),
                        created_from: Some("2026-07-03"),
                        created_to: Some("2026-07-10"),
                        ..admin_list_params()
                    },
                )
                .await
                .expect("search within date range");
                assert_eq!(total, 1);
                assert_eq!(clients[0].id, expected_id);
            }
        }

        #[tokio::test]
        async fn admin_list_multi_broker_filter_respects_runtime_policy_and_overlap() {
            let Some(db) = connect_test_database("oauth_admin_multi_broker").await else {
                eprintln!("skipping oauth_admin_multi_broker test: no local MongoDB available");
                return;
            };
            let now = Utc::now();
            let broker_scope = crate::services::oauth_broker_service::BROKER_BINDING_SCOPE;
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_many([
                    list_fixture(
                        "scope",
                        "Scope",
                        "system",
                        &format!("openid {broker_scope}"),
                        false,
                        true,
                        now,
                    ),
                    list_fixture("disabled", "Disabled", "system", "openid", false, true, now),
                    list_fixture("flag", "Flag", "system", "openid", true, true, now),
                    list_fixture(
                        "flag-and-scope",
                        "Flag and scope",
                        "system",
                        &format!("openid {broker_scope}"),
                        true,
                        true,
                        now,
                    ),
                ])
                .await
                .expect("insert multi-broker fixtures");

            let list = |broker, strict| AdminOAuthClientListParams {
                per_page: 100,
                broker: Some(broker),
                broker_require_admin_capability: strict,
                sort: "client_name",
                ..admin_list_params()
            };

            let (legacy, legacy_total) =
                list_clients(&db, list("scope,flag", false)).await.unwrap();
            assert_eq!(legacy_total, 3);
            assert_eq!(
                legacy
                    .iter()
                    .map(|client| client.id.as_str())
                    .collect::<HashSet<_>>(),
                HashSet::from(["scope", "flag", "flag-and-scope"])
            );

            let (strict, strict_total) = list_clients(&db, list("scope,flag", true)).await.unwrap();
            assert_eq!(strict_total, 2);
            assert_eq!(
                strict
                    .iter()
                    .map(|client| client.id.as_str())
                    .collect::<HashSet<_>>(),
                HashSet::from(["flag", "flag-and-scope"])
            );

            let (_, overlap_total) = list_clients(&db, list("enabled,flag", false))
                .await
                .unwrap();
            assert_eq!(
                overlap_total, 3,
                "overlapping branches must not duplicate rows"
            );

            let (_, exhaustive_total) = list_clients(&db, list("enabled,disabled", false))
                .await
                .unwrap();
            assert_eq!(exhaustive_total, 4);
        }

        #[tokio::test]
        async fn admin_list_sorts_broker_sources_with_runtime_policy() {
            let Some(db) = connect_test_database("oauth_admin_broker_sort").await else {
                eprintln!("skipping oauth_admin_broker_sort test: no local MongoDB available");
                return;
            };
            let now = Utc::now();
            let broker_scope = crate::services::oauth_broker_service::BROKER_BINDING_SCOPE;
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_many(vec![
                    list_fixture(
                        "scope",
                        "Scope",
                        "dynamic_registration",
                        &format!("openid {broker_scope}"),
                        false,
                        true,
                        now - chrono::Duration::minutes(4),
                    ),
                    list_fixture(
                        "disabled",
                        "Disabled",
                        "dynamic_registration",
                        "openid",
                        false,
                        true,
                        now - chrono::Duration::minutes(3),
                    ),
                    list_fixture(
                        "scope-lookalike",
                        "Scope lookalike",
                        "dynamic_registration",
                        &format!("openid {broker_scope}_extra"),
                        false,
                        true,
                        now - chrono::Duration::minutes(2),
                    ),
                    list_fixture(
                        "flag",
                        "Flag",
                        "dynamic_registration",
                        "openid email",
                        true,
                        true,
                        now - chrono::Duration::minutes(1),
                    ),
                ])
                .await
                .expect("insert broker-sort fixtures");

            let (legacy, total) = list_clients(
                &db,
                AdminOAuthClientListParams {
                    per_page: 100,
                    sort: "broker",
                    broker_require_admin_capability: false,
                    ..admin_list_params()
                },
            )
            .await
            .expect("sort legacy broker sources");
            assert_eq!(total, 4);
            assert_eq!(
                legacy
                    .iter()
                    .map(|client| client.id.as_str())
                    .collect::<Vec<_>>(),
                ["disabled", "scope-lookalike", "scope", "flag"]
            );

            let (descending, _) = list_clients(
                &db,
                AdminOAuthClientListParams {
                    per_page: 100,
                    sort: "-broker",
                    broker_require_admin_capability: false,
                    ..admin_list_params()
                },
            )
            .await
            .expect("sort broker sources descending");
            assert_eq!(
                descending
                    .iter()
                    .map(|client| client.id.as_str())
                    .collect::<Vec<_>>(),
                ["flag", "scope", "scope-lookalike", "disabled"]
            );

            let (strict, _) = list_clients(
                &db,
                AdminOAuthClientListParams {
                    per_page: 100,
                    sort: "broker",
                    broker_require_admin_capability: true,
                    ..admin_list_params()
                },
            )
            .await
            .expect("sort broker sources under strict policy");
            assert_eq!(
                strict
                    .iter()
                    .map(|client| client.id.as_str())
                    .collect::<Vec<_>>(),
                ["scope", "disabled", "scope-lookalike", "flag"]
            );
        }

        #[tokio::test]
        async fn admin_list_sorts_allowed_scopes_as_displayed() {
            let Some(db) = connect_test_database("oauth_admin_scope_sort").await else {
                eprintln!("skipping oauth_admin_scope_sort test: no local MongoDB available");
                return;
            };
            let now = Utc::now();
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_many(vec![
                    list_fixture(
                        "profile",
                        "Profile",
                        "system",
                        "openid profile",
                        false,
                        true,
                        now,
                    ),
                    list_fixture("minimal", "Minimal", "system", "openid", false, true, now),
                    list_fixture("email", "Email", "system", "openid email", false, true, now),
                ])
                .await
                .expect("insert scope-sort fixtures");

            let (clients, total) = list_clients(
                &db,
                AdminOAuthClientListParams {
                    per_page: 100,
                    sort: "allowed_scopes",
                    ..admin_list_params()
                },
            )
            .await
            .expect("sort allowed scopes");
            assert_eq!(total, 3);
            assert_eq!(
                clients
                    .iter()
                    .map(|client| client.id.as_str())
                    .collect::<Vec<_>>(),
                ["minimal", "email", "profile"]
            );
        }

        async fn insert_client_with_consent_and_refresh_token(
            db: &mongodb::Database,
            client_id: &str,
            created_by: &str,
        ) {
            let now = Utc::now();
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_one(&OauthClient {
                    id: client_id.to_string(),
                    client_name: "Cascade Test Client".to_string(),
                    client_secret_hash: "NONE".to_string(),
                    redirect_uris: vec!["http://localhost:3000/callback".to_string()],
                    allowed_scopes: DEFAULT_ALLOWED_SCOPES.to_string(),
                    scope_provenance: Default::default(),
                    grant_types: "authorization_code".to_string(),
                    client_type: "public".to_string(),
                    is_active: true,
                    delegation_scopes: String::new(),
                    default_service_catalog_slugs: Vec::new(),
                    broker_capability_enabled: false,
                    revocation_webhook_url: None,
                    revocation_webhook_secret_encrypted: None,
                    connection_webhook_url: None,
                    connection_webhook_secret_encrypted: None,
                    connection_webhook_enabled: false,
                    created_by: Some(created_by.to_string()),
                    created_at: now,
                    updated_at: now,
                })
                .await
                .expect("insert oauth client fixture");

            db.collection::<Consent>(CONSENTS)
                .insert_one(&Consent {
                    id: format!("consent-{client_id}"),
                    user_id: "user-with-consent".to_string(),
                    client_id: client_id.to_string(),
                    scopes: DEFAULT_ALLOWED_SCOPES.to_string(),
                    allow_all_services: false,
                    allowed_service_ids: None,
                    granted_at: now,
                    expires_at: None,
                })
                .await
                .expect("insert consent fixture");

            db.collection::<RefreshToken>(REFRESH_TOKENS)
                .insert_one(&RefreshToken {
                    id: format!("refresh-{client_id}"),
                    jti: format!("jti-{client_id}"),
                    client_id: client_id.to_string(),
                    user_id: "user-with-refresh-token".to_string(),
                    session_id: Some(format!("session-{client_id}")),
                    scope: Some(DEFAULT_ALLOWED_SCOPES.to_string()),
                    expires_at: now + chrono::Duration::days(1),
                    revoked: false,
                    replaced_by: None,
                    revoked_at: None,
                    resource_uris: Vec::new(),
                    allowed_service_ids: Vec::new(),
                    allow_all_services: true,
                    created_at: now,
                })
                .await
                .expect("insert refresh token fixture");
        }

        async fn count_consents(db: &mongodb::Database, client_id: &str) -> u64 {
            db.collection::<Consent>(CONSENTS)
                .count_documents(doc! { "client_id": client_id })
                .await
                .expect("count consents")
        }

        async fn count_refresh_tokens(db: &mongodb::Database, client_id: &str) -> u64 {
            db.collection::<RefreshToken>(REFRESH_TOKENS)
                .count_documents(doc! { "client_id": client_id })
                .await
                .expect("count refresh tokens")
        }

        async fn insert_authorization_code(
            db: &mongodb::Database,
            client_id: &str,
            code_id: &str,
            used: bool,
        ) {
            let now = Utc::now();
            db.collection::<AuthorizationCode>(AUTH_CODES)
                .insert_one(&AuthorizationCode {
                    id: code_id.to_string(),
                    code_hash: format!("hash-{code_id}"),
                    client_id: client_id.to_string(),
                    user_id: "user-with-auth-code".to_string(),
                    redirect_uri: "http://localhost:3000/callback".to_string(),
                    scope: DEFAULT_ALLOWED_SCOPES.to_string(),
                    code_challenge: None,
                    code_challenge_method: None,
                    nonce: None,
                    external_subject: None,
                    binding_grant_id: None,
                    resource_uris: Vec::new(),
                    allowed_service_ids: Vec::new(),
                    allow_all_services: true,
                    expires_at: now + chrono::Duration::minutes(5),
                    used,
                    created_at: now,
                })
                .await
                .expect("insert authorization code fixture");
        }

        async fn count_authorization_codes(
            db: &mongodb::Database,
            client_id: &str,
            used: bool,
        ) -> u64 {
            db.collection::<AuthorizationCode>(AUTH_CODES)
                .count_documents(doc! { "client_id": client_id, "used": used })
                .await
                .expect("count authorization codes")
        }

        async fn assert_client_deactivated_and_cascaded(db: &mongodb::Database, client_id: &str) {
            let client = get_client(db, client_id)
                .await
                .expect("client tombstone remains");
            assert!(!client.is_active, "client should be soft-deleted");
            assert_eq!(count_consents(db, client_id).await, 0);
            assert_eq!(count_refresh_tokens(db, client_id).await, 0);
        }

        #[tokio::test]
        async fn delete_client_for_creator_deactivates_and_cascades_grants() {
            let Some(db) = connect_test_database("oc_del_creator").await else {
                eprintln!("skipping oc_del_creator test: no local MongoDB available");
                return;
            };

            let client_id = "owned-client";
            insert_client_with_consent_and_refresh_token(&db, client_id, "owner").await;

            delete_client_for_creator(&db, client_id, "owner")
                .await
                .expect("delete owned client");

            assert_client_deactivated_and_cascaded(&db, client_id).await;
        }

        #[tokio::test]
        async fn delete_client_for_creator_does_not_cascade_when_owner_mismatches() {
            let Some(db) = connect_test_database("oc_del_wrong").await else {
                eprintln!("skipping oc_del_wrong test: no local MongoDB available");
                return;
            };

            let client_id = "cross-owned-client";
            insert_client_with_consent_and_refresh_token(&db, client_id, "owner").await;

            let err = delete_client_for_creator(&db, client_id, "other-owner")
                .await
                .expect_err("wrong owner must not delete");

            assert!(matches!(err, AppError::NotFound(_)));
            let client = get_client(&db, client_id)
                .await
                .expect("client should remain");
            assert!(client.is_active, "client should remain active");
            assert_eq!(count_consents(&db, client_id).await, 1);
            assert_eq!(count_refresh_tokens(&db, client_id).await, 1);
        }

        #[tokio::test]
        async fn delete_client_deactivates_and_cascades_grants() {
            let Some(db) = connect_test_database("oc_del_admin").await else {
                eprintln!("skipping oc_del_admin test: no local MongoDB available");
                return;
            };

            let client_id = "admin-delete-client";
            insert_client_with_consent_and_refresh_token(&db, client_id, "owner").await;

            delete_client(&db, client_id)
                .await
                .expect("admin delete client");

            assert_client_deactivated_and_cascaded(&db, client_id).await;
        }

        #[tokio::test]
        async fn admin_update_client_policy_edit_deletes_unused_authorization_codes() {
            let Some(db) = connect_test_database("oc_admin_policy_auth_codes").await else {
                eprintln!("skipping oc_admin_policy_auth_codes test: no local MongoDB available");
                return;
            };

            let client_id = "admin-policy-edit-client";
            insert_client_with_consent_and_refresh_token(&db, client_id, "dynamic_registration")
                .await;
            insert_authorization_code(&db, client_id, "unused-code", false).await;
            insert_authorization_code(&db, client_id, "used-code", true).await;

            admin_update_client(
                &db,
                client_id,
                AdminUpdateClient {
                    allowed_scopes: Some("openid"),
                    ..Default::default()
                },
            )
            .await
            .expect("admin policy edit succeeds");

            assert_eq!(count_authorization_codes(&db, client_id, false).await, 0);
            assert_eq!(count_authorization_codes(&db, client_id, true).await, 1);
            assert_eq!(count_consents(&db, client_id).await, 1);
            assert_eq!(count_refresh_tokens(&db, client_id).await, 1);
        }

        #[tokio::test]
        async fn migration_backfills_roles_and_groups_on_legacy_dcr_clients() {
            let Some(db) = connect_test_database("oauth_dcr_migration").await else {
                eprintln!("skipping oauth_dcr_migration test: no local MongoDB available");
                return;
            };

            // Pre-#434 DCR client: has proxy but missing roles/groups.
            insert_dcr_client(&db, "legacy-dcr", "openid profile email proxy").await;
            // Already up-to-date client: should stay unchanged.
            insert_dcr_client(&db, "current-dcr", DEFAULT_MCP_ALLOWED_SCOPES).await;

            migrate_dynamic_clients_grant_default_mcp_scopes(&db)
                .await
                .expect("migration runs cleanly");

            let upgraded = get_client(&db, "legacy-dcr").await.unwrap();
            for scope in DEFAULT_MCP_ALLOWED_SCOPES.split_whitespace() {
                assert!(
                    upgraded
                        .allowed_scopes
                        .split_whitespace()
                        .any(|s| s == scope),
                    "legacy DCR client should have {scope} after migration"
                );
            }

            // Idempotent: a second pass is a no-op.
            migrate_dynamic_clients_grant_default_mcp_scopes(&db)
                .await
                .expect("migration is idempotent");
        }

        #[tokio::test]
        async fn seed_upgrades_existing_mcp_client_with_missing_default_scopes() {
            let Some(db) = connect_test_database("oauth_seed_upgrade").await else {
                eprintln!("skipping oauth_seed_upgrade test: no local MongoDB available");
                return;
            };

            let now = Utc::now();
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_one(&OauthClient {
                    id: MCP_CLIENT_ID.to_string(),
                    client_name: "NyxID MCP Client".to_string(),
                    client_secret_hash: "NONE".to_string(),
                    redirect_uris: vec![],
                    allowed_scopes: "openid profile email proxy".to_string(),
                    scope_provenance: Default::default(),
                    grant_types: "authorization_code".to_string(),
                    client_type: "public".to_string(),
                    is_active: true,
                    delegation_scopes: String::new(),
                    default_service_catalog_slugs: Vec::new(),
                    broker_capability_enabled: false,
                    revocation_webhook_url: None,
                    revocation_webhook_secret_encrypted: None,
                    connection_webhook_url: None,
                    connection_webhook_secret_encrypted: None,
                    connection_webhook_enabled: false,
                    created_by: Some("system".to_string()),
                    created_at: now,
                    updated_at: now,
                })
                .await
                .expect("seed legacy mcp client");

            seed_default_clients(&db).await.expect("seed runs");

            let upgraded = get_client(&db, MCP_CLIENT_ID).await.unwrap();
            for scope in ["roles", "groups"] {
                assert!(
                    upgraded
                        .allowed_scopes
                        .split_whitespace()
                        .any(|s| s == scope),
                    "seeded mcp client should have {scope} after upgrade"
                );
            }
        }
    }
}
