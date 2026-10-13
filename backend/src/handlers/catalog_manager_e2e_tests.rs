//! Loopback-only Catalog CLI/OAuth software-flow proof. Real-vendor gates stay open.
use super::{DownstreamService, ROLES, Role, SERVICES, seed};
use crate::models::{provider_config::ProviderConfig, user_api_key::UserApiKey};
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use serde_json::{Value, json};
use std::{future::IntoFuture, path::Path, time::Duration};
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

const CLIENT_ID: &str = "local-catalog-client-id-canary";
const CLIENT_SECRET: &str = "local-catalog-client-secret-canary";
const ACCESS: &str = "local-catalog-access-token-canary";
const REFRESH: &str = "local-catalog-refresh-token-canary";

fn assert_redacted(text: &str) {
    for canary in [CLIENT_ID, CLIENT_SECRET, ACCESS, REFRESH] {
        assert!(
            !text.contains(canary),
            "Catalog output exposed a credential canary"
        );
    }
}

async fn cli(
    executable: &str,
    base: &str,
    token: &str,
    args: &[&str],
    file: Option<&Path>,
    poison: bool,
) -> Value {
    let mut command = tokio::process::Command::new(executable);
    command.kill_on_drop(true).args(args);
    if let Some(file) = file {
        command.arg("--file").arg(file);
    }
    command
        .args([
            "--base-url",
            base,
            "--output",
            "json",
            "--access-token-env",
            "LOCAL_CATALOG_ACCESS_TOKEN",
        ])
        .env("LOCAL_CATALOG_ACCESS_TOKEN", token)
        .env("LOCAL_OAUTH_CLIENT_ID", CLIENT_ID)
        .env("LOCAL_OAUTH_CLIENT_SECRET", CLIENT_SECRET)
        .env_remove("NYXID_ACCESS_TOKEN")
        .env_remove("NYXID_TELEMETRY_DSN")
        .env("NYXID_SHARE_ANALYTICS", "0")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("DO_NOT_TRACK", "1");
    #[cfg(unix)]
    if poison {
        use std::os::unix::ffi::OsStringExt;
        // env::var cannot decode these. A successful dry-run proves no read.
        command
            .env(
                "LOCAL_OAUTH_CLIENT_ID",
                std::ffi::OsString::from_vec(vec![0xff]),
            )
            .env(
                "LOCAL_OAUTH_CLIENT_SECRET",
                std::ffi::OsString::from_vec(vec![0xfe]),
            );
    }
    #[cfg(not(unix))]
    assert!(!poison, "invalid-Unicode env proof requires Unix");
    let output = tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .expect("local CLI deadline")
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_redacted(&stdout);
    assert_redacted(&stderr);
    assert!(
        output.status.success(),
        "local CLI {:?} exited {:?}: {stdout}\n{stderr}",
        args,
        output.status.code()
    );
    serde_json::from_str(&stdout).expect("structured CLI output")
}

async fn catalog_snapshot(db: &mongodb::Database) -> Vec<Vec<Document>> {
    let mut snapshot = Vec::new();
    for collection in [
        crate::models::provider_config::COLLECTION_NAME,
        SERVICES,
        crate::models::service_endpoint::COLLECTION_NAME,
    ] {
        snapshot.push(
            db.collection::<Document>(collection)
                .find(doc! {})
                .sort(doc! {"_id":1})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap(),
        );
    }
    snapshot
}

#[tokio::test]
#[ignore = "requires the locally built CLI; run scripts/tools/test-tools-manager-e2e.sh catalog"]
async fn catalog_manager_local_fake_oauth_e2e() {
    let executable = std::env::var("NYXID_TOOLS_E2E_CLI").expect("owned E2E harness CLI");
    let uri = std::env::var("NYXID_TEST_DATABASE_URL").expect("owned E2E replica set");
    assert!(
        uri.starts_with("mongodb://127.0.0.1:"),
        "E2E refuses nonlocal MongoDB"
    );
    let db = crate::test_utils::connect_test_database("catalog_manager_local_e2e")
        .await
        .unwrap();
    let admin = seed(&db, "admin", false, false).await;
    let manager = seed(&db, "operator", false, false).await;
    let user = seed(&db, "user", false, false).await;
    let vendor = MockServer::start().await;
    let revocation_url = format!("{}/revoke", vendor.uri());
    assert!(
        crate::services::provider_service::validate_revocation_url(&revocation_url)
            .await
            .is_err(),
        "unregistered loopback URLs retain the production validation guard"
    );
    let revocation_guard = crate::test_utils::register_loopback_revocation_url(&revocation_url);
    Mock::given(method("GET"))
        .and(path("/authorize"))
        .respond_with(|request: &Request| {
            let params: std::collections::HashMap<_, _> =
                request.url.query_pairs().into_owned().collect();
            assert_eq!(params.get("client_id").map(String::as_str), Some(CLIENT_ID));
            assert_eq!(
                params.get("scope").map(String::as_str),
                Some("accounts.read")
            );
            assert_eq!(
                params.get("response_type").map(String::as_str),
                Some("code")
            );
            assert!(params.contains_key("code_challenge"));
            let mut callback = url::Url::parse(params.get("redirect_uri").unwrap()).unwrap();
            assert_eq!(callback.host_str(), Some("127.0.0.1"));
            callback
                .query_pairs_mut()
                .append_pair("code", "local-authorization-code")
                .append_pair("state", params.get("state").unwrap());
            ResponseTemplate::new(302).insert_header("Location", callback.as_str())
        })
        .expect(1)
        .mount(&vendor)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token":ACCESS, "refresh_token":REFRESH, "token_type":"Bearer",
            "expires_in":3600, "scope":"accounts.read"
        })))
        .expect(1)
        .mount(&vendor)
        .await;
    Mock::given(method("POST"))
        .and(path("/revoke"))
        .respond_with(ResponseTemplate::new(200))
        .expect(2)
        .mount(&vendor)
        .await;
    Mock::given(method("GET"))
        .and(path("/accounts"))
        .and(wiremock::matchers::header(
            "Authorization",
            format!("Zoho-oauthtoken {ACCESS}"),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"accounts":[]})))
        .expect(1)
        .mount(&vendor)
        .await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut config = crate::test_utils::test_app_config();
    config.base_url = base.clone();
    config.frontend_url = base.clone();
    config.rate_limit_per_second = 1000;
    config.rate_limit_burst = 2000;
    let state = crate::test_utils::test_app_state_with_config(db.clone(), config);
    let (public, private) = crate::routes::build_router_with_state(state.clone());
    let server = tokio::spawn(
        axum::serve(listener, public.merge(private).with_state(state.clone())).into_future(),
    );
    let token = |id: &str| {
        crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &Uuid::parse_str(id).unwrap(),
            "openid profile email proxy write admin",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    };
    let admin_token = token(&admin);
    let manager_token = token(&manager);
    let user_token = token(&user);
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let role = db
        .collection::<Role>(ROLES)
        .find_one(doc! {"slug":"catalog_manager"})
        .await
        .unwrap()
        .unwrap();
    let grant = http
        .post(format!(
            "{base}/api/v1/admin/users/{manager}/roles/{}",
            role.id
        ))
        .bearer_auth(&admin_token)
        .json(&json!({"role_id":role.id}))
        .send()
        .await
        .unwrap();
    assert!(grant.status().is_success());

    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("catalog.json");
    let manifest = json!({"providers":[{
        "name":"Local OAuth", "slug":"local-oauth", "provider_type":"oauth2",
        "credential_mode":"admin", "authorization_url":format!("{}/authorize",vendor.uri()),
        "token_url":format!("{}/token",vendor.uri()), "revocation_url":revocation_url,
        "token_endpoint_auth_method":"client_secret_post", "supports_pkce":true,
        "default_scopes":["accounts.read"],
        "playbook":{"scope_source":{"kind":"documented_minimum","url":"https://vendor.example/scopes"},
            "revocation":{"kind":"configured","source":"https://vendor.example/revocation"}}
    }],"services":[{
        "name":"Local OAuth API", "slug":"api-local-oauth", "provider":"local-oauth",
        "base_url":vendor.uri(), "auth_method":"bearer", "auth_scheme":"Zoho-oauthtoken",
        "auth_key_name":"Authorization",
        "operations":[{"name":"accounts","method":"GET","path":"/accounts","description":"Read local accounts"}],
        "smoke_test":{"operation":"accounts"}
    }]});
    std::fs::write(&file, manifest.to_string()).unwrap();
    assert_redacted(&std::fs::read_to_string(&file).unwrap());
    let empty = catalog_snapshot(&db).await;
    let dry = cli(
        &executable,
        &base,
        &manager_token,
        &["admin", "catalog", "apply", "--dry-run"],
        Some(&file),
        true,
    )
    .await;
    assert!(!dry.to_string().contains("\"action\":\"failed\""));
    assert_eq!(
        empty,
        catalog_snapshot(&db).await,
        "poisoned-env dry run is write-free"
    );
    let applied = cli(
        &executable,
        &base,
        &manager_token,
        &["admin", "catalog", "apply"],
        Some(&file),
        false,
    )
    .await;
    assert!(!applied.to_string().contains("\"action\":\"failed\""));
    let provider = db
        .collection::<ProviderConfig>(crate::models::provider_config::COLLECTION_NAME)
        .find_one(doc! {"slug":"local-oauth"})
        .await
        .unwrap()
        .unwrap();
    let service = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"slug":"api-local-oauth"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(provider.created_by, manager);
    assert_eq!(service.created_by, manager);
    assert_eq!(service.auth_scheme.as_deref(), Some("Zoho-oauthtoken"));
    assert!(service.provider_config_id.as_deref() == Some(provider.id.as_str()));
    let svg = tmp.path().join("local.svg");
    std::fs::write(&svg, "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><path d=\"M1 1L23 23\"/></svg>").unwrap();
    let icon = cli(
        &executable,
        &base,
        &manager_token,
        &["admin", "catalog", "icon", "local-oauth"],
        Some(&svg),
        false,
    )
    .await;
    let icon_url = icon["icon_url"]
        .as_str()
        .expect("uploaded provider icon URL");
    assert!(icon_url.starts_with(&format!("{base}/catalog-icons/")));
    let served_icon = http.get(icon_url).send().await.unwrap();
    assert!(served_icon.status().is_success());
    assert_eq!(served_icon.headers()["content-type"], "image/svg+xml");
    assert_eq!(
        served_icon.text().await.unwrap(),
        std::fs::read_to_string(&svg).unwrap()
    );
    let report = cli(
        &executable,
        &base,
        &manager_token,
        &["admin", "catalog", "validate"],
        Some(&file),
        true,
    )
    .await;
    let row = &report[0];
    assert_eq!(row["status"], "ready", "{report}");
    assert_eq!(row["playbook"]["complete"], true, "{report}");
    for item in ["c1", "c2", "c3", "c4", "c5", "c6"] {
        assert_eq!(row["playbook"][item]["state"], "met", "{report}");
    }
    assert_eq!(
        row["verification"],
        json!({"status":"not_verified","pending_gates":["api_smoke_test_real_vendor","outside_user_connection_admin_confirmed"]})
    );
    assert!(
        row["lark"]["remarks"]
            .as_str()
            .unwrap()
            .contains("configured; not verified")
    );

    // Match `service add --oauth`: create the personal placeholder, then bind
    // OAuth initiation to that connection instead of the legacy provider row.
    let connected = http
        .post(format!("{base}/api/v1/keys"))
        .bearer_auth(&user_token)
        .json(&json!({"service_slug":service.slug,"slug":service.slug,"label":"Local OAuth API"}))
        .send()
        .await
        .unwrap();
    assert!(
        connected.status().is_success(),
        "second user's catalog connection"
    );
    let connected: Value = connected.json().await.unwrap();
    let key_id = connected["id"].as_str().expect("personal connection ID");
    let api_key_id = connected["api_key_id"].as_str().expect("OAuth key ID");
    assert_eq!(connected["status"], "pending_auth");
    // A second person completes the real local ingress flow, including vendor
    // authorization redirect and callback token exchange with the managed app.
    let started = http
        .get(format!(
            "{base}/api/v1/providers/{}/connect/oauth",
            provider.id
        ))
        .query(&[("key_id", key_id)])
        .bearer_auth(&user_token)
        .send()
        .await
        .unwrap();
    assert!(started.status().is_success());
    let started: Value = started.json().await.unwrap();
    let authorized = http
        .get(started["authorization_url"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(authorized.status(), 302);
    let callback = http
        .get(authorized.headers()["location"].to_str().unwrap())
        .bearer_auth(&user_token)
        .send()
        .await
        .unwrap();
    assert_eq!(callback.status(), 303);
    assert!(
        callback.headers()["location"]
            .to_str()
            .unwrap()
            .contains("status=success")
    );
    let stored = db
        .collection::<UserApiKey>(crate::models::user_api_key::COLLECTION_NAME)
        .find_one(doc! {"_id":api_key_id,"user_id":&user,"provider_config_id":&provider.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "active");
    assert_eq!(stored.token_scopes.as_deref(), Some("accounts.read"));
    assert_eq!(stored.credential_source.as_deref(), Some("platform"));
    assert!(stored.access_token_encrypted.is_some());
    assert!(stored.refresh_token_encrypted.is_some());
    assert_eq!(
        db.collection::<Document>(crate::models::user_provider_token::COLLECTION_NAME)
            .count_documents(doc! {"user_id":&user,"provider_config_id":&provider.id})
            .await
            .unwrap(),
        0,
        "OAuth must bind this personal connection, not a legacy provider token"
    );
    let request = vendor
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.url.path() == "/token")
        .unwrap();
    let form: std::collections::HashMap<_, _> = url::form_urlencoded::parse(&request.body)
        .into_owned()
        .collect();
    assert_eq!(form.get("client_id").map(String::as_str), Some(CLIENT_ID));
    assert_eq!(
        form.get("client_secret").map(String::as_str),
        Some(CLIENT_SECRET)
    );
    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("authorization_code")
    );
    assert!(
        form.get("code_verifier")
            .is_some_and(|value| !value.is_empty())
    );
    let smoke = row["smoke_command"].as_str().unwrap();
    assert_eq!(
        smoke,
        "nyxid proxy request api-local-oauth /accounts --method GET"
    );
    let args: Vec<_> = smoke.split_ascii_whitespace().skip(1).collect();
    cli(&executable, &base, &user_token, &args, None, false).await;
    let snapshot = catalog_snapshot(&db).await;
    let replay = cli(
        &executable,
        &base,
        &manager_token,
        &["admin", "catalog", "apply"],
        Some(&file),
        true,
    )
    .await;
    assert!(
        replay["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["action"] == "unchanged"),
        "{replay}"
    );
    assert_eq!(
        snapshot,
        catalog_snapshot(&db).await,
        "complete reapply must not write catalog rows"
    );
    let disconnected = http
        .delete(format!("{base}/api/v1/keys/{key_id}"))
        .bearer_auth(&user_token)
        .send()
        .await
        .unwrap();
    assert!(disconnected.status().is_success());
    let disconnected: Value = disconnected.json().await.unwrap();
    assert_eq!(disconnected["upstream_revocation_scheduled"], true);
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if vendor
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|request| request.url.path() == "/revoke")
                .count()
                == 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("owned fake revoke called");
    let requests = vendor.received_requests().await.unwrap();
    let revoked: Vec<_> = requests
        .iter()
        .filter(|request| request.url.path() == "/revoke")
        .map(|request| {
            url::form_urlencoded::parse(&request.body)
                .into_owned()
                .collect::<std::collections::HashMap<_, _>>()
        })
        .collect();
    for (value, hint) in [(ACCESS, "access_token"), (REFRESH, "refresh_token")] {
        let form = revoked
            .iter()
            .find(|form| form.get("token_type_hint").map(String::as_str) == Some(hint))
            .unwrap();
        assert_eq!(form.get("token").map(String::as_str), Some(value));
        assert_eq!(form.get("client_id").map(String::as_str), Some(CLIENT_ID));
        assert_eq!(
            form.get("client_secret").map(String::as_str),
            Some(CLIENT_SECRET)
        );
    }
    let audits: Vec<Document> = db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_redacted(&format!("{audits:?}"));
    vendor.verify().await;
    println!(
        "LOCAL_CATALOG_E2E_EVIDENCE: manager CLI apply/env credential transport, served SVG, six-item complete/not_verified readiness, second-person OAuth authorization/callback/managed client/scopes/PKCE, printed GET smoke with user custom bearer scheme, RFC7009 disconnect revoke, poisoned-env write-free dry-run/validate, write-free reapply, redacted output/manifest/audit: PASS; real-vendor gates remain open"
    );
    server.abort();
    drop(revocation_guard);
    assert!(
        crate::services::provider_service::validate_revocation_url(&revocation_url)
            .await
            .is_err(),
        "the exact mock exception ends with the fixture"
    );
    db.drop().await.unwrap();
}
