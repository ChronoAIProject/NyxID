use super::*;
use crate::models::downstream_service::ProxyOperationRule;

#[tokio::test]
async fn service_pool_credential_fallback_checks_selected_member_scope() {
    for strategy in ["priority", "round_robin", "weighted"] {
        for allowed in [false, true] {
            let mut fixture =
                fixture("pool_scoped_credentials", StatusCode::OK, strategy, true).await;
            bind_first_credential(&fixture).await;
            fixture
                .state
                .db
                .collection::<Document>("user_api_keys")
                .update_many(doc! {}, doc! {"$set":{"status":"failed"}})
                .await
                .unwrap();
            let members: Vec<crate::models::user_service::UserService> = fixture
                .state
                .db
                .collection("user_services")
                .find(doc! {})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            let selections: Vec<_> = members
                .into_iter()
                .map(|member| {
                    let rules = if member.slug == "review-first" || allowed {
                        vec![ProxyOperationRule {
                            method: "POST".into(),
                            path_template: "/perform".into(),
                            ..Default::default()
                        }]
                    } else {
                        vec![]
                    };
                    (member.id, rules)
                })
                .collect();
            fixture.auth = crate::test_utils::scoped_specialist_auth(
                &fixture.state,
                &fixture.auth.user_id.to_string(),
                &selections,
            )
            .await;
            if strategy != "priority" {
                // Legacy balancing makes one attempt per request; it does not
                // skip failed credentials. The following request rotates to
                // the second member, whose scope must still be enforced.
                assert!(matches!(
                    try_call(&fixture, "{}").await,
                    Err(crate::errors::AppError::CredentialUnavailable(_))
                ));
                assert!(fixture.first.requests.lock().await.is_empty());
                assert!(fixture.second.requests.lock().await.is_empty());
            }
            let response = try_call(&fixture, "{}").await;
            if allowed {
                let response = response.unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                to_bytes(response.into_body(), 1024).await.unwrap();
            } else {
                assert!(
                    matches!(response, Err(crate::errors::AppError::ApiKeyScopeForbidden(ref message)) if message.contains("Allowed operations"))
                );
            }
            assert!(fixture.first.requests.lock().await.is_empty());
            assert_eq!(
                fixture.second.requests.lock().await.len(),
                usize::from(allowed)
            );
            fixture.state.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn service_pool_selected_member_does_not_intersect_sibling_scopes() {
    for strategy in ["priority", "round_robin", "weighted"] {
        let mut fixture = fixture("pool_scoped_siblings", StatusCode::OK, strategy, true).await;
        let members: Vec<crate::models::user_service::UserService> = fixture
            .state
            .db
            .collection("user_services")
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let selections: Vec<_> = members
            .into_iter()
            .map(|member| {
                let rules = if member.slug == "review-first" {
                    vec![ProxyOperationRule {
                        method: "POST".into(),
                        path_template: "/perform".into(),
                        ..Default::default()
                    }]
                } else {
                    vec![]
                };
                (member.id, rules)
            })
            .collect();
        fixture.auth = crate::test_utils::scoped_specialist_auth(
            &fixture.state,
            &fixture.auth.user_id.to_string(),
            &selections,
        )
        .await;
        let response = try_call(&fixture, "{}").await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(fixture.first.requests.lock().await.len(), 1);
        assert!(fixture.second.requests.lock().await.is_empty());
        fixture.state.db.drop().await.unwrap();
    }
}
