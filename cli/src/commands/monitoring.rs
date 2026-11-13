use anyhow::Result;
use serde_json::{Value, json};

use crate::{
    api::{ApiClient, ApiError},
    cli::{AuthArgs, MonitoringCommands, OutputFormat},
};

pub async fn run(command: MonitoringCommands) -> Result<()> {
    match command {
        MonitoringCommands::Diagnose { handle, auth } => diagnose(handle, auth).await,
    }
}

async fn diagnose(handle: String, auth: AuthArgs) -> Result<()> {
    let output = auth.output;
    let mut api = ApiClient::from_auth_checked(&auth).await?;
    let body = json!({"handle":handle});
    let result = match api
        .post::<Value, _>("/monitoring/invocation-diagnostics", &body)
        .await
    {
        Err(error) if diagnostic_not_ready(&error) => {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            api.post("/monitoring/invocation-diagnostics", &body)
                .await?
        }
        result => result?,
    };
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
        OutputFormat::Table => {
            println!(
                "Observation: {}\nAssessment:  {}\nOutcome:     {}",
                result["observation"].as_str().unwrap_or("unknown"),
                result["assessment"].as_str().unwrap_or("unknown"),
                result["outcome"].as_str().unwrap_or("unknown"),
            );
        }
    }
    Ok(())
}

fn diagnostic_not_ready(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ApiError>().is_some_and(|error| {
        error.status() == reqwest::StatusCode::CONFLICT
            && error.body_json().and_then(|body| body["error"].as_str())
                == Some("diagnostic_not_ready")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_not_ready_conflicts_are_retryable() {
        let retryable = anyhow::Error::new(ApiError::new(
            "POST /monitoring/invocation-diagnostics",
            reqwest::StatusCode::CONFLICT,
            r#"{"error":"diagnostic_not_ready"}"#.into(),
        ));
        assert!(diagnostic_not_ready(&retryable));
        let other = anyhow::Error::new(ApiError::new(
            "POST /monitoring/invocation-diagnostics",
            reqwest::StatusCode::CONFLICT,
            r#"{"error":"conflict"}"#.into(),
        ));
        assert!(!diagnostic_not_ready(&other));
    }
}
