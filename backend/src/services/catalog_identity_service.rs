use chrono::{DateTime, Utc};
use mongodb::bson::{self, Bson, Document, doc};
use serde::Serialize;

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::audit_service::{self, AuditActor};
use crate::services::user_service_service::IdentityConfig;

const MAX_CATCH_UP_REVISIONS: usize = 32;

const IDENTITY_FIELDS: [&str; 8] = [
    "identity_propagation_mode",
    "identity_include_user_id",
    "identity_include_email",
    "identity_include_name",
    "identity_jwt_audience",
    "forward_access_token",
    "inject_delegation_token",
    "delegation_token_scope",
];

#[derive(Clone, Debug)]
pub struct CatalogIdentityState {
    pub revision: DateTime<Utc>,
    pub config: IdentityConfig,
}

impl CatalogIdentityState {
    pub fn from_service(service: &DownstreamService) -> Self {
        Self {
            revision: service.updated_at,
            config: effective_identity_config(service),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct FieldReconciliationCounts {
    pub field: String,
    pub matched_count: u64,
    pub modified_count: u64,
    pub skipped_customized_count: u64,
}

/// Counts materialized field snapshots rather than distinct rows because each
/// identity field has independent owner-customization semantics.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ReconciliationReport {
    pub fields: Vec<String>,
    pub matched_count: u64,
    pub modified_count: u64,
    pub skipped_customized_count: u64,
    pub field_results: Vec<FieldReconciliationCounts>,
}

impl ReconciliationReport {
    fn with_fields(fields: impl IntoIterator<Item = &'static str>) -> Self {
        let mut report = Self::default();
        for field in fields {
            report.ensure_field(field);
        }
        report
    }

    fn ensure_field(&mut self, field: &'static str) {
        if self
            .field_results
            .iter()
            .any(|result| result.field == field)
        {
            return;
        }
        self.fields.push(field.to_string());
        self.field_results.push(FieldReconciliationCounts {
            field: field.to_string(),
            ..FieldReconciliationCounts::default()
        });
    }

    fn record(&mut self, field: &'static str, matched: u64, modified: u64, skipped: u64) {
        self.ensure_field(field);
        let result = self
            .field_results
            .iter_mut()
            .find(|result| result.field == field)
            .expect("field result was inserted above");
        result.matched_count += matched;
        result.modified_count += modified;
        result.skipped_customized_count += skipped;
        self.matched_count += matched;
        self.modified_count += modified;
        self.skipped_customized_count += skipped;
    }

    fn merge(&mut self, other: Self) {
        for result in other.field_results {
            self.record(
                identity_field_name(&result.field),
                result.matched_count,
                result.modified_count,
                result.skipped_customized_count,
            );
        }
    }
}

struct ReconciliationFailure {
    report: ReconciliationReport,
    stage: &'static str,
    source: AppError,
}

#[derive(Clone)]
struct IdentityFieldChange {
    name: &'static str,
    previous: Bson,
    current: Bson,
    previous_is_model_default: bool,
}

/// Return the identity values a newly provisioned `UserService` receives.
/// This includes the existing active-mode include-flag defaults and scope
/// normalization performed by `user_service_service` at creation time.
pub fn effective_identity_config(service: &DownstreamService) -> IdentityConfig {
    let has_active_mode = matches!(
        service.identity_propagation_mode.as_str(),
        "headers" | "jwt" | "both"
    );
    let all_flags_off = !service.identity_include_user_id
        && !service.identity_include_email
        && !service.identity_include_name;
    let apply_defaults = has_active_mode && all_flags_off;
    let delegation_token_scope = {
        let scopes: Vec<&str> = service.delegation_token_scope.split_whitespace().collect();
        if scopes.is_empty() {
            "llm:proxy".to_string()
        } else {
            scopes.join(" ")
        }
    };

    IdentityConfig {
        identity_propagation_mode: service.identity_propagation_mode.clone(),
        identity_include_user_id: service.identity_include_user_id || apply_defaults,
        identity_include_email: service.identity_include_email || apply_defaults,
        identity_include_name: service.identity_include_name || apply_defaults,
        identity_jwt_audience: service.identity_jwt_audience.clone(),
        forward_access_token: service.forward_access_token,
        inject_delegation_token: service.inject_delegation_token,
        delegation_token_scope,
    }
}

/// Reconcile an admin catalog edit and catch up across any later catalog
/// revisions that committed while this request was updating instances.
///
/// Legacy `UserService` rows have no per-field ownership provenance, so an
/// explicit owner write equal to the catalog value (including an ABA change)
/// is indistinguishable from inheritance. Equality with the previous effective
/// catalog value is intentionally the bounded fallback until provenance exists.
pub async fn propagate_catalog_update(
    db: &mongodb::Database,
    actor: &AuditActor,
    catalog_service_id: &str,
    previous: CatalogIdentityState,
    committed: CatalogIdentityState,
) -> AppResult<ReconciliationReport> {
    let outcome = reconcile_catalog_revisions(db, catalog_service_id, previous, committed).await;

    match outcome {
        Ok(report) => {
            persist_outcome(
                db,
                actor,
                "catalog_identity_propagation_succeeded",
                catalog_service_id,
                "success",
                &report,
                None,
            )
            .await?;
            Ok(report)
        }
        Err(failure) => {
            if let Err(audit_error) = persist_outcome(
                db,
                actor,
                "catalog_identity_propagation_failed",
                catalog_service_id,
                "failure",
                &failure.report,
                Some(failure.stage),
            )
            .await
            {
                tracing::error!(
                    catalog_service_id,
                    error = %audit_error,
                    "Failed to persist catalog identity propagation failure audit"
                );
            }
            Err(failure.source)
        }
    }
}

/// Deliberately overwrite all identity fields for every instance of one
/// catalog service. This is the operator recovery path for legacy drift and
/// previously failed propagation.
pub async fn force_resync(
    db: &mongodb::Database,
    actor: &AuditActor,
    service: &DownstreamService,
) -> AppResult<u64> {
    let mut target = service.clone();
    let mut report = ReconciliationReport::with_fields(IDENTITY_FIELDS);

    for _ in 0..MAX_CATCH_UP_REVISIONS {
        let config = effective_identity_config(&target);
        let mut set_doc = identity_set_document(&config);
        set_doc.insert("updated_at", bson::DateTime::from_chrono(Utc::now()));
        let result = match db
            .collection::<UserService>(USER_SERVICES)
            .update_many(
                doc! { "catalog_service_id": &service.id },
                doc! { "$set": set_doc },
            )
            .await
        {
            Ok(result) => result,
            Err(error) => {
                audit_resync_failure(db, actor, &service.id, &report, "user_services_update").await;
                return Err(error.into());
            }
        };
        for field in IDENTITY_FIELDS {
            report.record(field, result.matched_count, result.modified_count, 0);
        }

        let latest = match db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "_id": &service.id })
            .await
        {
            Ok(Some(latest)) => latest,
            Ok(None) => {
                audit_resync_failure(db, actor, &service.id, &report, "catalog_revision_read")
                    .await;
                return Err(AppError::NotFound("Service not found".to_string()));
            }
            Err(error) => {
                audit_resync_failure(db, actor, &service.id, &report, "catalog_revision_read")
                    .await;
                return Err(error.into());
            }
        };

        if latest.updated_at == target.updated_at {
            persist_outcome(
                db,
                actor,
                "catalog_identity_resync_succeeded",
                &service.id,
                "success",
                &report,
                None,
            )
            .await?;
            return Ok(result.matched_count);
        }

        target = latest;
    }

    audit_resync_failure(db, actor, &service.id, &report, "catalog_revision_limit").await;
    Err(AppError::Conflict(
        "Catalog identity changed repeatedly during resync; retry the request".to_string(),
    ))
}

async fn audit_resync_failure(
    db: &mongodb::Database,
    actor: &AuditActor,
    catalog_service_id: &str,
    report: &ReconciliationReport,
    stage: &'static str,
) {
    if let Err(audit_error) = persist_outcome(
        db,
        actor,
        "catalog_identity_resync_failed",
        catalog_service_id,
        "failure",
        report,
        Some(stage),
    )
    .await
    {
        tracing::error!(
            catalog_service_id,
            error = %audit_error,
            "Failed to persist catalog identity resync failure audit"
        );
    }
}

async fn reconcile_catalog_revisions(
    db: &mongodb::Database,
    catalog_service_id: &str,
    mut previous: CatalogIdentityState,
    mut target: CatalogIdentityState,
) -> Result<ReconciliationReport, ReconciliationFailure> {
    let mut aggregate = ReconciliationReport::default();

    for _ in 0..MAX_CATCH_UP_REVISIONS {
        match reconcile_transition(db, catalog_service_id, &previous.config, &target.config).await {
            Ok(report) => aggregate.merge(report),
            Err(mut failure) => {
                aggregate.merge(failure.report);
                failure.report = aggregate;
                return Err(failure);
            }
        }

        let latest = db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "_id": catalog_service_id })
            .await
            .map_err(|error| ReconciliationFailure {
                report: aggregate.clone(),
                stage: "catalog_revision_read",
                source: error.into(),
            })?
            .ok_or_else(|| ReconciliationFailure {
                report: aggregate.clone(),
                stage: "catalog_revision_read",
                source: AppError::NotFound("Service not found".to_string()),
            })?;
        let latest = CatalogIdentityState::from_service(&latest);

        if latest.revision == target.revision {
            return Ok(aggregate);
        }

        previous = target;
        target = latest;
    }

    Err(ReconciliationFailure {
        report: aggregate,
        stage: "catalog_revision_limit",
        source: AppError::Conflict(
            "Catalog identity changed repeatedly during propagation; run identity resync"
                .to_string(),
        ),
    })
}

async fn reconcile_transition(
    db: &mongodb::Database,
    catalog_service_id: &str,
    previous: &IdentityConfig,
    current: &IdentityConfig,
) -> Result<ReconciliationReport, ReconciliationFailure> {
    let changes = identity_field_changes(previous, current);
    let mut report = ReconciliationReport::with_fields(changes.iter().map(|change| change.name));
    if changes.is_empty() {
        return Ok(report);
    }

    let total = db
        .collection::<UserService>(USER_SERVICES)
        .count_documents(doc! { "catalog_service_id": catalog_service_id })
        .await
        .map_err(|error| ReconciliationFailure {
            report: report.clone(),
            stage: "user_services_count",
            source: error.into(),
        })?;
    let updated_at = bson::DateTime::from_chrono(Utc::now());

    for change in changes {
        let filter = inherited_field_filter(catalog_service_id, &change);
        let mut set_doc = Document::new();
        set_doc.insert(change.name, change.current);
        set_doc.insert("updated_at", updated_at);
        let result = db
            .collection::<UserService>(USER_SERVICES)
            .update_many(filter, doc! { "$set": set_doc })
            .await
            .map_err(|error| ReconciliationFailure {
                report: report.clone(),
                stage: "user_services_update",
                source: error.into(),
            })?;
        report.record(
            change.name,
            result.matched_count,
            result.modified_count,
            total.saturating_sub(result.matched_count),
        );
    }

    Ok(report)
}

fn identity_field_changes(
    previous: &IdentityConfig,
    current: &IdentityConfig,
) -> Vec<IdentityFieldChange> {
    let defaults = IdentityConfig::none();
    let candidates = [
        field_change(
            "identity_propagation_mode",
            &previous.identity_propagation_mode,
            &current.identity_propagation_mode,
            &defaults.identity_propagation_mode,
        ),
        field_change(
            "identity_include_user_id",
            previous.identity_include_user_id,
            current.identity_include_user_id,
            defaults.identity_include_user_id,
        ),
        field_change(
            "identity_include_email",
            previous.identity_include_email,
            current.identity_include_email,
            defaults.identity_include_email,
        ),
        field_change(
            "identity_include_name",
            previous.identity_include_name,
            current.identity_include_name,
            defaults.identity_include_name,
        ),
        field_change(
            "identity_jwt_audience",
            previous.identity_jwt_audience.clone(),
            current.identity_jwt_audience.clone(),
            defaults.identity_jwt_audience,
        ),
        field_change(
            "forward_access_token",
            previous.forward_access_token,
            current.forward_access_token,
            defaults.forward_access_token,
        ),
        field_change(
            "inject_delegation_token",
            previous.inject_delegation_token,
            current.inject_delegation_token,
            defaults.inject_delegation_token,
        ),
        field_change(
            "delegation_token_scope",
            &previous.delegation_token_scope,
            &current.delegation_token_scope,
            &defaults.delegation_token_scope,
        ),
    ];

    candidates.into_iter().flatten().collect()
}

fn field_change<T: Serialize>(
    name: &'static str,
    previous: T,
    current: T,
    model_default: T,
) -> Option<IdentityFieldChange> {
    let previous = bson::to_bson(&previous).expect("identity fields always serialize to BSON");
    let current = bson::to_bson(&current).expect("identity fields always serialize to BSON");
    if previous == current {
        return None;
    }
    let model_default =
        bson::to_bson(&model_default).expect("identity defaults always serialize to BSON");
    Some(IdentityFieldChange {
        name,
        previous_is_model_default: previous == model_default,
        previous,
        current,
    })
}

fn inherited_field_filter(catalog_service_id: &str, change: &IdentityFieldChange) -> Document {
    let mut filter = doc! { "catalog_service_id": catalog_service_id };
    let mut equals_previous = Document::new();
    equals_previous.insert(change.name, change.previous.clone());

    // Missing legacy fields deserialize to the model default, so they are an
    // inherited match only when the previous effective value is that default.
    // `null` equality already matches missing fields in MongoDB.
    if change.previous_is_model_default && change.previous != Bson::Null {
        let mut missing = Document::new();
        missing.insert(change.name, doc! { "$exists": false });
        filter.insert("$or", vec![equals_previous, missing]);
    } else {
        filter.extend(equals_previous);
    }
    filter
}

fn identity_set_document(config: &IdentityConfig) -> Document {
    doc! {
        "identity_propagation_mode": &config.identity_propagation_mode,
        "identity_include_user_id": config.identity_include_user_id,
        "identity_include_email": config.identity_include_email,
        "identity_include_name": config.identity_include_name,
        "identity_jwt_audience": bson::to_bson(&config.identity_jwt_audience)
            .expect("identity audience always serializes to BSON"),
        "forward_access_token": config.forward_access_token,
        "inject_delegation_token": config.inject_delegation_token,
        "delegation_token_scope": &config.delegation_token_scope,
    }
}

async fn persist_outcome(
    db: &mongodb::Database,
    actor: &AuditActor,
    event_type: &'static str,
    catalog_service_id: &str,
    status: &'static str,
    report: &ReconciliationReport,
    failure_stage: Option<&'static str>,
) -> AppResult<()> {
    let mut data = serde_json::json!({
        "catalog_service_id": catalog_service_id,
        "status": status,
        "fields": report.fields,
        "matched_count": report.matched_count,
        "modified_count": report.modified_count,
        "skipped_customized_count": report.skipped_customized_count,
        "field_results": report.field_results,
    });
    if let Some(failure_stage) = failure_stage {
        data["failure_stage"] = serde_json::Value::String(failure_stage.to_string());
    }
    audit_service::log_actor_event(db.clone(), actor, event_type, Some(data)).await?;
    Ok(())
}

fn identity_field_name(field: &str) -> &'static str {
    IDENTITY_FIELDS
        .into_iter()
        .find(|candidate| *candidate == field)
        .expect("only identity field results are merged")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
    use crate::models::downstream_service::test_helpers::dummy_service;
    use crate::test_utils::{connect_test_database, test_user_service};

    fn actor() -> AuditActor {
        AuditActor {
            user_id: "admin-user".to_string(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        }
    }

    fn catalog_service(id: &str) -> DownstreamService {
        let mut service = dummy_service();
        service.id = id.to_string();
        service.delegation_token_scope = "llm:proxy".to_string();
        service.updated_at = Utc::now();
        service
    }

    fn field_result<'a>(
        report: &'a ReconciliationReport,
        field: &str,
    ) -> &'a FieldReconciliationCounts {
        report
            .field_results
            .iter()
            .find(|result| result.field == field)
            .expect("field result")
    }

    #[tokio::test]
    async fn propagation_is_per_field_and_preserves_owner_and_routing_fields() {
        let Some(db) = connect_test_database("catalog_identity_per_field").await else {
            eprintln!("skipping catalog identity test: no local MongoDB available");
            return;
        };
        let catalog_id = "catalog-per-field";
        let previous = catalog_service(catalog_id);
        let mut current = previous.clone();
        current.identity_propagation_mode = "headers".to_string();
        current.forward_access_token = true;
        current.delegation_token_scope = "proxy:*".to_string();
        current.updated_at = previous.updated_at + chrono::Duration::seconds(1);
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&current)
            .await
            .expect("insert current catalog service");

        let mut inherited = test_user_service(
            "inherited",
            "owner-a",
            "service-a",
            "endpoint-a",
            Some(catalog_id),
            Some("node-a"),
        );
        inherited.api_key_id = Some("credential-a".to_string());
        inherited.node_priority = 7;
        inherited.custom_user_agent = Some("OwnerAgent/1.0".to_string());
        inherited.admin_only = true;
        inherited.is_active = false;

        let mut customized_forward = test_user_service(
            "custom-forward",
            "owner-b",
            "service-b",
            "endpoint-b",
            Some(catalog_id),
            None,
        );
        customized_forward.forward_access_token = true;

        let mut customized_scope = test_user_service(
            "custom-scope",
            "owner-c",
            "service-c",
            "endpoint-c",
            Some(catalog_id),
            None,
        );
        customized_scope.delegation_token_scope = "owner:scope".to_string();

        db.collection::<UserService>(USER_SERVICES)
            .insert_many([&inherited, &customized_forward, &customized_scope])
            .await
            .expect("insert user services");

        let report = propagate_catalog_update(
            &db,
            &actor(),
            catalog_id,
            CatalogIdentityState::from_service(&previous),
            CatalogIdentityState::from_service(&current),
        )
        .await
        .expect("propagate identity update");

        let inherited_after = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": "inherited" })
            .await
            .expect("find inherited service")
            .expect("inherited service exists");
        assert_eq!(inherited_after.identity_propagation_mode, "headers");
        assert!(inherited_after.identity_include_user_id);
        assert!(inherited_after.identity_include_email);
        assert!(inherited_after.identity_include_name);
        assert!(inherited_after.forward_access_token);
        assert_eq!(inherited_after.delegation_token_scope, "proxy:*");
        assert_eq!(inherited_after.api_key_id.as_deref(), Some("credential-a"));
        assert_eq!(inherited_after.endpoint_id, "endpoint-a");
        assert_eq!(inherited_after.node_id.as_deref(), Some("node-a"));
        assert_eq!(inherited_after.node_priority, 7);
        assert_eq!(
            inherited_after.custom_user_agent.as_deref(),
            Some("OwnerAgent/1.0")
        );
        assert!(inherited_after.admin_only);
        assert!(!inherited_after.is_active);

        let customized_scope_after = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": "custom-scope" })
            .await
            .expect("find customized scope service")
            .expect("customized scope service exists");
        assert_eq!(customized_scope_after.identity_propagation_mode, "headers");
        assert!(customized_scope_after.forward_access_token);
        assert_eq!(customized_scope_after.delegation_token_scope, "owner:scope");

        let forward_counts = field_result(&report, "forward_access_token");
        assert_eq!(forward_counts.matched_count, 2);
        assert_eq!(forward_counts.skipped_customized_count, 1);
        let scope_counts = field_result(&report, "delegation_token_scope");
        assert_eq!(scope_counts.matched_count, 2);
        assert_eq!(scope_counts.skipped_customized_count, 1);

        let audit = db
            .collection::<AuditLog>(AUDIT_LOG)
            .find_one(doc! { "event_type": "catalog_identity_propagation_succeeded" })
            .await
            .expect("find propagation audit")
            .expect("propagation audit exists");
        let data = audit.event_data.expect("propagation audit data");
        assert_eq!(data["status"], "success");
        assert_eq!(data["catalog_service_id"], catalog_id);
        assert!(data.get("previous_value").is_none());
        assert!(data.get("current_value").is_none());
    }

    #[tokio::test]
    async fn unchanged_effective_identity_performs_no_user_service_write() {
        let Some(db) = connect_test_database("catalog_identity_noop").await else {
            eprintln!("skipping catalog identity test: no local MongoDB available");
            return;
        };
        let catalog_id = "catalog-noop";
        let previous = catalog_service(catalog_id);
        let mut current = previous.clone();
        current.updated_at = previous.updated_at + chrono::Duration::seconds(1);
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&current)
            .await
            .expect("insert current catalog service");
        let service = test_user_service(
            "user-service",
            "owner",
            "service",
            "endpoint",
            Some(catalog_id),
            None,
        );
        let original_updated_at = service.updated_at;
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert user service");

        let report = propagate_catalog_update(
            &db,
            &actor(),
            catalog_id,
            CatalogIdentityState::from_service(&previous),
            CatalogIdentityState::from_service(&current),
        )
        .await
        .expect("no-op propagation");

        assert!(report.fields.is_empty());
        let after = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": "user-service" })
            .await
            .expect("find user service")
            .expect("user service exists");
        assert_eq!(
            after.updated_at.timestamp_millis(),
            original_updated_at.timestamp_millis()
        );
    }

    #[tokio::test]
    async fn propagation_catches_up_to_a_later_catalog_revision() {
        let Some(db) = connect_test_database("catalog_identity_revision_catchup").await else {
            eprintln!("skipping catalog identity test: no local MongoDB available");
            return;
        };
        let catalog_id = "catalog-catchup";
        let previous = catalog_service(catalog_id);
        let mut committed = previous.clone();
        committed.delegation_token_scope = "proxy:*".to_string();
        committed.updated_at = previous.updated_at + chrono::Duration::seconds(1);
        let mut latest = committed.clone();
        latest.delegation_token_scope = "account:read".to_string();
        latest.updated_at = committed.updated_at + chrono::Duration::seconds(1);
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&latest)
            .await
            .expect("insert latest catalog service");
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                "user-service",
                "owner",
                "service",
                "endpoint",
                Some(catalog_id),
                None,
            ))
            .await
            .expect("insert user service");

        let report = propagate_catalog_update(
            &db,
            &actor(),
            catalog_id,
            CatalogIdentityState::from_service(&previous),
            CatalogIdentityState::from_service(&committed),
        )
        .await
        .expect("catch up propagation");

        let after = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": "user-service" })
            .await
            .expect("find user service")
            .expect("user service exists");
        assert_eq!(after.delegation_token_scope, "account:read");
        assert_eq!(
            field_result(&report, "delegation_token_scope").matched_count,
            2
        );
    }

    #[tokio::test]
    async fn propagation_failure_is_persisted_without_identity_values() {
        let Some(db) = connect_test_database("catalog_identity_failure_audit").await else {
            eprintln!("skipping catalog identity test: no local MongoDB available");
            return;
        };
        let catalog_id = "catalog-failure";
        let previous = catalog_service(catalog_id);
        let mut current = previous.clone();
        current.forward_access_token = true;
        current.updated_at = previous.updated_at + chrono::Duration::seconds(1);
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&current)
            .await
            .expect("insert current catalog service");
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                "user-service",
                "owner",
                "service",
                "endpoint",
                Some(catalog_id),
                None,
            ))
            .await
            .expect("insert user service");
        db.run_command(doc! {
            "collMod": USER_SERVICES,
            "validator": { "forward_access_token": { "$ne": true } },
            "validationLevel": "strict",
            "validationAction": "error",
        })
        .await
        .expect("install rejecting validator");

        let error = propagate_catalog_update(
            &db,
            &actor(),
            catalog_id,
            CatalogIdentityState::from_service(&previous),
            CatalogIdentityState::from_service(&current),
        )
        .await
        .expect_err("validator should reject propagation");
        assert!(matches!(error, AppError::DatabaseError(_)));

        let audit = db
            .collection::<AuditLog>(AUDIT_LOG)
            .find_one(doc! { "event_type": "catalog_identity_propagation_failed" })
            .await
            .expect("find failure audit")
            .expect("failure audit exists");
        let data = audit.event_data.expect("failure audit data");
        assert_eq!(data["status"], "failure");
        assert_eq!(data["failure_stage"], "user_services_update");
        assert_eq!(data["fields"], serde_json::json!(["forward_access_token"]));
        assert!(data.get("previous_value").is_none());
        assert!(data.get("current_value").is_none());
    }

    #[tokio::test]
    async fn force_resync_overwrites_custom_identity_only() {
        let Some(db) = connect_test_database("catalog_identity_force_resync").await else {
            eprintln!("skipping catalog identity test: no local MongoDB available");
            return;
        };
        let catalog_id = "catalog-force";
        let mut service = catalog_service(catalog_id);
        service.identity_propagation_mode = "jwt".to_string();
        service.forward_access_token = true;
        service.delegation_token_scope = "account:read".to_string();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert catalog service");
        let mut user_service = test_user_service(
            "user-service",
            "owner",
            "service",
            "endpoint",
            Some(catalog_id),
            Some("node-a"),
        );
        user_service.identity_propagation_mode = "both".to_string();
        user_service.delegation_token_scope = "owner:scope".to_string();
        user_service.node_priority = 9;
        user_service.custom_user_agent = Some("OwnerAgent/2.0".to_string());
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&user_service)
            .await
            .expect("insert user service");

        let affected = force_resync(&db, &actor(), &service)
            .await
            .expect("force resync");
        assert_eq!(affected, 1);

        let after = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": "user-service" })
            .await
            .expect("find user service")
            .expect("user service exists");
        assert_eq!(after.identity_propagation_mode, "jwt");
        assert!(after.identity_include_user_id);
        assert!(after.identity_include_email);
        assert!(after.identity_include_name);
        assert!(after.forward_access_token);
        assert_eq!(after.delegation_token_scope, "account:read");
        assert_eq!(after.node_id.as_deref(), Some("node-a"));
        assert_eq!(after.node_priority, 9);
        assert_eq!(after.custom_user_agent.as_deref(), Some("OwnerAgent/2.0"));
    }

    #[tokio::test]
    async fn force_resync_catches_up_when_given_a_stale_catalog_snapshot() {
        let Some(db) = connect_test_database("catalog_identity_force_catchup").await else {
            eprintln!("skipping catalog identity test: no local MongoDB available");
            return;
        };
        let catalog_id = "catalog-force-catchup";
        let stale = catalog_service(catalog_id);
        let mut latest = stale.clone();
        latest.identity_propagation_mode = "jwt".to_string();
        latest.forward_access_token = true;
        latest.delegation_token_scope = "account:read".to_string();
        latest.updated_at = stale.updated_at + chrono::Duration::seconds(1);
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&latest)
            .await
            .expect("insert latest catalog service");
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                "user-service",
                "owner",
                "service",
                "endpoint",
                Some(catalog_id),
                None,
            ))
            .await
            .expect("insert user service");

        let affected = force_resync(&db, &actor(), &stale)
            .await
            .expect("force resync should catch up");
        assert_eq!(affected, 1);

        let after = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": "user-service" })
            .await
            .expect("find user service")
            .expect("user service exists");
        assert_eq!(after.identity_propagation_mode, "jwt");
        assert!(after.forward_access_token);
        assert_eq!(after.delegation_token_scope, "account:read");
    }
}
