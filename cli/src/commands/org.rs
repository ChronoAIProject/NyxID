//! `nyxid org` -- manage organizations and shared credentials.
//!
//! Orgs in NyxID are special users with `user_type: "org"`. Resources owned
//! by an org (services, endpoints, API keys) can be shared with members,
//! and the proxy automatically falls back to org credentials when a
//! personal one is missing. See docs/ORG_MODEL_IMPLEMENTATION_PLAN.md.

use std::io::Write;

use anyhow::{Context, Result, anyhow};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{
    AuthArgs, OrgCommands, OrgInviteCommands, OrgMemberCommands, OrgRoleScopeCommands, OutputFormat,
};

pub async fn run(command: OrgCommands) -> Result<()> {
    match command {
        OrgCommands::Create {
            display_name,
            contact_email,
            avatar_url,
            auth,
        } => {
            create_org(
                &auth,
                &display_name,
                contact_email.as_deref(),
                avatar_url.as_deref(),
            )
            .await
        }

        OrgCommands::List { auth } => list_orgs(&auth).await,

        OrgCommands::Show { id, auth } => show_org(&auth, &id).await,

        OrgCommands::Update {
            id,
            display_name,
            slug,
            avatar_url,
            auth,
        } => {
            update_org(
                &auth,
                &id,
                display_name.as_deref(),
                slug.as_deref(),
                avatar_url.as_deref(),
            )
            .await
        }

        OrgCommands::Delete { id, yes, auth } => delete_org(&auth, &id, yes).await,

        OrgCommands::Join { nonce_or_url, auth } => join_org(&auth, &nonce_or_url).await,

        OrgCommands::SetPrimary {
            org_id,
            clear,
            auth,
        } => set_primary_org(&auth, org_id.as_deref(), clear).await,

        OrgCommands::Member { command } => run_member(command).await,

        OrgCommands::Invite { command } => run_invite(command).await,

        OrgCommands::RoleScope { command } => run_role_scope(command).await,
    }
}

async fn run_member(command: OrgMemberCommands) -> Result<()> {
    match command {
        OrgMemberCommands::List { org_id, auth } => list_members(&auth, &org_id).await,
        OrgMemberCommands::Add {
            org_id,
            user_id,
            role,
            scope_source,
            allowed_service_ids,
            auth,
        } => {
            add_member(
                &auth,
                &org_id,
                &user_id,
                &role,
                scope_source.as_deref(),
                allowed_service_ids.as_deref(),
            )
            .await
        }
        OrgMemberCommands::Update {
            org_id,
            member_id,
            role,
            scope_source,
            allowed_service_ids,
            auth,
        } => {
            update_member(
                &auth,
                &org_id,
                &member_id,
                role.as_deref(),
                scope_source.as_deref(),
                allowed_service_ids.as_deref(),
            )
            .await
        }
        OrgMemberCommands::Remove {
            org_id,
            member_id,
            yes,
            auth,
        } => remove_member(&auth, &org_id, &member_id, yes).await,
    }
}

async fn run_invite(command: OrgInviteCommands) -> Result<()> {
    match command {
        OrgInviteCommands::Create {
            org_id,
            role,
            scope_source,
            allowed_service_ids,
            ttl_hours,
            auth,
        } => {
            create_invite(
                &auth,
                &org_id,
                &role,
                scope_source.as_deref(),
                allowed_service_ids.as_deref(),
                ttl_hours,
            )
            .await
        }
        OrgInviteCommands::List { org_id, auth } => list_invites(&auth, &org_id).await,
        OrgInviteCommands::Cancel {
            org_id,
            invite_id,
            yes,
            auth,
        } => cancel_invite(&auth, &org_id, &invite_id, yes).await,
    }
}

async fn run_role_scope(command: OrgRoleScopeCommands) -> Result<()> {
    match command {
        OrgRoleScopeCommands::List { org_id, auth } => list_role_scopes(&auth, &org_id).await,
        OrgRoleScopeCommands::Set {
            org_id,
            role,
            allowed_service_ids,
            full_access,
            auth,
        } => {
            set_role_scope(
                &auth,
                &org_id,
                &role,
                allowed_service_ids.as_deref(),
                full_access,
            )
            .await
        }
        OrgRoleScopeCommands::Clear { org_id, role, auth } => {
            clear_role_scope(&auth, &org_id, &role).await
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Org CRUD
// ─────────────────────────────────────────────────────────────────────────────

async fn create_org(
    auth: &AuthArgs,
    display_name: &str,
    contact_email: Option<&str>,
    avatar_url: Option<&str>,
) -> Result<()> {
    let mut api = ApiClient::from_auth_checked(auth).await?;

    let mut body = serde_json::json!({ "display_name": display_name });
    if let Some(email) = contact_email
        && !email.is_empty()
    {
        body["contact_email"] = Value::String(email.to_string());
    }
    if let Some(url) = avatar_url
        && !url.is_empty()
    {
        body["avatar_url"] = Value::String(url.to_string());
    }

    let result: Value = api.post("/orgs", &body).await?;

    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
        OutputFormat::Table => {
            let id = result["id"].as_str().unwrap_or("-");
            let name = result["display_name"].as_str().unwrap_or("-");
            let role = result["your_role"].as_str().unwrap_or("-");
            eprintln!("Org created!");
            eprintln!();
            eprintln!("ID:        {id}");
            eprintln!("Name:      {name}");
            eprintln!("Your role: {role}");
            eprintln!();
            eprintln!("Next steps:");
            eprintln!("  nyxid org invite create {id} --role member");
            eprintln!("  nyxid org show {id}");
        }
    }
    Ok(())
}

async fn list_orgs(auth: &AuthArgs) -> Result<()> {
    let mut api = ApiClient::from_auth_checked(auth).await?;
    let resp: Value = api.get("/orgs").await?;

    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resp)?),
        OutputFormat::Table => {
            let items = resp.get("orgs").and_then(|v| v.as_array());
            let Some(items) = items else {
                eprintln!("No orgs.");
                return Ok(());
            };
            if items.is_empty() {
                eprintln!(
                    "No orgs. Create one with `nyxid org create --display-name \"My Team\"`."
                );
                return Ok(());
            }

            let mut table = Table::new();
            table.load_preset(UTF8_FULL_CONDENSED);
            table.set_header(["ID", "Slug", "Name", "Your role", "Created"]);
            for item in items {
                let id = item["id"].as_str().unwrap_or("-");
                let slug = item["slug"].as_str().unwrap_or("-");
                let name = item["display_name"].as_str().unwrap_or("-");
                let role = item["your_role"].as_str().unwrap_or("-");
                let created = item["created_at"].as_str().unwrap_or("-");
                table.add_row([id, slug, name, role, created]);
            }
            eprintln!("{table}");
        }
    }
    Ok(())
}

async fn show_org(auth: &AuthArgs, id: &str) -> Result<()> {
    let mut api = ApiClient::from_auth_checked(auth).await?;
    let org: Value = api.get(&format!("/orgs/{id}")).await?;

    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&org)?),
        OutputFormat::Table => {
            let org_id = org["id"].as_str().unwrap_or(id);
            let slug = org["slug"].as_str().unwrap_or("-");
            let name = org["display_name"].as_str().unwrap_or("-");
            let role = org["your_role"].as_str().unwrap_or("-");
            let count = org["member_count"].as_u64().unwrap_or(0);
            let created = org["created_at"].as_str().unwrap_or("-");

            eprintln!("ID:           {org_id}");
            eprintln!("Slug:         {slug}");
            eprintln!("Name:         {name}");
            eprintln!("Your role:    {role}");
            eprintln!("Members:      {count}");
            eprintln!("Created:      {created}");
            if let Some(avatar) = org["avatar_url"].as_str() {
                eprintln!("Avatar:       {avatar}");
            }
        }
    }
    Ok(())
}

async fn update_org(
    auth: &AuthArgs,
    id: &str,
    display_name: Option<&str>,
    slug: Option<&str>,
    avatar_url: Option<&str>,
) -> Result<()> {
    let mut api = ApiClient::from_auth_checked(auth).await?;
    let mut body = serde_json::Map::new();
    if let Some(name) = display_name {
        body.insert("display_name".into(), Value::String(name.to_string()));
    }
    if let Some(slug) = slug {
        body.insert("slug".into(), Value::String(slug.to_string()));
    }
    if let Some(url) = avatar_url {
        // Empty string is meaningful: clears the avatar.
        body.insert("avatar_url".into(), Value::String(url.to_string()));
    }
    if body.is_empty() {
        return Err(anyhow!(
            "Provide at least one of --display-name, --slug, or --avatar-url"
        ));
    }
    let updated: Value = api.patch(&format!("/orgs/{id}"), &body).await?;
    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&updated)?),
        OutputFormat::Table => eprintln!("Org updated."),
    }
    Ok(())
}

async fn delete_org(auth: &AuthArgs, id: &str, yes: bool) -> Result<()> {
    if !yes {
        eprint!("Delete org {id}? This is permanent. [y/N] ");
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            eprintln!("Cancelled.");
            return Ok(());
        }
    }

    let mut api = ApiClient::from_auth_checked(auth).await?;
    api.delete_empty(&format!("/orgs/{id}")).await?;
    match auth.output {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
        ),
        OutputFormat::Table => eprintln!("Org deleted."),
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Join + primary org
// ─────────────────────────────────────────────────────────────────────────────

/// Extract the invite nonce from either a raw nonce or a full join URL.
/// Examples:
///   "ORGINV-ABCDEF" -> "ORGINV-ABCDEF"
///   "https://app/orgs/join/ORGINV-ABCDEF" -> "ORGINV-ABCDEF"
fn parse_nonce(input: &str) -> &str {
    let trimmed = input.trim();
    if let Some(idx) = trimmed.rfind("/orgs/join/") {
        let after = &trimmed[idx + "/orgs/join/".len()..];
        // Strip any trailing query string or fragment.
        return after.split(['?', '#']).next().unwrap_or(after);
    }
    trimmed
}

async fn join_org(auth: &AuthArgs, nonce_or_url: &str) -> Result<()> {
    let nonce = parse_nonce(nonce_or_url);
    if nonce.is_empty() {
        return Err(anyhow!("Empty invite nonce"));
    }

    let mut api = ApiClient::from_auth_checked(auth).await?;
    let resp: Value = api
        .post(&format!("/orgs/join/{nonce}"), &serde_json::json!({}))
        .await
        .context("Failed to redeem invite")?;

    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resp)?),
        OutputFormat::Table => {
            let org_id = resp["org_id"].as_str().unwrap_or("-");
            let role = resp["role"].as_str().unwrap_or("-");
            eprintln!("Joined org!");
            eprintln!("ID:   {org_id}");
            eprintln!("Role: {role}");
            eprintln!();
            eprintln!("Run `nyxid org show {org_id}` for details.");
        }
    }
    Ok(())
}

async fn set_primary_org(auth: &AuthArgs, org_id: Option<&str>, clear: bool) -> Result<()> {
    let mut api = ApiClient::from_auth_checked(auth).await?;

    let body = if clear {
        serde_json::json!({ "primary_org_id": Value::Null })
    } else if let Some(id) = org_id {
        serde_json::json!({ "primary_org_id": id })
    } else {
        return Err(anyhow!("Pass --org-id <ID> to set, or --clear to unset"));
    };

    let updated: Value = api.patch("/users/me/primary-org", &body).await?;
    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&updated)?),
        OutputFormat::Table => {
            if clear {
                eprintln!("Primary org cleared.");
            } else if let Some(id) = org_id {
                eprintln!("Primary org set to {id}.");
            }
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Members
// ─────────────────────────────────────────────────────────────────────────────

async fn list_members(auth: &AuthArgs, org_id: &str) -> Result<()> {
    let mut api = ApiClient::from_auth_checked(auth).await?;
    let resp: Value = api.get(&format!("/orgs/{org_id}/members")).await?;

    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resp)?),
        OutputFormat::Table => {
            let items = resp.get("members").and_then(|v| v.as_array());
            let Some(items) = items else {
                eprintln!("No members.");
                return Ok(());
            };
            if items.is_empty() {
                eprintln!("No members.");
                return Ok(());
            }

            let mut table = Table::new();
            table.load_preset(UTF8_FULL_CONDENSED);
            table.set_header([
                "User ID",
                "Name / Email",
                "Role",
                "Mode",
                "Effective scope",
                "Joined",
            ]);
            for m in items {
                let user_id = m["user_id"].as_str().unwrap_or("-");
                let name = m["display_name"]
                    .as_str()
                    .or_else(|| m["email"].as_str())
                    .unwrap_or("-");
                let role = m["role"].as_str().unwrap_or("-");
                let mode = m["scope_source"].as_str().unwrap_or("override");
                let scope = match m["effective_allowed_service_ids"].as_array() {
                    Some(arr) if arr.is_empty() => "(none)".to_string(),
                    Some(arr) => format!("{} services", arr.len()),
                    None => "all".to_string(),
                };
                let joined = m["created_at"].as_str().unwrap_or("-");
                table.add_row([user_id, name, role, mode, &scope, joined]);
            }
            eprintln!("{table}");
        }
    }
    Ok(())
}

async fn add_member(
    auth: &AuthArgs,
    org_id: &str,
    user_id: &str,
    role: &str,
    scope_source: Option<&str>,
    allowed_service_ids: Option<&str>,
) -> Result<()> {
    validate_role(role)?;
    if let Some(src) = scope_source {
        validate_scope_source(src)?;
    }
    let mut api = ApiClient::from_auth_checked(auth).await?;

    let mut body = serde_json::json!({
        "user_id": user_id,
        "role": role,
    });
    if let Some(src) = scope_source {
        body["scope_source"] = Value::String(src.to_string());
    }
    if let Some(ids_csv) = allowed_service_ids {
        let ids: Vec<&str> = ids_csv
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        body["allowed_service_ids"] = serde_json::json!(ids);
    }

    let resp: Value = api.post(&format!("/orgs/{org_id}/members"), &body).await?;
    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resp)?),
        OutputFormat::Table => {
            eprintln!("Member added: {user_id} ({role})");
            eprintln!();
            eprintln!("Tip: prefer `nyxid org invite create` so the recipient explicitly opts in.");
        }
    }
    Ok(())
}

async fn update_member(
    auth: &AuthArgs,
    org_id: &str,
    member_id: &str,
    role: Option<&str>,
    scope_source: Option<&str>,
    allowed_service_ids: Option<&str>,
) -> Result<()> {
    if let Some(role) = role {
        validate_role(role)?;
    }
    if let Some(src) = scope_source {
        validate_scope_source(src)?;
    }
    if role.is_none() && scope_source.is_none() && allowed_service_ids.is_none() {
        return Err(anyhow!(
            "Provide at least one of --role, --scope-source, or --allowed-service-ids"
        ));
    }

    let mut api = ApiClient::from_auth_checked(auth).await?;
    let mut body = serde_json::Map::new();
    if let Some(role) = role {
        body.insert("role".into(), Value::String(role.to_string()));
    }
    if let Some(src) = scope_source {
        body.insert("scope_source".into(), Value::String(src.to_string()));
    }
    if let Some(ids_csv) = allowed_service_ids {
        // Empty string clears the scope (full access). Otherwise, parse a list.
        if ids_csv.is_empty() {
            body.insert("allowed_service_ids".into(), Value::Null);
        } else {
            let ids: Vec<&str> = ids_csv
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            body.insert("allowed_service_ids".into(), serde_json::json!(ids));
        }
    }

    let updated: Value = api
        .patch(&format!("/orgs/{org_id}/members/{member_id}"), &body)
        .await?;
    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&updated)?),
        OutputFormat::Table => eprintln!("Member updated."),
    }
    Ok(())
}

async fn remove_member(auth: &AuthArgs, org_id: &str, member_id: &str, yes: bool) -> Result<()> {
    if !yes {
        eprint!("Remove member {member_id} from org {org_id}? [y/N] ");
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            eprintln!("Cancelled.");
            return Ok(());
        }
    }

    let mut api = ApiClient::from_auth_checked(auth).await?;
    api.delete_empty(&format!("/orgs/{org_id}/members/{member_id}"))
        .await?;
    match auth.output {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
        ),
        OutputFormat::Table => eprintln!("Member removed."),
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Invites
// ─────────────────────────────────────────────────────────────────────────────

async fn create_invite(
    auth: &AuthArgs,
    org_id: &str,
    role: &str,
    scope_source: Option<&str>,
    allowed_service_ids: Option<&str>,
    ttl_hours: Option<i64>,
) -> Result<()> {
    validate_role(role)?;
    if let Some(src) = scope_source {
        validate_scope_source(src)?;
    }
    let mut api = ApiClient::from_auth_checked(auth).await?;

    let mut body = serde_json::json!({ "role": role });
    if let Some(src) = scope_source {
        body["scope_source"] = Value::String(src.to_string());
    }
    if let Some(ids_csv) = allowed_service_ids {
        let ids: Vec<&str> = ids_csv
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        body["allowed_service_ids"] = serde_json::json!(ids);
    }
    if let Some(hours) = ttl_hours {
        // Mirror the server-side bound (1..=720, see
        // `backend/src/handlers/orgs.rs::ORG_INVITE_MAX_TTL_HOURS`) so the
        // CLI fails fast with a clear message instead of a 400 round-trip.
        if !(1..=24 * 30).contains(&hours) {
            anyhow::bail!("--ttl-hours must be between 1 and 720 (30 days)");
        }
        body["ttl_hours"] = serde_json::json!(hours);
    }

    let invite: Value = api.post(&format!("/orgs/{org_id}/invites"), &body).await?;

    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&invite)?),
        OutputFormat::Table => {
            let nonce = invite["nonce"].as_str().unwrap_or("-");
            let id = invite["id"].as_str().unwrap_or("-");
            let expires = invite["expires_at"].as_str().unwrap_or("-");
            let join_url = build_join_url(auth, nonce);

            eprintln!("Invite created!");
            eprintln!();
            eprintln!("Invite ID: {id}");
            eprintln!("Role:      {role}");
            eprintln!("Expires:   {expires}");
            eprintln!();
            eprintln!("Share one of these with the recipient:");
            eprintln!("  Join link: {join_url}");
            eprintln!("  CLI:       nyxid org join {nonce}");
            eprintln!();
            eprintln!(
                "The recipient must already be a NyxID user. The link / nonce is single-use."
            );
        }
    }
    Ok(())
}

/// Build a redemption URL using the configured frontend origin if known,
/// otherwise fall back to the API base URL. Either form works because the
/// frontend route auto-redeems on load and the bare nonce works with
/// `nyxid org join` too.
fn build_join_url(auth: &AuthArgs, nonce: &str) -> String {
    match auth.resolved_base_url() {
        Ok(url) => format!("{}/orgs/join/{nonce}", url.trim_end_matches('/')),
        Err(_) => format!("/orgs/join/{nonce}"),
    }
}

async fn list_invites(auth: &AuthArgs, org_id: &str) -> Result<()> {
    let mut api = ApiClient::from_auth_checked(auth).await?;
    let resp: Value = api.get(&format!("/orgs/{org_id}/invites")).await?;

    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resp)?),
        OutputFormat::Table => {
            let items = resp.get("invites").and_then(|v| v.as_array());
            let Some(items) = items else {
                eprintln!("No invites.");
                return Ok(());
            };
            if items.is_empty() {
                eprintln!("No outstanding invites.");
                return Ok(());
            }

            let mut table = Table::new();
            table.load_preset(UTF8_FULL_CONDENSED);
            table.set_header(["Invite ID", "Role", "Status", "Used by", "Expires"]);
            let now = chrono::Utc::now();
            for inv in items {
                let id = inv["id"].as_str().unwrap_or("-");
                let role = inv["role"].as_str().unwrap_or("-");
                let expires = inv["expires_at"].as_str().unwrap_or("-");
                let status = if !inv["redeemed_at"].is_null() {
                    "redeemed"
                } else {
                    // Post-#407 the TTL index is gone, so expired rows
                    // stay visible. Surface them as a distinct status so
                    // admins can see them in the table instead of having
                    // to eyeball expires_at.
                    let expired = chrono::DateTime::parse_from_rfc3339(expires)
                        .map(|t| t.with_timezone(&chrono::Utc) <= now)
                        .unwrap_or(false);
                    if expired { "expired" } else { "pending" }
                };
                let used_by = inv["redeemed_by_email"]
                    .as_str()
                    .or_else(|| inv["redeemed_by_display_name"].as_str())
                    .or_else(|| inv["redeemed_by"].as_str())
                    .unwrap_or("-");
                table.add_row([id, role, status, used_by, expires]);
            }
            eprintln!("{table}");
        }
    }
    Ok(())
}

async fn cancel_invite(auth: &AuthArgs, org_id: &str, invite_id: &str, yes: bool) -> Result<()> {
    if !yes {
        eprint!("Cancel invite {invite_id}? [y/N] ");
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            eprintln!("Cancelled.");
            return Ok(());
        }
    }
    let mut api = ApiClient::from_auth_checked(auth).await?;
    api.delete_empty(&format!("/orgs/{org_id}/invites/{invite_id}"))
        .await?;
    match auth.output {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
        ),
        OutputFormat::Table => eprintln!("Invite cancelled."),
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Role scopes
// ─────────────────────────────────────────────────────────────────────────────

async fn list_role_scopes(auth: &AuthArgs, org_id: &str) -> Result<()> {
    let mut api = ApiClient::from_auth_checked(auth).await?;
    let resp: Value = api.get(&format!("/orgs/{org_id}/role-scopes")).await?;

    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resp)?),
        OutputFormat::Table => {
            let items = resp.get("role_scopes").and_then(|v| v.as_array());
            let Some(items) = items else {
                eprintln!("No role scopes.");
                return Ok(());
            };
            let mut table = Table::new();
            table.load_preset(UTF8_FULL_CONDENSED);
            table.set_header(["Role", "Mode", "Services", "Updated", "Updated by"]);
            for s in items {
                let role = s["role"].as_str().unwrap_or("-");
                let is_default = s["is_default"].as_bool().unwrap_or(false);
                let mode = if is_default { "default" } else { "configured" };
                let scope = match s["allowed_service_ids"].as_array() {
                    Some(arr) if arr.is_empty() => "(none)".to_string(),
                    Some(arr) => format!("{} services", arr.len()),
                    None => "all".to_string(),
                };
                let updated = s["updated_at"].as_str().unwrap_or("-");
                let updated_by = s["updated_by"].as_str().unwrap_or("-");
                table.add_row([role, mode, &scope, updated, updated_by]);
            }
            eprintln!("{table}");
            eprintln!();
            eprintln!(
                "Mode `default` = no row stored (full access). `configured` = scope pinned via `nyxid org role-scope set`."
            );
        }
    }
    Ok(())
}

async fn set_role_scope(
    auth: &AuthArgs,
    org_id: &str,
    role: &str,
    allowed_service_ids: Option<&str>,
    full_access: bool,
) -> Result<()> {
    validate_role(role)?;
    if !full_access && allowed_service_ids.is_none() {
        return Err(anyhow!(
            "Provide either --allowed-service-ids or --full-access"
        ));
    }

    let body = if full_access {
        serde_json::json!({ "allowed_service_ids": Value::Null })
    } else {
        let ids: Vec<&str> = allowed_service_ids
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        serde_json::json!({ "allowed_service_ids": ids })
    };

    let mut api = ApiClient::from_auth_checked(auth).await?;
    let updated: Value = api
        .put(&format!("/orgs/{org_id}/role-scopes/{role}"), &body)
        .await?;
    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&updated)?),
        OutputFormat::Table => {
            if full_access {
                eprintln!("Role scope set: {role} → full access");
            } else {
                let count = updated["allowed_service_ids"]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);
                eprintln!("Role scope set: {role} → {count} service(s)");
            }
        }
    }
    Ok(())
}

async fn clear_role_scope(auth: &AuthArgs, org_id: &str, role: &str) -> Result<()> {
    validate_role(role)?;
    let mut api = ApiClient::from_auth_checked(auth).await?;
    api.delete_empty(&format!("/orgs/{org_id}/role-scopes/{role}"))
        .await?;
    match auth.output {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
        ),
        OutputFormat::Table => eprintln!("Role scope cleared: {role} now defaults to full access."),
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn validate_role(role: &str) -> Result<()> {
    match role {
        "admin" | "member" | "viewer" => Ok(()),
        _ => Err(anyhow!(
            "Role must be one of: admin, member, viewer (got '{role}')"
        )),
    }
}

fn validate_scope_source(source: &str) -> Result<()> {
    match source {
        "inherit" | "override" => Ok(()),
        _ => Err(anyhow!(
            "Scope source must be one of: inherit, override (got '{source}')"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nonce_extracts_from_url() {
        assert_eq!(
            parse_nonce("https://nyx.example/orgs/join/ORGINV-ABC123"),
            "ORGINV-ABC123"
        );
    }

    #[test]
    fn parse_nonce_strips_query_string() {
        assert_eq!(
            parse_nonce("https://nyx.example/orgs/join/ORGINV-XYZ?source=email"),
            "ORGINV-XYZ"
        );
    }

    #[test]
    fn parse_nonce_passes_raw_nonce_through() {
        assert_eq!(parse_nonce("ORGINV-RAW"), "ORGINV-RAW");
    }

    #[test]
    fn parse_nonce_trims_whitespace() {
        assert_eq!(parse_nonce("  ORGINV-NONCE  "), "ORGINV-NONCE");
    }

    #[test]
    fn validate_role_accepts_valid() {
        assert!(validate_role("admin").is_ok());
        assert!(validate_role("member").is_ok());
        assert!(validate_role("viewer").is_ok());
    }

    #[test]
    fn validate_role_rejects_invalid() {
        assert!(validate_role("owner").is_err());
        assert!(validate_role("ADMIN").is_err());
        assert!(validate_role("").is_err());
    }

    #[test]
    fn validate_scope_source_accepts_valid() {
        assert!(validate_scope_source("inherit").is_ok());
        assert!(validate_scope_source("override").is_ok());
    }

    #[test]
    fn validate_scope_source_rejects_invalid() {
        assert!(validate_scope_source("custom").is_err());
        assert!(validate_scope_source("INHERIT").is_err());
        assert!(validate_scope_source("").is_err());
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;
    use crate::test_support::{mock_auth, mock_auth_with_output};
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // A UUID org id short-circuits `resolve_org_id` (no `/orgs/{slug}` lookup),
    // so command bodies hit the real endpoint with a single request.
    const ORG: &str = "11111111-1111-1111-1111-111111111111";

    // ── Org CRUD ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_posts_only_display_name_when_optionals_absent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/orgs"))
            .and(body_json(serde_json::json!({ "display_name": "Acme" })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "id": "org-1", "your_role": "admin" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Create {
            display_name: "Acme".to_string(),
            contact_email: None,
            avatar_url: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("create should succeed");
    }

    #[tokio::test]
    async fn create_includes_contact_email_and_avatar_when_present() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/orgs"))
            .and(body_json(serde_json::json!({
                "display_name": "Acme",
                "contact_email": "team@acme.test",
                "avatar_url": "https://acme.test/a.png",
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "org-1" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Create {
            display_name: "Acme".to_string(),
            contact_email: Some("team@acme.test".to_string()),
            avatar_url: Some("https://acme.test/a.png".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("create should succeed");
    }

    #[tokio::test]
    async fn create_omits_empty_optional_strings() {
        // Empty contact_email / avatar_url must NOT be sent (guarded by
        // `!email.is_empty()`); body stays display_name-only.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/orgs"))
            .and(body_json(serde_json::json!({ "display_name": "Acme" })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "org-1" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Create {
            display_name: "Acme".to_string(),
            contact_email: Some(String::new()),
            avatar_url: Some(String::new()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("create should succeed");
    }

    #[tokio::test]
    async fn list_gets_orgs() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/orgs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "orgs": [{ "id": "org-1", "slug": "acme", "display_name": "Acme",
                           "your_role": "admin", "created_at": "2026-01-01T00:00:00Z" }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::List {
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("list should succeed");
    }

    #[tokio::test]
    async fn show_gets_org_by_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/orgs/{ORG}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": ORG, "slug": "acme", "display_name": "Acme", "your_role": "admin",
                "member_count": 3, "created_at": "2026-01-01T00:00:00Z"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Show {
            id: ORG.to_string(),
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("show should succeed");
    }

    #[tokio::test]
    async fn show_surfaces_404_as_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/orgs/{ORG}")))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let result = run(OrgCommands::Show {
            id: ORG.to_string(),
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "404 should surface as an error");
    }

    #[tokio::test]
    async fn update_patches_only_provided_fields() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path(format!("/api/v1/orgs/{ORG}")))
            .and(body_json(
                serde_json::json!({ "display_name": "Renamed", "slug": "renamed" }),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": ORG })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Update {
            id: ORG.to_string(),
            display_name: Some("Renamed".to_string()),
            slug: Some("renamed".to_string()),
            avatar_url: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("update should succeed");
    }

    #[tokio::test]
    async fn update_sends_empty_avatar_to_clear_it() {
        // Empty avatar_url IS meaningful for update (clears it), unlike create.
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path(format!("/api/v1/orgs/{ORG}")))
            .and(body_json(serde_json::json!({ "avatar_url": "" })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": ORG })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Update {
            id: ORG.to_string(),
            display_name: None,
            slug: None,
            avatar_url: Some(String::new()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("update should succeed");
    }

    #[tokio::test]
    async fn update_with_no_fields_bails_without_request() {
        // No mocks mounted: any HTTP call would 404 → error. The bail must
        // happen before the request, so the only error is the validation one.
        let server = MockServer::start().await;
        let result = run(OrgCommands::Update {
            id: ORG.to_string(),
            display_name: None,
            slug: None,
            avatar_url: None,
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "empty update should bail");
        assert!(
            result.unwrap_err().to_string().contains("at least one"),
            "should be the validation message"
        );
    }

    #[tokio::test]
    async fn delete_with_yes_issues_delete() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/orgs/{ORG}")))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Delete {
            id: ORG.to_string(),
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("delete should succeed");
    }

    #[tokio::test]
    async fn delete_surfaces_conflict_error() {
        // Server refuses delete (org still owns resources) → 409.
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/orgs/{ORG}")))
            .respond_with(ResponseTemplate::new(409).set_body_string("org owns resources"))
            .mount(&server)
            .await;

        let result = run(OrgCommands::Delete {
            id: ORG.to_string(),
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "409 should surface as an error");
    }

    // ── Join + primary org ──────────────────────────────────────────────────

    #[tokio::test]
    async fn join_redeems_parsed_nonce_from_url() {
        // Verifies parse_nonce feeds the POST path: the URL form must collapse
        // to the bare nonce in /orgs/join/{nonce}.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/orgs/join/ORGINV-ABC123"))
            .and(body_json(serde_json::json!({})))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "org_id": "org-1", "role": "member" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Join {
            nonce_or_url: "https://nyx.example/orgs/join/ORGINV-ABC123".to_string(),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("join should succeed");
    }

    #[tokio::test]
    async fn join_empty_nonce_bails() {
        let server = MockServer::start().await;
        let result = run(OrgCommands::Join {
            nonce_or_url: "   ".to_string(),
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "empty nonce should bail");
    }

    #[tokio::test]
    async fn set_primary_sets_org_id() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/v1/users/me/primary-org"))
            .and(body_json(serde_json::json!({ "primary_org_id": ORG })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": 1 })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::SetPrimary {
            org_id: Some(ORG.to_string()),
            clear: false,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("set primary should succeed");
    }

    #[tokio::test]
    async fn set_primary_clear_sends_null() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/v1/users/me/primary-org"))
            .and(body_json(serde_json::json!({ "primary_org_id": null })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": 1 })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::SetPrimary {
            org_id: None,
            clear: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("clear primary should succeed");
    }

    #[tokio::test]
    async fn set_primary_without_args_bails() {
        let server = MockServer::start().await;
        let result = run(OrgCommands::SetPrimary {
            org_id: None,
            clear: false,
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "no org-id and no --clear should bail");
    }

    // ── Members ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn member_add_posts_user_and_role() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/orgs/{ORG}/members")))
            .and(body_json(
                serde_json::json!({ "user_id": "u-9", "role": "member" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": 1 })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Member {
            command: OrgMemberCommands::Add {
                org_id: ORG.to_string(),
                user_id: "u-9".to_string(),
                role: "member".to_string(),
                scope_source: None,
                allowed_service_ids: None,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("member add should succeed");
    }

    #[tokio::test]
    async fn member_add_parses_csv_into_service_id_array() {
        // CSV is split, trimmed, and empties dropped before send.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/orgs/{ORG}/members")))
            .and(body_json(serde_json::json!({
                "user_id": "u-9",
                "role": "admin",
                "scope_source": "override",
                "allowed_service_ids": ["svc-a", "svc-b", "svc-c"],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": 1 })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Member {
            command: OrgMemberCommands::Add {
                org_id: ORG.to_string(),
                user_id: "u-9".to_string(),
                role: "admin".to_string(),
                scope_source: Some("override".to_string()),
                allowed_service_ids: Some(" svc-a, svc-b ,, svc-c ".to_string()),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("member add with scope should succeed");
    }

    #[tokio::test]
    async fn member_add_rejects_invalid_role_before_request() {
        let server = MockServer::start().await;
        let result = run(OrgCommands::Member {
            command: OrgMemberCommands::Add {
                org_id: ORG.to_string(),
                user_id: "u-9".to_string(),
                role: "owner".to_string(),
                scope_source: None,
                allowed_service_ids: None,
                auth: mock_auth(server.uri()),
            },
        })
        .await;
        assert!(result.is_err(), "invalid role should bail before request");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Role must be one of")
        );
    }

    #[tokio::test]
    async fn member_add_rejects_invalid_scope_source() {
        let server = MockServer::start().await;
        let result = run(OrgCommands::Member {
            command: OrgMemberCommands::Add {
                org_id: ORG.to_string(),
                user_id: "u-9".to_string(),
                role: "member".to_string(),
                scope_source: Some("custom".to_string()),
                allowed_service_ids: None,
                auth: mock_auth(server.uri()),
            },
        })
        .await;
        assert!(result.is_err(), "invalid scope source should bail");
        assert!(result.unwrap_err().to_string().contains("Scope source"));
    }

    #[tokio::test]
    async fn member_list_gets_members() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/orgs/{ORG}/members")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "members": [{ "user_id": "u-1", "role": "admin", "scope_source": "inherit" }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Member {
            command: OrgMemberCommands::List {
                org_id: ORG.to_string(),
                auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
            },
        })
        .await
        .expect("member list should succeed");
    }

    #[tokio::test]
    async fn member_update_patches_role() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path(format!("/api/v1/orgs/{ORG}/members/u-9")))
            .and(body_json(serde_json::json!({ "role": "viewer" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": 1 })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Member {
            command: OrgMemberCommands::Update {
                org_id: ORG.to_string(),
                member_id: "u-9".to_string(),
                role: Some("viewer".to_string()),
                scope_source: None,
                allowed_service_ids: None,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("member update should succeed");
    }

    #[tokio::test]
    async fn member_update_empty_service_ids_clears_with_null() {
        // Empty --allowed-service-ids string maps to JSON null (clear scope),
        // a distinct branch from the CSV-parsing path.
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path(format!("/api/v1/orgs/{ORG}/members/u-9")))
            .and(body_json(
                serde_json::json!({ "allowed_service_ids": null }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": 1 })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Member {
            command: OrgMemberCommands::Update {
                org_id: ORG.to_string(),
                member_id: "u-9".to_string(),
                role: None,
                scope_source: None,
                allowed_service_ids: Some(String::new()),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("member update clear should succeed");
    }

    #[tokio::test]
    async fn member_update_with_no_fields_bails() {
        let server = MockServer::start().await;
        let result = run(OrgCommands::Member {
            command: OrgMemberCommands::Update {
                org_id: ORG.to_string(),
                member_id: "u-9".to_string(),
                role: None,
                scope_source: None,
                allowed_service_ids: None,
                auth: mock_auth(server.uri()),
            },
        })
        .await;
        assert!(result.is_err(), "empty member update should bail");
    }

    #[tokio::test]
    async fn member_remove_with_yes_deletes() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/orgs/{ORG}/members/u-9")))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Member {
            command: OrgMemberCommands::Remove {
                org_id: ORG.to_string(),
                member_id: "u-9".to_string(),
                yes: true,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("member remove should succeed");
    }

    // ── Invites ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn invite_create_posts_role_and_ttl() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/orgs/{ORG}/invites")))
            .and(body_json(
                serde_json::json!({ "role": "member", "ttl_hours": 48 }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "inv-1", "nonce": "ORGINV-XYZ", "expires_at": "2026-02-01T00:00:00Z"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Invite {
            command: OrgInviteCommands::Create {
                org_id: ORG.to_string(),
                role: "member".to_string(),
                scope_source: None,
                allowed_service_ids: None,
                ttl_hours: Some(48),
                auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
            },
        })
        .await
        .expect("invite create should succeed");
    }

    #[tokio::test]
    async fn invite_create_includes_scope_and_service_ids() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/orgs/{ORG}/invites")))
            .and(body_json(serde_json::json!({
                "role": "viewer",
                "scope_source": "override",
                "allowed_service_ids": ["svc-a", "svc-b"],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "inv-1", "nonce": "ORGINV-XYZ", "expires_at": "2026-02-01T00:00:00Z"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Invite {
            command: OrgInviteCommands::Create {
                org_id: ORG.to_string(),
                role: "viewer".to_string(),
                scope_source: Some("override".to_string()),
                allowed_service_ids: Some("svc-a, svc-b".to_string()),
                ttl_hours: None,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("invite create with scope should succeed");
    }

    #[tokio::test]
    async fn invite_create_rejects_out_of_range_ttl() {
        // ttl bound 1..=720 enforced client-side, before any request.
        let server = MockServer::start().await;
        let result = run(OrgCommands::Invite {
            command: OrgInviteCommands::Create {
                org_id: ORG.to_string(),
                role: "member".to_string(),
                scope_source: None,
                allowed_service_ids: None,
                ttl_hours: Some(721),
                auth: mock_auth(server.uri()),
            },
        })
        .await;
        assert!(result.is_err(), "ttl > 720 should bail");
        assert!(result.unwrap_err().to_string().contains("ttl-hours"));
    }

    #[tokio::test]
    async fn invite_create_rejects_invalid_role() {
        let server = MockServer::start().await;
        let result = run(OrgCommands::Invite {
            command: OrgInviteCommands::Create {
                org_id: ORG.to_string(),
                role: "superuser".to_string(),
                scope_source: None,
                allowed_service_ids: None,
                ttl_hours: None,
                auth: mock_auth(server.uri()),
            },
        })
        .await;
        assert!(result.is_err(), "invalid role should bail");
    }

    #[tokio::test]
    async fn invite_list_gets_invites() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/orgs/{ORG}/invites")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "invites": [{ "id": "inv-1", "role": "member",
                              "expires_at": "2099-01-01T00:00:00Z", "redeemed_at": null }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Invite {
            command: OrgInviteCommands::List {
                org_id: ORG.to_string(),
                auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
            },
        })
        .await
        .expect("invite list should succeed");
    }

    #[tokio::test]
    async fn invite_cancel_with_yes_deletes() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/orgs/{ORG}/invites/inv-1")))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::Invite {
            command: OrgInviteCommands::Cancel {
                org_id: ORG.to_string(),
                invite_id: "inv-1".to_string(),
                yes: true,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("invite cancel should succeed");
    }

    // ── Role scopes ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn role_scope_list_gets_scopes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/orgs/{ORG}/role-scopes")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "role_scopes": [{ "role": "member", "is_default": true,
                                  "allowed_service_ids": null }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::RoleScope {
            command: OrgRoleScopeCommands::List {
                org_id: ORG.to_string(),
                auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
            },
        })
        .await
        .expect("role-scope list should succeed");
    }

    #[tokio::test]
    async fn role_scope_set_with_service_ids_puts_array() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/orgs/{ORG}/role-scopes/member")))
            .and(body_json(
                serde_json::json!({ "allowed_service_ids": ["svc-a", "svc-b"] }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "role": "member", "allowed_service_ids": ["svc-a", "svc-b"]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::RoleScope {
            command: OrgRoleScopeCommands::Set {
                org_id: ORG.to_string(),
                role: "member".to_string(),
                allowed_service_ids: Some("svc-a, svc-b".to_string()),
                full_access: false,
                auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
            },
        })
        .await
        .expect("role-scope set should succeed");
    }

    #[tokio::test]
    async fn role_scope_set_full_access_puts_null() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/orgs/{ORG}/role-scopes/admin")))
            .and(body_json(
                serde_json::json!({ "allowed_service_ids": null }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "role": "admin", "allowed_service_ids": null
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::RoleScope {
            command: OrgRoleScopeCommands::Set {
                org_id: ORG.to_string(),
                role: "admin".to_string(),
                allowed_service_ids: None,
                full_access: true,
                auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
            },
        })
        .await
        .expect("role-scope full-access should succeed");
    }

    #[tokio::test]
    async fn role_scope_set_without_ids_or_full_access_bails() {
        let server = MockServer::start().await;
        let result = run(OrgCommands::RoleScope {
            command: OrgRoleScopeCommands::Set {
                org_id: ORG.to_string(),
                role: "member".to_string(),
                allowed_service_ids: None,
                full_access: false,
                auth: mock_auth(server.uri()),
            },
        })
        .await;
        assert!(result.is_err(), "set with neither flag should bail");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("--allowed-service-ids")
        );
    }

    #[tokio::test]
    async fn role_scope_set_rejects_invalid_role() {
        let server = MockServer::start().await;
        let result = run(OrgCommands::RoleScope {
            command: OrgRoleScopeCommands::Set {
                org_id: ORG.to_string(),
                role: "root".to_string(),
                allowed_service_ids: None,
                full_access: true,
                auth: mock_auth(server.uri()),
            },
        })
        .await;
        assert!(result.is_err(), "invalid role should bail before request");
    }

    #[tokio::test]
    async fn role_scope_clear_deletes_row() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/orgs/{ORG}/role-scopes/viewer")))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(OrgCommands::RoleScope {
            command: OrgRoleScopeCommands::Clear {
                org_id: ORG.to_string(),
                role: "viewer".to_string(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("role-scope clear should succeed");
    }

    #[tokio::test]
    async fn role_scope_clear_rejects_invalid_role() {
        let server = MockServer::start().await;
        let result = run(OrgCommands::RoleScope {
            command: OrgRoleScopeCommands::Clear {
                org_id: ORG.to_string(),
                role: "boss".to_string(),
                auth: mock_auth(server.uri()),
            },
        })
        .await;
        assert!(result.is_err(), "invalid role should bail before delete");
    }
}
