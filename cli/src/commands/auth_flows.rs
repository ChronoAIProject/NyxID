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
