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
            let mut api = ApiClient::from_auth(&auth)?;

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
            let mut api = ApiClient::from_auth(&auth)?;
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
                        let short_id = if id.len() > 8 { &id[..8] } else { id };
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
            let mut api = ApiClient::from_auth(&auth)?;
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
            let mut api = ApiClient::from_auth(&auth)?;
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
                        let short_id = if id.len() > 8 { &id[..8] } else { id };
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
            let mut api = ApiClient::from_auth(&auth)?;
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
            let mut api = ApiClient::from_auth(&auth)?;
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
