//! External Machine resource grants. This module does not dispatch work.
//! Every admission/renewal caller must retain its transaction through job/lease
//! insertion; returning a checked snapshot and committing later is not a fence.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::IndexOptions,
};
use nyxid_machine::authority::Capabilities;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::api_key_mutation_service::{map_transaction_error, transaction_result};
use crate::{
    crypto::jwt::Claims,
    errors::{AppError, AppResult},
    models::{
        api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as KEYS},
        consent::{COLLECTION_NAME as CONSENTS, Consent, ProtectedConsentRef},
        machine_access::{Assignment, Policy},
        machine_external_grant::{
            COLLECTION_NAME, ExternalGrantBinding, ExternalRevocationSelector, GrantStatus,
            MachineExternalGrant, MachineRestrictions,
        },
        node::{COLLECTION_NAME as NODES, Node},
        oauth_client::{COLLECTION_NAME as CLIENTS, OauthClient},
        user::{COLLECTION_NAME as USERS, User},
        user_service::{COLLECTION_NAME as SERVICES, UserService},
    },
};

pub const DEFAULT_GRANT_DAYS: i64 = 30;
const MAX_ASSIGNMENTS: usize = 100;

// A task-local, exact-owner barrier lets regressions establish a real Mongo
// snapshot before a competing production caller commits its write fence.
#[cfg(test)]
#[derive(Clone)]
pub(super) struct SourceSnapshotHook {
    database: String,
    owner: String,
    reached: std::sync::Arc<tokio::sync::Semaphore>,
    resume: std::sync::Arc<tokio::sync::Semaphore>,
    reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(test)]
tokio::task_local! {
    pub(super) static SOURCE_SNAPSHOT_HOOK: SourceSnapshotHook;
}

#[cfg(test)]
impl SourceSnapshotHook {
    pub(super) fn new(db: &Database, owner: &str) -> Self {
        Self {
            database: db.name().to_owned(),
            owner: owner.to_owned(),
            reached: std::sync::Arc::new(tokio::sync::Semaphore::new(0)),
            resume: std::sync::Arc::new(tokio::sync::Semaphore::new(0)),
            reads: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
    pub(super) async fn wait(&self) {
        self.reached.acquire().await.unwrap().forget();
    }
    pub(super) fn release(&self) {
        self.resume.add_permits(1);
    }
    pub(super) fn reads(&self) -> usize {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
async fn source_snapshot_loaded(db: &Database, owner: &str) {
    let Some(hook) = SOURCE_SNAPSHOT_HOOK.try_with(Clone::clone).ok() else {
        return;
    };
    if hook.database != db.name() || hook.owner != owner {
        return;
    }
    if hook.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
        hook.reached.add_permits(1);
        hook.resume.acquire().await.unwrap().forget();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantAssignment {
    pub capabilities: Capabilities,
    pub file_roots: Vec<String>,
    #[serde(default)]
    pub service_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantSelection {
    pub assignments: BTreeMap<String, GrantAssignment>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

const MAX_GRANT_PAGE: i64 = 100;

/// Stable, intentionally coarse reasons safe for owner-facing projections.
/// These values never contain source identifiers, secrets, database errors, or
/// token state. A missing reason on a valid value means the live check passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidityReason {
    Missing,
    Disabled,
    Expired,
    Revoked,
    Changed,
    Ineligible,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidityStatus {
    pub valid: bool,
    pub reason: Option<ValidityReason>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantValidity {
    pub source: ValidityStatus,
    pub grant: ValidityStatus,
    pub combined: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub enum SourceReference {
    ApiKey {
        api_key_id: String,
    },
    OAuth {
        oauth_client_id: String,
        consent: Option<ProtectedConsentRef>,
    },
}

impl std::fmt::Debug for SourceReference {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SourceReference { [REDACTED] }")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineGrantPage {
    pub grants: Vec<MachineExternalGrant>,
    pub next_cursor: Option<String>,
}

fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.into())
}
fn conflict() -> AppError {
    AppError::Conflict("Machine grant or source changed; review current authority".into())
}
fn has_scope(scopes: &str, value: &str) -> bool {
    scopes.split_whitespace().any(|s| s == value)
}
fn expiry_filter() -> Document {
    doc! {"$or":[{"expires_at":bson::Bson::Null},{"expires_at":{"$gt":bson::DateTime::now()}}]}
}
fn source_epoch(row: &Document) -> AppResult<i64> {
    match row.get("machine_external_security_revision") {
        None => Ok(0),
        Some(bson::Bson::Int64(v)) if *v >= 0 => Ok(*v),
        Some(bson::Bson::Int32(v)) if *v >= 0 => Ok(i64::from(*v)),
        _ => Err(AppError::MachineAuthorityStale),
    }
}

/// Run once at startup before mounting the owner routes. Never TTL grant history.
pub async fn create_indexes(db: &Database) -> AppResult<()> {
    let indexes = [
        IndexModel::builder()
            .keys(doc! {"user_id":1,"binding.oauth_client_id":1})
            .options(
                IndexOptions::builder()
                    .name("active_oauth_machine_grant".to_string())
                    .unique(true)
                    .partial_filter_expression(doc! {"status":"active","binding.kind":"oauth"})
                    .build(),
            )
            .build(),
        IndexModel::builder()
            .keys(doc! {"user_id":1,"binding.api_key_id":1})
            .options(
                IndexOptions::builder()
                    .name("active_key_machine_grant".to_string())
                    .unique(true)
                    .partial_filter_expression(doc! {"status":"active","binding.kind":"api_key"})
                    .build(),
            )
            .build(),
        IndexModel::builder()
            .keys(doc! {"binding.oauth_client_id":1,"status":1,"_id":1})
            .build(),
        IndexModel::builder()
            .keys(doc! {"user_id":1,"status":1,"_id":1})
            .build(),
        IndexModel::builder()
            .keys(doc! {"user_id":1,"created_at":-1,"_id":-1})
            .options(
                IndexOptions::builder()
                    .name("owner_machine_grant_pages".to_string())
                    .build(),
            )
            .build(),
        IndexModel::builder()
            .keys(doc! {"status":1,"expires_at":1,"_id":1})
            .build(),
        IndexModel::builder()
            .keys(doc! {"status":1,"_id":1})
            .build(),
    ];
    db.collection::<Document>(COLLECTION_NAME)
        .create_indexes(indexes)
        .await?;
    db.collection::<Document>(crate::models::api_key_credential::COLLECTION_NAME)
        .create_index(IndexModel::builder()
            .keys(doc! {"is_active":1,"revoked_at":1,"machine_external_expiry_enqueued_at":1,"expires_at":1,"_id":1})
            .build())
        .await?;
    db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"credential_api_key_id":1})
                .options(
                    IndexOptions::builder()
                        .name("machine_source_conversation_key".to_string())
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

// These are lexical bounds only. The signed protocol and Node must canonicalize
// the path and enforce symlink/TOCTOU safety on the remote filesystem.
fn normalized_root(root: &str) -> bool {
    if root == "/" {
        return true;
    }
    root.starts_with('/')
        && root.len() <= 4096
        && !root.contains('\0')
        && !root.contains('\\')
        && (root == "/" || !root.ends_with('/'))
        && !root
            .split('/')
            .skip(1)
            .any(|part| part == "." || part == ".." || part.is_empty())
}
fn within(root: &str, ceiling: &str) -> bool {
    root == ceiling
        || ceiling == "/"
        || root
            .strip_prefix(ceiling)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
fn selection_shape(selection: &GrantSelection) -> AppResult<()> {
    if selection.assignments.is_empty() || selection.assignments.len() > MAX_ASSIGNMENTS {
        return Err(invalid("Choose between one and 100 Machines"));
    }
    if selection
        .expires_at
        .is_some_and(|expiry| expiry <= Utc::now())
    {
        return Err(invalid("Grant expiry must be in the future"));
    }
    for (id, value) in &selection.assignments {
        Uuid::parse_str(id).map_err(|_| invalid("Machine IDs must be UUIDs"))?;
        let c = &value.capabilities;
        if c.browser || c.computer || c.developer_browser || !(c.shell || c.files) {
            return Err(invalid(
                "External Machine grants support shell and files only",
            ));
        }
        if value.file_roots.is_empty()
            || value.file_roots.len() > 16
            || value.file_roots.iter().any(|r| !normalized_root(r))
        {
            return Err(invalid(
                "Choose one to 16 normalized absolute workspace roots",
            ));
        }
        if value.service_ids.len() > 100
            || value
                .service_ids
                .iter()
                .any(|id| Uuid::parse_str(id).is_err())
        {
            return Err(invalid("Service IDs must be a bounded list of UUIDs"));
        }
        if !c.shell && !value.service_ids.is_empty() {
            return Err(invalid("Gateway services require shell capability"));
        }
    }
    Ok(())
}

async fn owner_in_session(
    db: &Database,
    user: &str,
    session: &mut ClientSession,
) -> AppResult<i64> {
    let row = db
        .collection::<Document>(USERS)
        .find_one(doc! {"_id":user,"is_active":true})
        .session(&mut *session)
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    let owner: User = bson::from_document(row.clone()).map_err(|_| AppError::MachineNotAllowed)?;
    if !owner.user_type.is_person() {
        return Err(AppError::MachineNotAllowed);
    }
    let epoch = source_epoch(&row)?;
    #[cfg(test)]
    source_snapshot_loaded(db, user).await;
    touch_source(db, USERS, doc! {"_id":user,"is_active":true}, session).await?;
    Ok(epoch)
}

/// Genuine write, not a no-op: creates conflicts with source mutation/removal.
async fn touch_source(
    db: &Database,
    collection: &str,
    filter: Document,
    session: &mut ClientSession,
) -> AppResult<()> {
    let result = db
        .collection::<Document>(collection)
        .update_one(filter, doc! {"$inc":{"machine_external_fence":1_i64}})
        .session(session)
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::MachinePermissionRevoked);
    }
    Ok(())
}

pub async fn validate_selection_in_session(
    db: &Database,
    user: &str,
    selection: &GrantSelection,
    session: &mut ClientSession,
) -> AppResult<()> {
    selection_shape(selection)?;
    owner_in_session(db, user, session).await?;
    for (id, choice) in &selection.assignments {
        let node = db
            .collection::<Node>(NODES)
            .find_one(doc! {"_id":id,"user_id":user,"is_active":true})
            .session(&mut *session)
            .await?
            .ok_or(AppError::MachineNotAllowed)?;
        let profile = node
            .machine
            .as_ref()
            .ok_or(AppError::MachineCapabilityDisabled)?;
        if !profile.enabled() || !choice.capabilities.subset_of(Capabilities::legacy(profile)) {
            return Err(AppError::MachineCapabilityDisabled);
        }
        if choice.file_roots.iter().any(|r| {
            !profile
                .roots
                .iter()
                .any(|c| normalized_root(c) && within(r, c))
        }) {
            return Err(AppError::MachinePathOutsideRoots);
        }
        // Serialize approval with ownership/capability changes as well.
        touch_source(
            db,
            NODES,
            doc! {"_id":id,"user_id":user,"is_active":true},
            session,
        )
        .await?;
        super::api_key_scope_service::validate_service_ids_with_session(
            db,
            user,
            &choice.service_ids,
            super::api_key_scope_service::ScopeAuthorization::ActorPermissions {
                actor_user_id: user,
            },
            session,
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
pub async fn validate_selection(
    db: &Database,
    user: &str,
    selection: &GrantSelection,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    let result = validate_selection_in_session(db, user, selection, &mut session).await;
    session.abort_transaction().await?;
    result
}

fn key_eligible(key: &ApiKey, user: &str) -> bool {
    key.user_id == user
        && key.is_active
        && key.purpose == ApiKeyPurpose::General
        && key.expires_at.is_none_or(|x| x > Utc::now())
        && crate::mw::auth::scope_allows_rest_proxy(&key.scopes)
        && key.assistant_agent_owner_id.is_none()
        && key.assistant_group_id.is_none()
}

async fn load_key(
    db: &Database,
    user: &str,
    id: &str,
    session: &mut ClientSession,
) -> AppResult<ApiKey> {
    let key = db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id":id,"user_id":user})
        .session(&mut *session)
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    if !key_eligible(&key, user) {
        return Err(AppError::MachineNotAllowed);
    }
    if key_has_chat_binding(db, id, session).await? {
        return Err(AppError::MachineNotAllowed);
    }
    Ok(key)
}

async fn key_has_chat_binding(
    db: &Database,
    id: &str,
    session: &mut ClientSession,
) -> AppResult<bool> {
    // Durable chat binding is authoritative, never a product/display label.
    for (collection, field) in [
        (
            crate::models::assistant_agent_credential::COLLECTION_NAME,
            "api_key_id",
        ),
        (
            crate::models::assistant_conversation::COLLECTION_NAME,
            "credential_api_key_id",
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one(doc! {field:id})
            .projection(doc! {"_id":1})
            .session(&mut *session)
            .await?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn key_selection(
    db: &Database,
    key: &ApiKey,
    selection: &GrantSelection,
    session: &mut ClientSession,
) -> AppResult<()> {
    for (node, choice) in &selection.assignments {
        if !key.allow_all_nodes && !key.allowed_node_ids.contains(node) {
            return Err(AppError::MachineNotAllowed);
        }
        for service in &choice.service_ids {
            if key.allow_all_services || key.allowed_service_ids.contains(service) {
                continue;
            }
            let automatic = if key.allow_auto_connected_services {
                db.collection::<UserService>(SERVICES).find_one(doc! {"_id":service,"user_id":&key.user_id,"is_active":true,"deleted_at":bson::Bson::Null,"$or":[{"source":"auto_provision"},{"credential_binding":"platform"}]}).session(&mut *session).await?.is_some()
            } else {
                false
            };
            if !automatic {
                return Err(AppError::MachineNotAllowed);
            }
        }
    }
    Ok(())
}

fn build_grant(
    user: &str,
    binding: ExternalGrantBinding,
    selection: &GrantSelection,
    user_epoch: i64,
    client_epoch: Option<i64>,
) -> MachineExternalGrant {
    let now = Utc::now();
    let mut policy = Policy::default();
    let mut restrictions = BTreeMap::new();
    for (node, choice) in &selection.assignments {
        policy.assignments.insert(
            node.clone(),
            Assignment {
                capabilities: choice.capabilities,
                mode: "shared_legacy".into(),
                revision: 1,
                legacy: false,
                saved_login_ids: Some(Vec::new()),
            },
        );
        restrictions.insert(
            node.clone(),
            MachineRestrictions {
                file_roots: choice
                    .file_roots
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                service_ids: choice
                    .service_ids
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            },
        );
    }
    MachineExternalGrant {
        id: Uuid::new_v4().to_string(),
        user_id: user.into(),
        binding,
        policy,
        restrictions,
        revision: 1,
        status: GrantStatus::Active,
        admission_fence: 0,
        user_security_revision: user_epoch,
        client_security_revision: client_epoch,
        expires_at: selection
            .expires_at
            .unwrap_or(now + Duration::days(DEFAULT_GRANT_DAYS)),
        created_at: now,
        updated_at: now,
        revoked_at: None,
        revoked_reason: None,
    }
}

pub async fn approve_api_key(
    db: &Database,
    user: &str,
    key_id: &str,
    expected_state_version: i64,
    selection: &GrantSelection,
) -> AppResult<MachineExternalGrant> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let user = user.to_owned();
    let key_id = key_id.to_owned();
    let selection = selection.clone();
    session.start_transaction().and_run2(async move |session| {
        let (db,user,key_id,selection)=(&db,user.as_str(),key_id.as_str(),&selection);
        transaction_result(async {
            validate_selection_in_session(db,user,selection,session).await?;
            let key = load_key(db,user,key_id,session).await?;
            if key.state_version != expected_state_version { return Err(conflict()); }
            key_selection(db,&key,selection,session).await?;
            super::api_key_mutation_service::approve_machine_grant_source_in_session(
                db,user,key_id,expected_state_version,session).await?;
            let existing = principal_filter(user,"api_key","binding.api_key_id",key_id);
            if let Some(existing) = db.collection::<MachineExternalGrant>(COLLECTION_NAME).find_one(existing).session(&mut *session).await? {
                if existing.expires_at > Utc::now() { return Err(conflict()); }
                invalidate(db,doc! {"_id":&existing.id,"status":"active","revision":existing.revision},"grant_expired",session).await?;
            }
            let epoch = owner_in_session(db,user,session).await?;
            let row = build_grant(user,ExternalGrantBinding::ApiKey {api_key_id:key_id.into(),api_key_state_version_at_approval:expected_state_version},selection,epoch,None);
            db.collection::<MachineExternalGrant>(COLLECTION_NAME).insert_one(&row).session(&mut *session).await?;
            Ok(row)
        }.await)
    }).await.map_err(map_transaction_error)
}

fn principal_filter(user: &str, kind: &str, field: &str, id: &str) -> Document {
    let mut filter = doc! {"user_id":user,"binding.kind":kind,"status":"active"};
    filter.insert(field, id);
    filter
}

/// Called only after the signed one-time consent request was consumed, and the
/// new Consent revision written, in this SAME transaction.
pub async fn approve_oauth_in_session(
    db: &Database,
    user: &str,
    client: &str,
    consent: &Consent,
    selection: &GrantSelection,
    session: &mut ClientSession,
) -> AppResult<MachineExternalGrant> {
    let reference = ProtectedConsentRef {
        consent_id: consent.id.clone(),
        consent_revision: consent
            .revision
            .clone()
            .filter(|v| !v.is_empty())
            .ok_or(AppError::MachinePermissionRevoked)?,
    };
    validate_selection_in_session(db, user, selection, session).await?;
    let client_epoch = validate_oauth_source(db, user, client, &reference, session).await?;
    consent_selection(consent, selection)?;
    // A new contract requires a newly approved consent revision. Even trusted
    // service callers cannot attach a replacement grant to an old token era.
    if db.collection::<Document>(COLLECTION_NAME).find_one(doc! {
        "user_id":user,"binding.kind":"oauth","binding.oauth_client_id":client,
        "binding.consent_id":&reference.consent_id,"binding.consent_revision":&reference.consent_revision
    }).projection(doc! {"_id":1}).session(&mut *session).await?.is_some() {
        return Err(conflict());
    }
    invalidate_oauth_in_session(db, user, client, "consent_reapproved", session).await?;
    let user_epoch = owner_in_session(db, user, session).await?;
    let row = build_grant(
        user,
        ExternalGrantBinding::OAuth {
            oauth_client_id: client.into(),
            consent_id: reference.consent_id,
            consent_revision: reference.consent_revision,
        },
        selection,
        user_epoch,
        Some(client_epoch),
    );
    db.collection::<MachineExternalGrant>(COLLECTION_NAME)
        .insert_one(&row)
        .session(session)
        .await?;
    Ok(row)
}

fn consent_selection(consent: &Consent, selection: &GrantSelection) -> AppResult<()> {
    for choice in selection.assignments.values() {
        if !consent.allow_all_services
            && choice.service_ids.iter().any(|id| {
                consent
                    .allowed_service_ids
                    .as_ref()
                    .is_none_or(|ids| !ids.contains(id))
            })
        {
            return Err(AppError::MachineNotAllowed);
        }
    }
    Ok(())
}

/// Capture the live source epochs while serializing a prepared consent request
/// with owner/client security changes. The caller saves the request in this same
/// transaction; approval must compare both captured epochs before granting.
pub(crate) async fn prepare_oauth_source_in_session(
    db: &Database,
    user: &str,
    client: &str,
    session: &mut ClientSession,
) -> AppResult<(i64, i64)> {
    let user_epoch = owner_in_session(db, user, session).await?;
    let client_epoch = oauth_client_in_session(db, client, session).await?;
    Ok((user_epoch, client_epoch))
}

async fn oauth_client_in_session(
    db: &Database,
    client: &str,
    session: &mut ClientSession,
) -> AppResult<i64> {
    let client_row = db
        .collection::<Document>(CLIENTS)
        .find_one(doc! {"_id":client,"is_active":true})
        .session(&mut *session)
        .await?
        .ok_or(AppError::MachinePermissionRevoked)?;
    let source: OauthClient =
        bson::from_document(client_row.clone()).map_err(|_| AppError::MachineNotAllowed)?;
    if !has_scope(&source.allowed_scopes, "machine")
        || !crate::mw::auth::scope_allows_rest_proxy(&source.allowed_scopes)
    {
        return Err(AppError::MachinePermissionRevoked);
    }
    touch_source(db, CLIENTS, doc! {"_id":client,"is_active":true}, session).await?;
    source_epoch(&client_row)
}

async fn validate_oauth_source(
    db: &Database,
    user: &str,
    client: &str,
    reference: &ProtectedConsentRef,
    session: &mut ClientSession,
) -> AppResult<i64> {
    if reference.consent_id.is_empty() || reference.consent_revision.is_empty() {
        return Err(AppError::MachinePermissionRevoked);
    }
    let client_epoch = oauth_client_in_session(db, client, session).await?;
    let consent = db.collection::<Consent>(CONSENTS).find_one(doc! {"_id":&reference.consent_id,"user_id":user,"client_id":client,"revision":&reference.consent_revision}).session(&mut *session).await?.ok_or(AppError::MachinePermissionRevoked)?;
    if !has_scope(&consent.scopes, "machine")
        || !crate::mw::auth::scope_allows_rest_proxy(&consent.scopes)
        || consent.expires_at.is_some_and(|x| x <= Utc::now())
    {
        return Err(AppError::MachinePermissionRevoked);
    }
    let mut filter = super::consent_service::version_filter(&consent);
    filter.extend(expiry_filter());
    // issuance_fence is intentionally excluded from consent fingerprints.
    let updated = db
        .collection::<Consent>(CONSENTS)
        .update_one(
            filter,
            doc! {"$set":{"issuance_fence":Uuid::new_v4().to_string()}},
        )
        .session(&mut *session)
        .await?;
    if updated.matched_count != 1 {
        return Err(AppError::MachinePermissionRevoked);
    }
    Ok(client_epoch)
}

/// Only the execution version captured before the mutation is cancelled. A
/// source-wide selector would also match a later explicit reauthorization when
/// an old outbox receipt is replayed; source_fence is a dedupe value, not a
/// predicate compared by the current external consumer.
async fn enqueue_grant_revision_in_session(
    db: &Database,
    user: &str,
    grant_id: &str,
    through_revision: i64,
    admission_fence: i64,
    reason: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    if through_revision < 1 || admission_fence < 1 {
        return Err(AppError::MachineAuthorityStale);
    }
    super::machine_external_service::enqueue_external_in_session(
        db,
        &ExternalRevocationSelector::Grant {
            user_id: user.to_owned(),
            grant_id: grant_id.to_owned(),
            through_revision,
        },
        admission_fence,
        reason,
        session,
    )
    .await
}

#[derive(Deserialize)]
struct GrantInvalidationSnapshot {
    #[serde(rename = "_id")]
    id: String,
    user_id: String,
    revision: i64,
    #[serde(default)]
    admission_fence: i64,
}

async fn invalidate(
    db: &Database,
    mut filter: Document,
    reason: &str,
    session: &mut ClientSession,
) -> AppResult<u64> {
    filter.insert("status", "active");
    let mut cursor = db
        .collection::<GrantInvalidationSnapshot>(COLLECTION_NAME)
        .find(filter)
        .projection(doc! {"_id":1,"user_id":1,"revision":1,"admission_fence":1})
        .session(&mut *session)
        .await?;
    let rows: Vec<GrantInvalidationSnapshot> = cursor.stream(&mut *session).try_collect().await?;
    let now = bson::DateTime::now();
    for row in &rows {
        let revision = row
            .revision
            .checked_add(1)
            .ok_or(AppError::MachineAuthorityStale)?;
        let fence = row
            .admission_fence
            .checked_add(1)
            .ok_or(AppError::MachineAuthorityStale)?;
        let changed = db.collection::<Document>(COLLECTION_NAME).update_one(
            doc! {"_id":&row.id,"user_id":&row.user_id,"status":"active","revision":row.revision},
            doc! {"$set":{"status":"revoked","revoked_at":now,"updated_at":now,"revoked_reason":reason,"revision":revision,"admission_fence":fence}},
        ).session(&mut *session).await?;
        if changed.matched_count != 1 {
            return Err(conflict());
        }
        enqueue_grant_revision_in_session(
            db,
            &row.user_id,
            &row.id,
            row.revision,
            fence,
            reason,
            session,
        )
        .await?;
    }
    Ok(rows.len() as u64)
}
pub async fn invalidate_api_key_in_session(
    db: &Database,
    user: &str,
    key: &str,
    reason: &str,
    session: &mut ClientSession,
) -> AppResult<u64> {
    invalidate(
        db,
        principal_filter(user, "api_key", "binding.api_key_id", key),
        reason,
        session,
    )
    .await
}
pub async fn invalidate_oauth_in_session(
    db: &Database,
    user: &str,
    client: &str,
    reason: &str,
    session: &mut ClientSession,
) -> AppResult<u64> {
    invalidate(
        db,
        principal_filter(user, "oauth", "binding.oauth_client_id", client),
        reason,
        session,
    )
    .await
}

async fn invalidate_bounded(
    db: &Database,
    mut filter: Document,
    reason: &str,
    limit: i64,
    session: &mut ClientSession,
) -> AppResult<u64> {
    filter.insert("status", "active");
    let mut cursor = db
        .collection::<Document>(COLLECTION_NAME)
        .find(filter)
        .projection(doc! {"_id":1})
        .sort(doc! {"_id":1})
        .limit(limit.clamp(1, 1000))
        .session(&mut *session)
        .await?;
    let mut ids = Vec::new();
    while let Some(row) = cursor.next(&mut *session).await.transpose()? {
        ids.push(
            row.get_str("_id")
                .map_err(|_| AppError::MachineAuthorityStale)?
                .to_string(),
        );
    }
    invalidate(
        db,
        doc! {"_id":{"$in":ids},"status":"active"},
        reason,
        session,
    )
    .await
}
/// Call after the client source security epoch increment in the SAME transaction.
/// Repeat bounded batches outside that mutation for cancellation convergence.
pub async fn invalidate_client_in_session(
    db: &Database,
    client: &str,
    reason: &str,
    limit: i64,
    session: &mut ClientSession,
) -> AppResult<u64> {
    let source = db
        .collection::<Document>(CLIENTS)
        .find_one(doc! {"_id":client})
        .session(&mut *session)
        .await?
        .ok_or(AppError::MachinePermissionRevoked)?;
    let epoch = source_epoch(&source)?;
    invalidate_bounded(
        db,
        doc! {"binding.kind":"oauth","binding.oauth_client_id":client,"client_security_revision":{"$lt":epoch}},
        reason,
        limit,
        session,
    )
    .await
}
pub async fn invalidate_user_in_session(
    db: &Database,
    user: &str,
    reason: &str,
    limit: i64,
    session: &mut ClientSession,
) -> AppResult<u64> {
    let source = db
        .collection::<Document>(USERS)
        .find_one(doc! {"_id":user})
        .session(&mut *session)
        .await?
        .ok_or(AppError::MachinePermissionRevoked)?;
    let epoch = source_epoch(&source)?;
    invalidate_bounded(
        db,
        doc! {"user_id":user,"user_security_revision":{"$lt":epoch}},
        reason,
        limit,
        session,
    )
    .await
}

/// A bounded pass over active grants, including grants beyond a source
/// mutation's inline batch. The source epoch/revision remains the authority;
/// the cursor is only progress and can safely restart after a crash.
pub struct SourceSweepBatch {
    #[cfg(test)]
    pub examined: u64,
    #[cfg(test)]
    pub invalidated: u64,
    /// Continue the same pass after this ID; None means restart on the next pass.
    pub next_after: Option<String>,
}

/// Invoke from Machine maintenance, never from ordinary auth/proxy traffic.
/// Each invalid source/grant and its exact cancellation receipt commit in the
/// same transaction. Live admission already refuses expiry or epoch drift even
/// before this bounded pass reaches the row. Transient database errors abort
/// the batch; they must not be mistaken for a revoked source.
pub async fn sweep_invalid_sources(
    db: &Database,
    after_grant_id: Option<&str>,
    limit: i64,
) -> AppResult<SourceSweepBatch> {
    let db = db.clone();
    let after = after_grant_id.map(str::to_owned);
    let limit = limit.clamp(1, 1000);
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        transaction_result(async {
            let mut filter = doc! {"status":"active"};
            if let Some(after) = &after {
                filter.insert("_id", doc! {"$gt":after});
            }
            let mut cursor = db.collection::<MachineExternalGrant>(COLLECTION_NAME)
                .find(filter).sort(doc! {"_id":1}).limit(limit)
                .session(&mut *session).await?;
            let grants: Vec<MachineExternalGrant> = cursor.stream(&mut *session).try_collect().await?;
            let mut _invalidated = 0;
            for grant in &grants {
                match validate_grant_source(&db, grant, session).await {
                    Ok(()) => {}
                    Err(AppError::MachinePermissionRevoked | AppError::MachineNotAllowed | AppError::MachineAuthorityStale) => {
                        let reason = if grant.expires_at <= Utc::now() {"grant_expired"} else {"source_invalid"};
                        _invalidated += invalidate(&db,
                            doc! {"_id":&grant.id,"user_id":&grant.user_id,"revision":grant.revision},
                            reason, session).await?;
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(SourceSweepBatch {
                #[cfg(test)] examined: grants.len() as u64,
                #[cfg(test)] invalidated: _invalidated,
                next_after: if grants.len() == limit as usize {grants.last().map(|g|g.id.clone())} else {None},
            })
        }.await)
    }).await.map_err(map_transaction_error)
}

pub async fn get_active_for_key(
    db: &Database,
    user: &str,
    key: &str,
) -> AppResult<Option<MachineExternalGrant>> {
    Ok(db
        .collection(COLLECTION_NAME)
        .find_one(principal_filter(user, "api_key", "binding.api_key_id", key))
        .await?)
}
pub async fn get_active_for_oauth(
    db: &Database,
    user: &str,
    client: &str,
) -> AppResult<Option<MachineExternalGrant>> {
    Ok(db
        .collection(COLLECTION_NAME)
        .find_one(principal_filter(
            user,
            "oauth",
            "binding.oauth_client_id",
            client,
        ))
        .await?)
}

/// Owner edits preserve ID; reauthorization after revoke always creates a new ID.
/// Key HTTP editors use edit_api_key to bind the browser's source CAS as well.
pub async fn edit(
    db: &Database,
    user: &str,
    id: &str,
    expected_revision: i64,
    selection: &GrantSelection,
) -> AppResult<MachineExternalGrant> {
    edit_inner(db, user, id, expected_revision, None, selection).await
}
pub async fn edit_api_key(
    db: &Database,
    user: &str,
    id: &str,
    expected_revision: i64,
    expected_key_version: i64,
    selection: &GrantSelection,
) -> AppResult<MachineExternalGrant> {
    edit_inner(
        db,
        user,
        id,
        expected_revision,
        Some(expected_key_version),
        selection,
    )
    .await
}
async fn edit_inner(
    db: &Database,
    user: &str,
    id: &str,
    expected_revision: i64,
    key_version: Option<i64>,
    selection: &GrantSelection,
) -> AppResult<MachineExternalGrant> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let user = user.to_owned();
    let id = id.to_owned();
    let selection = selection.clone();
    session.start_transaction().and_run2(async move |session| {
        let (db,user,id,selection)=(&db,user.as_str(),id.as_str(),&selection);transaction_result(async {
        let old=db.collection::<MachineExternalGrant>(COLLECTION_NAME).find_one(doc! {"_id":id,"user_id":user,"status":"active","revision":expected_revision}).session(&mut *session).await?.ok_or_else(conflict)?;
        if old.expires_at<=Utc::now() { return Err(AppError::MachinePermissionRevoked); }
        validate_selection_in_session(db,user,selection,session).await?;
        validate_grant_source(db,&old,session).await?;
        match &old.binding {
            ExternalGrantBinding::ApiKey{api_key_id,..} => {
                let key=load_key(db,user,api_key_id,session).await?;
                if key_version.is_some_and(|v| v!=key.state_version) {return Err(conflict());}
                key_selection(db,&key,selection,session).await?;
            }
            ExternalGrantBinding::OAuth{consent_id,..} => {
                if key_version.is_some() {return Err(AppError::MachineNotAllowed);}
                let consent=db.collection::<Consent>(CONSENTS).find_one(doc! {"_id":consent_id}).session(&mut *session).await?.ok_or(AppError::MachinePermissionRevoked)?;
                consent_selection(&consent,selection)?;
            }
        }
        let next_revision=old.revision.checked_add(1).ok_or(AppError::MachineAuthorityStale)?;
        let policy_revision=old.policy.revision.checked_add(1).ok_or(AppError::MachineAuthorityStale)?;
        let mut row=build_grant(user,old.binding.clone(),selection,old.user_security_revision,old.client_security_revision);
        row.id=old.id.clone();row.created_at=old.created_at;row.revision=next_revision;row.admission_fence=old.admission_fence.checked_add(1).ok_or(AppError::MachineAuthorityStale)?;
        row.expires_at=selection.expires_at.unwrap_or(old.expires_at);
        row.policy.revision=policy_revision;
        for (node,assignment) in &mut row.policy.assignments {
            assignment.revision=old.policy.assignments.get(node).filter(|previous| previous.capabilities==assignment.capabilities && old.restrictions.get(node)==row.restrictions.get(node)).map_or(policy_revision,|previous|previous.revision);
        }
        let changed=db.collection::<MachineExternalGrant>(COLLECTION_NAME).replace_one(doc! {"_id":id,"user_id":user,"status":"active","revision":expected_revision},&row).session(&mut *session).await?;
        if changed.matched_count!=1 {return Err(conflict());}
        enqueue_grant_revision_in_session(db,user,id,old.revision,row.admission_fence,"grant_changed",session).await?;
        Ok(row)
    }.await)}).await.map_err(map_transaction_error)
}

pub async fn revoke(
    db: &Database,
    user: &str,
    id: &str,
    expected_revision: i64,
) -> AppResult<MachineExternalGrant> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let user = user.to_owned();
    let id = id.to_owned();
    session.start_transaction().and_run2(async move |session| {
        let (db,user,id)=(&db,user.as_str(),id.as_str());transaction_result(async {
        owner_in_session(db,user,session).await?;
        let filter=doc! {"_id":id,"user_id":user,"status":"active","revision":expected_revision};
        if invalidate(db,filter,"owner_revoked",session).await?!=1 {return Err(conflict());}
        db.collection::<MachineExternalGrant>(COLLECTION_NAME).find_one(doc! {"_id":id,"user_id":user}).session(&mut *session).await?.ok_or_else(conflict)
    }.await)}).await.map_err(map_transaction_error)
}

async fn validate_grant_source(
    db: &Database,
    grant: &MachineExternalGrant,
    session: &mut ClientSession,
) -> AppResult<()> {
    if grant.status != GrantStatus::Active || grant.expires_at <= Utc::now() {
        return Err(AppError::MachinePermissionRevoked);
    }
    if owner_in_session(db, &grant.user_id, session).await? != grant.user_security_revision {
        return Err(AppError::MachinePermissionRevoked);
    }
    match &grant.binding {
        ExternalGrantBinding::ApiKey { api_key_id, .. } => {
            load_key(db, &grant.user_id, api_key_id, session).await?;
            let mut filter = doc! {"_id":api_key_id,"user_id":&grant.user_id,"is_active":true};
            filter.extend(expiry_filter());
            touch_source(db, KEYS, filter, session).await?;
        }
        ExternalGrantBinding::OAuth {
            oauth_client_id,
            consent_id,
            consent_revision,
        } => {
            let reference = ProtectedConsentRef {
                consent_id: consent_id.clone(),
                consent_revision: consent_revision.clone(),
            };
            let epoch =
                validate_oauth_source(db, &grant.user_id, oauth_client_id, &reference, session)
                    .await?;
            if grant.client_security_revision != Some(epoch) {
                return Err(AppError::MachinePermissionRevoked);
            }
        }
    }
    Ok(())
}

/// A typed view created from already verified authentication, never Deserialize.
/// The MCP/auth adapter must verify signature, audience/resource and sender proof
/// before invoking these constructors; this module repeats live source checks.
#[derive(Clone)]
pub struct VerifiedExternalSource {
    user_id: String,
    source: Source,
    expires_at: DateTime<Utc>,
    service_ceiling: Option<Vec<String>>,
    node_ceiling: Option<Vec<String>>,
    discovery_possible: bool,
}
#[derive(Clone)]
enum Source {
    OAuth {
        client_id: String,
        consent: ProtectedConsentRef,
    },
    ApiKey {
        api_key_id: String,
        credential_id: Option<String>,
    },
}
impl VerifiedExternalSource {
    /// The already-loaded Key carries this hint. Explicit Machine calls still
    /// perform live admission even when this snapshot predates a new grant.
    pub fn discovery_possible(&self) -> bool {
        self.discovery_possible
    }

    pub(super) fn authentication_expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    /// Reconstruct only authority that NyxID already admitted and persisted.
    /// This is not authentication or a bearer: every consumer must re-admit
    /// the original grant ID/revision in its job/lease transaction.
    pub(super) fn from_persisted_job(
        job: &crate::models::machine_job::MachineJob,
    ) -> AppResult<Self> {
        use crate::models::machine_job::MachineJobBinding;
        let Some(MachineJobBinding::External {
            version: 1,
            principal,
            protected_consent,
            authentication_expires_at,
            service_ceiling,
            ..
        }) = &job.binding
        else {
            return Err(AppError::MachinePermissionRevoked);
        };
        Self::from_persisted_binding(
            &job.user_id,
            &job.node_id,
            principal,
            protected_consent.as_ref(),
            *authentication_expires_at,
            service_ceiling,
        )
    }

    pub(super) fn from_persisted_confirmation(
        confirmation: &crate::models::machine_external_execution::ExternalConfirmation,
    ) -> AppResult<Self> {
        Self::from_persisted_binding(
            &confirmation.user_id,
            &confirmation.node_id,
            &confirmation.principal,
            confirmation.protected_consent.as_ref(),
            confirmation
                .authentication_expires_at
                .ok_or(AppError::MachinePermissionRevoked)?,
            &confirmation.service_ceiling,
        )
    }

    fn from_persisted_binding(
        owner: &str,
        node: &str,
        principal: &crate::models::machine_external_grant::ExternalPrincipalRef,
        consent: Option<&ProtectedConsentRef>,
        expires_at: DateTime<Utc>,
        service_ceiling: &[String],
    ) -> AppResult<Self> {
        use crate::models::machine_external_grant::ExternalPrincipalRef;
        if owner.is_empty() || node.is_empty() || expires_at <= Utc::now() {
            return Err(AppError::MachinePermissionRevoked);
        }
        let source = match principal {
            ExternalPrincipalRef::OAuth {
                user_id,
                oauth_client_id,
            } if user_id == owner && !oauth_client_id.is_empty() => {
                let consent = consent
                    .filter(|c| !c.consent_id.is_empty() && !c.consent_revision.is_empty())
                    .ok_or(AppError::MachinePermissionRevoked)?;
                Source::OAuth {
                    client_id: oauth_client_id.clone(),
                    consent: consent.clone(),
                }
            }
            ExternalPrincipalRef::ApiKey {
                user_id,
                api_key_id,
                api_key_credential_id,
            } if user_id == owner
                && !api_key_id.is_empty()
                && api_key_credential_id
                    .as_ref()
                    .is_none_or(|id| !id.is_empty())
                && consent.is_none() =>
            {
                Source::ApiKey {
                    api_key_id: api_key_id.clone(),
                    credential_id: api_key_credential_id.clone(),
                }
            }
            _ => return Err(AppError::MachinePermissionRevoked),
        };
        Ok(Self {
            user_id: owner.to_owned(),
            source,
            expires_at,
            service_ceiling: Some(service_ceiling.to_vec()),
            node_ceiling: Some(vec![node.to_owned()]),
            discovery_possible: true,
        })
    }

    pub fn principal(&self) -> crate::models::machine_external_grant::ExternalPrincipalRef {
        use crate::models::machine_external_grant::ExternalPrincipalRef;
        match &self.source {
            Source::OAuth { client_id, .. } => ExternalPrincipalRef::OAuth {
                user_id: self.user_id.clone(),
                oauth_client_id: client_id.clone(),
            },
            Source::ApiKey {
                api_key_id,
                credential_id,
            } => ExternalPrincipalRef::ApiKey {
                user_id: self.user_id.clone(),
                api_key_id: api_key_id.clone(),
                api_key_credential_id: credential_id.clone(),
            },
        }
    }
    pub fn protected_consent(&self) -> Option<&ProtectedConsentRef> {
        match &self.source {
            Source::OAuth { consent, .. } => Some(consent),
            Source::ApiKey { .. } => None,
        }
    }
    pub fn from_verified_oauth(claims: &Claims) -> AppResult<Self> {
        if claims.token_type != "access"
            || claims.delegated == Some(true)
            || claims.act.is_some()
            || claims.sa == Some(true)
            || claims.relay == Some(true)
            || claims.assistant_forward == Some(true)
            || !has_scope(&claims.scope, "machine")
            || !crate::mw::auth::scope_allows_rest_proxy(&claims.scope)
        {
            return Err(AppError::MachineNotAllowed);
        }
        let consent = claims
            .protected_consent
            .clone()
            .filter(|c| !c.consent_id.is_empty() && !c.consent_revision.is_empty())
            .ok_or(AppError::MachinePermissionRevoked)?;
        let client_id = claims
            .client_id
            .clone()
            .filter(|id| !id.is_empty() && id != &Uuid::nil().to_string())
            .ok_or(AppError::MachineNotAllowed)?;
        let expires_at = DateTime::from_timestamp(claims.exp, 0)
            .filter(|x| *x > Utc::now())
            .ok_or(AppError::MachinePermissionRevoked)?;
        Ok(Self {
            user_id: claims.sub.clone(),
            source: Source::OAuth { client_id, consent },
            expires_at,
            service_ceiling: if claims.allow_all_services == Some(true) {
                None
            } else {
                Some(claims.allowed_service_ids.clone().unwrap_or_default())
            },
            node_ceiling: if claims.allow_all_nodes == Some(false) {
                Some(claims.allowed_node_ids.clone().unwrap_or_default())
            } else {
                None
            },
            discovery_possible: true,
        })
    }
    pub fn from_verified_api_key(key: &ApiKey, credential_id: Option<&str>) -> AppResult<Self> {
        if !key_eligible(key, &key.user_id) {
            return Err(AppError::MachineNotAllowed);
        }
        Ok(Self {
            user_id: key.user_id.clone(),
            source: Source::ApiKey {
                api_key_id: key.id.clone(),
                credential_id: credential_id.map(str::to_owned),
            },
            expires_at: key.expires_at.unwrap_or(DateTime::<Utc>::MAX_UTC),
            // Parent live service resolution includes explicit auto-connects;
            // an admission snapshot is returned below for persisted job binding.
            service_ceiling: None,
            node_ceiling: (!key.allow_all_nodes).then(|| key.allowed_node_ids.clone()),
            discovery_possible: key.machine_external_grants_present,
        })
    }
}

/// Cancellation validates and fences its current source without requiring an
/// active Machine grant. It cannot admit execution, output access or renewal.
pub(super) async fn admit_stop_source_in_session(
    db: &Database,
    source: &VerifiedExternalSource,
    session: &mut ClientSession,
) -> AppResult<()> {
    if source.authentication_expires_at() <= Utc::now() {
        return Err(AppError::MachinePermissionRevoked);
    }
    owner_in_session(db, &source.user_id, session).await?;
    match &source.source {
        Source::ApiKey {
            api_key_id,
            credential_id,
        } => {
            load_key(db, &source.user_id, api_key_id, session).await?;
            let mut filter = doc! {"_id":api_key_id,"user_id":&source.user_id,"is_active":true};
            filter.extend(expiry_filter());
            touch_source(db, KEYS, filter, session).await?;
            if let Some(child) = credential_id {
                super::machine_external_source_service::fence_child_in_session(
                    db,
                    &source.user_id,
                    api_key_id,
                    child,
                    session,
                )
                .await?;
            }
        }
        Source::OAuth { client_id, consent } => {
            validate_oauth_source(db, &source.user_id, client_id, consent, session).await?;
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct AdmittedGrant {
    pub grant: MachineExternalGrant,
    pub assignment: Assignment,
    pub restrictions: MachineRestrictions,
    pub authentication_expires_at: DateTime<Utc>,
    #[cfg(test)]
    pub api_key_credential_id: Option<String>,
}

impl std::fmt::Debug for VerifiedExternalSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("VerifiedExternalSource { [REDACTED] }")
    }
}
impl std::fmt::Debug for Source {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Source { [REDACTED] }")
    }
}
impl std::fmt::Debug for AdmittedGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AdmittedGrant { [REDACTED] }")
    }
}

/// Must be called in the same transaction as job/lease/confirmation admission or
/// renewal. Expected ID+revision prevents a new grant adopting an old job.
pub async fn admit_in_session(
    db: &Database,
    source: &VerifiedExternalSource,
    node: &str,
    expected: Option<(&str, i64)>,
    session: &mut ClientSession,
) -> AppResult<AdmittedGrant> {
    if source.expires_at <= Utc::now()
        || source
            .node_ceiling
            .as_ref()
            .is_some_and(|ids| !ids.iter().any(|id| id == node))
    {
        return Err(AppError::MachinePermissionRevoked);
    }
    let mut filter = match &source.source {
        Source::OAuth { client_id, .. } => principal_filter(
            &source.user_id,
            "oauth",
            "binding.oauth_client_id",
            client_id,
        ),
        Source::ApiKey { api_key_id, .. } => {
            principal_filter(&source.user_id, "api_key", "binding.api_key_id", api_key_id)
        }
    };
    if let Some((id, revision)) = expected {
        filter.insert("_id", id);
        filter.insert("revision", revision);
    }
    let grant = db
        .collection::<MachineExternalGrant>(COLLECTION_NAME)
        .find_one(filter)
        .session(&mut *session)
        .await?
        .ok_or(AppError::MachinePermissionRevoked)?;
    validate_grant_source(db, &grant, session).await?;
    let assignment = grant
        .policy
        .assignments
        .get(node)
        .cloned()
        .ok_or(AppError::MachineNotAllowed)?;
    let mut restrictions = grant
        .restrictions
        .get(node)
        .cloned()
        .ok_or(AppError::MachineNotAllowed)?;
    if assignment.mode != "shared_legacy"
        || assignment
            .saved_login_ids
            .as_ref()
            .is_none_or(|ids| !ids.is_empty())
    {
        return Err(AppError::MachineNotAllowed);
    }
    let mut selection = GrantSelection {
        assignments: BTreeMap::from([(
            node.into(),
            GrantAssignment {
                capabilities: assignment.capabilities,
                file_roots: restrictions.file_roots.clone(),
                service_ids: Vec::new(),
            },
        )]),
        expires_at: None,
    };
    validate_selection_in_session(db, &source.user_id, &selection, session).await?;
    let mut expires_at = source.expires_at.min(grant.expires_at);
    let _credential_id = match (&source.source, &grant.binding) {
        (
            Source::OAuth { client_id, consent },
            ExternalGrantBinding::OAuth {
                oauth_client_id,
                consent_id,
                consent_revision,
            },
        ) if client_id == oauth_client_id
            && &consent.consent_id == consent_id
            && &consent.consent_revision == consent_revision =>
        {
            let live = db
                .collection::<Consent>(CONSENTS)
                .find_one(doc! {"_id":consent_id})
                .session(&mut *session)
                .await?
                .ok_or(AppError::MachinePermissionRevoked)?;
            if let Some(expiry) = live.expires_at {
                expires_at = expires_at.min(expiry);
            }
            restrictions.service_ids.retain(|id| {
                live.allow_all_services
                    || live
                        .allowed_service_ids
                        .as_ref()
                        .is_some_and(|ids| ids.contains(id))
            });
            None
        }
        (
            Source::ApiKey {
                api_key_id,
                credential_id,
            },
            ExternalGrantBinding::ApiKey {
                api_key_id: bound, ..
            },
        ) if api_key_id == bound => {
            let key = load_key(db, &source.user_id, api_key_id, session).await?;
            if let Some(expiry) = key.expires_at {
                expires_at = expires_at.min(expiry);
            }
            if !key.allow_all_nodes && !key.allowed_node_ids.iter().any(|id| id == node) {
                return Err(AppError::MachineNotAllowed);
            }
            let mut allowed = Vec::new();
            for id in &restrictions.service_ids {
                selection.assignments.get_mut(node).unwrap().service_ids = vec![id.clone()];
                match key_selection(db, &key, &selection, session).await {
                    Ok(()) => allowed.push(id.clone()),
                    Err(AppError::MachineNotAllowed) => {}
                    Err(error) => return Err(error),
                }
            }
            restrictions.service_ids = allowed;
            if let Some(child) = credential_id {
                let fenced = super::machine_external_source_service::fence_child_in_session(
                    db,
                    &source.user_id,
                    api_key_id,
                    child,
                    session,
                )
                .await?;
                if let Some(expiry) = fenced.expires_at {
                    expires_at = expires_at.min(expiry);
                }
            }
            credential_id.clone()
        }
        _ => return Err(AppError::MachinePermissionRevoked),
    };
    if let Some(ceiling) = &source.service_ceiling {
        restrictions.service_ids.retain(|id| ceiling.contains(id));
    }
    let mut live_services = Vec::new();
    for id in &restrictions.service_ids {
        match super::api_key_scope_service::validate_service_ids_with_session(
            db,
            &source.user_id,
            std::slice::from_ref(id),
            super::api_key_scope_service::ScopeAuthorization::ActorPermissions {
                actor_user_id: &source.user_id,
            },
            session,
        )
        .await
        {
            Ok(()) => live_services.push(id.clone()),
            Err(AppError::ValidationError(_) | AppError::Forbidden(_)) => {}
            Err(error) => return Err(error),
        }
    }
    restrictions.service_ids = live_services;
    // The execution gateway must repeat the full current service ACL. The
    // persisted job must use this snapshot intersected with explicit services.
    let changed=db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":&grant.id,"revision":grant.revision,"status":"active","expires_at":{"$gt":bson::DateTime::now()}},doc! {"$inc":{"admission_fence":1_i64}}).session(session).await?;
    if changed.matched_count != 1 {
        return Err(AppError::MachinePermissionRevoked);
    }
    Ok(AdmittedGrant {
        grant,
        assignment,
        restrictions,
        authentication_expires_at: expires_at,
        #[cfg(test)]
        api_key_credential_id: _credential_id,
    })
}

/// Prefer the unique active contract even when reauthorization shares a BSON
/// millisecond with its predecessor. Otherwise retain the latest revoked row.
pub async fn get_latest_for_key(
    db: &Database,
    user: &str,
    key: &str,
) -> AppResult<Option<MachineExternalGrant>> {
    Ok(db
        .collection(COLLECTION_NAME)
        .find_one(doc! {"user_id":user,"binding.kind":"api_key","binding.api_key_id":key})
        .sort(doc! {"status":1,"created_at":-1,"_id":-1})
        .await?)
}
pub async fn get_latest_for_oauth(
    db: &Database,
    user: &str,
    client: &str,
) -> AppResult<Option<MachineExternalGrant>> {
    Ok(db
        .collection(COLLECTION_NAME)
        .find_one(doc! {"user_id":user,"binding.kind":"oauth","binding.oauth_client_id":client})
        .sort(doc! {"status":1,"created_at":-1,"_id":-1})
        .await?)
}

fn decode_machine_grant_cursor(cursor: &str) -> AppResult<(DateTime<Utc>, String)> {
    if cursor.len() > 80 {
        return Err(AppError::BadRequest("machine_grant_cursor_invalid".into()));
    }
    let (timestamp, id) = cursor
        .split_once('|')
        .ok_or_else(|| AppError::BadRequest("machine_grant_cursor_invalid".into()))?;
    if timestamp.is_empty() || Uuid::parse_str(id).is_err() {
        return Err(AppError::BadRequest("machine_grant_cursor_invalid".into()));
    }
    let timestamp = DateTime::parse_from_rfc3339(timestamp)
        .map_err(|_| AppError::BadRequest("machine_grant_cursor_invalid".into()))?
        .with_timezone(&Utc);
    Ok((timestamp, id.to_owned()))
}

/// Owner- and Machine-scoped retained grant history with a stable descending
/// `(created_at, _id)` cursor. Every request repeats the Machine owner ACL;
/// every row filter is applied before the bounded page limit.
pub async fn list_for_machine_page(
    db: &Database,
    owner: &str,
    machine_id: &str,
    cursor: Option<&str>,
    limit: i64,
) -> AppResult<MachineGrantPage> {
    Uuid::parse_str(machine_id).map_err(|_| invalid("Machine IDs must be UUIDs"))?;
    let decoded = cursor.map(decode_machine_grant_cursor).transpose()?;
    let node = super::node_service::get_node_by_id(db, machine_id).await?;
    let managed = db
        .collection::<crate::models::remote_machine::ManagedMachine>(
            crate::models::remote_machine::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":machine_id})
        .await?;
    if node.as_ref().is_some_and(|node| node.user_id != owner)
        || managed
            .as_ref()
            .is_some_and(|machine| machine.owner_id != owner)
    {
        return Err(AppError::MachineNotAllowed);
    }
    if node.is_none() && managed.is_none() {
        return Err(AppError::MachineJobNotFound);
    }
    let limit = limit.clamp(1, MAX_GRANT_PAGE);
    let assignment = format!("policy.assignments.{machine_id}");
    let mut filter = doc! {
        "user_id": owner,
        assignment: {"$exists": true},
    };
    if let Some((created_at, id)) = decoded {
        filter.insert(
            "$or",
            vec![
                doc! {"created_at": {"$lt": bson::DateTime::from_chrono(created_at)}},
                doc! {
                    "created_at": bson::DateTime::from_chrono(created_at),
                    "_id": {"$lt": id},
                },
            ],
        );
    }
    let mut rows: Vec<MachineExternalGrant> = db
        .collection(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"created_at": -1, "_id": -1})
        .limit(limit + 1)
        .max_time(std::time::Duration::from_secs(3))
        .await?
        .try_collect()
        .await?;
    let next_cursor = if rows.len() > limit as usize {
        rows.truncate(limit as usize);
        rows.last().map(|row| {
            format!(
                "{}|{}",
                row.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                row.id
            )
        })
    } else {
        None
    };
    Ok(MachineGrantPage {
        grants: rows,
        next_cursor,
    })
}

pub async fn source_and_grant_valid(
    db: &Database,
    grant: &MachineExternalGrant,
) -> AppResult<bool> {
    Ok(evaluate_validity_for_grant(db, grant).await?.combined)
}

fn valid_status() -> ValidityStatus {
    ValidityStatus {
        valid: true,
        reason: None,
    }
}

fn invalid_status(reason: ValidityReason) -> ValidityStatus {
    ValidityStatus {
        valid: false,
        reason: Some(reason),
    }
}

fn grant_status(
    snapshot: Option<&MachineExternalGrant>,
    current: Option<&MachineExternalGrant>,
) -> ValidityStatus {
    let Some(snapshot) = snapshot else {
        return invalid_status(ValidityReason::Missing);
    };
    let Some(current) = current else {
        return invalid_status(ValidityReason::Missing);
    };
    if current.user_id != snapshot.user_id || current.revision != snapshot.revision {
        return invalid_status(ValidityReason::Changed);
    }
    if current.status == GrantStatus::Revoked {
        return invalid_status(ValidityReason::Revoked);
    }
    if current.expires_at <= Utc::now() {
        return invalid_status(ValidityReason::Expired);
    }
    valid_status()
}

/// Evidence read in one database snapshot. These fields are never returned as
/// authentication material; they only relate the live source to a grant.
struct LiveSourceStatus {
    status: ValidityStatus,
    user_epoch: Option<i64>,
    client_epoch: Option<i64>,
    consent: Option<ProtectedConsentRef>,
}
impl LiveSourceStatus {
    fn invalid(reason: ValidityReason) -> Self {
        Self {
            status: invalid_status(reason),
            user_epoch: None,
            client_epoch: None,
            consent: None,
        }
    }
}

async fn source_status_in_session(
    db: &Database,
    user: &str,
    source: &SourceReference,
    session: &mut ClientSession,
) -> AppResult<LiveSourceStatus> {
    let owner_row = db
        .collection::<Document>(USERS)
        .find_one(doc! {"_id": user})
        .session(&mut *session)
        .await?;
    let Some(owner_row) = owner_row else {
        return Ok(LiveSourceStatus::invalid(ValidityReason::Missing));
    };
    let owner: User =
        bson::from_document(owner_row.clone()).map_err(|_| AppError::MachineAuthorityStale)?;
    let mut result = LiveSourceStatus {
        status: valid_status(),
        user_epoch: Some(source_epoch(&owner_row)?),
        client_epoch: None,
        consent: None,
    };
    if !owner.is_active {
        result.status = invalid_status(ValidityReason::Disabled);
        return Ok(result);
    }
    if !owner.user_type.is_person() {
        result.status = invalid_status(ValidityReason::Ineligible);
        return Ok(result);
    }
    match source {
        SourceReference::ApiKey { api_key_id } => {
            let key = db
                .collection::<ApiKey>(KEYS)
                .find_one(doc! {"_id": api_key_id, "user_id": user})
                .session(&mut *session)
                .await?;
            result.status = if let Some(key) = key {
                if !key.is_active {
                    invalid_status(ValidityReason::Disabled)
                } else if key
                    .expires_at
                    .is_some_and(|expires_at| expires_at <= Utc::now())
                {
                    invalid_status(ValidityReason::Expired)
                } else if !key_eligible(&key, user)
                    || key_has_chat_binding(db, api_key_id, session).await?
                {
                    invalid_status(ValidityReason::Ineligible)
                } else {
                    valid_status()
                }
            } else {
                invalid_status(ValidityReason::Missing)
            };
        }
        SourceReference::OAuth {
            oauth_client_id, ..
        } => {
            let client_row = db
                .collection::<Document>(CLIENTS)
                .find_one(doc! {"_id": oauth_client_id})
                .session(&mut *session)
                .await?;
            let Some(client_row) = client_row else {
                result.status = invalid_status(ValidityReason::Missing);
                return Ok(result);
            };
            result.client_epoch = Some(source_epoch(&client_row)?);
            let client: OauthClient =
                bson::from_document(client_row).map_err(|_| AppError::MachineAuthorityStale)?;
            if !client.is_active {
                result.status = invalid_status(ValidityReason::Disabled);
                return Ok(result);
            }
            if !has_scope(&client.allowed_scopes, "machine")
                || !crate::mw::auth::scope_allows_rest_proxy(&client.allowed_scopes)
            {
                result.status = invalid_status(ValidityReason::Ineligible);
                return Ok(result);
            }
            // Current source authentication is independent of the old grant's
            // consent reference. Binding drift invalidates the grant below.
            let live_consent = db
                .collection::<Consent>(CONSENTS)
                .find_one(doc! {"user_id": user, "client_id": oauth_client_id})
                .session(&mut *session)
                .await?;
            let Some(live_consent) = live_consent else {
                result.status = invalid_status(ValidityReason::Missing);
                return Ok(result);
            };
            result.consent = live_consent
                .revision
                .as_ref()
                .filter(|revision| !revision.is_empty())
                .map(|revision| ProtectedConsentRef {
                    consent_id: live_consent.id.clone(),
                    consent_revision: revision.clone(),
                });
            if live_consent
                .expires_at
                .is_some_and(|expires_at| expires_at <= Utc::now())
            {
                result.status = invalid_status(ValidityReason::Expired);
            } else if result.consent.is_none()
                || !has_scope(&live_consent.scopes, "machine")
                || !crate::mw::auth::scope_allows_rest_proxy(&live_consent.scopes)
            {
                result.status = invalid_status(ValidityReason::Ineligible);
            }
        }
    }
    Ok(result)
}

fn source_binding_matches(
    grant: &MachineExternalGrant,
    source: &SourceReference,
    live: &LiveSourceStatus,
) -> bool {
    if live
        .user_epoch
        .is_some_and(|epoch| epoch != grant.user_security_revision)
    {
        return false;
    }
    match (&grant.binding, source) {
        (
            ExternalGrantBinding::ApiKey {
                api_key_id: bound, ..
            },
            SourceReference::ApiKey { api_key_id },
        ) => bound == api_key_id,
        (
            ExternalGrantBinding::OAuth {
                oauth_client_id: bound,
                consent_id,
                consent_revision,
            },
            SourceReference::OAuth {
                oauth_client_id,
                consent,
            },
        ) => {
            bound == oauth_client_id
                && live
                    .client_epoch
                    .is_none_or(|epoch| grant.client_security_revision == Some(epoch))
                && consent.as_ref().is_none_or(|reference| {
                    reference.consent_id == *consent_id
                        && reference.consent_revision == *consent_revision
                })
                && live.consent.as_ref().is_none_or(|reference| {
                    reference.consent_id == *consent_id
                        && reference.consent_revision == *consent_revision
                })
        }
        _ => false,
    }
}

/// Owner-facing checks, not execution authority. Source validity describes the
/// current parent Key or Client/Consent, never a client's unobserved token or a
/// particular child login. Execution retains its existing fenced evaluator.
pub async fn evaluate_validity(
    db: &Database,
    user: &str,
    source: &SourceReference,
    grant: Option<&MachineExternalGrant>,
) -> AppResult<GrantValidity> {
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .await?;
    let current = if let Some(grant) = grant {
        db.collection::<MachineExternalGrant>(COLLECTION_NAME)
            .find_one(doc! {"_id": &grant.id, "user_id": user})
            .session(&mut session)
            .await?
    } else {
        None
    };
    let mut grant_status = grant_status(grant, current.as_ref());
    let live = source_status_in_session(db, user, source, &mut session).await?;
    if grant_status.valid
        && current
            .as_ref()
            .is_some_and(|row| !source_binding_matches(row, source, &live))
    {
        grant_status = invalid_status(ValidityReason::Changed);
    }
    session.abort_transaction().await?;
    let combined = live.status.valid && grant_status.valid;
    Ok(GrantValidity {
        source: live.status,
        grant: grant_status,
        combined,
    })
}

pub async fn evaluate_validity_for_grant(
    db: &Database,
    grant: &MachineExternalGrant,
) -> AppResult<GrantValidity> {
    let source = match &grant.binding {
        ExternalGrantBinding::ApiKey { api_key_id, .. } => SourceReference::ApiKey {
            api_key_id: api_key_id.clone(),
        },
        ExternalGrantBinding::OAuth {
            oauth_client_id,
            consent_id,
            consent_revision,
        } => SourceReference::OAuth {
            oauth_client_id: oauth_client_id.clone(),
            consent: Some(ProtectedConsentRef {
                consent_id: consent_id.clone(),
                consent_revision: consent_revision.clone(),
            }),
        },
    };
    evaluate_validity(db, &grant.user_id, &source, Some(grant)).await
}

/// User disable/delete integration hook. Call in the transaction containing the
/// actual source security update, BEFORE deleting the user. No unrelated auth
/// path invokes this. The epoch immediately rejects every old grant, even if
/// the user is reenabled before the bounded cancellation sweep finishes.
pub async fn user_security_changed_in_session(
    db: &Database,
    user: &str,
    reason: &str,
    session: &mut ClientSession,
) -> AppResult<u64> {
    let changed=db.collection::<Document>(USERS).update_one(doc! {"_id":user},doc! {"$inc":{"machine_external_security_revision":1_i64,"machine_external_fence":1_i64}}).session(&mut *session).await?;
    if changed.matched_count != 1 {
        return Err(AppError::MachinePermissionRevoked);
    }
    invalidate_user_in_session(db, user, reason, 1000, session).await
}

#[cfg(test)]
#[path = "machine_external_grant_tests.rs"]
mod tests;
