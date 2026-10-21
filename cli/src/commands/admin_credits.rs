use std::io::{BufRead, IsTerminal, Write};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::{Value, json};

use super::billing_support as support;
use crate::{
    api::ApiClient,
    cli::{AdminCreditsCommands, CreditGrantCommands, OutputFormat},
    error_format::RetryContext,
};

fn grant_error(error: anyhow::Error, request_id: &str) -> anyhow::Error {
    let conflict = error
        .downcast_ref::<crate::api::ApiError>()
        .is_some_and(|error| error.status() == reqwest::StatusCode::CONFLICT);
    let prefix = if conflict {
        "409 Conflict: a reused request ID requires identical settings. "
    } else {
        ""
    };
    error.context(RetryContext {
        request_id: request_id.into(),
        message: format!("Grant request {request_id}"),
        guidance: format!(
            "{prefix}Retry with --request-id {request_id} and identical settings using the same account \
             to return the original batch. A new invocation without --request-id creates a new batch."
        ),
    })
}

fn grant_amount(raw: &str) -> Result<String> {
    let pico = support::decimal_pico(raw.trim())?;
    if pico <= 0 || pico > 1_000_000 * 1_000_000_000_000_i128 {
        bail!("Grant amount must be greater than 0 and at most 1000000 credits");
    }
    Ok(support::credits(pico))
}

fn announce_request(request_id: &str, format: OutputFormat, mut output: impl Write) -> Result<()> {
    if matches!(format, OutputFormat::Json) {
        writeln!(output, "{}", json!({"request_id": request_id}))?;
    } else {
        writeln!(output, "Request ID: {request_id}")?;
    }
    output.flush()?;
    Ok(())
}

fn confirm_grant(amount: &str, mut input: impl BufRead, mut output: impl Write) -> Result<()> {
    write!(
        output,
        "Issue {amount} credits to the selected recipient? Retrying the same --request-id with \
         identical settings returns the original batch; a new invocation without it creates a new batch [y/N]: "
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
            request_id,
            service,
            all_services,
            expires_at,
            reason,
            yes,
            auth,
        } => {
            support::require_human(&auth)?;
            let amount = grant_amount(&amount)?;
            let retry = request_id.is_some();
            let request_id = request_id
                .as_deref()
                .map(support::uuid)
                .transpose()
                .context("--request-id must be a UUID")?
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            if reason.trim().is_empty() || reason.trim().len() > 2000 {
                bail!("A reason of 1..=2000 bytes is required");
            }
            if all_services != service.is_empty() {
                bail!("Choose --all-services or one or more --service values");
            }
            if service.len() > 100 {
                bail!("At most 100 service references are allowed");
            }
            let expires_at = expires_at.as_deref().map(support::timestamp).transpose()?;
            // Existing submissions remain retryable after their grants expire.
            if !retry && expires_at.is_some_and(|expires| expires <= Utc::now()) {
                bail!("expires-at must be in the future");
            }
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
                        "Confirm grant issuance in a terminal or pass --yes. Retrying the same \
                         --request-id with identical settings returns the original batch; a new \
                         invocation without it creates a new batch"
                    );
                }
                confirm_grant(&amount, std::io::stdin().lock(), std::io::stderr().lock())?;
            }
            // Old replicas require amount_credits; omit it so they reject this retry-safe payload.
            let body = json!({
                "amount": amount,
                "request_id": request_id,
                "target_kind": "selected_users",
                "target_user_ids": [
                    user
                ],
                "all_services": all_services,
                "service_refs": refs,
                "expires_at": expires_at.map(support::rfc3339),
                "reason": reason
            });
            announce_request(&request_id, auth.output, std::io::stderr().lock())?;
            let mut result: Value = api
                .post("/admin/credits/grants", &body)
                .await
                .map_err(|error| grant_error(error, &request_id))?;
            result
                .as_object_mut()
                .context("Grant issuance returned a non-object response")
                .map_err(|error| grant_error(error, &request_id))?
                .insert("request_id".into(), json!(request_id));
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
            support::print(&value, auth.output, || {
                format!(
                    "Page: {}  Per page: {}  Total: {}\n{}",
                    support::text(&value["page"]),
                    support::text(&value["per_page"]),
                    support::text(&value["total"]),
                    grants_table(value["grants"].as_array().map(Vec::as_slice).unwrap_or(&[])),
                )
            })
        }
        CreditGrantCommands::Revoke { grant_id, auth } => {
            support::require_human(&auth)?;
            let id = support::uuid(&grant_id)?;
            let mut api = support::human_api(&auth).await?;
            let value: Value = api.delete(&format!("/admin/credits/grants/{id}")).await?;
            support::print(&value, auth.output, || {
                grants_table(std::slice::from_ref(&value))
            })
        }
    }
}

fn grants_table(grants: &[Value]) -> String {
    let rows = grants.iter().map(|grant| {
        let revoke_state = if grant["status"] == "revoked" {
            "complete".into()
        } else if !grant["revoke_requested_at"].is_null() {
            format!("requested {}", support::text(&grant["revoke_requested_at"]))
        } else {
            "-".into()
        };
        let mut row: Vec<_> = [
            "id",
            "recipient_user_id",
            "amount",
            "remaining",
            "reserved",
            "held",
            "reclaimed",
            "status",
            "activation_state",
        ]
        .iter()
        .map(|field| support::text(&grant[field]))
        .collect();
        row.push(revoke_state);
        row
    });
    let mut rendered = support::table(
        &[
            "Grant",
            "Recipient",
            "Amount",
            "Remaining",
            "Reserved",
            "Held",
            "Reclaimed",
            "Status",
            "Activation",
            "Revoke state",
        ],
        rows,
    );
    let mut limits = vec![];
    for grant in grants {
        for limit in grant["service_limits"].as_array().into_iter().flatten() {
            let mut row = vec![support::text(&grant["id"])];
            row.extend(
                [
                    "key",
                    "label",
                    "service_slugs",
                    "limit",
                    "consumed",
                    "reserved",
                    "held",
                ]
                .iter()
                .map(|field| support::text(&limit[field])),
            );
            limits.push(row);
        }
    }
    if !limits.is_empty() {
        rendered.push_str("\nService limits\n");
        rendered.push_str(&support::table(
            &[
                "Grant", "Key", "Label", "Services", "Limit", "Consumed", "Reserved", "Held",
            ],
            limits,
        ));
    }
    rendered
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
    const REQUEST_ID: &str = "00000000-0000-4000-8000-000000000002";

    fn issue(server: &MockServer, yes: bool) -> AdminCreditsCommands {
        AdminCreditsCommands::Grant {
            command: CreditGrantCommands::Issue {
                user: "pilot@example.test".into(),
                amount: "5".into(),
                request_id: None,
                service: vec![ID.into()],
                all_services: false,
                expires_at: None,
                reason: "Pilot".into(),
                yes,
                auth: mock_auth(server.uri()),
            },
        }
    }

    fn retry(server: &MockServer, amount: &str, request_id: &str) -> AdminCreditsCommands {
        AdminCreditsCommands::Grant {
            command: CreditGrantCommands::Issue {
                user: ID.into(),
                amount: amount.into(),
                request_id: Some(request_id.into()),
                service: vec![],
                all_services: true,
                expires_at: None,
                reason: "Pilot".into(),
                yes: true,
                auth: mock_auth(server.uri()),
            },
        }
    }

    fn retry_body(amount: &str) -> Value {
        json!({
            "amount": amount,
            "request_id": REQUEST_ID,
            "target_kind": "selected_users",
            "target_user_ids": [ID],
            "all_services": true,
            "service_refs": [],
            "expires_at": null,
            "reason": "Pilot"
        })
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
    async fn grant_issue_generates_uuid_v4_resolves_email_and_scope_and_posts_once() {
        let server = MockServer::start().await;
        lookups(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/credits/grants"))
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
        let requests = server.received_requests().await.unwrap();
        let post = requests
            .iter()
            .find(|request| request.method == "POST")
            .unwrap();
        let body: Value = serde_json::from_slice(&post.body).unwrap();
        let request_id = body["request_id"].as_str().unwrap();
        assert_eq!(
            uuid::Uuid::parse_str(request_id).unwrap().get_version_num(),
            4
        );
        assert_eq!(
            body,
            json!({
                "amount": "5",
                "request_id": request_id,
                "target_kind": "selected_users",
                "target_user_ids": [ID],
                "all_services": false,
                "service_refs": [ID],
                "expires_at": null,
                "reason": "Pilot"
            })
        );
    }

    #[tokio::test]
    async fn grant_retry_preserves_identical_body_and_exact_decimal_amounts() {
        for (input, expected) in [
            ("5", "5"),
            ("5.000000000000", "5"),
            ("5.123456789012", "5.123456789012"),
            ("0.000000000001", "0.000000000001"),
            ("1000000", "1000000"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/api/v1/admin/credits/grants"))
                .and(body_json(retry_body(expected)))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"batch_id": ID})))
                .expect(2)
                .mount(&server)
                .await;
            for _ in 0..2 {
                run(retry(&server, input, REQUEST_ID)).await.unwrap();
            }
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests[0].body, requests[1].body);
        }
    }

    #[tokio::test]
    async fn grant_invalid_request_id_and_amount_are_rejected_without_any_request() {
        let server = MockServer::start().await;
        for amount in [
            "0",
            "-1",
            "1000000.000000000001",
            "0.0000000000001",
            "1.0000000000000",
            "1e2",
            "NaN",
            "",
            "1.2.3",
            "9999999999999999999999999999999999999999",
        ] {
            assert!(
                run(retry(&server, amount, REQUEST_ID)).await.is_err(),
                "{amount}"
            );
        }
        for id in ["not-a-uuid", "", "00000000-0000-4000-8000-00000000000z"] {
            let error = run(retry(&server, "1", id)).await.unwrap_err();
            assert!(format!("{error:#}").contains("--request-id must be a UUID"));
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[test]
    fn grant_request_receipt_is_flushed_in_table_and_json_modes() {
        #[derive(Default)]
        struct Output {
            bytes: Vec<u8>,
            flushed: bool,
        }
        impl Write for Output {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                self.flushed = true;
                Ok(())
            }
        }
        for format in [OutputFormat::Table, OutputFormat::Json] {
            let mut output = Output::default();
            announce_request(REQUEST_ID, format, &mut output).unwrap();
            assert!(output.flushed);
            if matches!(format, OutputFormat::Json) {
                let receipt: Value = serde_json::from_slice(&output.bytes).unwrap();
                assert_eq!(receipt, json!({"request_id": REQUEST_ID}));
            } else {
                assert_eq!(
                    output.bytes,
                    format!("Request ID: {REQUEST_ID}\n").as_bytes()
                );
            }
        }
    }

    #[tokio::test]
    async fn grant_retry_can_return_an_original_batch_after_its_expiry() {
        let server = MockServer::start().await;
        let mut body = retry_body("5");
        body["expires_at"] = json!("2020-01-01T00:00:00Z");
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/credits/grants"))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"batch_id": ID})))
            .expect(1)
            .mount(&server)
            .await;
        let mut command = retry(&server, "5", REQUEST_ID);
        let AdminCreditsCommands::Grant {
            command: CreditGrantCommands::Issue { expires_at, .. },
        } = &mut command
        else {
            unreachable!()
        };
        *expires_at = Some("2020-01-01T00:00:00Z".into());
        run(command).await.unwrap();
        let mut command = retry(&server, "5", REQUEST_ID);
        let AdminCreditsCommands::Grant {
            command:
                CreditGrantCommands::Issue {
                    expires_at,
                    request_id,
                    ..
                },
        } = &mut command
        else {
            unreachable!()
        };
        *request_id = None;
        *expires_at = Some("2020-01-01T00:00:00Z".into());
        let error = run(command).await.unwrap_err();
        assert_eq!(error.to_string(), "expires-at must be in the future");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
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
            let result = confirm_grant("5.000000000001", answer.as_bytes(), &mut output);
            let prompt = String::from_utf8(output).unwrap();
            assert!(prompt.contains("Issue 5.000000000001 credits to the selected recipient?"));
            assert!(prompt.contains("identical settings returns the original batch"));
            assert!(prompt.contains("new invocation without it creates a new batch [y/N]"));
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
