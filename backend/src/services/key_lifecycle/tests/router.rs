use super::*;
use crate::{
    AppState,
    crypto::jwt,
    models::user::{User, UserType},
    services::{key_service, role_service, service_account_service},
    test_utils,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn human(state: &AppState, role: String) -> (String, String) {
    let id = Uuid::new_v4();
    let mut user = test_utils::test_user(&id.to_string(), UserType::Person);
    user.role_ids.push(role);
    state
        .db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(user)
        .await
        .unwrap();
    (
        id.to_string(),
        jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &id,
            "openid profile",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap(),
    )
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    header: &str,
    credential: &str,
    body: &Value,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(if path.starts_with("/encrypted-records/") {
            format!("/api/v1/admin{path}")
        } else {
            format!("/api/v1/admin/key-lifecycle{path}")
        })
        .header(header, credential)
        .header("content-type", "application/json")
        .body(if method == "GET" {
            Body::empty()
        } else {
            Body::from(body.to_string())
        })
        .unwrap();
    let rsp = app.clone().oneshot(req).await.unwrap();
    let status = rsp.status();
    let bytes = to_bytes(rsp.into_body(), 128 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn real_admin_router_fences_credentials_and_commits_metadata_only_audit() {
    let db = connect_transaction_test_database("admin_ring_routes").await;
    role_service::seed_system_roles(&db).await.unwrap();
    let roles = role_service::get_platform_role_ids(&db).await.unwrap();
    let state = test_utils::test_app_state(db.clone());
    let (admin, token) = human(&state, roles.admin.clone()).await;
    let (operator, op_token) = human(&state, roles.operator.clone()).await;
    let owner = Uuid::parse_str(&admin).unwrap();
    let delegated = jwt::generate_delegated_access_token(
        &state.jwt_keys,
        &state.config,
        &owner,
        "account:read",
        "test-client",
        60,
        None,
    )
    .unwrap();
    let (sa, secret) = service_account_service::create_service_account(
        &db,
        "ring fixture",
        None,
        "account:read",
        &[],
        None,
        &admin,
    )
    .await
    .unwrap();
    let sa = service_account_service::authenticate_client_credentials(
        &db,
        &state.config,
        &state.jwt_keys,
        &sa.client_id,
        &secret,
        None,
    )
    .await
    .unwrap()
    .access_token;
    let api = key_service::create_api_key(
        &db, &admin, "fixture", "proxy", None, None, None, None, None, None, None, None, None,
        None, None,
    )
    .await
    .unwrap()
    .full_key;
    let (public, private) = crate::routes::build_router_with_state(state.clone());
    let app = public.merge(private).with_state(state);
    let enroll = serde_json::to_value(enroll_request()).unwrap();
    let admin_bearer = format!("Bearer {token}");
    let op_bearer = format!("Bearer {op_token}");
    let (code, absent) = call(
        &app,
        "GET",
        "",
        "authorization",
        &admin_bearer,
        &Value::Null,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert!(absent.is_null());
    assert_eq!(
        call(
            &app,
            "POST",
            "/enroll",
            "authorization",
            &op_bearer,
            &enroll
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (code, response) = call(
        &app,
        "POST",
        "/enroll",
        "authorization",
        &admin_bearer,
        &enroll,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{response}");
    let first: Receipt = serde_json::from_value(response).unwrap();
    let record = serde_json::to_value(input(
        &first,
        Change::Attest {
            assertion: Assertion::OldWritersWithdrawn,
            affirmed: true,
            valid_until_ms: None,
        },
    ))
    .unwrap();
    let rotate = json!({"scope":first.scope,"recording":recording(first.revision),"transition":{"kind":"begin_roll"}});
    for (token, expected) in [
        (&op_bearer, StatusCode::FORBIDDEN),
        (&admin_bearer, StatusCode::CONFLICT),
    ] {
        assert_eq!(
            call(&app, "POST", "/rotation", "authorization", token, &rotate)
                .await
                .0,
            expected
        );
    }
    for (header, cred) in [
        ("authorization", format!("Bearer {delegated}")),
        ("authorization", format!("Bearer {sa}")),
        ("x-api-key", api),
    ] {
        for (method, path, body) in [
            ("GET", "", &Value::Null),
            ("GET", "/evidence", &Value::Null),
            ("GET", "/dependencies", &Value::Null),
            ("POST", "/enroll", &enroll),
            ("POST", "/evidence", &record),
            ("POST", "/rotation", &rotate),
        ] {
            let (s, _) = call(&app, method, path, header, &cred, body).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{method} {path}");
        }
    }
    for auth in [&admin_bearer, &op_bearer] {
        for path in ["", "/evidence", "/dependencies"] {
            let (s, body) = call(&app, "GET", path, "authorization", auth, &Value::Null).await;
            assert_eq!(s, StatusCode::OK);
            for private in [
                "proofs",
                "envelope",
                "request_digest",
                "key_arn",
                "ciphertext",
            ] {
                assert!(!body.to_string().contains(private))
            }
        }
    }
    assert_eq!(
        call(
            &app,
            "POST",
            "/evidence",
            "authorization",
            &op_bearer,
            &record
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // A failed chained audit must abort both the singleton CAS and the immutable entry.
    let logs = crate::models::audit_log::COLLECTION_NAME;
    db.run_command(doc! {"collMod":logs, "validator":{"event_type":{"$ne":"encryption_key_lifecycle.recorded"}}}).await.unwrap();
    let (code, _) = call(
        &app,
        "POST",
        "/evidence",
        "authorization",
        &admin_bearer,
        &record,
    )
    .await;
    assert!(code.is_server_error());
    assert_eq!(
        db.collection::<Document>(ENTRIES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let before = db
        .collection::<Document>(HEAD)
        .find_one(doc! {"_id":SINGLETON})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.get_i64("revision").unwrap(), 1);
    db.run_command(doc! {"collMod":logs, "validator":{}})
        .await
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            call(
                &app,
                "POST",
                "/evidence",
                "authorization",
                &admin_bearer,
                &record
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    let history = call(
        &app,
        "GET",
        "/evidence?after=1&limit=1",
        "authorization",
        &admin_bearer,
        &Value::Null,
    )
    .await
    .1;
    assert_eq!(history[0]["schema_version"], 1);
    assert_eq!(history[0]["issuer"], admin);
    assert_eq!(history[0]["owner"], record["recording"]["owner"]);
    assert_eq!(
        history[0]["evidence_ref"],
        record["recording"]["evidence_ref"]
    );
    let logs = db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME);
    assert_eq!(logs.count_documents(doc!{"event_type":"encryption_key_lifecycle.recorded","user_id":&admin,"entry_hash":{"$type":"string"}}).await.unwrap(),2);
    let log = logs
        .find_one(doc! {"event_data.entry_id":record["recording"]["request_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    let data = log.get_document("event_data").unwrap();
    assert_eq!(data.len(), 5);
    assert_eq!(
        data.get_str("evidence_ref").unwrap(),
        record["recording"]["evidence_ref"].as_str().unwrap()
    );
    tokio::time::timeout(std::time::Duration::from_secs(8),async{
        loop {
            let count=logs.count_documents(doc!{"event_type":"admin.read.by_operator","user_id":&operator,"event_data.endpoint":{"$in":["admin.key_lifecycle.status","admin.key_lifecycle.evidence","admin.key_lifecycle.dependencies"]}}).await.unwrap();
            if count==3 {break} tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert_eq!(
        serde_json::to_value(data.get_i64("revision").unwrap()).unwrap(),
        json!(2)
    );
}

#[tokio::test]
async fn census_router_authorizes_admin_and_operator_reads_and_audits_the_actions() {
    let db = connect_transaction_test_database("census_roles").await;
    role_service::seed_system_roles(&db).await.unwrap();
    let roles = role_service::get_platform_role_ids(&db).await.unwrap();
    let state = test_utils::test_app_state(db.clone());
    let (admin, token) = human(&state, roles.admin.clone()).await;
    let (operator, op) = human(&state, roles.operator.clone()).await;
    let (_, other) = human(&state, Uuid::new_v4().to_string()).await;
    let delegated = jwt::generate_delegated_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&admin).unwrap(),
        "account:read",
        "fixture",
        60,
        None,
    )
    .unwrap();
    let (public, private) = crate::routes::build_router_with_state(state.clone());
    let app = public.merge(private).with_state(state.clone());
    let token = format!("Bearer {token}");
    let op = format!("Bearer {op}");
    let denied = [format!("Bearer {other}"), format!("Bearer {delegated}")];
    let base = "/encrypted-records/censuses";
    crate::services::encrypted_record_census::tests::with_history(&db,120,async{
        for credential in [&op,&denied[0],&denied[1]] {
            assert_eq!(call(&app,"POST",base,"authorization",credential,&Value::Null).await.0,StatusCode::FORBIDDEN);
        }
        let (code,body)=call(&app,"POST",base,"authorization",&token,&Value::Null).await;
        assert_eq!(code,StatusCode::OK,"{body}");
        let id=body["generation"].as_str().unwrap();
        let advance=format!("{base}/{id}/advance");
        assert_eq!(call(&app,"POST",&advance,"authorization",&op,&Value::Null).await.0,StatusCode::FORBIDDEN);
        assert_eq!(call(&app,"POST",&advance,"authorization",&token,&Value::Null).await.0,StatusCode::OK);
        let report=crate::services::encrypted_record_census::report::advance;
        for _ in 0..128 {
            if report(&db,&state.encryption_keys,&state.config,id).await.unwrap().complete {break;}
        }
        for suffix in ["","/families","/findings"]{
            let path=format!("{base}/{id}{suffix}");
            for auth in [&token,&op]{
                assert_eq!(call(&app,"GET",&path,"authorization",auth,&Value::Null).await.0,StatusCode::OK);
            }
            for auth in &denied{
                assert_eq!(call(&app,"GET",&path,"authorization",auth,&Value::Null).await.0,StatusCode::FORBIDDEN);
            }
        }
        let logs=db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME);
        assert_eq!(logs.count_documents(doc!{"event_type":{"$in":["encrypted_records.census_requested","encrypted_records.census_advance_requested"]},"user_id":&admin}).await.unwrap(),2);
        tokio::time::timeout(std::time::Duration::from_secs(8),async{
            loop {
                if logs.count_documents(doc!{"event_type":"admin.read.by_operator","user_id":&operator,"event_data.endpoint":{"$in":["admin.census.status","admin.census.families","admin.census.findings"]}}).await.unwrap()==3 {break;}
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }).await.unwrap();
    }).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn retirement_evaluation_is_readable_and_full_admin_issuance_refuses_the_release_gate() {
    let db = connect_transaction_test_database("retirement_routes").await;
    role_service::seed_system_roles(&db).await.unwrap();
    let roles = role_service::get_platform_role_ids(&db).await.unwrap();
    let state = test_utils::test_app_state(db.clone());
    let (_, admin) = human(&state, roles.admin.clone()).await;
    let (_, operator) = human(&state, roles.operator.clone()).await;
    let (_, person) = human(&state, fresh()).await;
    let (public, private) = crate::routes::build_router_with_state(state.clone());
    let app = public.merge(private).with_state(state);
    let target = json!({"generation":fresh(),"retiring":fresh(),"survivor":fresh()});
    let path = format!(
        "/retirement?generation={}&retiring={}&survivor={}",
        target["generation"].as_str().unwrap(),
        target["retiring"].as_str().unwrap(),
        target["survivor"].as_str().unwrap()
    );
    let body = json!({"recording":recording(1),"target":target,"report_revision":1,"expected_step_revision":1});
    for (token, read, write) in [
        (admin, true, StatusCode::CONFLICT),
        (operator, true, StatusCode::FORBIDDEN),
        (person, false, StatusCode::FORBIDDEN),
    ] {
        let bearer = format!("Bearer {token}");
        let (code, result) = call(&app, "GET", &path, "authorization", &bearer, &Value::Null).await;
        assert_eq!(
            code,
            if read {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            }
        );
        if read {
            assert_eq!(result["certification_enabled"], false);
            assert!(
                result["blockers"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(retirement::LOAD_BLOCKER))
            );
        }
        let (code, result) =
            call(&app, "POST", "/retirement", "authorization", &bearer, &body).await;
        assert_eq!(code, write);
        if code == StatusCode::CONFLICT {
            assert!(result.to_string().contains(retirement::LOAD_BLOCKER));
        }
    }
    assert!(
        db.collection::<Document>(HEAD)
            .find_one(doc! {})
            .await
            .unwrap()
            .is_none()
    );
    db.drop().await.unwrap();
}
