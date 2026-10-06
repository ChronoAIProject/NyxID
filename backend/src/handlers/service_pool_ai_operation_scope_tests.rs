use super::*;
use crate::models::downstream_service::ProxyOperationRule;

async fn restrict(fixture: &mut Fixture, primary: Option<&str>, backup: Option<&str>) {
    let mut selections = Vec::new();
    for (slug, path) in [("review-ai-0", primary), ("review-ai-1", backup)] {
        let member = fixture
            .state
            .db
            .collection::<crate::models::user_service::UserService>("user_services")
            .find_one(doc! {"slug":slug})
            .await
            .unwrap()
            .unwrap();
        selections.push((
            member.id,
            path.into_iter()
                .map(|path| ProxyOperationRule {
                    method: "POST".into(),
                    path_template: path.into(),
                    ..Default::default()
                })
                .collect(),
        ));
    }
    fixture.auth = crate::test_utils::scoped_specialist_auth(
        &fixture.state,
        &fixture.auth.user_id.to_string(),
        &selections,
    )
    .await;
}

#[tokio::test]
async fn service_pool_operation_scope_checks_translated_backup_on_every_entry() {
    for entry in [Entry::Slug, Entry::Gateway, Entry::Machine] {
        for allowed in [false, true] {
            let mut fixture = fixture(
                "pool_scoped_translation",
                ResponseTemplate::new(200).set_body_json(anthropic_text_response()),
            )
            .await;
            // The public pool route is NOT the Anthropic member operation.
            restrict(
                &mut fixture,
                Some("/chat/completions"),
                Some(if allowed {
                    "/messages"
                } else {
                    "/chat/completions"
                }),
            )
            .await;
            let response = call(&fixture, entry, "chat/completions", basic_request()).await;
            if allowed {
                let response = response.unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                to_bytes(response.into_body(), 16 * 1024).await.unwrap();
            } else {
                assert!(
                    matches!(response, Err(crate::errors::AppError::ApiKeyScopeForbidden(ref message)) if message.contains("Allowed operations"))
                );
            }
            assert_eq!(fixture.first.received_requests().await.unwrap().len(), 1);
            let backup = fixture.second.received_requests().await.unwrap();
            assert_eq!(backup.len(), usize::from(allowed));
            if allowed {
                assert_eq!(backup[0].url.path(), "/v1/messages");
            }
            fixture.state.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn service_pool_alias_discovery_intersects_native_member_operations() {
    for (primary, backup, expected) in [
        (None, None, false),
        (None, Some("/chat/completions"), false),
        (None, Some("/messages"), true),
        (Some("/chat/{operation}"), None, true),
    ] {
        let mut fixture = fixture("pool_scoped_discovery", ResponseTemplate::new(200)).await;
        restrict(&mut fixture, primary, backup).await;
        let before = fixture.state.encryption_keys.decrypt_stats();
        let axum::Json(aliases) = super::super::llm_gateway::pool_aliases(
            State(fixture.state.clone()),
            fixture.auth.clone(),
            axum::extract::Query(super::super::llm_gateway::PoolAliasesQuery {
                offset: None,
                limit: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(aliases.pools.len(), usize::from(expected));
        if expected {
            assert!(aliases.pools[0].available);
        }
        assert_eq!(before, fixture.state.encryption_keys.decrypt_stats());
        assert!(fixture.first.received_requests().await.unwrap().is_empty());
        assert!(fixture.second.received_requests().await.unwrap().is_empty());
        fixture.state.db.drop().await.unwrap();
    }
}
