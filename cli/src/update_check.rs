use std::future::Future;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::cli::{AiSetupCommands, Commands};
use crate::commands::repo::CLI_VERSION;
use crate::settings::{
    UpdateCheckCache, effective_update_check, read_update_cache, should_refresh_update_cache,
    write_update_cache,
};
use crate::update_support::{CliReleaseManifest, fetch_release_manifest, is_newer_version};

// Keep the periodic refresh unobtrusive even if the network is slow or hung.
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug)]
pub struct UpdateCheckContext {
    allow_update_check: bool,
    profile: Option<String>,
    base_url: Option<String>,
}

impl UpdateCheckContext {
    pub fn from_command(command: &Commands) -> Self {
        let profile = resolve_profile_from_argv();
        let base_url = resolve_base_url_from_argv(profile.as_deref());
        let requests_json = invocation_requests_json();
        let allow_update_check = !requests_json
            && !matches!(
                command,
                Commands::Repo(_)
                    | Commands::Info
                    | Commands::Update(_)
                    | Commands::Config { .. }
                    | Commands::AiSetup {
                        command: AiSetupCommands::Status
                    }
            );

        Self {
            allow_update_check,
            profile,
            base_url,
        }
    }
}

pub async fn maybe_refresh_update_check(context: &UpdateCheckContext) {
    maybe_refresh_update_check_with(
        context,
        Utc::now(),
        effective_update_check,
        read_update_cache,
        write_update_cache,
        |base_url| async move { fetch_release_manifest(&base_url).await },
    )
    .await;
}

pub fn maybe_print_update_banner(context: &UpdateCheckContext) {
    if !context.allow_update_check {
        return;
    }

    let Some(base_url) = context.base_url.as_deref() else {
        return;
    };

    if !effective_update_check(context.profile.as_deref()).unwrap_or(true) {
        return;
    }

    let Some(cache) = read_update_cache(context.profile.as_deref()).ok().flatten() else {
        return;
    };

    if cache.base_url != base_url {
        return;
    }

    if !is_newer_version(CLI_VERSION, &cache.latest_version).unwrap_or(false) {
        return;
    }

    eprintln!(
        "A new nyxid is available: {} → {}. Run `nyxid update` to upgrade.",
        CLI_VERSION, cache.latest_version
    );
}

async fn maybe_refresh_update_check_with<
    IsEnabled,
    ReadCache,
    WriteCache,
    FetchManifest,
    FetchFuture,
>(
    context: &UpdateCheckContext,
    now: DateTime<Utc>,
    is_enabled: IsEnabled,
    read_cache: ReadCache,
    write_cache: WriteCache,
    fetch_manifest: FetchManifest,
) where
    IsEnabled: FnOnce(Option<&str>) -> Result<bool>,
    ReadCache: FnOnce(Option<&str>) -> Result<Option<UpdateCheckCache>>,
    WriteCache: FnOnce(Option<&str>, &UpdateCheckCache) -> Result<()>,
    FetchManifest: FnOnce(String) -> FetchFuture,
    FetchFuture: Future<Output = Result<CliReleaseManifest>>,
{
    if !context.allow_update_check {
        return;
    }

    let Some(base_url) = context.base_url.clone() else {
        return;
    };

    let profile = context.profile.clone();
    if !is_enabled(profile.as_deref()).unwrap_or(true) {
        return;
    }

    let cache = read_cache(profile.as_deref()).ok().flatten();
    if !should_refresh_update_cache(cache.as_ref(), &base_url, now) {
        return;
    }

    let refresh = async move {
        let manifest = fetch_manifest(base_url.clone()).await?;
        let cache = UpdateCheckCache {
            base_url,
            checked_at: now,
            latest_version: manifest.version,
        };
        write_cache(profile.as_deref(), &cache)
    };

    let _ = tokio::time::timeout(UPDATE_CHECK_TIMEOUT, refresh).await;
}

fn resolve_profile_from_argv() -> Option<String> {
    value_after_flag("--profile").or_else(|| std::env::var("NYXID_PROFILE").ok())
}

fn resolve_base_url_from_argv(profile: Option<&str>) -> Option<String> {
    value_after_flag("--base-url")
        .or_else(|| std::env::var("NYXID_URL").ok())
        .or_else(|| crate::auth::read_saved_base_url_for(profile))
}

fn invocation_requests_json() -> bool {
    let mut args = std::env::args().skip(1).peekable();

    while let Some(arg) = args.next() {
        if arg == "--json" {
            return true;
        }

        if let Some(value) = arg.strip_prefix("--output=") {
            if value.eq_ignore_ascii_case("json") {
                return true;
            }
            continue;
        }

        if arg == "--output"
            && args
                .peek()
                .is_some_and(|value| value.eq_ignore_ascii_case("json"))
        {
            return true;
        }
    }

    false
}

fn value_after_flag(flag: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == flag {
            return args.next();
        }

        if let Some(value) = arg.strip_prefix(&(flag.to_string() + "=")) {
            return Some(value.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use chrono::{TimeDelta, Utc};

    use super::{UPDATE_CHECK_TIMEOUT, UpdateCheckContext, maybe_refresh_update_check_with};
    use crate::settings::UpdateCheckCache;
    use crate::update_support::CliReleaseManifest;

    #[tokio::test]
    async fn stale_cache_refresh_times_out_without_overwriting_cache() {
        let now = Utc::now();
        let stale_cache = UpdateCheckCache {
            base_url: "https://auth.example.com".to_string(),
            checked_at: now - TimeDelta::hours(25),
            latest_version: "0.3.0".to_string(),
        };
        let cache_store = Arc::new(Mutex::new(Some(stale_cache.clone())));
        let fetch_started = Arc::new(AtomicBool::new(false));
        let context = UpdateCheckContext {
            allow_update_check: true,
            profile: Some("race".to_string()),
            base_url: Some("https://auth.example.com".to_string()),
        };

        let read_store = Arc::clone(&cache_store);
        let write_store = Arc::clone(&cache_store);
        let fetch_started_for_task = Arc::clone(&fetch_started);
        let started_at = std::time::Instant::now();

        maybe_refresh_update_check_with(
            &context,
            now,
            |_| Ok(true),
            move |_| Ok(read_store.lock().expect("cache store poisoned").clone()),
            move |_, cache| {
                *write_store.lock().expect("cache store poisoned") = Some(cache.clone());
                Ok(())
            },
            move |_| {
                fetch_started_for_task.store(true, Ordering::SeqCst);
                pending::<anyhow::Result<CliReleaseManifest>>()
            },
        )
        .await;

        let elapsed = started_at.elapsed();
        assert!(fetch_started.load(Ordering::SeqCst));
        assert!(elapsed >= UPDATE_CHECK_TIMEOUT);
        assert!(elapsed < Duration::from_secs(3));

        let cache = cache_store
            .lock()
            .expect("cache store poisoned")
            .clone()
            .expect("stale cache should remain");
        assert_eq!(cache.base_url, stale_cache.base_url);
        assert_eq!(cache.checked_at, stale_cache.checked_at);
        assert_eq!(cache.latest_version, stale_cache.latest_version);
    }

    #[tokio::test]
    async fn fresh_cache_skips_network_refresh() {
        let now = Utc::now();
        let cache_store = Arc::new(Mutex::new(Some(UpdateCheckCache {
            base_url: "https://auth.example.com".to_string(),
            checked_at: now - TimeDelta::hours(2),
            latest_version: "0.3.0".to_string(),
        })));
        let fetch_started = Arc::new(AtomicBool::new(false));
        let context = UpdateCheckContext {
            allow_update_check: true,
            profile: Some("race".to_string()),
            base_url: Some("https://auth.example.com".to_string()),
        };

        let read_store = Arc::clone(&cache_store);
        let write_store = Arc::clone(&cache_store);
        let fetch_started_for_task = Arc::clone(&fetch_started);

        maybe_refresh_update_check_with(
            &context,
            now,
            |_| Ok(true),
            move |_| Ok(read_store.lock().expect("cache store poisoned").clone()),
            move |_, cache| {
                *write_store.lock().expect("cache store poisoned") = Some(cache.clone());
                Ok(())
            },
            move |_| {
                fetch_started_for_task.store(true, Ordering::SeqCst);
                async {
                    Ok(CliReleaseManifest {
                        version: "9.9.9".to_string(),
                        commit: "mock".to_string(),
                        release_tag: "v9.9.9".to_string(),
                        release_url: "https://example.com/releases/v9.9.9".to_string(),
                        asset_base_url: "https://example.com/releases/download/v9.9.9/".to_string(),
                        checksums_url: "https://example.com/releases/download/v9.9.9/SHA256SUMS"
                            .to_string(),
                        checksums_signature_url:
                            "https://example.com/releases/download/v9.9.9/SHA256SUMS.sig"
                                .to_string(),
                        checksums_cert_url:
                            "https://example.com/releases/download/v9.9.9/SHA256SUMS.pem"
                                .to_string(),
                        asset_name_template: "nyxid-{version}-{target}.{ext}".to_string(),
                        cosign_identity: "https://example.com/workflow".to_string(),
                        cosign_issuer: "https://token.actions.githubusercontent.com".to_string(),
                    })
                }
            },
        )
        .await;

        assert!(!fetch_started.load(Ordering::SeqCst));
    }
}
