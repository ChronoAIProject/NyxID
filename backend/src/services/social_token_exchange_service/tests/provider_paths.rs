use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const PROVIDER_HOSTS: [&str; 3] = ["www.googleapis.com", "appleid.apple.com", "api.github.com"];

/// Only this fixture's client trusts its ephemeral TLS certificate and resolves
/// the three provider names to its loopback listener.
async fn provider_fixture() -> (reqwest::Client, Arc<Mutex<Vec<String>>>) {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cert =
        rcgen::generate_simple_self_signed(PROVIDER_HOSTS.map(str::to_owned).to_vec()).unwrap();
    let root = reqwest::Certificate::from_der(cert.cert.der()).unwrap();
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
        )
        .unwrap();
    let tls = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let tls = tls.clone();
            let seen = seen.clone();
            tokio::spawn(async move {
                let stream = tls.accept(socket).await.unwrap();
                let service = hyper::service::service_fn(
                    move |r: Request<hyper::body::Incoming>| {
                        let path = r.uri().path();
                        seen.lock().unwrap().push(format!("{} {path}", r.method()));
                        let body = match path {
                            "/oauth2/v3/certs" | "/auth/keys" => {
                                json!({"keys":[crate::test_utils::cached_test_jwk()]})
                            }
                            "/applications/qa-client/token" => {
                                json!({"app":{"client_id":"qa-client"}})
                            }
                            "/user" => json!({"id":123,"login":"social-qa","name":"Social QA"}),
                            "/user/emails" => {
                                json!([{"email":"github@example.test","verified":true,"primary":true}])
                            }
                            _ => panic!("unexpected fixture request to {path}"),
                        };
                        std::future::ready(Ok::<_, std::convert::Infallible>(hyper::Response::new(
                            http_body_util::Full::new(bytes::Bytes::from(body.to_string())),
                        )))
                    },
                );
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                    .await
                    .unwrap();
            });
        }
    });
    let mut client = reqwest::Client::builder()
        .no_proxy()
        .add_root_certificate(root);
    for host in PROVIDER_HOSTS {
        client = client.resolve(host, addr);
    }
    (client.build().unwrap(), calls)
}

fn fixture_proof(provider: &str) -> String {
    if provider == "github" {
        return "local-test-access-token".into();
    }
    let keys = cached_test_jwt_keys();
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(keys.kid.clone());
    let now = chrono::Utc::now().timestamp();
    let issuer = if provider == "google" {
        "https://accounts.google.com"
    } else {
        "https://appleid.apple.com"
    };
    jsonwebtoken::encode(&header, &json!({
        "sub": format!("qa-{provider}"), "iss":issuer, "aud":"qa-client",
        "iat":now, "exp":now+300, "email":format!("{provider}@example.test"), "email_verified":true
    }), &keys.encoding).unwrap()
}

async fn exchange(
    app: &axum::Router,
    provider: &str,
    proof: &str,
    kind: &str,
) -> (StatusCode, Value) {
    let form = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("provider", provider),
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:token-exchange",
            ),
            ("client_id", "qa-client"),
            ("subject_token", proof),
            ("subject_token_type", kind),
        ])
        .finish();
    let request = Request::post("/oauth/token")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(form))
        .unwrap();
    let result = app.clone().oneshot(request).await.unwrap();
    let status = result.status();
    let body = axum::body::to_bytes(result.into_body(), 65536)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn provider_paths_through_oauth_token() {
    use crate::models::{session, user};
    let db = crate::test_utils::connect_transaction_test_database("social_provider_routes").await;
    let mut state = crate::test_utils::test_app_state(db.clone());
    let (http, calls) = provider_fixture().await;
    state.jwks_cache = Arc::new(JwksCache::new(http.clone()));
    state.http_client = http;
    state.config.apple_client_id = Some("qa-client".into());
    state.config.google_client_id = Some("qa-client".into());
    state.config.github_client_id = Some("qa-client".into());
    state.config.github_client_secret = Some("local-test-secret".into());
    insert_public_oauth_client(&db, "qa-client").await;
    let users = db.collection::<mongodb::bson::Document>(user::COLLECTION_NAME);
    let sessions = db.collection::<mongodb::bson::Document>(session::COLLECTION_NAME);
    for (flag, enabled) in [
        (feature_flag_service::INVITATION_CODE_FLAG_KEY, false),
        (feature_flag_service::MOBILE_MFA_FLAG_KEY, true),
    ] {
        feature_flag_service::set_platform_override(
            &db,
            flag,
            &feature_flag_service::FlagTarget::Global,
            enabled,
            "qa",
        )
        .await
        .unwrap();
    }
    let (public, private) = crate::routes::build_router_with_state(state.clone());
    let app = public.merge(private).with_state(state);
    for (provider, accepted, wrong, first_calls) in [
        (
            "google",
            SUBJECT_TOKEN_TYPE_ID_TOKEN,
            SUBJECT_TOKEN_TYPE_ACCESS_TOKEN,
            &["GET /oauth2/v3/certs"][..],
        ),
        (
            "apple",
            SUBJECT_TOKEN_TYPE_ID_TOKEN,
            SUBJECT_TOKEN_TYPE_ACCESS_TOKEN,
            &["GET /auth/keys"][..],
        ),
        (
            "github",
            SUBJECT_TOKEN_TYPE_ACCESS_TOKEN,
            SUBJECT_TOKEN_TYPE_ID_TOKEN,
            &[
                "POST /applications/qa-client/token",
                "GET /user",
                "GET /user/emails",
            ][..],
        ),
    ] {
        let email = format!("{provider}@example.test");
        let proof = fixture_proof(provider);
        calls.lock().unwrap().clear();
        let (status, body) = exchange(&app, provider, &proof, wrong).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_request");
        let display_name = match provider {
            "google" => "Google",
            "apple" => "Apple",
            "github" => "GitHub",
            _ => unreachable!(),
        };
        assert!(
            body["error_description"]
                .as_str()
                .is_some_and(|description| description.ends_with(&format!(
                    "{display_name} social exchange requires subject_token_type={accepted}"
                )))
        );
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(
            users.count_documents(doc! {"email":&email}).await.unwrap(),
            0
        );
        for attempt in 0..3 {
            if attempt == 2 {
                users
                    .update_one(doc! {"email":&email}, doc! {"$set":{"mfa_enabled":true}})
                    .await
                    .unwrap();
            }
            calls.lock().unwrap().clear();
            let (status, body) = exchange(&app, provider, &proof, accepted).await;
            let expected_calls = if attempt == 0 || provider == "github" {
                first_calls
            } else {
                &[]
            };
            assert_eq!(calls.lock().unwrap().as_slice(), expected_calls);
            if attempt < 2 {
                assert_eq!(
                    status,
                    StatusCode::OK,
                    "{provider} exchange error {:?}",
                    body.get("error")
                );
                for field in ["access_token", "refresh_token", "id_token"] {
                    assert!(
                        body[field].as_str().is_some_and(|s| !s.is_empty()),
                        "{field}"
                    );
                }
            } else {
                assert_eq!(status, StatusCode::BAD_REQUEST);
                assert_eq!(body["error"], "mfa_required");
                assert_eq!(body["error_code"], 2002);
                assert!(
                    body["session_token"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
                );
                for field in ["access_token", "refresh_token", "id_token"] {
                    assert!(body.get(field).is_none(), "{field} returned before MFA");
                }
            }
            assert_eq!(
                users.count_documents(doc! {"email":&email}).await.unwrap(),
                1
            );
            let u = users
                .find_one(doc! {"email":&email})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                sessions
                    .count_documents(doc! {"user_id":u.get_str("_id").unwrap()})
                    .await
                    .unwrap(),
                (attempt + 1).min(2)
            );
        }
    }
}
