use std::io::{BufRead, Write};

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{OutputFormat, ProfileCommands};

const ACCOUNT_DELETION_POLICY_VERSION: &str = "machine-data-provider-revocation-v2";
const ACCOUNT_DELETION_DISCLOSURE: &str = "Deleting your account queues deletion of all of your managed Machines and their data, including paused /workspace disks and temporary files. This is irreversible: no snapshots, no recovery or restore.\n\nNyxID revokes every connected provider authorization where the provider supports it, including your other connections sharing the same grant. Unsupported or unconfirmed revocations are reported in the deletion status rather than assumed complete.\n\nAccess to Machine work and job output is blocked immediately. Resource cleanup targets 24 hours and keeps cleanup pending until applicable control-plane stop or absence, Sandbox object-cascade checks, creation obligations and accounting are verified. Delays escalate to operators. Underlying storage follows its reclaim policy; physical storage release is not verified. The cleanup target is not a guaranteed erasure deadline.\n\nRaw job output is held only in memory for at most 1 hour after a job ends and may disappear sooner.\n\nSafe job metadata is retained for 30 days after the job ends, and operation history for 90 days after completion. Necessary deletion, retry, cleanup and settlement records remain until resource cleanup, billing and related retries finish. Audit and financial records follow the platform’s existing retention policy.";

fn erasure_preparing(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<crate::api::ApiError>()
        .is_some_and(|error| {
            error.status() == reqwest::StatusCode::CONFLICT
                && error
                    .response()
                    .is_some_and(|response| response.error_code == 12801)
        })
}

fn confirm_account_deletion(
    yes: bool,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<bool> {
    writeln!(output, "{ACCOUNT_DELETION_DISCLOSURE}")?;
    if yes {
        return Ok(true);
    }
    write!(output, "Permanently delete your account? [y/N] ")?;
    output.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    let confirmed = answer.trim().eq_ignore_ascii_case("y");
    if !confirmed {
        writeln!(output, "Cancelled.")?;
    }
    Ok(confirmed)
}

pub async fn run(command: ProfileCommands) -> Result<()> {
    match command {
        ProfileCommands::Update { name, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let mut body = serde_json::Map::new();

            if let Some(name) = name {
                body.insert("display_name".into(), Value::String(name));
            }

            if body.is_empty() {
                eprintln!("No updates specified. Use --name to update your display name.");
                return Ok(());
            }

            let result: Value = api.put("/users/me", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("Profile updated.");
                    if let Some(name) = result["display_name"].as_str() {
                        eprintln!("Name: {name}");
                    }
                }
            }
            Ok(())
        }

        ProfileCommands::Delete { yes, auth } => {
            if !confirm_account_deletion(
                yes,
                &mut std::io::stdin().lock(),
                &mut std::io::stderr().lock(),
            )? {
                return Ok(());
            }

            let deletion_generation = crate::auth::login_generation(auth.profile.as_deref());
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                "x-nyxid-account-deletion-policy",
                reqwest::header::HeaderValue::from_static(ACCOUNT_DELETION_POLICY_VERSION),
            );
            let mut attempt = 0_u32;
            let result: Value = loop {
                match api.delete_with_headers("/users/me", headers.clone()).await {
                    Ok(result) => break result,
                    Err(error) if erasure_preparing(&error) && attempt < 9 => {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            (500_u64 << attempt).min(5000),
                        ))
                        .await;
                        attempt += 1;
                    }
                    Err(error) if erasure_preparing(&error) => {
                        anyhow::bail!(
                            "Account deletion is still being prepared; try again in a minute."
                        );
                    }
                    Err(error) => return Err(error),
                }
            };
            let status = result["status"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Account deletion returned no durable status"))?;
            if !matches!(
                status,
                "accepted" | "pending" | "operator_action" | "complete"
            ) {
                anyhow::bail!("Account deletion returned an unsupported status");
            }
            if api
                .clear_deleted_account_credentials(deletion_generation.as_deref())
                .is_err()
            {
                // Remote acceptance is already durable. A local I/O failure
                // cannot turn it into a failed DELETE or justify clearing a
                // successor login. Keep the server result authoritative.
                eprintln!(
                    "Account deletion accepted; local credential cleanup could not be confirmed. Review this profile's saved login."
                );
            }
            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
                OutputFormat::Table => eprintln!(
                    "{}",
                    if status == "complete" {
                        "Account erasure complete."
                    } else {
                        "Account deletion accepted. Cleanup continues until verified."
                    }
                ),
            }
            Ok(())
        }

        ProfileCommands::Consents { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let consents: Value = api.get("/users/me/consents").await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&consents)?);
                }
                OutputFormat::Table => {
                    let items = consents
                        .get("consents")
                        .and_then(|v| v.as_array())
                        .or_else(|| consents.as_array());
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No OAuth consents.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header([
                            "Client ID",
                            "App Name",
                            "Scopes",
                            "Service Access",
                            "Granted",
                        ]);

                        for consent in items {
                            let client_id = consent["client_id"].as_str().unwrap_or("-");
                            let app_name = consent["client_name"].as_str().unwrap_or("-");
                            let scopes = consent["scopes"].as_str().unwrap_or("-");
                            let service_access = format_consent_service_access(consent);
                            let granted = consent["granted_at"].as_str().unwrap_or("-");
                            table.add_row([client_id, app_name, scopes, &service_access, granted]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        ProfileCommands::RevokeConsent {
            client_id,
            yes,
            auth,
        } => {
            if !yes {
                eprint!("Revoke consent for client {client_id}? [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.delete_empty(&format!("/users/me/consents/{client_id}"))
                .await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({"ok":true}))?
                ),
                OutputFormat::Table => eprintln!("Consent revoked for client {client_id}."),
            }
            Ok(())
        }
    }
}

fn format_consent_service_access(consent: &Value) -> String {
    if consent["legacy_unrestricted"].as_bool().unwrap_or(false) {
        return "legacy (unrestricted until next sign-in)".to_string();
    }
    if consent["allow_all_services"].as_bool().unwrap_or(false) {
        return "all services".to_string();
    }

    let labels = consent["allowed_services"]
        .as_array()
        .map(|services| {
            services
                .iter()
                .filter_map(|service| service["label"].as_str())
                .filter(|label| !label.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if labels.is_empty() {
        return "sign-in only".to_string();
    }

    let visible_count = labels.len().min(3);
    let mut summary = labels[..visible_count].join(", ");
    if labels.len() > visible_count {
        summary.push_str(&format!(" and {} more", labels.len() - visible_count));
    }
    format!("{} services: {summary}", labels.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_auth;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // `Update` maps the optional `--name` flag onto a `display_name` field
    // in the PUT body. Assert the exact request body so the field-name
    // mapping (name -> display_name) is locked in.
    #[tokio::test]
    async fn update_sends_name_as_display_name() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/users/me"))
            .and(body_json(serde_json::json!({ "display_name": "Alice" })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "display_name": "Alice" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ProfileCommands::Update {
            name: Some("Alice".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("profile update should succeed");
    }

    // With no fields to update the handler must short-circuit and make no
    // HTTP call at all. A mock with `.expect(0)` fails the test (on server
    // drop) if any request reaches it, proving the early return fired.
    #[tokio::test]
    async fn update_with_no_fields_makes_no_request() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/users/me"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        run(ProfileCommands::Update {
            name: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("empty update should be a no-op Ok");
    }

    // `--yes` bypasses the interactive confirmation prompt and issues the
    // account-deletion DELETE directly.
    #[tokio::test]
    async fn delete_with_yes_issues_delete() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/users/me"))
            .and(header("x-nyxid-account-deletion-policy", "machine-data-provider-revocation-v2"))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({"status":"accepted","accepted_at":"2026-01-01T00:00:00Z","completed_at":null,"domains":[]})))
            .expect(1)
            .mount(&server)
            .await;

        run(ProfileCommands::Delete {
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("account delete should succeed");
    }

    #[tokio::test]
    async fn deletion_retries_preparation_but_preserves_other_conflicts() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let server = MockServer::start().await;
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        Mock::given(method("DELETE")).and(path("/api/v1/users/me"))
            .and(header("x-nyxid-account-deletion-policy","machine-data-provider-revocation-v2"))
            .respond_with(move |_: &wiremock::Request| {
                match calls.fetch_add(1,Ordering::SeqCst) {
                    0=>ResponseTemplate::new(409).set_body_json(serde_json::json!({"error":"erasure_preparing","error_code":12801,"message":"Preparing"})),
                    _=>ResponseTemplate::new(409).set_body_json(serde_json::json!({"error":"conflict","error_code":1004,"message":"Other owners depend on your OAuth app"})),
                }
            }).expect(2).mount(&server).await;
        let error = run(ProfileCommands::Delete {
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<crate::api::ApiError>()
                .unwrap()
                .response()
                .unwrap()
                .error_code,
            1004
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn deletion_preparation_exhaustion_explains_when_to_retry() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE")).and(path("/api/v1/users/me"))
            .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({"error":"erasure_preparing","error_code":12801,"message":"Preparing"})))
            .expect(10).mount(&server).await;
        let error = run(ProfileCommands::Delete {
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Account deletion is still being prepared; try again in a minute."
        );
    }

    #[test]
    fn delete_confirmation_discloses_all_retained_machine_data_before_prompt() {
        let mut output = Vec::new();
        assert!(
            confirm_account_deletion(false, &mut std::io::Cursor::new(b"y\n"), &mut output)
                .unwrap()
        );
        let output = String::from_utf8(output).unwrap();
        let prompt = output.find("Permanently delete your account?").unwrap();
        for disclosure in [
            "every connected provider authorization",
            "your other connections sharing the same grant",
            "Unsupported or unconfirmed revocations are reported",
            "all of your managed Machines",
            "paused /workspace disks",
            "irreversible",
            "no snapshots, no recovery or restore",
            "cleanup pending until applicable control-plane stop or absence",
            "Sandbox object-cascade checks",
            "creation obligations and accounting are verified",
            "physical storage release is not verified",
            "not a guaranteed erasure deadline",
            "30 days after the job ends",
            "90 days after completion",
            "cleanup and settlement records remain until resource cleanup",
            "Audit and financial records",
        ] {
            assert!(
                output.find(disclosure).is_some_and(|index| index < prompt),
                "actual confirmation must disclose {disclosure} before accepting consent",
            );
        }
    }

    #[test]
    fn delete_confirmation_requires_yes_or_explicit_noninteractive_flag() {
        for answer in [b"".as_slice(), b"\n", b"n\n", b"yes\n"] {
            let mut output = Vec::new();
            assert!(
                !confirm_account_deletion(false, &mut std::io::Cursor::new(answer), &mut output)
                    .unwrap(),
                "default or unrecognized input must cancel",
            );
            assert!(String::from_utf8(output).unwrap().ends_with("Cancelled.\n"));
        }
        let mut output = Vec::new();
        assert!(
            confirm_account_deletion(true, &mut std::io::Cursor::new(b""), &mut output).unwrap()
        );
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("paused /workspace disks"));
        assert!(!output.contains("[y/N]"), "--yes skips only the prompt");
    }

    #[tokio::test]
    async fn delete_with_yes_preserves_policy_denial() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/users/me"))
            .and(header(
                "x-nyxid-account-deletion-policy",
                "machine-data-provider-revocation-v2",
            ))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "validation_error",
                "error_code": 1000,
                "message": "Account deletion policy is no longer current"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let error = run(ProfileCommands::Delete {
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect_err("a policy denial cannot be reported as successful deletion");
        let api_error = error
            .downcast_ref::<crate::api::ApiError>()
            .expect("retain the typed API denial");
        assert_eq!(api_error.status(), reqwest::StatusCode::BAD_REQUEST);
        assert_eq!(api_error.response().map(|body| body.error_code), Some(1000));
    }

    #[tokio::test]
    async fn consents_lists_via_get() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/users/me/consents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "consents": [
                    {"client_id": "c-1", "client_name": "App", "scopes": "openid",
                     "allow_all_services": false, "allowed_services": [],
                     "legacy_unrestricted": false,
                     "granted_at": "2026-01-01"}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ProfileCommands::Consents {
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("consents list should succeed");
    }

    // The client_id argument must be interpolated into the DELETE path.
    // The mock only matches the exact `/consents/client-1` path, so a
    // mismatched/missing interpolation fails the `.expect(1)` assertion.
    #[tokio::test]
    async fn revoke_consent_with_yes_targets_client_id_path() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/users/me/consents/client-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(ProfileCommands::RevokeConsent {
            client_id: "client-1".to_string(),
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("revoke consent should succeed");
    }

    // 5xx from the upstream must surface as an error, not be swallowed.
    #[tokio::test]
    async fn consents_surfaces_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/users/me/consents"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let result = run(ProfileCommands::Consents {
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "5xx should surface as an error");
    }

    #[test]
    fn consent_service_access_formats_all_services() {
        let consent = serde_json::json!({
            "allow_all_services": true,
            "allowed_services": [],
            "legacy_unrestricted": false
        });
        assert_eq!(format_consent_service_access(&consent), "all services");
    }

    #[test]
    fn consent_service_access_formats_explicit_services_with_deterministic_truncation() {
        let consent = serde_json::json!({
            "allow_all_services": false,
            "allowed_services": [
                {"label": "OpenAI"},
                {"label": "GitHub"},
                {"label": "Lark"},
                {"label": "Slack"}
            ],
            "legacy_unrestricted": false
        });
        assert_eq!(
            format_consent_service_access(&consent),
            "4 services: OpenAI, GitHub, Lark and 1 more"
        );
    }

    #[test]
    fn consent_service_access_formats_sign_in_only_for_explicit_empty_access() {
        let consent = serde_json::json!({
            "allow_all_services": false,
            "allowed_services": [],
            "legacy_unrestricted": false
        });
        assert_eq!(format_consent_service_access(&consent), "sign-in only");
    }

    #[test]
    fn consent_service_access_formats_legacy_unrestricted_first() {
        let consent = serde_json::json!({
            "allow_all_services": false,
            "allowed_services": [],
            "legacy_unrestricted": true
        });
        assert_eq!(
            format_consent_service_access(&consent),
            "legacy (unrestricted until next sign-in)"
        );
    }
}
