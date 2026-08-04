use anyhow::{Context, Result, bail};

use crate::api;
use crate::cli::{ForgotPasswordArgs, RegisterArgs, ResetPasswordArgs, VerifyEmailArgs};

pub async fn run_register(args: RegisterArgs) -> Result<()> {
    let password = if let Some(env_var) = &args.password_env {
        std::env::var(env_var).with_context(|| format!("Environment variable {env_var} not set"))?
    } else {
        rpassword::prompt_password("Password: ").map_err(|e| anyhow::anyhow!("{e}"))?
    };
    if password.is_empty() {
        bail!("Password is required");
    }
    let confirm = if args.password_env.is_some() {
        password.clone()
    } else {
        rpassword::prompt_password("Confirm password: ").map_err(|e| anyhow::anyhow!("{e}"))?
    };
    if password != confirm {
        bail!("Passwords do not match");
    }

    let mut body = serde_json::json!({
        "email": args.email,
        "password": password,
        // Normalize to the backend's canonical form so users don't get a
        // confusing "invalid code" when they type the code in lowercase.
        "invite_code": args.invite_code.trim().to_uppercase(),
    });
    if let Some(name) = &args.name {
        body["display_name"] = serde_json::Value::String(name.clone());
    }

    // Pre-auth flow: no profile has been selected yet, so consent is
    // resolved against the default profile's config.
    let result: serde_json::Value =
        api::anonymous_post(&args.base_url, "/auth/register", &body, None).await?;

    let msg = result["message"]
        .as_str()
        .unwrap_or("Registration successful. Check your email to verify your account.");
    eprintln!("{msg}");
    Ok(())
}

pub async fn run_verify_email(args: VerifyEmailArgs) -> Result<()> {
    let body = serde_json::json!({ "token": args.token });
    api::anonymous_post_empty(&args.base_url, "/auth/verify-email", &body, None).await?;
    eprintln!("Email verified successfully.");
    Ok(())
}

pub async fn run_forgot_password(args: ForgotPasswordArgs) -> Result<()> {
    let body = serde_json::json!({ "email": args.email });
    api::anonymous_post_empty(&args.base_url, "/auth/forgot-password", &body, None).await?;
    eprintln!(
        "If an account exists for {}, a password reset email has been sent.",
        args.email
    );
    Ok(())
}

pub async fn run_reset_password(args: ResetPasswordArgs) -> Result<()> {
    let password = if let Some(env_var) = &args.password_env {
        std::env::var(env_var).with_context(|| format!("Environment variable {env_var} not set"))?
    } else {
        rpassword::prompt_password("New password: ").map_err(|e| anyhow::anyhow!("{e}"))?
    };
    if password.is_empty() {
        bail!("Password is required");
    }
    let confirm = if args.password_env.is_some() {
        password.clone()
    } else {
        rpassword::prompt_password("Confirm new password: ").map_err(|e| anyhow::anyhow!("{e}"))?
    };
    if password != confirm {
        bail!("Passwords do not match");
    }

    let body = serde_json::json!({
        "token": args.token,
        "password": password,
    });
    api::anonymous_post_empty(&args.base_url, "/auth/reset-password", &body, None).await?;
    eprintln!("Password reset successfully. You can now log in with your new password.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn verify_email_posts_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/verify-email"))
            .and(body_json(serde_json::json!({ "token": "verif-tok" })))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        run_verify_email(VerifyEmailArgs {
            base_url: server.uri(),
            token: "verif-tok".to_string(),
        })
        .await
        .expect("verify-email should succeed");
    }

    #[tokio::test]
    async fn forgot_password_posts_email() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/forgot-password"))
            .and(body_json(serde_json::json!({ "email": "a@b.com" })))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        run_forgot_password(ForgotPasswordArgs {
            base_url: server.uri(),
            email: "a@b.com".to_string(),
        })
        .await
        .expect("forgot-password should succeed");
    }

    // register/reset read the password from an env var, so these
    // serialize env mutation with the shared env_lock (matches the
    // HOME-mutating tests in api.rs).
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn register_uppercases_invite_code_and_posts() {
        let _guard = crate::test_support::env_lock().lock().expect("env lock");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/register"))
            .and(body_json(serde_json::json!({
                "email": "new@user.com",
                "password": "s3cret-pw",
                "invite_code": "WELCOME"
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "message": "ok" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        // SAFETY: env mutation is serialized by env_lock above.
        unsafe {
            std::env::set_var("NYXID_TEST_REGISTER_PW", "s3cret-pw");
        }
        let result = run_register(RegisterArgs {
            base_url: server.uri(),
            email: "new@user.com".to_string(),
            name: None,
            password_env: Some("NYXID_TEST_REGISTER_PW".to_string()),
            invite_code: "welcome".to_string(),
        })
        .await;
        unsafe {
            std::env::remove_var("NYXID_TEST_REGISTER_PW");
        }
        result.expect("register should succeed");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn reset_password_posts_token_and_password() {
        let _guard = crate::test_support::env_lock().lock().expect("env lock");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/reset-password"))
            .and(body_json(serde_json::json!({
                "token": "reset-tok",
                "password": "new-pw-123"
            })))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        // SAFETY: env mutation is serialized by env_lock above.
        unsafe {
            std::env::set_var("NYXID_TEST_RESET_PW", "new-pw-123");
        }
        let result = run_reset_password(ResetPasswordArgs {
            base_url: server.uri(),
            token: "reset-tok".to_string(),
            password_env: Some("NYXID_TEST_RESET_PW".to_string()),
        })
        .await;
        unsafe {
            std::env::remove_var("NYXID_TEST_RESET_PW");
        }
        result.expect("reset-password should succeed");
    }
}
