use anyhow::Result;
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::OutputFormat;

pub async fn run(api: &mut ApiClient, output: OutputFormat) -> Result<()> {
    let user: Value = api.get("/users/me").await?;

    match output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&user)?);
        }
        OutputFormat::Table => {
            let id = user["id"].as_str().unwrap_or("-");
            let email = user["email"].as_str().unwrap_or("-");
            let name = user["display_name"].as_str().unwrap_or("-");
            // Backend includes a derived `role` string ("admin" / "operator"
            // / "user"); fall back to the legacy `is_admin` flag for older
            // backends that haven't been redeployed yet.
            let role = user["role"].as_str().unwrap_or_else(|| {
                if user["is_admin"].as_bool().unwrap_or(false) {
                    "admin"
                } else {
                    "user"
                }
            });
            let mfa = if user["mfa_enabled"].as_bool().unwrap_or(false) {
                "enabled"
            } else {
                "disabled"
            };
            let verified = if user["email_verified"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                "no"
            };

            eprintln!("User ID:  {id}");
            eprintln!("Email:    {email}");
            eprintln!("Name:     {name}");
            eprintln!("Role:     {role}");
            eprintln!("MFA:      {mfa}");
            eprintln!("Verified: {verified}");
        }
    }

    Ok(())
}
