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

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn whoami_fetches_user_json() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/users/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "u1", "email": "a@b.com", "display_name": "Alice",
                "role": "admin", "mfa_enabled": true, "email_verified": true
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut api = ApiClient::new(&server.uri(), "test-token".to_string()).unwrap();
        run(&mut api, OutputFormat::Json)
            .await
            .expect("whoami json should succeed");
    }

    #[tokio::test]
    async fn whoami_table_uses_role_fallback() {
        let server = MockServer::start().await;
        // No `role`/`is_admin`/`mfa_enabled` → exercises the fallback branches.
        Mock::given(method("GET"))
            .and(path("/api/v1/users/me"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "id": "u1", "email": "a@b.com" })),
            )
            .mount(&server)
            .await;

        let mut api = ApiClient::new(&server.uri(), "test-token".to_string()).unwrap();
        run(&mut api, OutputFormat::Table)
            .await
            .expect("whoami table should succeed");
    }

    #[tokio::test]
    async fn whoami_table_is_admin_true_fallback() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/users/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "u1", "email": "a@b.com", "is_admin": true, "mfa_enabled": true,
                "email_verified": true, "display_name": "Admin User"
            })))
            .mount(&server)
            .await;

        let mut api = ApiClient::new(&server.uri(), "test-token".to_string()).unwrap();
        run(&mut api, OutputFormat::Table)
            .await
            .expect("whoami admin fallback should succeed");
    }
}
