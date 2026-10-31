#[tokio::test]
async fn proxy_2113_write_preconditions_reach_direct_and_real_node_http() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    // Read raw values: Wiremock's generic header matcher splits HTTP dates at commas.
    fn received_header<'a>(request: &'a wiremock::Request, name: &str) -> Option<&'a str> {
        request
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
    }
    let db = crate::test_utils::connect_transaction_test_database("proxy_2113").await;
    let state = test_app_state(db.clone());
    let owner = Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    crate::test_utils::set_test_approval_preference(&db, &owner, false).await;
    let downstream = MockServer::start().await;
    const DATE: &str = "Wed, 21 Oct 2015 07:28:00 GMT";
    const OLD_DATE: &str = "Tue, 20 Oct 2015 07:28:00 GMT";
    Mock::given(method("PATCH"))
        .and(path("/rules/one"))
        .respond_with(|request: &wiremock::Request| {
            let (status, result) = match (
                received_header(request, "if-match"),
                received_header(request, "if-unmodified-since"),
            ) {
                (None, None) => (428, "revision_required"),
                (Some("\"0\""), _) | (_, Some(OLD_DATE)) => (409, "stale_write"),
                (Some("\"1\""), None | Some(DATE)) | (None, Some(DATE)) => (200, "updated"),
                _ => (400, "invalid_precondition"),
            };
            ResponseTemplate::new(status).set_body_json(serde_json::json!({"result":result}))
        })
        .mount(&downstream)
        .await;
    for via_node in [false, true] {
        for via_uuid in [false, true] {
            for (revision, unmodified_since, expected, result) in [
                (
                    None,
                    None,
                    StatusCode::PRECONDITION_REQUIRED,
                    "revision_required",
                ),
                (Some("\"1\""), None, StatusCode::OK, "updated"),
                (None, Some(DATE), StatusCode::OK, "updated"),
                (Some("\"1\""), Some(DATE), StatusCode::OK, "updated"),
                (
                    Some("\"0\""),
                    Some(DATE),
                    StatusCode::CONFLICT,
                    "stale_write",
                ),
                (None, Some(OLD_DATE), StatusCode::CONFLICT, "stale_write"),
            ] {
                let catalog_id = if via_uuid {
                    let mut catalog =
                        crate::models::downstream_service::test_helpers::dummy_service();
                    catalog.id = Uuid::new_v4().to_string();
                    catalog.slug = format!("conditional-{}", Uuid::new_v4());
                    catalog.base_url = downstream.uri();
                    db.collection::<crate::models::downstream_service::DownstreamService>(
                        "downstream_services",
                    )
                    .insert_one(&catalog)
                    .await
                    .unwrap();
                    Some(catalog.id)
                } else {
                    None
                };
                let service = insert_user_service(
                    &db,
                    &owner,
                    &format!("preconditions-{}", Uuid::new_v4()),
                    &downstream.uri(),
                    catalog_id.as_deref(),
                )
                .await;
                let executor = if via_node {
                    let node = insert_online_node(&state, &owner, "preconditions-node").await;
                    db.collection::<UserService>(USER_SERVICES)
                        .update_one(doc! {"_id":&service.id}, doc! {"$set":{"node_id":&node.id}})
                        .await
                        .unwrap();
                    Some(
                        start_node_executor(state.clone(), &node, &service.slug, &downstream.uri())
                            .await,
                    )
                } else {
                    None
                };
                let uri = if via_uuid {
                    format!("/proxy/{}/rules/one", catalog_id.as_ref().unwrap())
                } else {
                    format!("/proxy/s/{}/rules/one", service.slug)
                };
                let mut request = proxy_json_request(&uri, r#"{"enabled":true}"#);
                *request.method_mut() = Method::PATCH;
                if let Some(revision) = revision {
                    request
                        .headers_mut()
                        .insert("if-match", revision.parse().unwrap());
                }
                if let Some(date) = unmodified_since {
                    request
                        .headers_mut()
                        .insert("if-unmodified-since", date.parse().unwrap());
                }
                for name in [
                    "authorization",
                    "cookie",
                    "x-nyxid-user-id",
                    "x-forwarded-for",
                ] {
                    request
                        .headers_mut()
                        .insert(name, "must-not-forward".parse().unwrap());
                }
                let response = if via_uuid {
                    Box::pin(proxy_request_inner(
                        &state,
                        &access_token_auth(&owner),
                        catalog_id.as_ref().unwrap(),
                        "rules/one",
                        request,
                        &mut String::new(),
                    ))
                    .await
                } else {
                    Box::pin(proxy_request_by_slug_inner(
                        &state,
                        &access_token_auth(&owner),
                        &service.slug,
                        "rules/one",
                        request,
                        &mut String::new(),
                    ))
                    .await
                }
                .unwrap();
                let received = downstream.received_requests().await.unwrap();
                let last = received.last().expect("downstream request");
                assert_eq!(received_header(last, "if-match"), revision);
                assert_eq!(
                    received_header(last, "if-unmodified-since"),
                    unmodified_since
                );
                assert_eq!(
                    response.status(),
                    expected,
                    "node={via_node} uuid={via_uuid} revision={revision:?}, received={:?}",
                    downstream.received_requests().await.unwrap()
                );
                let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["result"],
                    result
                );
                if let Some((executor, server)) = executor {
                    executor.await.unwrap();
                    server.abort();
                }
            }
        }
    }
    let received = downstream.received_requests().await.unwrap();
    assert_eq!(received.len(), 24);
    for request in received {
        for name in [
            "authorization",
            "cookie",
            "x-nyxid-user-id",
            "x-forwarded-for",
        ] {
            assert!(!request.headers.contains_key(name), "forwarded {name}");
        }
    }
}
