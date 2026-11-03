use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        crm_object::{CrmField, CrmFieldKind, CrmObject},
        crm_record::CrmActor,
    },
    mw::auth::AuthUser,
    services::{
        audit_service,
        crm::{self, activities, links, objects, query, records, views},
        feature_flag_service, org_service,
    },
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/objects", get(list_objects))
        .route("/objects/{key}", get(get_object))
        .route("/objects/{key}/fields", post(add_field))
        .route("/objects/{key}/fields/{field_key}", patch(patch_field))
        .route("/query", post(table_query))
        .route("/aggregate", post(aggregate_query))
        .route("/records", post(create_record))
        .route("/records/{id}", get(get_record).patch(patch_record))
        .route("/records/{id}/archive", post(archive_record))
        .route("/records/{id}/restore", post(restore_record))
        .route("/records/{id}/move", post(move_record))
        .route("/records/{id}/activities", get(list_activities))
        .route("/records/{id}/notes", post(add_note))
        .route("/records/{id}/related", get(related_records))
        .route("/views", get(list_views).post(create_view))
        .route(
            "/views/{id}",
            get(get_view).put(update_view).delete(delete_view),
        )
        .route("/views/validate", post(validate_view))
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    Read,
    Work,
    Schema,
}

#[derive(Debug, Serialize)]
struct ObjectResponse {
    #[serde(flatten)]
    object: CrmObject,
    access: AccessLevel,
}

#[derive(Debug, Serialize)]
struct ObjectListResponse {
    objects: Vec<ObjectResponse>,
}

#[derive(Debug, Deserialize, Default)]
pub struct OwnerQuery {
    owner_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ActivityQuery {
    owner_id: Option<String>,
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AddFieldBody {
    owner_id: Option<String>,
    expected_schema_revision: i64,
    key: String,
    label: String,
    required: bool,
    #[serde(default)]
    archived: bool,
    #[serde(flatten)]
    kind: CrmFieldKind,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PatchFieldBody {
    owner_id: Option<String>,
    expected_schema_revision: i64,
    label: Option<String>,
    required: Option<bool>,
    archived: Option<bool>,
    options: Option<Vec<crate::models::crm_object::CrmSelectOption>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TableBody {
    owner_id: Option<String>,
    #[serde(flatten)]
    query: query::CrmTableQuery,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AggregateBody {
    owner_id: Option<String>,
    #[serde(flatten)]
    query: query::CrmAggregateQuery,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CreateRecordBody {
    owner_id: Option<String>,
    #[serde(flatten)]
    input: records::CreateRecordInput,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PatchRecordBody {
    owner_id: Option<String>,
    #[serde(flatten)]
    input: records::PatchRecordInput,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RevisionBody {
    owner_id: Option<String>,
    expected_revision: i64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MoveBody {
    owner_id: Option<String>,
    expected_revision: i64,
    to: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NoteBody {
    owner_id: Option<String>,
    body: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CreateViewBody {
    owner_id: Option<String>,
    title: String,
    envelope: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct UpdateViewBody {
    owner_id: Option<String>,
    expected_revision: i64,
    title: String,
    envelope: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ValidateViewBody {
    owner_id: Option<String>,
    envelope: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize)]
struct ValidateViewResponse {
    valid: bool,
    errors: Vec<crate::errors::CrmValidationIssue>,
}

async fn list_objects(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<OwnerQuery>,
) -> AppResult<Json<ObjectListResponse>> {
    let actor = guard(&state, &auth).await?;
    let (owner, access) = owner_access(&state, &actor, query.owner_id.as_deref()).await?;
    let objects = objects::list(&state.db, &owner)
        .await?
        .into_iter()
        .map(|object| ObjectResponse { object, access })
        .collect();
    Ok(Json(ObjectListResponse { objects }))
}

async fn get_object(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(key): Path<String>,
    Query(query): Query<OwnerQuery>,
) -> AppResult<Json<ObjectResponse>> {
    let actor = guard(&state, &auth).await?;
    let (owner, access) = owner_access(&state, &actor, query.owner_id.as_deref()).await?;
    Ok(Json(ObjectResponse {
        object: objects::get(&state.db, &owner, &key).await?,
        access,
    }))
}

async fn add_field(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(key): Path<String>,
    headers: HeaderMap,
    Json(body): Json<AddFieldBody>,
) -> AppResult<Json<ObjectResponse>> {
    let actor = guard(&state, &auth).await?;
    let (owner, access) = owner_access(&state, &actor, body.owner_id.as_deref()).await?;
    require_schema(access)?;
    let field = CrmField {
        key: body.key,
        label: body.label,
        required: body.required,
        archived: body.archived,
        kind: body.kind,
    };
    let receipt = mutation_receipt(
        &headers,
        &owner,
        &actor,
        "POST",
        "/objects/{key}/fields",
        &[("key", key.as_str())],
        &serde_json::json!({"expected_schema_revision": body.expected_schema_revision, "field": &field}),
    )?;
    let object = objects::add_field(
        &state.db,
        &owner,
        &key,
        body.expected_schema_revision,
        field,
        receipt,
    )
    .await?;
    audit(&state, &auth, "crm_field_added", &owner, Some(&object.id));
    Ok(Json(ObjectResponse { object, access }))
}

async fn patch_field(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((key, field_key)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<PatchFieldBody>,
) -> AppResult<Json<ObjectResponse>> {
    let actor = guard(&state, &auth).await?;
    let (owner, access) = owner_access(&state, &actor, body.owner_id.as_deref()).await?;
    require_schema(access)?;
    let receipt = mutation_receipt(
        &headers,
        &owner,
        &actor,
        "PATCH",
        "/objects/{key}/fields/{field_key}",
        &[("key", key.as_str()), ("field_key", field_key.as_str())],
        &body,
    )?;
    let object = objects::patch_field(
        &state.db,
        &owner,
        &key,
        &field_key,
        objects::PatchField {
            expected_schema_revision: body.expected_schema_revision,
            label: body.label,
            required: body.required,
            archived: body.archived,
            options: body.options,
        },
        receipt,
    )
    .await?;
    audit(&state, &auth, "crm_field_updated", &owner, Some(&object.id));
    Ok(Json(ObjectResponse { object, access }))
}

async fn table_query(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(body): Json<TableBody>,
) -> AppResult<Json<query::TableResult>> {
    let actor = guard(&state, &auth).await?;
    let (owner, _) = owner_access(&state, &actor, body.owner_id.as_deref()).await?;
    let receipt = mutation_receipt(&headers, &owner, &actor, "POST", "/query", &[], &body)?;
    if let Some(response) = records::lookup_receipt(&state.db, receipt.as_ref()).await? {
        return Ok(Json(response));
    }
    let response = query::table(&state.db, &owner, &body.query).await?;
    Ok(Json(
        records::store_response(&state.db, receipt, response).await?,
    ))
}

async fn aggregate_query(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(body): Json<AggregateBody>,
) -> AppResult<Json<query::AggregateResult>> {
    let actor = guard(&state, &auth).await?;
    let (owner, _) = owner_access(&state, &actor, body.owner_id.as_deref()).await?;
    let receipt = mutation_receipt(&headers, &owner, &actor, "POST", "/aggregate", &[], &body)?;
    if let Some(response) = records::lookup_receipt(&state.db, receipt.as_ref()).await? {
        return Ok(Json(response));
    }
    let response = query::aggregate(&state.db, &owner, &body.query).await?;
    Ok(Json(
        records::store_response(&state.db, receipt, response).await?,
    ))
}

async fn create_record(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(body): Json<CreateRecordBody>,
) -> AppResult<Json<records::RecordResponse>> {
    let actor = guard(&state, &auth).await?;
    let (owner, access) = owner_access(&state, &actor, body.owner_id.as_deref()).await?;
    require_work(access)?;
    let receipt = mutation_receipt(&headers, &owner, &actor, "POST", "/records", &[], &body)?;
    let response = records::create(
        &state.db,
        &owner,
        CrmActor::human(&actor),
        body.input,
        receipt,
    )
    .await?;
    audit(
        &state,
        &auth,
        "crm_record_created",
        &owner,
        Some(&response.id),
    );
    Ok(Json(response))
}

async fn get_record(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<OwnerQuery>,
) -> AppResult<Json<records::RecordResponse>> {
    let actor = guard(&state, &auth).await?;
    let raw = records::raw_by_id(&state.db, &id).await?;
    ensure_owner_hint(query.owner_id.as_deref(), &raw.owner_id)?;
    owner_access(&state, &actor, Some(&raw.owner_id)).await?;
    Ok(Json(records::get(&state.db, &raw.owner_id, &id).await?))
}

async fn patch_record(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PatchRecordBody>,
) -> AppResult<Json<records::RecordResponse>> {
    let actor = guard(&state, &auth).await?;
    let raw = records::raw_by_id(&state.db, &id).await?;
    ensure_owner_hint(body.owner_id.as_deref(), &raw.owner_id)?;
    let (_, access) = owner_access(&state, &actor, Some(&raw.owner_id)).await?;
    require_work(access)?;
    let receipt = mutation_receipt(
        &headers,
        &raw.owner_id,
        &actor,
        "PATCH",
        "/records/{id}",
        &[("id", id.as_str())],
        &body,
    )?;
    let response = records::patch(
        &state.db,
        &raw.owner_id,
        &id,
        CrmActor::human(&actor),
        body.input,
        receipt,
    )
    .await?;
    audit(
        &state,
        &auth,
        "crm_record_updated",
        &raw.owner_id,
        Some(&id),
    );
    Ok(Json(response))
}

async fn archive_record(
    state: State<AppState>,
    auth: AuthUser,
    path: Path<String>,
    headers: HeaderMap,
    body: Json<RevisionBody>,
) -> AppResult<Json<records::RecordResponse>> {
    set_record_archived(state, auth, path, headers, body, true).await
}

async fn restore_record(
    state: State<AppState>,
    auth: AuthUser,
    path: Path<String>,
    headers: HeaderMap,
    body: Json<RevisionBody>,
) -> AppResult<Json<records::RecordResponse>> {
    set_record_archived(state, auth, path, headers, body, false).await
}

async fn set_record_archived(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<RevisionBody>,
    archived: bool,
) -> AppResult<Json<records::RecordResponse>> {
    let actor = guard(&state, &auth).await?;
    let raw = records::raw_by_id(&state.db, &id).await?;
    ensure_owner_hint(body.owner_id.as_deref(), &raw.owner_id)?;
    let (_, access) = owner_access(&state, &actor, Some(&raw.owner_id)).await?;
    require_work(access)?;
    let action = if archived { "archive" } else { "restore" };
    let receipt = mutation_receipt(
        &headers,
        &raw.owner_id,
        &actor,
        "POST",
        if archived {
            "/records/{id}/archive"
        } else {
            "/records/{id}/restore"
        },
        &[("id", id.as_str())],
        &body,
    )?;
    let response = records::set_archived(
        &state.db,
        &raw.owner_id,
        &id,
        CrmActor::human(&actor),
        records::RevisionInput {
            expected_revision: body.expected_revision,
        },
        archived,
        receipt,
    )
    .await?;
    audit(
        &state,
        &auth,
        format!("crm_record_{action}"),
        &raw.owner_id,
        Some(&id),
    );
    Ok(Json(response))
}

async fn move_record(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<MoveBody>,
) -> AppResult<Json<records::RecordResponse>> {
    let actor = guard(&state, &auth).await?;
    let raw = records::raw_by_id(&state.db, &id).await?;
    ensure_owner_hint(body.owner_id.as_deref(), &raw.owner_id)?;
    let (_, access) = owner_access(&state, &actor, Some(&raw.owner_id)).await?;
    require_work(access)?;
    let receipt = mutation_receipt(
        &headers,
        &raw.owner_id,
        &actor,
        "POST",
        "/records/{id}/move",
        &[("id", id.as_str())],
        &body,
    )?;
    let response = records::move_stage(
        &state.db,
        &raw.owner_id,
        &id,
        CrmActor::human(&actor),
        records::MoveRecordInput {
            expected_revision: body.expected_revision,
            to: body.to,
        },
        receipt,
    )
    .await?;
    audit(&state, &auth, "crm_record_moved", &raw.owner_id, Some(&id));
    Ok(Json(response))
}

async fn list_activities(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<ActivityQuery>,
) -> AppResult<Json<activities::ActivityPage>> {
    let actor = guard(&state, &auth).await?;
    let raw = records::raw_by_id(&state.db, &id).await?;
    ensure_owner_hint(query.owner_id.as_deref(), &raw.owner_id)?;
    owner_access(&state, &actor, Some(&raw.owner_id)).await?;
    Ok(Json(
        activities::list(&state.db, &raw.owner_id, &id, query.cursor.as_deref()).await?,
    ))
}

async fn add_note(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<NoteBody>,
) -> AppResult<Json<activities::ActivityResponse>> {
    let actor = guard(&state, &auth).await?;
    let raw = records::raw_by_id(&state.db, &id).await?;
    ensure_owner_hint(body.owner_id.as_deref(), &raw.owner_id)?;
    let (_, access) = owner_access(&state, &actor, Some(&raw.owner_id)).await?;
    require_work(access)?;
    let receipt = mutation_receipt(
        &headers,
        &raw.owner_id,
        &actor,
        "POST",
        "/records/{id}/notes",
        &[("id", id.as_str())],
        &body,
    )?;
    let response = activities::add_note(
        &state.db,
        &raw.owner_id,
        &id,
        CrmActor::human(&actor),
        activities::NoteInput { body: body.body },
        receipt,
    )
    .await?;
    audit(&state, &auth, "crm_note_created", &raw.owner_id, Some(&id));
    Ok(Json(response))
}

async fn related_records(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<OwnerQuery>,
) -> AppResult<Json<links::RelatedResponse>> {
    let actor = guard(&state, &auth).await?;
    let raw = records::raw_by_id(&state.db, &id).await?;
    ensure_owner_hint(query.owner_id.as_deref(), &raw.owner_id)?;
    owner_access(&state, &actor, Some(&raw.owner_id)).await?;
    Ok(Json(links::related(&state.db, &raw.owner_id, &id).await?))
}

async fn list_views(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<OwnerQuery>,
) -> AppResult<Json<Vec<views::ViewResponse>>> {
    let actor = guard(&state, &auth).await?;
    let (owner, _) = owner_access(&state, &actor, query.owner_id.as_deref()).await?;
    Ok(Json(views::list(&state.db, &owner).await?))
}

async fn create_view(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(body): Json<CreateViewBody>,
) -> AppResult<Json<views::ViewResponse>> {
    let actor = guard(&state, &auth).await?;
    let (owner, access) = owner_access(&state, &actor, body.owner_id.as_deref()).await?;
    require_work(access)?;
    let receipt = mutation_receipt(&headers, &owner, &actor, "POST", "/views", &[], &body)?;
    let response = views::create(
        &state.db,
        &owner,
        CrmActor::human(&actor),
        views::CreateViewInput {
            title: body.title,
            envelope: body.envelope,
        },
        receipt,
    )
    .await?;
    audit(
        &state,
        &auth,
        "crm_view_created",
        &owner,
        Some(&response.id),
    );
    Ok(Json(response))
}

async fn get_view(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<OwnerQuery>,
) -> AppResult<Json<views::ViewResponse>> {
    let actor = guard(&state, &auth).await?;
    let raw = views::raw_by_id(&state.db, &id).await?;
    ensure_owner_hint(query.owner_id.as_deref(), &raw.owner_id)?;
    owner_access(&state, &actor, Some(&raw.owner_id)).await?;
    Ok(Json(views::get(&state.db, &raw.owner_id, &id).await?))
}

async fn update_view(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<UpdateViewBody>,
) -> AppResult<Json<views::ViewResponse>> {
    let actor = guard(&state, &auth).await?;
    let raw = views::raw_by_id(&state.db, &id).await?;
    ensure_owner_hint(body.owner_id.as_deref(), &raw.owner_id)?;
    let (_, access) = owner_access(&state, &actor, Some(&raw.owner_id)).await?;
    require_work(access)?;
    let receipt = mutation_receipt(
        &headers,
        &raw.owner_id,
        &actor,
        "PUT",
        "/views/{id}",
        &[("id", id.as_str())],
        &body,
    )?;
    let response = views::update(
        &state.db,
        &raw.owner_id,
        &id,
        views::UpdateViewInput {
            expected_revision: body.expected_revision,
            title: body.title,
            envelope: body.envelope,
        },
        receipt,
    )
    .await?;
    audit(&state, &auth, "crm_view_updated", &raw.owner_id, Some(&id));
    Ok(Json(response))
}

async fn delete_view(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<OwnerQuery>,
) -> AppResult<Json<views::DeleteViewResponse>> {
    let actor = guard(&state, &auth).await?;
    let raw = views::raw_by_id(&state.db, &id).await?;
    ensure_owner_hint(query.owner_id.as_deref(), &raw.owner_id)?;
    let (_, access) = owner_access(&state, &actor, Some(&raw.owner_id)).await?;
    if raw.created_by.user_id == actor {
        require_work(access)?;
    } else {
        require_schema(access)?;
    }
    let receipt = mutation_receipt(
        &headers,
        &raw.owner_id,
        &actor,
        "DELETE",
        "/views/{id}",
        &[("id", id.as_str())],
        &serde_json::Value::Null,
    )?;
    let response = views::delete(&state.db, &raw.owner_id, &id, receipt).await?;
    audit(&state, &auth, "crm_view_deleted", &raw.owner_id, Some(&id));
    Ok(Json(response))
}

async fn validate_view(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(body): Json<ValidateViewBody>,
) -> AppResult<Json<ValidateViewResponse>> {
    let actor = guard(&state, &auth).await?;
    let (owner, _) = owner_access(&state, &actor, body.owner_id.as_deref()).await?;
    let receipt = mutation_receipt(
        &headers,
        &owner,
        &actor,
        "POST",
        "/views/validate",
        &[],
        &body,
    )?;
    if let Some(response) = records::lookup_receipt(&state.db, receipt.as_ref()).await? {
        return Ok(Json(response));
    }
    let errors = views::validate(&state.db, &owner, &body.envelope).await;
    let response = ValidateViewResponse {
        valid: errors.is_empty(),
        errors,
    };
    Ok(Json(
        records::store_response(&state.db, receipt, response).await?,
    ))
}

async fn guard(state: &AppState, auth: &AuthUser) -> AppResult<String> {
    super::login_client_context::require_first_party_human(auth)?;
    let actor = auth.user_id.to_string();
    if !feature_flag_service::personal_flag_enabled(&state.db, &actor, crm::FLAG_KEY).await? {
        return Err(AppError::NotFound("CRM not found".into()));
    }
    Ok(actor)
}

async fn owner_access(
    state: &AppState,
    actor: &str,
    requested_owner: Option<&str>,
) -> AppResult<(String, AccessLevel)> {
    let owner = requested_owner.unwrap_or(actor);
    let access = org_service::resolve_owner_access(&state.db, actor, owner).await?;
    let level = match access {
        org_service::OwnerAccess::Direct | org_service::OwnerAccess::AsOrgAdmin { .. } => {
            AccessLevel::Schema
        }
        org_service::OwnerAccess::AsOrgMember { role, .. } if role.can_proxy() => AccessLevel::Work,
        access if access.can_read() => AccessLevel::Read,
        _ => return Err(AppError::NotFound("CRM owner not found".into())),
    };
    Ok((owner.to_owned(), level))
}

fn require_work(access: AccessLevel) -> AppResult<()> {
    if matches!(access, AccessLevel::Work | AccessLevel::Schema) {
        Ok(())
    } else {
        Err(AppError::NotFound("CRM resource not found".into()))
    }
}

fn require_schema(access: AccessLevel) -> AppResult<()> {
    if access == AccessLevel::Schema {
        Ok(())
    } else {
        Err(AppError::NotFound("CRM resource not found".into()))
    }
}

fn ensure_owner_hint(hint: Option<&str>, owner: &str) -> AppResult<()> {
    if hint.is_none_or(|hint| hint == owner) {
        Ok(())
    } else {
        Err(AppError::NotFound("CRM resource not found".into()))
    }
}

fn mutation_receipt<T: Serialize>(
    headers: &HeaderMap,
    owner: &str,
    actor: &str,
    method: &str,
    route: &str,
    path: &[(&str, &str)],
    body: &T,
) -> AppResult<Option<records::ReceiptContext>> {
    let key = headers
        .get("Idempotency-Key")
        .map(|value| {
            value
                .to_str()
                .map_err(|_| crm::validation_error("Idempotency-Key", "must be visible ASCII"))
        })
        .transpose()?;
    records::receipt_context(owner, actor, key, method, route, path, body)
}

fn audit(
    state: &AppState,
    auth: &AuthUser,
    event: impl Into<String>,
    owner_id: &str,
    id: Option<&str>,
) {
    audit_service::log_for_user(
        state.db.clone(),
        auth,
        event,
        Some(serde_json::json!({"owner_id": owner_id, "id": id})),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole},
        user::{COLLECTION_NAME as USERS, User, UserType},
    };
    use uuid::Uuid;

    #[tokio::test]
    async fn crm_gate_and_three_access_levels_are_authoritative() {
        let db = crate::test_utils::connect_transaction_test_database("crm_handler_access").await;
        let state = crate::test_utils::test_app_state(db.clone());
        let direct = Uuid::new_v4().to_string();
        let org = Uuid::new_v4().to_string();
        let admin = Uuid::new_v4().to_string();
        let member = Uuid::new_v4().to_string();
        let viewer = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(&org, UserType::Org))
            .await
            .unwrap();
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .insert_many([
                crate::test_utils::test_membership(&org, &admin, OrgRole::Admin, None),
                crate::test_utils::test_membership(&org, &member, OrgRole::Member, None),
                crate::test_utils::test_membership(&org, &viewer, OrgRole::Viewer, None),
            ])
            .await
            .unwrap();

        let flag_error = guard(&state, &crate::test_utils::test_auth_user(&direct))
            .await
            .unwrap_err();
        assert!(matches!(flag_error, AppError::NotFound(_)));
        assert_eq!(
            owner_access(&state, &direct, None).await.unwrap().1,
            AccessLevel::Schema
        );
        assert_eq!(
            owner_access(&state, &admin, Some(&org)).await.unwrap().1,
            AccessLevel::Schema
        );
        assert_eq!(
            owner_access(&state, &member, Some(&org)).await.unwrap().1,
            AccessLevel::Work
        );
        assert_eq!(
            owner_access(&state, &viewer, Some(&org)).await.unwrap().1,
            AccessLevel::Read
        );
        assert!(matches!(
            ensure_owner_hint(Some(&direct), &org),
            Err(AppError::NotFound(_))
        ));
    }
}
