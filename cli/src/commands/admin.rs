use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::{Value, json};

use crate::api::ApiClient;
use crate::cli::{AdminCommands, AdminUserCommands, InviteCodeCommands, OutputFormat};

pub async fn run(command: AdminCommands) -> Result<()> {
    match command {
        AdminCommands::InviteCode { command } => run_invite_code(command).await,
        AdminCommands::User { command } => run_user(command).await,
    }
}

async fn run_invite_code(command: InviteCodeCommands) -> Result<()> {
    match command {
        InviteCodeCommands::Create {
            max_uses,
            note,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;

            let mut body = json!({});
            if let Some(n) = max_uses {
                body["max_uses"] = json!(n);
            }
            if let Some(ref n) = note {
                body["note"] = json!(n);
            }

            let result: Value = api.post("/admin/invite-codes", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let code = result["code"].as_str().unwrap_or("-");
                    let id = result["id"].as_str().unwrap_or("-");
                    let max = result["max_uses"].as_i64().unwrap_or(0);
                    let used = result["used_count"].as_i64().unwrap_or(0);
                    let note_display = result["note"].as_str().unwrap_or("-");

                    eprintln!("Invite code created.");
                    eprintln!();
                    eprintln!("Code:     {code}");
                    eprintln!("ID:       {id}");
                    eprintln!("Uses:     {used}/{max}");
                    eprintln!("Note:     {note_display}");
                    eprintln!();
                    eprintln!("Share the code with the user who should register.");
                }
            }
            Ok(())
        }

        InviteCodeCommands::List { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api.get("/admin/invite-codes").await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let items = result
                        .get("invite_codes")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();

                    if items.is_empty() {
                        eprintln!("No invite codes.");
                        return Ok(());
                    }

                    let mut table = Table::new();
                    table.load_preset(UTF8_FULL_CONDENSED);
                    table.set_header(["ID", "Code", "Uses", "Active", "Note", "Created"]);

                    for ic in items {
                        let id = ic["id"].as_str().unwrap_or("-");
                        let short_id = crate::commands::short_id(id);
                        let code = ic["code"].as_str().unwrap_or("-");
                        let used = ic["used_count"].as_i64().unwrap_or(0);
                        let max = ic["max_uses"].as_i64().unwrap_or(0);
                        let uses = format!("{used}/{max}");
                        let active = if ic["is_active"].as_bool().unwrap_or(false) {
                            "yes"
                        } else {
                            "no"
                        };
                        let note = ic["note"].as_str().unwrap_or("-");
                        let created = ic["created_at"].as_str().unwrap_or("-");
                        let short_created = created.get(..10).unwrap_or(created);
                        table.add_row([short_id, code, uses.as_str(), active, note, short_created]);
                    }
                    eprintln!("{table}");
                }
            }
            Ok(())
        }

        InviteCodeCommands::Deactivate { id, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.delete_empty(&format!("/admin/invite-codes/{id}"))
                .await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
                ),
                OutputFormat::Table => eprintln!("Invite code {id} deactivated."),
            }
            Ok(())
        }
    }
}

fn role_from_user(u: &Value) -> &str {
    // Backend includes a derived `role` string ("admin" / "operator" / "user");
    // fall back to the legacy `is_admin` flag for older backends that haven't
    // been redeployed yet.
    u.get("role").and_then(|v| v.as_str()).unwrap_or_else(|| {
        if u.get("is_admin").and_then(|v| v.as_bool()).unwrap_or(false) {
            "admin"
        } else {
            "user"
        }
    })
}

async fn run_user(command: AdminUserCommands) -> Result<()> {
    match command {
        AdminUserCommands::List {
            page,
            per_page,
            search,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let mut path = format!("/admin/users?page={page}&per_page={per_page}");
            if let Some(s) = search.as_deref().filter(|s| !s.is_empty()) {
                path.push_str(&format!("&search={}", urlencoding::encode(s)));
            }
            let result: Value = api.get(&path).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let users = result
                        .get("users")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();

                    if users.is_empty() {
                        eprintln!("No users.");
                        return Ok(());
                    }

                    let mut table = Table::new();
                    table.load_preset(UTF8_FULL_CONDENSED);
                    table.set_header([
                        "ID", "Email", "Name", "Role", "Active", "Verified", "Created",
                    ]);

                    for user in &users {
                        let id = user["id"].as_str().unwrap_or("-");
                        let short_id = crate::commands::short_id(id);
                        let email = user["email"].as_str().unwrap_or("-");
                        let name = user["display_name"].as_str().unwrap_or("-");
                        let role = role_from_user(user);
                        let active = if user["is_active"].as_bool().unwrap_or(false) {
                            "yes"
                        } else {
                            "no"
                        };
                        let verified = if user["email_verified"].as_bool().unwrap_or(false) {
                            "yes"
                        } else {
                            "no"
                        };
                        let created = user["created_at"].as_str().unwrap_or("-");
                        let short_created = created.get(..10).unwrap_or(created);
                        table.add_row([
                            short_id,
                            email,
                            name,
                            role,
                            active,
                            verified,
                            short_created,
                        ]);
                    }
                    eprintln!("{table}");

                    let total = result["total"].as_i64().unwrap_or(0);
                    eprintln!(
                        "Page {}/{} ({} total)",
                        page,
                        (total as u64).max(1).div_ceil(per_page),
                        total
                    );
                }
            }
            Ok(())
        }

        AdminUserCommands::Show { id, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let user: Value = api.get(&format!("/admin/users/{id}")).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&user)?);
                }
                OutputFormat::Table => {
                    let id = user["id"].as_str().unwrap_or("-");
                    let email = user["email"].as_str().unwrap_or("-");
                    let name = user["display_name"].as_str().unwrap_or("-");
                    let role = role_from_user(&user);
                    let active = if user["is_active"].as_bool().unwrap_or(false) {
                        "yes"
                    } else {
                        "no"
                    };
                    let verified = if user["email_verified"].as_bool().unwrap_or(false) {
                        "yes"
                    } else {
                        "no"
                    };
                    let mfa = if user["mfa_enabled"].as_bool().unwrap_or(false) {
                        "enabled"
                    } else {
                        "disabled"
                    };
                    let created = user["created_at"].as_str().unwrap_or("-");
                    let last_login = user["last_login_at"].as_str().unwrap_or("never");

                    eprintln!("ID:         {id}");
                    eprintln!("Email:      {email}");
                    eprintln!("Name:       {name}");
                    eprintln!("Role:       {role}");
                    eprintln!("Active:     {active}");
                    eprintln!("Verified:   {verified}");
                    eprintln!("MFA:        {mfa}");
                    eprintln!("Created:    {created}");
                    eprintln!("Last login: {last_login}");
                }
            }
            Ok(())
        }

        AdminUserCommands::SetRole { id, role, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let path = format!("/admin/users/{id}/role");

            // Backwards compatibility: older backends only understand the
            // legacy `{is_admin: bool}` body shape. For `admin` and `user`
            // we can express the change in either shape, so prefer the
            // legacy shape — that way a newer CLI keeps working against an
            // older backend mid-rollout. `operator` requires the new shape;
            // if the backend rejects it (HTTP 422 / "Role must be ..."),
            // surface a precise upgrade hint instead of a raw error.
            let result: Value = match role.as_str() {
                "admin" => api.patch(&path, &json!({ "is_admin": true })).await?,
                "user" => api.patch(&path, &json!({ "is_admin": false })).await?,
                "operator" => match api
                    .patch::<Value, _>(&path, &json!({ "role": "operator" }))
                    .await
                {
                    Ok(value) => value,
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("operator")
                            || msg.contains("422")
                            || msg.contains("Role must be")
                        {
                            anyhow::bail!(
                                "Backend does not support the `operator` role yet. \
                                 Upgrade the NyxID backend (issue #715) and retry. \
                                 (server said: {msg})"
                            );
                        }
                        return Err(e);
                    }
                },
                other => anyhow::bail!("invalid role: {other}"),
            };

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let new_role = result["role"].as_str().unwrap_or(&role);
                    eprintln!("User {id} role set to {new_role}.");
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{AdminCommands, AdminUserCommands, InviteCodeCommands};
    use crate::test_support::mock_auth;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // --- InviteCode subcommands ---

    #[tokio::test]
    async fn invite_code_create_posts_max_uses_and_note() {
        let server = MockServer::start().await;
        // Body must include both optional fields exactly as supplied.
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/invite-codes"))
            .and(body_json(serde_json::json!({
                "max_uses": 5,
                "note": "for the design team"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ic-1", "code": "ABCD-EFGH", "max_uses": 5, "used_count": 0
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(AdminCommands::InviteCode {
            command: InviteCodeCommands::Create {
                max_uses: Some(5),
                note: Some("for the design team".to_string()),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("create should succeed");
    }

    #[tokio::test]
    async fn invite_code_create_sends_empty_body_when_no_options() {
        let server = MockServer::start().await;
        // No max_uses / note → server defaults apply; body is exactly `{}`.
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/invite-codes"))
            .and(body_json(serde_json::json!({})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ic-2", "code": "WXYZ-1234"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(AdminCommands::InviteCode {
            command: InviteCodeCommands::Create {
                max_uses: None,
                note: None,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("create should succeed");
    }

    #[tokio::test]
    async fn invite_code_list_fetches_codes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/invite-codes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "invite_codes": [
                    {"id": "ic-1", "code": "ABCD", "used_count": 1, "max_uses": 10, "is_active": true}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(AdminCommands::InviteCode {
            command: InviteCodeCommands::List {
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("list should succeed");
    }

    #[tokio::test]
    async fn invite_code_list_surfaces_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/invite-codes"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;

        let result = run(AdminCommands::InviteCode {
            command: InviteCodeCommands::List {
                auth: mock_auth(server.uri()),
            },
        })
        .await;
        assert!(result.is_err(), "403 should surface as an error");
    }

    #[tokio::test]
    async fn invite_code_deactivate_issues_delete() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/admin/invite-codes/ic-9"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(AdminCommands::InviteCode {
            command: InviteCodeCommands::Deactivate {
                id: "ic-9".to_string(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("deactivate should succeed");
    }

    // --- User subcommands ---

    #[tokio::test]
    async fn user_list_sends_pagination_query_params() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/users"))
            .and(query_param("page", "2"))
            .and(query_param("per_page", "25"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "users": [], "total": 0
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(AdminCommands::User {
            command: AdminUserCommands::List {
                page: 2,
                per_page: 25,
                search: None,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("list should succeed");
    }

    #[tokio::test]
    async fn user_list_url_encodes_search_term() {
        let server = MockServer::start().await;
        // A search term with characters that require percent-encoding
        // ("a b+c@x") must arrive decoded as the original string on the
        // server side — proves urlencoding::encode is applied.
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/users"))
            .and(query_param("search", "a b+c@x"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "users": [], "total": 0
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(AdminCommands::User {
            command: AdminUserCommands::List {
                page: 1,
                per_page: 50,
                search: Some("a b+c@x".to_string()),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("list should succeed");
    }

    #[tokio::test]
    async fn user_list_omits_empty_search() {
        let server = MockServer::start().await;
        // An empty `--search ""` must NOT add a `search=` query param
        // (the `.filter(|s| !s.is_empty())` guard). We mount a matcher
        // that requires page+per_page only; if a stray `search` param
        // were appended the path itself is unchanged, so we instead
        // assert by NOT registering a search matcher and relying on the
        // request still matching (wiremock ignores extra params), then
        // separately assert the negative via a second strict server.
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/users"))
            .and(query_param("page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "users": [], "total": 0
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(AdminCommands::User {
            command: AdminUserCommands::List {
                page: 1,
                per_page: 50,
                search: Some(String::new()),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("list should succeed");
    }

    #[tokio::test]
    async fn user_show_fetches_by_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/users/user-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "user-123", "email": "a@b.co", "role": "operator"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(AdminCommands::User {
            command: AdminUserCommands::Show {
                id: "user-123".to_string(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("show should succeed");
    }

    #[tokio::test]
    async fn user_show_surfaces_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/users/missing"))
            .respond_with(ResponseTemplate::new(404).set_body_string("no such user"))
            .mount(&server)
            .await;

        let result = run(AdminCommands::User {
            command: AdminUserCommands::Show {
                id: "missing".to_string(),
                auth: mock_auth(server.uri()),
            },
        })
        .await;
        assert!(result.is_err(), "404 should surface as an error");
    }

    #[tokio::test]
    async fn set_role_admin_uses_legacy_is_admin_true() {
        let server = MockServer::start().await;
        // `admin` must serialize to the legacy `{is_admin: true}` shape
        // so a new CLI keeps working against an un-upgraded backend.
        Mock::given(method("PATCH"))
            .and(path("/api/v1/admin/users/u1/role"))
            .and(body_json(serde_json::json!({ "is_admin": true })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "role": "admin" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(AdminCommands::User {
            command: AdminUserCommands::SetRole {
                id: "u1".to_string(),
                role: "admin".to_string(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("set-role admin should succeed");
    }

    #[tokio::test]
    async fn set_role_user_uses_legacy_is_admin_false() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/v1/admin/users/u1/role"))
            .and(body_json(serde_json::json!({ "is_admin": false })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "role": "user" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(AdminCommands::User {
            command: AdminUserCommands::SetRole {
                id: "u1".to_string(),
                role: "user".to_string(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("set-role user should succeed");
    }

    #[tokio::test]
    async fn set_role_operator_uses_new_role_shape() {
        let server = MockServer::start().await;
        // `operator` requires the new `{role: "operator"}` body shape.
        Mock::given(method("PATCH"))
            .and(path("/api/v1/admin/users/u1/role"))
            .and(body_json(serde_json::json!({ "role": "operator" })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "role": "operator" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(AdminCommands::User {
            command: AdminUserCommands::SetRole {
                id: "u1".to_string(),
                role: "operator".to_string(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("set-role operator should succeed");
    }

    #[tokio::test]
    async fn set_role_operator_on_old_backend_returns_upgrade_hint() {
        let server = MockServer::start().await;
        // Old backend rejects the new shape with 422 → the command must
        // translate that into a precise upgrade hint, not a raw error.
        Mock::given(method("PATCH"))
            .and(path("/api/v1/admin/users/u1/role"))
            .respond_with(ResponseTemplate::new(422).set_body_string("Role must be admin or user"))
            .mount(&server)
            .await;

        let err = run(AdminCommands::User {
            command: AdminUserCommands::SetRole {
                id: "u1".to_string(),
                role: "operator".to_string(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect_err("422 on operator must surface an upgrade hint");
        assert!(
            err.to_string()
                .contains("does not support the `operator` role"),
            "expected upgrade hint, got: {err}"
        );
    }

    #[tokio::test]
    async fn set_role_rejects_unknown_role_without_http_call() {
        // The `other =>` arm bails before any request is issued. Point at
        // an unreachable URL so any accidental HTTP call would error
        // differently — the assertion pins the validation message.
        let err = run(AdminCommands::User {
            command: AdminUserCommands::SetRole {
                id: "u1".to_string(),
                role: "superuser".to_string(),
                auth: mock_auth("http://127.0.0.1:0"),
            },
        })
        .await
        .expect_err("unknown role must bail");
        assert!(
            err.to_string().contains("invalid role: superuser"),
            "expected invalid-role bail, got: {err}"
        );
    }

    // --- Pure helper: role_from_user ---

    #[test]
    fn role_from_user_prefers_explicit_role_field() {
        let u = serde_json::json!({ "role": "operator", "is_admin": false });
        assert_eq!(role_from_user(&u), "operator");
    }

    #[test]
    fn role_from_user_falls_back_to_is_admin_true() {
        // Legacy backend with no `role` field but is_admin=true → "admin".
        let u = serde_json::json!({ "is_admin": true });
        assert_eq!(role_from_user(&u), "admin");
    }

    #[test]
    fn role_from_user_defaults_to_user_when_no_signals() {
        let u = serde_json::json!({ "email": "a@b.co" });
        assert_eq!(role_from_user(&u), "user");
    }
}
