use anyhow::{Context, Result};
use serde_json::Value;
use tokio::time::{Duration, Instant};

use super::billing_support as support;
use crate::cli::BillingExecutionCommands;

pub async fn run(command: BillingExecutionCommands) -> Result<()> {
    match command {
        BillingExecutionCommands::List { since, auth } => {
            support::require_human(&auth)?;
            let since = since.as_deref().map(support::timestamp).transpose()?;
            let mut api = support::human_api(&auth).await?;
            let mut rows: Vec<Value> = api.get("/billing/executions").await?;
            if let Some(since) = since {
                let mut filtered = vec![];
                for row in rows {
                    let created = support::timestamp(
                        row["created_at"]
                            .as_str()
                            .context("Execution has no creation timestamp")?,
                    )?;
                    if created >= since {
                        filtered.push(row);
                    }
                }
                rows = filtered;
            }
            let value = serde_json::json!(rows);
            support::print(&value, auth.output, || execution_table(&rows))
        }
        BillingExecutionCommands::Show {
            id,
            wait,
            timeout_secs,
            auth,
        } => {
            support::require_human(&auth)?;
            let id = support::uuid(&id)?;
            let mut api = support::human_api(&auth).await?;
            let deadline = Instant::now()
                .checked_add(Duration::from_secs(timeout_secs))
                .context("Timeout is too large")?;
            loop {
                let row: Value = api.get(&format!("/billing/executions/{id}")).await?;
                let terminal = matches!(row["state"].as_str(), Some("settled" | "released"));
                if terminal || !wait || Instant::now() >= deadline {
                    support::print(&row, auth.output, || {
                        execution_table(std::slice::from_ref(&row))
                    })?;
                    return support::exit_if_not_ready(!wait || terminal);
                }
                tokio::time::sleep_until((Instant::now() + Duration::from_secs(1)).min(deadline))
                    .await;
            }
        }
    }
}

fn execution_table(rows: &[Value]) -> String {
    support::table(
        &[
            "ID",
            "Service",
            "Model",
            "State",
            "Maximum credits",
            "Pending reason",
        ],
        rows.iter().map(|row| {
            [
                "id",
                "service_slug",
                "model",
                "state",
                "maximum_credits",
                "pending_reason",
            ]
            .iter()
            .map(|key| support::text(&row[*key]))
            .collect()
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_auth;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    const ID: &str = "00000000-0000-4000-8000-000000000001";

    #[tokio::test]
    async fn execution_list_and_since_filter_use_the_self_scoped_route() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/billing/executions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {
                    "id": ID,
                    "created_at": "2026-10-20T00:00:00Z",
                    "state": "settled",
                    "maximum_credits": "0.000000000001",
                    "pending_reason": null
                },
                {
                    "id": ID,
                    "created_at": "2026-10-18T00:00:00Z",
                    "state": "pending"
                }
            ])))
            .expect(1)
            .mount(&server)
            .await;
        run(BillingExecutionCommands::List {
            since: Some("2026-10-19T00:00:00Z".into()),
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn execution_show_wait_settled_released_and_pending_timeout() {
        for state in ["settled", "released", "pending", "dispatch_admitted"] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(format!("/api/v1/billing/executions/{ID}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": ID,
                    "state": state,
                    "maximum_credits": "0.002048",
                    "pending_reason": "verification"
                })))
                .expect(1)
                .mount(&server)
                .await;
            let result = run(BillingExecutionCommands::Show {
                id: ID.into(),
                wait: true,
                timeout_secs: 0,
                auth: mock_auth(server.uri()),
            })
            .await;
            if matches!(state, "settled" | "released") {
                result.unwrap();
            } else {
                assert_eq!(
                    result
                        .unwrap_err()
                        .downcast_ref::<super::super::admin_tools::PendingReadiness>()
                        .unwrap()
                        .exit_code,
                    2
                );
            }
        }
    }

    #[tokio::test]
    async fn executions_reject_invalid_uuid_since_and_agent_key_before_requests() {
        let server = MockServer::start().await;
        assert!(
            run(BillingExecutionCommands::Show {
                id: "slug".into(),
                wait: false,
                timeout_secs: 0,
                auth: mock_auth(server.uri())
            })
            .await
            .is_err()
        );
        assert!(
            run(BillingExecutionCommands::List {
                since: Some("yesterday".into()),
                auth: mock_auth(server.uri())
            })
            .await
            .is_err()
        );
        let mut auth = mock_auth(server.uri());
        auth.access_token = Some("nyxid_ag_SECRET".into());
        assert!(
            run(BillingExecutionCommands::List { since: None, auth })
                .await
                .unwrap_err()
                .to_string()
                .contains("nyxid login")
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
