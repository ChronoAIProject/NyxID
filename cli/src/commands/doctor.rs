use std::cmp::Ordering;
use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::auth;
use crate::cli::DoctorArgs;
use crate::commands::update;
use crate::telemetry::consent::{self, ConsentSource};
use crate::update_check;

pub async fn run(args: DoctorArgs) -> Result<()> {
    let report = build_report().await;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", format_report(&report));
    }

    if report.has_failures() {
        anyhow::bail!("doctor found failing checks");
    }

    Ok(())
}

async fn build_report() -> DoctorReport {
    let mut sections = Vec::new();
    sections.push(installation_section());
    sections.push(github_section().await);
    sections.push(authentication_section());
    sections.push(telemetry_section());
    sections.push(update_check_section());
    DoctorReport { sections }
}

#[derive(Debug, Clone, Serialize)]
struct DoctorReport {
    sections: Vec<DoctorSection>,
}

impl DoctorReport {
    fn has_failures(&self) -> bool {
        self.sections
            .iter()
            .flat_map(|section| &section.rows)
            .any(|row| row.status == DoctorStatus::Fail)
    }
}

#[derive(Debug, Clone, Serialize)]
struct DoctorSection {
    title: String,
    rows: Vec<DoctorRow>,
}

#[derive(Debug, Clone, Serialize)]
struct DoctorRow {
    label: String,
    detail: String,
    status: DoctorStatus,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum DoctorStatus {
    Pass,
    Warn,
    Fail,
}

impl DoctorStatus {
    fn glyph(self) -> &'static str {
        match self {
            Self::Pass => "✓",
            Self::Warn => "!",
            Self::Fail => "✗",
        }
    }
}

fn row(label: impl Into<String>, detail: impl Into<String>, status: DoctorStatus) -> DoctorRow {
    DoctorRow {
        label: label.into(),
        detail: detail.into(),
        status,
    }
}

fn format_report(report: &DoctorReport) -> String {
    let mut lines = vec!["nyxid doctor".to_string(), String::new()];
    for section in &report.sections {
        lines.push(format!("  {}", section.title));
        for row in &section.rows {
            if row.label.is_empty() {
                lines.push(format!(
                    "    {:<24} {:<58} {}",
                    "",
                    row.detail,
                    row.status.glyph()
                ));
            } else {
                lines.push(format!(
                    "    {:<24} {:<58} {}",
                    row.label,
                    row.detail,
                    row.status.glyph()
                ));
            }
        }
        lines.push(String::new());
    }
    lines.pop();
    lines.join("\n")
}

fn installation_section() -> DoctorSection {
    let mut rows = Vec::new();
    let current_exe = std::env::current_exe();
    rows.push(install_version_row(
        current_exe.as_ref().ok().map(|path| path.as_path()),
    ));
    match &current_exe {
        Ok(path) => rows.push(row(
            "Binary path",
            path.display().to_string(),
            DoctorStatus::Pass,
        )),
        Err(err) => rows.push(row(
            "Binary path",
            format!("unavailable: {err}"),
            DoctorStatus::Fail,
        )),
    }

    rows.push(active_symlink_row());
    rows.push(path_row());

    DoctorSection {
        title: "Installation".to_string(),
        rows,
    }
}

fn active_symlink_row() -> DoctorRow {
    let active_path = match update::active_binary_path() {
        Ok(path) => path,
        Err(err) => {
            return row(
                "Active symlink",
                format!("unavailable: {err:#}"),
                DoctorStatus::Warn,
            );
        }
    };

    let metadata = match std::fs::symlink_metadata(&active_path) {
        Ok(metadata) => metadata,
        Err(_) => {
            return row(
                "Active symlink",
                format!("{} is not present", active_path.display()),
                DoctorStatus::Warn,
            );
        }
    };

    if metadata.file_type().is_symlink() {
        match std::fs::read_link(&active_path) {
            Ok(target) => {
                let display_target = display_symlink_target(&target);
                let resolved_target = if target.is_absolute() {
                    target.clone()
                } else {
                    active_path
                        .parent()
                        .unwrap_or_else(|| Path::new(""))
                        .join(&target)
                };
                let status = if resolved_target.exists() {
                    DoctorStatus::Pass
                } else {
                    DoctorStatus::Fail
                };
                row(
                    "Active symlink",
                    format!("{} -> {display_target}", active_path.display()),
                    status,
                )
            }
            Err(err) => row(
                "Active symlink",
                format!("{} unreadable: {err}", active_path.display()),
                DoctorStatus::Fail,
            ),
        }
    } else if metadata.is_file() {
        row(
            "Active symlink",
            format!("{} is a legacy regular file", active_path.display()),
            DoctorStatus::Warn,
        )
    } else {
        row(
            "Active symlink",
            format!("{} is not a file or symlink", active_path.display()),
            DoctorStatus::Fail,
        )
    }
}

fn display_symlink_target(target: &Path) -> String {
    if let Ok(root) = update::install_versions_root()
        && let Ok(relative) = target.strip_prefix(root)
    {
        return format!("versions/{}", relative.display());
    }
    target.display().to_string()
}

fn path_row() -> DoctorRow {
    match update::active_binary_path() {
        Ok(path) => {
            let dir = path.parent().unwrap_or_else(|| Path::new(""));
            if update::path_contains_dir(dir) {
                row(
                    "PATH",
                    format!("{} is in PATH", dir.display()),
                    DoctorStatus::Pass,
                )
            } else {
                row(
                    "PATH",
                    format!("{} is not in PATH", dir.display()),
                    DoctorStatus::Warn,
                )
            }
        }
        Err(err) => row("PATH", format!("unavailable: {err:#}"), DoctorStatus::Warn),
    }
}

fn install_version_row(current_exe: Option<&Path>) -> DoctorRow {
    let version = update_check::installed_release_tag();
    let status = current_exe
        .zip(update::install_versions_root().ok())
        .map(|(path, root)| {
            if path.starts_with(root) {
                DoctorStatus::Pass
            } else {
                DoctorStatus::Warn
            }
        })
        .unwrap_or(DoctorStatus::Warn);

    row("nyxid version", version, status)
}

async fn github_section() -> DoctorSection {
    let url = format!(
        "{}/repos/{}/{}/releases/latest",
        update::GITHUB_API_URL,
        update::GITHUB_OWNER,
        update::GITHUB_REPO
    );
    let mut rows = Vec::new();
    let client = match update::github_client() {
        Ok(client) => client,
        Err(err) => {
            rows.push(row(
                "API reachable",
                format!("client error: {err:#}"),
                DoctorStatus::Fail,
            ));
            return DoctorSection {
                title: "GitHub Releases".to_string(),
                rows,
            };
        }
    };

    match client.get(&url).send().await {
        Ok(response) => {
            let headers = response.headers().clone();
            if response.status().is_success() {
                rows.push(row("API reachable", url, DoctorStatus::Pass));
                match response.json::<update::GitHubRelease>().await {
                    Ok(release) => rows.push(latest_release_row(&release.tag_name)),
                    Err(err) => rows.push(row(
                        "Latest release",
                        format!("response parse failed: {err}"),
                        DoctorStatus::Fail,
                    )),
                }
            } else if response.status() == reqwest::StatusCode::NOT_FOUND {
                rows.push(row("API reachable", url, DoctorStatus::Pass));
                rows.push(row(
                    "Latest release",
                    "no releases published yet",
                    DoctorStatus::Warn,
                ));
            } else {
                rows.push(row(
                    "API reachable",
                    format!("{url} returned {}", response.status()),
                    DoctorStatus::Fail,
                ));
            }
            rows.push(rate_limit_row(&headers));
        }
        Err(err) => {
            rows.push(row(
                "API reachable",
                format!("{url}: {err}"),
                DoctorStatus::Fail,
            ));
        }
    }

    DoctorSection {
        title: "GitHub Releases".to_string(),
        rows,
    }
}

fn latest_release_row(latest: &str) -> DoctorRow {
    let installed = update_check::installed_release_tag();
    let detail = match update::compare_release_tags(latest, &installed) {
        Ok(Ordering::Greater) => format!("{latest} (update available; installed {installed})"),
        Ok(Ordering::Equal) => format!("{latest} (you are up to date)"),
        Ok(Ordering::Less) => format!("{latest} (installed {installed})"),
        Err(_) => format!("{latest} (installed {installed})"),
    };
    row("Latest release", detail, DoctorStatus::Pass)
}

fn rate_limit_row(headers: &reqwest::header::HeaderMap) -> DoctorRow {
    let remaining = headers
        .get("x-ratelimit-remaining")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown");
    let limit = headers
        .get("x-ratelimit-limit")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown");
    let reset = headers
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0))
        .map(|reset| format!("resets at {}", reset.format("%H:%M UTC")))
        .unwrap_or_else(|| "reset unavailable".to_string());

    row(
        "Rate limit",
        format!("{remaining} / {limit} remaining ({reset})"),
        DoctorStatus::Pass,
    )
}

fn authentication_section() -> DoctorSection {
    let mut rows = Vec::new();
    match auth::read_saved_base_url_for(None) {
        Some(base_url) => rows.push(row("Stored base URL", base_url, DoctorStatus::Pass)),
        None => rows.push(row("Stored base URL", "not configured", DoctorStatus::Warn)),
    }

    match auth::read_saved_token_for(None) {
        Some(token) => {
            let login = auth::jwt_claim_string_from_token(&token, "email")
                .or_else(|| auth::jwt_claim_string_from_token(&token, "preferred_username"))
                .or_else(|| auth::jwt_sub_from_token(&token))
                .unwrap_or_else(|| "unknown user".to_string());
            rows.push(row(
                "Login state",
                format!("logged in as {login}"),
                DoctorStatus::Pass,
            ));
            let expiry = auth::jwt_exp_from_token(&token)
                .map(|exp| exp.to_rfc3339())
                .unwrap_or_else(|| "unknown expiry".to_string());
            rows.push(row(
                "",
                format!("(token redacted; expires {expiry})"),
                DoctorStatus::Pass,
            ));
        }
        None => rows.push(row("Login state", "not logged in", DoctorStatus::Warn)),
    }

    DoctorSection {
        title: "Authentication".to_string(),
        rows,
    }
}

fn telemetry_section() -> DoctorSection {
    let state = consent::resolve_consent(None);
    let label = match state.source {
        ConsentSource::ConfigEnabled | ConsentSource::EnvVarOn => "opted in",
        ConsentSource::ConfigDeclined | ConsentSource::DoNotTrack | ConsentSource::EnvVarOff => {
            "opted out"
        }
        ConsentSource::FirstRunPending => "not asked",
    };
    DoctorSection {
        title: "Telemetry".to_string(),
        rows: vec![row("Consent status", label, DoctorStatus::Pass)],
    }
}

fn update_check_section() -> DoctorSection {
    let mut rows = Vec::new();
    match update_check::read_cache() {
        Some(cache) => rows.push(row(
            "Last check",
            format!(
                "{} ({})",
                cache.last_checked.to_rfc3339(),
                human_age(cache.last_checked)
            ),
            DoctorStatus::Pass,
        )),
        None => rows.push(row("Last check", "not checked yet", DoctorStatus::Warn)),
    }

    let auto_enabled = !update_check::env_opt_out_is_enabled(
        std::env::var("NYXID_NO_UPDATE_CHECK").ok().as_deref(),
    ) && !std::env::var("CI")
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    rows.push(row(
        "Auto-check enabled",
        if auto_enabled {
            "yes (set NYXID_NO_UPDATE_CHECK=1 to disable)"
        } else {
            "no"
        },
        if auto_enabled {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Warn
        },
    ));

    DoctorSection {
        title: "Update check".to_string(),
        rows,
    }
}

fn human_age(then: DateTime<Utc>) -> String {
    let elapsed = Utc::now().signed_duration_since(then);
    if elapsed.num_days() > 0 {
        format!("{} days ago", elapsed.num_days())
    } else if elapsed.num_hours() > 0 {
        format!("{} hours ago", elapsed.num_hours())
    } else if elapsed.num_minutes() > 0 {
        format!("{} minutes ago", elapsed.num_minutes())
    } else {
        "just now".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_format_includes_sections_rows_and_status_glyphs() {
        let report = DoctorReport {
            sections: vec![DoctorSection {
                title: "Installation".to_string(),
                rows: vec![
                    row("Binary path", "/tmp/nyxid", DoctorStatus::Pass),
                    row("Active symlink", "legacy regular file", DoctorStatus::Warn),
                    row("GitHub", "unreachable", DoctorStatus::Fail),
                ],
            }],
        };

        let rendered = format_report(&report);
        assert!(rendered.contains("nyxid doctor"));
        assert!(rendered.contains("Installation"));
        assert!(rendered.contains("Binary path"));
        assert!(rendered.contains("✓"));
        assert!(rendered.contains("!"));
        assert!(rendered.contains("✗"));
        assert!(report.has_failures());
    }

    #[test]
    fn latest_release_row_reports_update_available() {
        let row = latest_release_row("v99.0.0");
        assert_eq!(row.status, DoctorStatus::Pass);
        assert!(row.detail.contains("update available"));
    }

    #[test]
    fn display_symlink_target_prefers_versions_relative_path() {
        let _lock = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("versions");
        let _guard = EnvGuard::set("NYXID_INSTALL_ROOT", root.as_os_str());
        let target = root.join("v0.5.0").join("nyxid");

        assert_eq!(display_symlink_target(&target), "versions/v0.5.0/nyxid");
    }

    struct EnvGuard {
        key: &'static str,
        old: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &std::ffi::OsStr) -> Self {
            let old = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.old {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn status_glyphs_are_distinct_per_variant() {
        assert_eq!(DoctorStatus::Pass.glyph(), "✓");
        assert_eq!(DoctorStatus::Warn.glyph(), "!");
        assert_eq!(DoctorStatus::Fail.glyph(), "✗");
    }

    #[test]
    fn row_constructor_populates_all_fields() {
        let r = row("Label", "detail text", DoctorStatus::Warn);
        assert_eq!(r.label, "Label");
        assert_eq!(r.detail, "detail text");
        assert_eq!(r.status, DoctorStatus::Warn);
    }

    #[test]
    fn has_failures_is_true_only_with_a_fail_row() {
        let pass_warn = DoctorReport {
            sections: vec![DoctorSection {
                title: "S".to_string(),
                rows: vec![
                    row("a", "ok", DoctorStatus::Pass),
                    row("b", "meh", DoctorStatus::Warn),
                ],
            }],
        };
        assert!(!pass_warn.has_failures());

        let with_fail = DoctorReport {
            sections: vec![DoctorSection {
                title: "S".to_string(),
                rows: vec![
                    row("a", "ok", DoctorStatus::Pass),
                    row("c", "broken", DoctorStatus::Fail),
                ],
            }],
        };
        assert!(with_fail.has_failures());
    }

    #[test]
    fn has_failures_is_false_for_empty_report() {
        let empty = DoctorReport { sections: vec![] };
        assert!(!empty.has_failures());
    }

    #[test]
    fn format_report_blank_label_row_renders_detail_without_label() {
        let report = DoctorReport {
            sections: vec![DoctorSection {
                title: "Authentication".to_string(),
                rows: vec![
                    row("Login state", "logged in as a@b.c", DoctorStatus::Pass),
                    row("", "(token redacted; expires 2026)", DoctorStatus::Pass),
                ],
            }],
        };
        let rendered = format_report(&report);
        // The blank-label branch still emits the detail text.
        assert!(rendered.contains("(token redacted; expires 2026)"));
        // Title indentation and trailing-newline trimming behavior.
        assert!(rendered.starts_with("nyxid doctor\n"));
        assert!(!rendered.ends_with('\n'));
    }

    #[test]
    fn latest_release_row_equal_reports_up_to_date() {
        // installed_release_tag() returns the compiled-in tag; comparing it to
        // itself must land on the Ordering::Equal branch.
        let installed = update_check::installed_release_tag();
        let row = latest_release_row(&installed);
        assert_eq!(row.status, DoctorStatus::Pass);
        assert!(
            row.detail.contains("you are up to date"),
            "detail was: {}",
            row.detail
        );
    }

    #[test]
    fn rate_limit_row_formats_present_headers_with_reset_time() {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-ratelimit-remaining"),
            HeaderValue::from_static("57"),
        );
        headers.insert(
            HeaderName::from_static("x-ratelimit-limit"),
            HeaderValue::from_static("60"),
        );
        // 2021-01-01T00:00:00Z -> "00:00 UTC"
        headers.insert(
            HeaderName::from_static("x-ratelimit-reset"),
            HeaderValue::from_static("1609459200"),
        );

        let r = rate_limit_row(&headers);
        assert_eq!(r.status, DoctorStatus::Pass);
        assert_eq!(r.label, "Rate limit");
        assert_eq!(r.detail, "57 / 60 remaining (resets at 00:00 UTC)");
    }

    #[test]
    fn rate_limit_row_handles_missing_and_unparseable_headers() {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        // remaining/limit absent -> "unknown"; reset present but non-numeric.
        headers.insert(
            HeaderName::from_static("x-ratelimit-reset"),
            HeaderValue::from_static("not-a-number"),
        );

        let r = rate_limit_row(&headers);
        assert_eq!(r.detail, "unknown / unknown remaining (reset unavailable)");
    }

    #[test]
    fn human_age_buckets_by_largest_unit() {
        let now = Utc::now();
        assert_eq!(human_age(now - Duration::days(3)), "3 days ago");
        assert_eq!(human_age(now - Duration::hours(5)), "5 hours ago");
        assert_eq!(human_age(now - Duration::minutes(2)), "2 minutes ago");
        // Sub-minute elapsed falls through to "just now".
        assert_eq!(human_age(now - Duration::seconds(10)), "just now");
    }
}
