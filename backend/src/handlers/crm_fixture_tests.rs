use std::{collections::HashMap, path::PathBuf};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::DateTime;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    crypto::jwt,
    models::user::{COLLECTION_NAME as USERS, User, UserType},
    services::feature_flag_service::{self, FlagTarget},
    test_utils,
};

const BASE: &str = "/api/v1/assistant/crm";

async fn request(
    app: &Router,
    token: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
    idempotency_key: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    if let Some(key) = idempotency_key {
        request = request.header("idempotency-key", key);
    }
    let response = app
        .clone()
        .oneshot(
            request
                .header("content-type", "application/json")
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"raw": String::from_utf8_lossy(&bytes)}));
    (status, body)
}

struct FixtureWriter {
    update: bool,
    directory: PathBuf,
    ids: HashMap<String, String>,
    next_id: u64,
}

impl FixtureWriter {
    fn new() -> Self {
        Self {
            update: std::env::var_os("UPDATE_CRM_FIXTURES").is_some(),
            directory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/fixtures/crm"),
            ids: HashMap::new(),
            next_id: 1,
        }
    }

    fn assert(&mut self, name: &str, mut value: Value) {
        self.normalize(&mut value, None);
        let encoded = format!("{}\n", serde_json::to_string_pretty(&value).unwrap());
        let path = self.directory.join(format!("{name}.json"));
        if self.update {
            std::fs::create_dir_all(&self.directory).unwrap();
            std::fs::write(path, encoded).unwrap();
        } else {
            let expected = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read CRM fixture {}: {error}", path.display()));
            assert_eq!(encoded, expected, "CRM fixture {} changed", path.display());
        }
    }

    fn normalize(&mut self, value: &mut Value, key: Option<&str>) {
        match value {
            Value::Array(values) => {
                for value in values {
                    self.normalize(value, key);
                }
            }
            Value::Object(values) => {
                for (key, value) in values {
                    self.normalize(value, Some(key));
                }
            }
            Value::String(text) => {
                if Uuid::parse_str(text).is_ok() {
                    let replacement = if let Some(existing) = self.ids.get(text) {
                        existing.clone()
                    } else {
                        let replacement = format!("00000000-0000-4000-8000-{:012x}", self.next_id);
                        self.next_id += 1;
                        self.ids.insert(text.clone(), replacement.clone());
                        replacement
                    };
                    *text = replacement;
                } else if DateTime::parse_from_rfc3339(text).is_ok() {
                    *text = "2000-01-01T00:00:00.000Z".into();
                } else if key == Some("next_cursor") {
                    *text = "Y3JtLWZpeHR1cmUtY3Vyc29y".into();
                }
            }
            _ => {}
        }
    }
}

async fn ok(
    app: &Router,
    token: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
    key: Option<&str>,
) -> Value {
    let (status, response) = request(app, token, method, path, body, key).await;
    assert_eq!(status, StatusCode::OK, "{method} {path}: {response}");
    response
}

fn view_envelope() -> Value {
    json!({
        "formatVersion": 1,
        "renderer": "json-render",
        "rendererSpecVersion": "nyxid-crm-v1",
        "queries": {
            "Companies": {
                "kind": "table",
                "query": {
                    "object": "company",
                    "select": ["name", "domain", "is_partner"],
                    "filters": [],
                    "sort": {"field": "name", "direction": "asc"},
                    "page_size": 20,
                    "include_archived": false
                }
            }
        },
        "spec": {
            "root": "Root",
            "elements": {
                "Root": {
                    "type": "Stack",
                    "props": {"gap": "md", "direction": "vertical"},
                    "children": ["CompanyTable"]
                },
                "CompanyTable": {
                    "type": "ObjectTable",
                    "props": {"queryId": "Companies", "columns": ["name", "domain"]},
                    "children": []
                }
            }
        }
    })
}

#[tokio::test]
async fn crm_http_contract_fixtures_match_real_routes() {
    let db = test_utils::connect_transaction_test_database("crm_http_fixtures").await;
    crate::db::ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4();
    db.collection::<User>(USERS)
        .insert_one(test_utils::test_user(&owner.to_string(), UserType::Person))
        .await
        .unwrap();
    feature_flag_service::set_platform_override(
        &db,
        feature_flag_service::CMS_POC_V1_FLAG_KEY,
        &FlagTarget::User(owner.to_string()),
        true,
        &owner.to_string(),
    )
    .await
    .unwrap();
    let state = test_utils::test_app_state(db);
    let token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &owner,
        "openid profile",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    let app = private.with_state(state.clone());
    let mut fixtures = FixtureWriter::new();
    feature_flag_service::set_platform_override(
        &state.db,
        feature_flag_service::CMS_POC_V1_FLAG_KEY,
        &FlagTarget::User(owner.to_string()),
        false,
        &owner.to_string(),
    )
    .await
    .unwrap();
    for (method, path, body) in [
        ("GET", "/objects", None),
        ("GET", "/views/count", None),
        ("GET", "/people", None),
        (
            "POST",
            "/objects",
            Some(json!({"key":"asset","label":"Asset","plural_label":"Assets","fields":[]})),
        ),
        ("POST", "/query", Some(json!({"object":"company"}))),
        ("POST", "/views/validate", Some(json!({"envelope":{}}))),
    ] {
        let (status, _) = request(&app, &token, method, &format!("{BASE}{path}"), body, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
    }
    feature_flag_service::set_platform_override(
        &state.db,
        feature_flag_service::CMS_POC_V1_FLAG_KEY,
        &FlagTarget::User(owner.to_string()),
        true,
        &owner.to_string(),
    )
    .await
    .unwrap();

    let objects = ok(&app, &token, "GET", &format!("{BASE}/objects"), None, None).await;
    fixtures.assert("objects", objects);
    let company = ok(
        &app,
        &token,
        "GET",
        &format!("{BASE}/objects/company"),
        None,
        None,
    )
    .await;
    fixtures.assert("object_company", company);

    let boolean_field = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/objects/company/fields"),
        Some(json!({
            "expected_schema_revision": 1,
            "key": "is_partner",
            "label": "Partner",
            "required": false,
            "type": "boolean"
        })),
        Some("fixture-add-partner-field"),
    )
    .await;
    fixtures.assert("field_create", boolean_field);
    let segment_options: Vec<Value> = (0..52)
        .map(|index| {
            json!({
                "value": format!("group_{index:02}"),
                "label": format!("Group {index:02}"),
                "color": null,
                "archived": false
            })
        })
        .collect();
    ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/objects/company/fields"),
        Some(json!({
            "expected_schema_revision": 2,
            "key": "segment",
            "label": "Segment",
            "required": false,
            "type": "select",
            "options": segment_options
        })),
        Some("fixture-add-segment-field"),
    )
    .await;

    let company = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/records"),
        Some(json!({
            "object": "company",
            "values": {
                "name": "Acme Pte Ltd",
                "domain": "acme.example",
                "is_partner": true,
                "segment": "group_00"
            }
        })),
        Some("fixture-create-company"),
    )
    .await;
    let company_id = company["id"].as_str().unwrap().to_owned();
    fixtures.assert("record_company_create", company);
    let contact = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/records"),
        Some(json!({
            "object": "contact",
            "values": {
                "name": "Avery Tan",
                "email": "avery@acme.example",
                "company": company_id
            }
        })),
        Some("fixture-create-contact"),
    )
    .await;
    fixtures.assert("record_contact_create", contact);
    let opportunity = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/records"),
        Some(json!({
            "object": "opportunity",
            "values": {
                "name": "Acme renewal",
                "company": company_id,
                "amount": {"amount_minor": 125000, "currency": "USD"},
                "stage": "new",
                "expected_close": "2027-03-31"
            }
        })),
        Some("fixture-create-opportunity"),
    )
    .await;
    let opportunity_id = opportunity["id"].as_str().unwrap().to_owned();
    fixtures.assert("record_opportunity_create", opportunity);

    let patched = ok(
        &app,
        &token,
        "PATCH",
        &format!("{BASE}/records/{company_id}"),
        Some(json!({
            "expected_revision": 1,
            "values": {"domain": "www.acme.example"}
        })),
        Some("fixture-patch-company"),
    )
    .await;
    fixtures.assert("record_patch", patched);
    let (status, conflict) = request(
        &app,
        &token,
        "PATCH",
        &format!("{BASE}/records/{company_id}"),
        Some(json!({
            "expected_revision": 1,
            "values": {"domain": "stale.acme.example"}
        })),
        Some("fixture-record-conflict"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    fixtures.assert("record_revision_conflict", conflict);
    let (status, validation) = request(
        &app,
        &token,
        "POST",
        &format!("{BASE}/records"),
        Some(json!({"object": "contact", "values": {"email": 12}})),
        Some("fixture-record-validation"),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{validation}");
    fixtures.assert("record_validation_error", validation);
    for (index, blank) in ["", " \t\n"].into_iter().enumerate() {
        let (status, validation) = request(
            &app,
            &token,
            "POST",
            &format!("{BASE}/records"),
            Some(json!({"object": "contact", "values": {"name": blank}})),
            Some(&format!("fixture-blank-required-text-{index}")),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{validation}");
        fixtures.assert("record_blank_required_text", validation);
    }

    let moved = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/records/{opportunity_id}/move"),
        Some(json!({"expected_revision": 1, "to": "qualified"})),
        Some("fixture-move-opportunity"),
    )
    .await;
    fixtures.assert("record_move", moved);
    let note = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/records/{opportunity_id}/notes"),
        Some(json!({"body": "Procurement review scheduled."})),
        Some("fixture-add-note"),
    )
    .await;
    fixtures.assert("activity_note_create", note);
    let activities = ok(
        &app,
        &token,
        "GET",
        &format!("{BASE}/records/{opportunity_id}/activities"),
        None,
        None,
    )
    .await;
    fixtures.assert("activities", activities);
    let related = ok(
        &app,
        &token,
        "GET",
        &format!("{BASE}/records/{company_id}/related"),
        None,
        None,
    )
    .await;
    fixtures.assert("related", related);

    for index in 1..52 {
        ok(
            &app,
            &token,
            "POST",
            &format!("{BASE}/records"),
            Some(json!({
                "object": "company",
                "values": {
                    "name": format!("Company {index:02}"),
                    "is_partner": index % 2 == 0,
                    "segment": format!("group_{index:02}")
                }
            })),
            Some(&format!("fixture-company-{index:02}")),
        )
        .await;
    }

    let table_first = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/query"),
        Some(json!({
            "object": "company",
            "select": ["name", "domain", "is_partner"],
            "filters": [],
            "sort": {"field": "name", "direction": "asc"},
            "page_size": 1,
            "include_archived": false
        })),
        Some("fixture-table-first"),
    )
    .await;
    let cursor = table_first["next_cursor"].as_str().unwrap().to_owned();
    fixtures.assert("table_page_1", table_first);
    let table_second = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/query"),
        Some(json!({
            "object": "company",
            "select": ["name", "domain", "is_partner"],
            "filters": [],
            "sort": {"field": "name", "direction": "asc"},
            "page_size": 1,
            "cursor": cursor,
            "include_archived": false
        })),
        Some("fixture-table-second"),
    )
    .await;
    fixtures.assert("table_page_2", table_second);

    let route_identity_body = json!({
        "object": "company",
        "filters": [],
        "page_size": 1,
        "include_archived": false
    });
    ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/query"),
        Some(route_identity_body.clone()),
        Some("fixture-query-versus-export"),
    )
    .await;
    let (route_status, route_error) = request(
        &app,
        &token,
        "POST",
        &format!("{BASE}/export"),
        Some(route_identity_body),
        Some("fixture-query-versus-export"),
    )
    .await;
    assert_eq!(route_status, StatusCode::CONFLICT);
    assert_eq!(route_error["error"], "crm_idempotency_mismatch");

    let (bound_status, bound_error) = request(
        &app,
        &token,
        "POST",
        &format!("{BASE}/export"),
        Some(json!({
            "object": "company",
            "filters": [],
            "page_size": 101,
            "include_archived": false
        })),
        Some("fixture-export-101-records"),
    )
    .await;
    assert_eq!(bound_status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bound_error["errors"][0]["path"], "page_size");
    assert_eq!(
        bound_error["errors"][0]["message"],
        "exports require between 1 and 100 records"
    );

    let boolean_groups = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/aggregate"),
        Some(json!({
            "object": "company",
            "filters": [],
            "group_by": "is_partner",
            "measure": {"op": "count"},
            "order": "label_asc"
        })),
        Some("fixture-aggregate-boolean"),
    )
    .await;
    fixtures.assert("aggregate_boolean", boolean_groups);
    let empty_average = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/aggregate"),
        Some(json!({
            "object": "opportunity",
            "filters": [{"field": "stage", "op": "eq", "value": "lost"}],
            "measure": {"op": "avg", "field": "amount"},
            "order": "value_desc"
        })),
        Some("fixture-aggregate-empty-average"),
    )
    .await;
    fixtures.assert("aggregate_empty_average", empty_average);
    let truncated = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/aggregate"),
        Some(json!({
            "object": "company",
            "filters": [],
            "group_by": "segment",
            "measure": {"op": "count"},
            "order": "option_order"
        })),
        Some("fixture-aggregate-truncated"),
    )
    .await;
    fixtures.assert("aggregate_truncated", truncated);

    let envelope = view_envelope();
    let created_view = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/views"),
        Some(json!({"title": "Company directory", "envelope": envelope})),
        Some("fixture-create-view"),
    )
    .await;
    let view_id = created_view["id"].as_str().unwrap().to_owned();
    fixtures.assert("view_create", created_view);
    let listed_views = ok(&app, &token, "GET", &format!("{BASE}/views"), None, None).await;
    fixtures.assert("views_list", listed_views);
    let fetched_view = ok(
        &app,
        &token,
        "GET",
        &format!("{BASE}/views/{view_id}"),
        None,
        None,
    )
    .await;
    fixtures.assert("view_get", fetched_view);
    let valid_view = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/views/validate"),
        Some(json!({"envelope": view_envelope()})),
        Some("fixture-validate-view"),
    )
    .await;
    fixtures.assert("view_validate_valid", valid_view);
    let mut invalid_envelope = view_envelope();
    invalid_envelope["spec"]["elements"]["CompanyTable"]["props"]["columns"] =
        json!(["unknown_field"]);
    let invalid_view = ok(
        &app,
        &token,
        "POST",
        &format!("{BASE}/views/validate"),
        Some(json!({"envelope": invalid_envelope})),
        Some("fixture-validate-invalid-view"),
    )
    .await;
    assert_eq!(invalid_view["valid"], false);
    fixtures.assert("view_validate_invalid", invalid_view);
    let deleted_view = ok(
        &app,
        &token,
        "DELETE",
        &format!("{BASE}/views/{view_id}"),
        None,
        Some("fixture-delete-view"),
    )
    .await;
    fixtures.assert("view_delete", deleted_view.clone());
    let replayed_delete = ok(
        &app,
        &token,
        "DELETE",
        &format!("{BASE}/views/{view_id}"),
        None,
        Some("fixture-delete-view"),
    )
    .await;
    assert_eq!(replayed_delete, deleted_view);
    let (status, deleted_read) = request(
        &app,
        &token,
        "GET",
        &format!("{BASE}/views/{view_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{deleted_read}");
    assert_eq!(deleted_read["message"], "Not found: CRM view not found");

    let missing_id = Uuid::new_v4().to_string();
    let (missing_status, missing_record) = request(
        &app,
        &token,
        "GET",
        &format!("{BASE}/records/{missing_id}"),
        None,
        None,
    )
    .await;
    let (hint_status, mismatched_hint) = request(
        &app,
        &token,
        "GET",
        &format!("{BASE}/records/{company_id}?owner_id={missing_id}"),
        None,
        None,
    )
    .await;
    let foreign_owner = Uuid::new_v4().to_string();
    state
        .db
        .collection::<User>(USERS)
        .insert_one(test_utils::test_user(&foreign_owner, UserType::Person))
        .await
        .unwrap();
    crate::services::crm::objects::ensure_seeded(&state.db, &foreign_owner)
        .await
        .unwrap();
    let foreign = crate::services::crm::records::create(
        &state.db,
        &foreign_owner,
        crate::models::crm_record::CrmActor::human(&foreign_owner),
        crate::services::crm::records::CreateRecordInput {
            object: "company".into(),
            values: [("name".into(), json!("Foreign"))].into_iter().collect(),
            protect: vec![],
        },
        None,
    )
    .await
    .unwrap();
    let (foreign_status, foreign_record) = request(
        &app,
        &token,
        "GET",
        &format!("{BASE}/records/{}", foreign.id),
        None,
        None,
    )
    .await;
    assert_eq!(missing_status, StatusCode::NOT_FOUND);
    assert_eq!(hint_status, missing_status);
    assert_eq!(foreign_status, missing_status);
    assert_eq!(mismatched_hint, missing_record);
    assert_eq!(foreign_record, missing_record);
    let custom = ok(&app,&token,"POST",&format!("{BASE}/objects"),Some(json!({"key":"asset","label":"Asset","plural_label":"Assets","fields":[{"key":"name","label":"Name","type":"text","max_length":200,"required":true,"archived":false}]})),Some("create-object")).await;
    assert_eq!(custom["key"], "asset");
    assert_eq!(custom["seeded"], false);
}
