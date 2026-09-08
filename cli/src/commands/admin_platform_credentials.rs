use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::fmt::Write;
use zeroize::Zeroizing;

use crate::{
    api::ApiClient,
    cli::{AdminPlatformCredentialsCommands, OutputFormat},
};

async fn descriptor(api: &mut ApiClient, provider: &str) -> Result<Value> {
    let rows: Vec<Value> = api.get("/admin/platform-credentials").await?;
    rows.into_iter()
        .find(|row| row["provider"].as_str() == Some(provider))
        .context("Unknown platform credential provider")
}

fn field<'a>(descriptor: &'a Value, name: &str) -> Result<&'a Value> {
    descriptor["fields"]
        .as_array()
        .and_then(|fields| {
            fields
                .iter()
                .find(|field| field["name"].as_str() == Some(name))
        })
        .context("Unknown platform credential field")
}

fn pair(value: &str) -> Result<(&str, &str)> {
    value.split_once('=').context("Expected NAME=VALUE")
}

fn print(row: &Value, output: OutputFormat) -> Result<()> {
    if matches!(output, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(row)?);
        return Ok(());
    }
    eprint!("{}", human_output(row)?);
    Ok(())
}

fn shared_provider(row: &Value) -> Option<&str> {
    (row["backing"]["type"] == "provider_oauth")
        .then(|| row["backing"]["provider_slug"].as_str())
        .flatten()
}

fn shared_clear_warning(provider: &str) -> String {
    format!(
        "These credentials are shared with the {provider} provider. Clearing them stops all of its OAuth connections and logins until credentials are restored."
    )
}

fn human_output(row: &Value) -> Result<String> {
    let mut output = String::new();
    writeln!(
        output,
        "Provider: {}",
        row["provider"].as_str().unwrap_or("-")
    )?;
    if let Some(provider) = shared_provider(row) {
        writeln!(output, "Shared with the {provider} provider.")?;
    }
    for field in row["fields"].as_array().into_iter().flatten() {
        writeln!(
            output,
            "{}: {}",
            field["label"].as_str().unwrap_or("Field"),
            if field["configured"] == true {
                "Configured"
            } else {
                "Not configured"
            }
        )?;
    }
    if let Some(url) = row["callback_url"].as_str() {
        writeln!(output, "Platform Callback URL: {url}")?;
    }
    if let Some(token) = row["webhook_verify_token"].as_str() {
        writeln!(output, "Platform Verify Token: {token}")?;
    }
    Ok(output)
}

pub async fn run(command: AdminPlatformCredentialsCommands) -> Result<()> {
    match command {
        AdminPlatformCredentialsCommands::Show { provider, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            print(&descriptor(&mut api, &provider).await?, auth.output)
        }
        AdminPlatformCredentialsCommands::Set {
            provider,
            mut fields,
            mut field_envs,
            app_id,
            embedded_signup_config_id,
            app_secret_env,
            regenerate_verify_token,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let descriptor = descriptor(&mut api, &provider).await?;
            if let Some(value) = app_id {
                fields.push(format!("app_id={value}"));
            }
            if let Some(value) = embedded_signup_config_id {
                fields.push(format!("embedded_signup_config_id={value}"));
            }
            if let Some(value) = app_secret_env {
                field_envs.push(format!("app_secret={value}"));
            }
            let mut values = serde_json::Map::new();
            for item in fields {
                let (name, value) = pair(&item)?;
                if field(&descriptor, name)?["secret"] == true {
                    bail!("Secret fields require --field-env NAME=ENV_VAR");
                }
                if values.insert(name.to_string(), json!(value)).is_some() {
                    bail!("A field was provided more than once");
                }
            }
            for item in field_envs {
                let (name, env) = pair(&item)?;
                field(&descriptor, name)?;
                let value = Zeroizing::new(
                    std::env::var(env).context("Credential environment variable is not set")?,
                );
                if value.trim().is_empty() {
                    bail!("Credential environment variable is empty");
                }
                if values
                    .insert(name.to_string(), json!(value.as_str()))
                    .is_some()
                {
                    bail!("A field was provided more than once");
                }
            }
            if values.is_empty() && !regenerate_verify_token {
                bail!("Provide at least one field or --regenerate-verify-token");
            }
            let result: Value = api.patch(&format!("/admin/platform-credentials/{}", urlencoding::encode(&provider)), &json!({ "fields": values, "regenerate_verify_token": regenerate_verify_token })).await?;
            print(&result, auth.output)
        }
        AdminPlatformCredentialsCommands::Clear {
            provider,
            fields,
            confirm_shared_provider,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let descriptor = descriptor(&mut api, &provider).await?;
            if let Some(shared) = shared_provider(&descriptor) {
                let warning = shared_clear_warning(shared);
                eprintln!("{warning}");
                if !confirm_shared_provider {
                    bail!("{warning} Pass --confirm-shared-provider to confirm.");
                }
            }
            let path = format!(
                "/admin/platform-credentials/{}",
                urlencoding::encode(&provider)
            );
            if fields.is_empty() {
                api.delete_empty(&path).await?;
            } else {
                let mut values = serde_json::Map::new();
                for name in fields {
                    field(&descriptor, &name)?;
                    values.insert(name, Value::Null);
                }
                let _: Value = api.patch(&path, &json!({ "fields": values })).await?;
            }
            if matches!(auth.output, OutputFormat::Json) {
                println!("{}", json!({ "ok": true }));
            } else {
                eprintln!("Platform credentials cleared.");
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{env_lock, mock_auth};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    async fn server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/platform-credentials"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!([{ "provider": "meta", "fields": [
                { "name": "app_id", "secret": false }, { "name": "app_secret", "secret": true }
            ] }]),
            ))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn platform_credentials_show_clear_and_delete_use_admin_routes() {
        let server = server().await;
        Mock::given(method("PATCH"))
            .and(path("/api/v1/admin/platform-credentials/meta"))
            .and(body_json(json!({ "fields": { "app_secret": null } })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/admin/platform-credentials/meta"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        run(AdminPlatformCredentialsCommands::Show {
            provider: "meta".into(),
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap();
        for fields in [vec!["app_secret".into()], vec![]] {
            run(AdminPlatformCredentialsCommands::Clear {
                provider: "meta".into(),
                fields,
                confirm_shared_provider: false,
                auth: mock_auth(server.uri()),
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn platform_credentials_set_reads_secret_env_and_rejects_raw_secret() {
        let _guard = env_lock().lock().unwrap();
        let server = server().await;
        let env = "NYXID_TEST_MANAGED_META_SECRET";
        unsafe {
            std::env::set_var(env, "test-secret");
        }
        Mock::given(method("PATCH")).and(path("/api/v1/admin/platform-credentials/meta"))
            .and(body_json(json!({ "fields": { "app_id": "111", "app_secret": "test-secret" }, "regenerate_verify_token": true })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "provider": "meta" }))).expect(1).mount(&server).await;
        let command = |fields, app_id, app_secret_env| AdminPlatformCredentialsCommands::Set {
            provider: "meta".into(),
            fields,
            field_envs: vec![],
            app_id,
            embedded_signup_config_id: None,
            app_secret_env,
            regenerate_verify_token: true,
            auth: mock_auth(server.uri()),
        };
        let result = run(command(vec![], Some("111".into()), Some(env.into()))).await;
        unsafe {
            std::env::remove_var(env);
        }
        result.unwrap();
        assert!(
            run(command(vec!["app_secret=raw-secret".into()], None, None))
                .await
                .unwrap_err()
                .to_string()
                .contains("Secret fields require")
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn x_show_and_set_use_the_same_multi_provider_commands() {
        let _guard = env_lock().lock().unwrap();
        let server = MockServer::start().await;
        let descriptor = json!({"provider": "x", "backing": {"type": "provider_oauth", "provider_slug": "twitter"},
            "fields": [{"name": "client_id", "secret": true}, {"name": "client_secret", "secret": true}]});
        assert!(
            human_output(&descriptor)
                .unwrap()
                .lines()
                .any(|line| line == "Shared with the twitter provider.")
        );
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/platform-credentials"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{"provider": "meta", "fields": []}, descriptor])),
            )
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("PATCH")).and(path("/api/v1/admin/platform-credentials/x"))
            .and(body_json(json!({"fields": {"client_id": "x-client", "client_secret": "x-secret"}, "regenerate_verify_token": false})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"provider": "x"}))).expect(1).mount(&server).await;
        run(AdminPlatformCredentialsCommands::Show {
            provider: "x".into(),
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap();
        unsafe {
            std::env::set_var("NYXID_TEST_X_CLIENT", "x-client");
            std::env::set_var("NYXID_TEST_X_SECRET", "x-secret");
        }
        let result = run(AdminPlatformCredentialsCommands::Set {
            provider: "x".into(),
            fields: vec![],
            field_envs: vec![
                "client_id=NYXID_TEST_X_CLIENT".into(),
                "client_secret=NYXID_TEST_X_SECRET".into(),
            ],
            app_id: None,
            embedded_signup_config_id: None,
            app_secret_env: None,
            regenerate_verify_token: false,
            auth: mock_auth(server.uri()),
        })
        .await;
        unsafe {
            std::env::remove_var("NYXID_TEST_X_CLIENT");
            std::env::remove_var("NYXID_TEST_X_SECRET");
        }
        result.unwrap();
    }

    #[tokio::test]
    async fn shared_provider_clears_require_explicit_confirmation_for_fields_and_provider() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/platform-credentials"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "provider": "x", "backing": {"type": "provider_oauth", "provider_slug": "twitter"},
                "fields": [{"name": "client_secret", "secret": true}]
            }])))
            .expect(4)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/api/v1/admin/platform-credentials/x"))
            .and(body_json(json!({"fields": {"client_secret": null}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/admin/platform-credentials/x"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        for fields in [vec!["client_secret".into()], vec![]] {
            let command = |confirmed| AdminPlatformCredentialsCommands::Clear {
                provider: "x".into(),
                fields: fields.clone(),
                confirm_shared_provider: confirmed,
                auth: mock_auth(server.uri()),
            };
            let error = run(command(false)).await.unwrap_err().to_string();
            assert!(error.contains(&shared_clear_warning("twitter")));
            assert!(error.contains("--confirm-shared-provider"));
            run(command(true)).await.unwrap();
        }
    }

    #[test]
    fn clear_accepts_shared_provider_confirmation_flag() {
        use clap::Parser;
        let cli = crate::cli::Cli::try_parse_from([
            "nyxid",
            "admin",
            "platform-credentials",
            "clear",
            "x",
            "--confirm-shared-provider",
        ]);
        assert!(cli.is_ok());
    }
}
