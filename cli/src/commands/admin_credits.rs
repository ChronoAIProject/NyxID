use std::io::{BufRead, IsTerminal, Write};

use anyhow::{Result, bail};
use chrono::Utc;
use serde_json::{Value, json};

use super::billing_support as support;
use crate::cli::{AdminCreditsCommands, CreditGrantCommands};

fn confirm_grant(amount: i64, mut input: impl BufRead, mut output: impl Write) -> Result<()> {
    write!(
        output,
        "Issue {amount} credits to the selected recipient? This creates a new grant batch [y/N]: "
    )?;
    output.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    if !matches!(answer.trim(), "y" | "Y" | "yes" | "YES") {
        bail!("Grant issuance cancelled");
    }
    Ok(())
}

pub async fn run(command: AdminCreditsCommands) -> Result<()> {
    let AdminCreditsCommands::Grant { command } = command;
    match command {
        CreditGrantCommands::Issue {
            user,
            amount,
            service,
            all_services,
            expires_at,
            reason,
            yes,
            auth,
        } => {
            support::require_human(&auth)?;
            if !(1..=1_000_000).contains(&amount)
                || reason.trim().is_empty()
                || reason.trim().len() > 2000
            {
                bail!(
                    "An amount of 1..=1000000 whole credits and a reason of 1..=2000 bytes are required"
                );
            }
            if all_services != service.is_empty() {
                bail!("Choose --all-services or one or more --service values");
            }
            if service.len() > 100 {
                bail!("At most 100 service references are allowed");
            }
            let expires_at = expires_at.as_deref().map(support::timestamp).transpose()?;
            if expires_at.is_some_and(|expires| expires <= Utc::now()) {
                bail!("expires-at must be in the future");
            }
            let mut api = support::human_api(&auth).await?;
            let user = super::admin::resolve_user_id(&mut api, &user).await?;
            let mut refs = vec![];
            for reference in service {
                let row = support::service(&mut api, &reference).await?;
                let id = support::text(&row["id"]);
                if !refs.contains(&id) {
                    refs.push(id);
                }
            }
            if !yes {
                if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
                    bail!(
                        "Grant issue is not idempotent. Confirm in a terminal or pass --yes to issue once"
                    );
                }
                confirm_grant(amount, std::io::stdin().lock(), std::io::stderr().lock())?;
            }
            let body = json!({
                "amount_credits": amount,
                "target_kind": "selected_users",
                "target_user_ids": [
                    user
                ],
                "all_services": all_services,
                "service_refs": refs,
                "expires_at": expires_at.map(support::rfc3339),
                "reason": reason
            });
            let result: Value = api.post("/admin/credits/grants", &body).await?;
            support::print(&result, auth.output, || support::fields(&result))
        }
        CreditGrantCommands::List {
            user,
            page,
            per_page,
            auth,
        } => {
            let mut api = support::human_api(&auth).await?;
            let query = match user {
                Some(user) => format!(
                    "&recipient_user_id={}",
                    urlencoding::encode(&super::admin::resolve_user_id(&mut api, &user).await?)
                ),
                None => String::new(),
            };
            if page == 0 || !(1..=500).contains(&per_page) {
                bail!("page must be positive and per-page must be in 1..=500");
            }
            let value: Value = api
                .get(&format!(
                    "/admin/credits/grants?page={page}&per_page={per_page}{query}"
                ))
                .await?;
            support::print(&value, auth.output, || support::fields(&value))
        }
        CreditGrantCommands::Revoke { grant_id, auth } => {
            support::require_human(&auth)?;
            let id = support::uuid(&grant_id)?;
            let mut api = support::human_api(&auth).await?;
            let value: Value = api.delete(&format!("/admin/credits/grants/{id}")).await?;
            support::print(&value, auth.output, || support::fields(&value))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_auth;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path, query_param},
    };
    const ID: &str = "00000000-0000-4000-8000-000000000001";
    fn issue(server: &MockServer, yes: bool) -> AdminCreditsCommands {
        AdminCreditsCommands::Grant {
            command: CreditGrantCommands::Issue {
                user: "pilot@example.test".into(),
                amount: 5,
                service: vec![ID.into()],
                all_services: false,
                expires_at: None,
                reason: "Pilot".into(),
                yes,
                auth: mock_auth(server.uri()),
            },
        }
    }
    async fn lookups(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/users"))
            .and(query_param("search", "pilot@example.test"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"users":[{"id":ID,"email":"pilot@example.test"}]})),
            )
            .expect(1)
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/services/{ID}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":ID})))
            .expect(1)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn grant_issue_resolves_email_scope_and_posts_once_with_activation_states() {
        let server = MockServer::start().await;
        lookups(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/credits/grants"))
            .and(body_json(json!({
                "amount_credits": 5,
                "target_kind": "selected_users",
                "target_user_ids": [
                    ID
                ],
                "all_services": false,
                "service_refs": [
                    ID
                ],
                "expires_at": null,
                "reason": "Pilot"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "batch_id": "batch-test",
                "recipients": [
                    {
                        "recipient_user_id": ID,
                        "activation_state": "pending_activation"
                    }
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;
        run(issue(&server, true)).await.unwrap();
    }

    #[tokio::test]
    async fn grant_issue_without_yes_refuses_nonterminal_and_never_posts() {
        let server = MockServer::start().await;
        lookups(&server).await;
        let error = run(issue(&server, false)).await.unwrap_err();
        assert!(error.to_string().contains("--yes"));
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.method == "GET")
        );
    }

    #[test]
    fn grant_confirmation_displays_amount_without_recipient_and_requires_affirmative_answer() {
        for answer in ["y\n", "Y\n", "yes\n", "YES\n", "n\n", "\n", ""] {
            let mut output = Vec::new();
            let result = confirm_grant(5, answer.as_bytes(), &mut output);
            let prompt = String::from_utf8(output).unwrap();
            assert!(prompt.contains("Issue 5 credits to the selected recipient?"));
            assert!(prompt.contains("new grant batch [y/N]"));
            assert!(!prompt.contains(ID));
            assert!(!prompt.contains("pilot@example.test"));
            if matches!(answer, "y\n" | "Y\n" | "yes\n" | "YES\n") {
                result.unwrap();
            } else {
                assert_eq!(result.unwrap_err().to_string(), "Grant issuance cancelled");
            }
        }
    }

    #[tokio::test]
    async fn grant_list_user_and_revoke_routes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/credits/grants"))
            .and(query_param("recipient_user_id", ID))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"grants":[]})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/admin/credits/grants/{ID}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id":ID,"status":"revoked"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        run(AdminCreditsCommands::Grant {
            command: CreditGrantCommands::List {
                page: 1,
                per_page: 50,
                user: Some(ID.into()),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .unwrap();
        run(AdminCreditsCommands::Grant {
            command: CreditGrantCommands::Revoke {
                grant_id: ID.into(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn grant_errors_are_not_retried_and_invalid_revoke_is_local() {
        let server = MockServer::start().await;
        lookups(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/credits/grants"))
            .respond_with(ResponseTemplate::new(503).set_body_string("unavailable"))
            .expect(1)
            .mount(&server)
            .await;
        assert!(run(issue(&server, true)).await.is_err());
        assert!(
            run(AdminCreditsCommands::Grant {
                command: CreditGrantCommands::Revoke {
                    grant_id: "not-uuid".into(),
                    auth: mock_auth(server.uri())
                }
            })
            .await
            .is_err()
        );
    }
}
