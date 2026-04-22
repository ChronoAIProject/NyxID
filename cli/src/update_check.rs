use chrono::Utc;

use crate::cli::{AiSetupCommands, Commands};
use crate::commands::repo::CLI_VERSION;
use crate::settings::{
    UpdateCheckCache, effective_update_check, read_update_cache, should_refresh_update_cache,
    write_update_cache,
};
use crate::update_support::{fetch_release_manifest, is_newer_version};

#[derive(Clone, Debug)]
pub struct UpdateCheckContext {
    allow_after_success: bool,
    suppress_banner: bool,
    profile: Option<String>,
    base_url: Option<String>,
}

impl UpdateCheckContext {
    pub fn from_command(command: &Commands) -> Self {
        let profile = resolve_profile_from_argv();
        let base_url = resolve_base_url_from_argv(profile.as_deref());
        let allow_after_success = !matches!(
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
            allow_after_success,
            suppress_banner: matches!(command, Commands::Update(_)) || invocation_requests_json(),
            profile,
            base_url,
        }
    }
}

pub fn maybe_spawn_update_check(context: &UpdateCheckContext) {
    if !context.allow_after_success {
        return;
    }

    let Some(base_url) = context.base_url.clone() else {
        return;
    };

    let profile = context.profile.clone();
    if !effective_update_check(profile.as_deref()).unwrap_or(true) {
        return;
    }

    let cache = read_update_cache(profile.as_deref()).ok().flatten();
    if !should_refresh_update_cache(cache.as_ref(), &base_url, Utc::now()) {
        return;
    }

    tokio::spawn(async move {
        let Ok(manifest) = fetch_release_manifest(&base_url).await else {
            return;
        };

        let cache = UpdateCheckCache {
            base_url,
            checked_at: Utc::now(),
            latest_version: manifest.version,
        };

        let _ = write_update_cache(profile.as_deref(), &cache);
    });
}

pub fn maybe_print_update_banner(context: &UpdateCheckContext) {
    if context.suppress_banner {
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
