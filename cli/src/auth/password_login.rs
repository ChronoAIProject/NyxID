//! Bounded password/MFA exchange. First-factor challenges remain private to this
//! continuation; server response bodies never become CLI error diagnostics.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const MAX_RESPONSE: usize = 64 * 1024;
#[derive(Deserialize)]
pub(super) struct PasswordTokens {
    pub access_token: Zeroizing<String>,
    pub refresh_token: Option<Zeroizing<String>>,
}
#[derive(Deserialize)]
struct Challenge {
    error_code: u32,
    session_token: Zeroizing<String>,
}
#[derive(Serialize)]
struct PasswordRequest<'a> {
    email: &'a str,
    password: &'a str,
    client: &'static str,
}
#[derive(Serialize)]
struct MfaRequest<'a> {
    code: &'a str,
    mfa_token: &'a str,
    client: &'static str,
}
async fn body(mut response: reqwest::Response) -> Result<Zeroizing<Vec<u8>>> {
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RESPONSE as u64)
    {
        bail!("Login response exceeded its size limit");
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("Login response could not be read"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE {
            bail!("Login response exceeded its size limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn tokens(bytes: &[u8]) -> Result<PasswordTokens> {
    let tokens: PasswordTokens =
        serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("Invalid login response"))?;
    if tokens.access_token.is_empty()
        || tokens.access_token.len() > 32 * 1024
        || tokens
            .refresh_token
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 32 * 1024)
    {
        bail!("Invalid login response");
    }
    Ok(tokens)
}
fn valid_code(code: &str) -> bool {
    (code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()))
        || (code.len() == 8
            && code
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
        || (code.len() == 32 && code.bytes().all(|b| b.is_ascii_hexdigit()))
}
pub(super) async fn exchange(
    client: &reqwest::Client,
    base_url: &str,
    email: &str,
    password: &str,
    interactive: bool,
    mut prompt: impl FnMut() -> Result<Zeroizing<String>>,
) -> Result<PasswordTokens> {
    let base_url = base_url.trim_end_matches('/');
    let response = client
        .post(format!("{base_url}/api/v1/auth/login"))
        .json(&PasswordRequest {
            email,
            password,
            client: "cli",
        })
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Failed to connect to NyxID server"))?;
    let status = response.status();
    let bytes = body(response).await?;
    if status.is_success() {
        return tokens(&bytes);
    }
    if status != reqwest::StatusCode::FORBIDDEN {
        bail!("Password login failed (HTTP {status})");
    }
    let challenge: Challenge =
        serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Password login failed"))?;
    if challenge.error_code != 2002
        || !(32..=256).contains(&challenge.session_token.len())
        || !challenge
            .session_token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        bail!("Password login failed");
    }
    if !interactive {
        bail!(
            "MFA is required. Re-run `nyxid login --password` in an interactive terminal to enter an authenticator or recovery code, or use browser/device login."
        );
    }
    let code = prompt().map_err(|_| anyhow::anyhow!("MFA code could not be read securely"))?;
    if !valid_code(&code) {
        bail!(
            "Enter six authenticator digits, a legacy eight-character recovery code, or a 32-character recovery code"
        );
    }
    let response = client
        .post(format!("{base_url}/api/v1/auth/mfa/verify"))
        .json(&MfaRequest {
            code: &code,
            mfa_token: &challenge.session_token,
            client: "cli",
        })
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("MFA completion could not connect to NyxID"))?;
    if !response.status().is_success() {
        bail!(
            "MFA verification failed. Start password login again with a valid authenticator or unused recovery code."
        );
    }
    tokens(&body(response).await?)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn exchange_and_save(
    client: &reqwest::Client,
    base_url: &str,
    email: &str,
    password: &str,
    profile: Option<&str>,
    interactive: bool,
    prompt: impl FnMut() -> Result<Zeroizing<String>>,
) -> Result<()> {
    let login = exchange(client, base_url, email, password, interactive, prompt).await?;
    super::replace_login_files(profile, || {
        super::save_base_url_for(profile, base_url.trim_end_matches('/'))?;
        super::save_tokens_for(
            profile,
            &login.access_token,
            login.refresh_token.as_ref().map(|token| token.as_str()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };
    #[tokio::test]
    async fn password_mfa_continues_all_code_formats_at_deployment_prefix() {
        for code in ["123456", "Ab_12-xy", "00112233001122330011223300112233"] {
            let server = MockServer::start().await;
            let challenge = "distinctive-first-factor-challenge-00112233445566778899";
            Mock::given(method("POST")).and(path("/nyxid/api/v1/auth/login"))
                .and(body_json(serde_json::json!({"email":"user@example.test","password":"distinctive-password","client":"cli"})))
                .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({"error_code":2002,"session_token":challenge,"message":"MFA required"})))
                .expect(1).mount(&server).await;
            Mock::given(method("POST")).and(path("/nyxid/api/v1/auth/mfa/verify"))
                .and(body_json(serde_json::json!({"mfa_token":challenge,"code":code,"client":"cli"})))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token":"access-result","refresh_token":"refresh-result"})))
                .expect(1).mount(&server).await;
            let mut prompts = 0;
            let result = exchange(
                &reqwest::Client::new(),
                &format!("{}/nyxid/", server.uri()),
                "user@example.test",
                "distinctive-password",
                true,
                || {
                    prompts += 1;
                    Ok(Zeroizing::new(code.to_owned()))
                },
            )
            .await
            .unwrap();
            assert_eq!(prompts, 1);
            assert_eq!(result.access_token.as_str(), "access-result");
            assert_eq!(
                result.refresh_token.as_deref().map(|v| v.as_str()),
                Some("refresh-result")
            );
        }
    }
    #[tokio::test]
    async fn password_mfa_noninteractive_and_malformed_failures_never_echo_material() {
        for (status, payload) in [
            (
                403,
                serde_json::json!({"error_code":2002,"session_token":"distinctive-secret-challenge-0011223344556677"}),
            ),
            (
                403,
                serde_json::json!({"error_code":"distinctive-secret","session_token":"distinctive-secret"}),
            ),
            (
                403,
                serde_json::json!({"error_code":9999,"session_token":"distinctive-secret"}),
            ),
            (500, serde_json::json!({"error":"distinctive-secret"})),
            (
                200,
                serde_json::json!({"access_token":["distinctive-secret"]}),
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(path("/api/v1/auth/login"))
                .respond_with(ResponseTemplate::new(status).set_body_json(payload))
                .expect(1)
                .mount(&server)
                .await;
            let result = exchange(
                &reqwest::Client::new(),
                &server.uri(),
                "user@example.test",
                "distinctive-secret-password",
                false,
                || panic!("noninteractive prompt"),
            )
            .await
            .err()
            .unwrap();
            assert!(!format!("{result:?} {result}").contains("distinctive-secret"));
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/auth/login"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_string("distinctive-secret".repeat(MAX_RESPONSE)),
            )
            .mount(&server)
            .await;
        let error = exchange(
            &reqwest::Client::new(),
            &server.uri(),
            "user@example.test",
            "password",
            true,
            || panic!("oversized response must not prompt"),
        )
        .await
        .err()
        .unwrap();
        assert!(!format!("{error:?}").contains("distinctive-secret"));
    }
}
