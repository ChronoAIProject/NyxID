//! Test-only helpers for serializing `$HOME` / env-var mutations across
//! test modules. Cargo's default test runner is multi-threaded, so any
//! two tests that override `HOME` (we have several across `auth`,
//! `api`, and `telemetry::consent`) must share a single mutex or they
//! race. Using one shared `OnceLock<Mutex<()>>` here ensures that.

use std::sync::{Mutex, OnceLock};

use crate::cli::{AuthArgs, OutputFormat};

/// Return the process-global env-mutation lock. Acquire this for the
/// entire duration of a test that calls `std::env::set_var("HOME", …)`
/// (or similar env-var manipulation) before doing file-system IO that
/// resolves relative to `$HOME`.
pub fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Build an [`AuthArgs`] pointed at `base_url` (typically a wiremock
/// `MockServer::uri()`), with an explicit access-token override so
/// command tests resolve their `ApiClient` without touching `$HOME` or
/// the saved-token store. `auth::resolve_access_token` returns the
/// explicit `access_token` before consulting env or disk, so no token
/// file is required. Defaults to JSON output.
pub fn mock_auth(base_url: impl Into<String>) -> AuthArgs {
    mock_auth_with_output(base_url, OutputFormat::Json)
}

/// Like [`mock_auth`] but selects the output format — some commands
/// branch on Table vs JSON for their human-facing output.
pub fn mock_auth_with_output(base_url: impl Into<String>, output: OutputFormat) -> AuthArgs {
    AuthArgs {
        base_url: Some(base_url.into()),
        access_token: Some("test-access-token".to_string()),
        access_token_env: "NYXID_ACCESS_TOKEN".to_string(),
        profile: None,
        output,
    }
}
