//! Best-effort detection and refresh of partially-installed NyxID skills.
//!
//! Prior CLI binaries did not know about per-topic `references/*.md` files
//! introduced when the skill migrated to the Anthropic Agent Skills spec.
//! Their `nyxid update` flow successfully writes `SKILL.md` (which now points
//! at those references) but leaves the references directory mostly empty.
//!
//! This module detects that state on every CLI invocation, runs a one-shot
//! `ai_setup::run(Update)` to refresh the install, and then lets the user's
//! actual command proceed. Any failure here is logged as a warning -- it must
//! never block the user's command.
//!
//! Disabled by setting `NYXID_SKIP_SKILL_SELF_HEAL=1`.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::cli::{AiSetupCommands, AiToolTarget, Commands};

/// Tools whose installs are filesystem-based (Cursor uses a single `.mdc`
/// rule file and is intentionally excluded; Generic has no skill).
const FILESYSTEM_TOOLS: &[AiToolTarget] = &[
    AiToolTarget::ClaudeCode,
    AiToolTarget::Codex,
    AiToolTarget::Openclaw,
];

/// Reference file checked as a presence canary. Picked because `services.md`
/// is one of the first per-topic references the agent loads.
const CANARY_REFERENCE: &str = "references/services.md";

/// Minimum interval between self-heal attempts when the previous one failed.
const COOLDOWN_SECS: u64 = 3600;

const COOLDOWN_FILE: &str = ".nyxid/.skill-self-heal-attempt";

/// Entry point. Run before dispatching the user's command.
pub async fn maybe_self_heal(command: &Commands) {
    if !should_run(command) {
        return;
    }

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return,
    };

    let needs_heal = detect_partial_installs(&home);
    if needs_heal.is_empty() {
        return;
    }

    if !cooldown_elapsed(&home) {
        return;
    }

    eprintln!(
        "Detected partial NyxID skill install ({}); refreshing references...",
        needs_heal
            .iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Write the cooldown marker before attempting so we don't retry on every
    // invocation if the refresh itself fails.
    record_attempt(&home);

    if let Err(e) = run_refresh().await {
        eprintln!("Warning: skill self-heal failed ({e}). Run `nyxid update` to retry.");
        return;
    }

    eprintln!("Skill refresh complete.");
}

fn should_run(command: &Commands) -> bool {
    if env_flag("NYXID_SKIP_SKILL_SELF_HEAL") {
        return false;
    }
    !is_self_referential(command)
}

fn is_self_referential(command: &Commands) -> bool {
    matches!(
        command,
        Commands::Update(_)
            | Commands::AiSetup { .. }
            | Commands::Login(_)
            | Commands::Logout(_)
            | Commands::Register(_)
            | Commands::VerifyEmail(_)
            | Commands::ForgotPassword(_)
            | Commands::ResetPassword(_)
    )
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1" | "true" | "yes" | "on")
    )
}

/// Return the list of filesystem-based tools whose install is partial.
/// "Partial" = SKILL.md is present but the canary reference is missing.
fn detect_partial_installs(home: &Path) -> Vec<AiToolTarget> {
    FILESYSTEM_TOOLS
        .iter()
        .copied()
        .filter(|tool| is_partial_install(home, *tool))
        .collect()
}

fn is_partial_install(home: &Path, tool: AiToolTarget) -> bool {
    let dir = match skill_install_dir(home, tool) {
        Some(d) => d,
        None => return false,
    };
    dir.join("SKILL.md").exists() && !dir.join(CANARY_REFERENCE).exists()
}

fn skill_install_dir(home: &Path, tool: AiToolTarget) -> Option<PathBuf> {
    match tool {
        AiToolTarget::ClaudeCode => Some(home.join(".claude/skills/nyxid")),
        AiToolTarget::Codex => Some(home.join(".codex/skills/nyxid")),
        AiToolTarget::Openclaw => Some(home.join(".openclaw/skills/nyxid")),
        AiToolTarget::Cursor | AiToolTarget::Generic => None,
    }
}

fn cooldown_elapsed(home: &Path) -> bool {
    let path = home.join(COOLDOWN_FILE);
    let Ok(meta) = std::fs::metadata(&path) else {
        return true;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|d| d.as_secs() >= COOLDOWN_SECS)
        .unwrap_or(true)
}

fn record_attempt(home: &Path) {
    let path = home.join(COOLDOWN_FILE);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, b"");
}

async fn run_refresh() -> anyhow::Result<()> {
    crate::commands::ai_setup::run(AiSetupCommands::Update {
        tool: None,
        base_url: None,
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_home() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nyxid-self-heal-tests-{}-{nanos}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn install_skill(home: &Path, tool: AiToolTarget, with_canary: bool) -> PathBuf {
        let dir = skill_install_dir(home, tool).unwrap();
        fs::create_dir_all(dir.join("references")).unwrap();
        fs::write(dir.join("SKILL.md"), "---\nname: nyxid\n---\n").unwrap();
        if with_canary {
            fs::write(dir.join(CANARY_REFERENCE), "stub\n").unwrap();
        }
        dir
    }

    #[test]
    fn detect_returns_empty_when_no_skill_installed() {
        let home = temp_home();
        assert!(detect_partial_installs(&home).is_empty());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn detect_returns_empty_for_complete_install() {
        let home = temp_home();
        install_skill(&home, AiToolTarget::ClaudeCode, true);
        assert!(detect_partial_installs(&home).is_empty());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn detect_flags_partial_claude_install() {
        let home = temp_home();
        install_skill(&home, AiToolTarget::ClaudeCode, false);
        let needs = detect_partial_installs(&home);
        assert_eq!(needs, vec![AiToolTarget::ClaudeCode]);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn detect_flags_multiple_partial_installs() {
        let home = temp_home();
        install_skill(&home, AiToolTarget::ClaudeCode, false);
        install_skill(&home, AiToolTarget::Codex, true);
        install_skill(&home, AiToolTarget::Openclaw, false);
        let needs = detect_partial_installs(&home);
        assert_eq!(
            needs,
            vec![AiToolTarget::ClaudeCode, AiToolTarget::Openclaw]
        );
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn cooldown_says_elapsed_when_no_marker_exists() {
        let home = temp_home();
        assert!(cooldown_elapsed(&home));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn cooldown_blocks_immediately_after_recording() {
        let home = temp_home();
        record_attempt(&home);
        assert!(!cooldown_elapsed(&home));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn env_flag_recognizes_true_values() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for val in ["1", "true", "yes", "on"] {
            unsafe {
                std::env::set_var("NYXID_TEST_ENV_FLAG", val);
            }
            assert!(env_flag("NYXID_TEST_ENV_FLAG"), "should be true for {val}");
        }
        unsafe {
            std::env::set_var("NYXID_TEST_ENV_FLAG", "0");
        }
        assert!(!env_flag("NYXID_TEST_ENV_FLAG"));
        unsafe {
            std::env::remove_var("NYXID_TEST_ENV_FLAG");
        }
        assert!(!env_flag("NYXID_TEST_ENV_FLAG"));
    }

    #[test]
    fn is_self_referential_matches_login() {
        assert!(is_self_referential(&Commands::Login(
            crate::cli::LoginArgs {
                base_url: String::new(),
                password: false,
                email: None,
                profile: None
            }
        )));
    }

    #[test]
    fn should_run_returns_false_for_login() {
        assert!(!should_run(&Commands::Login(crate::cli::LoginArgs {
            base_url: String::new(),
            password: false,
            email: None,
            profile: None
        })));
    }

    #[test]
    fn skill_install_dir_returns_none_for_cursor_and_generic() {
        let home = PathBuf::from("/tmp/test");
        assert!(skill_install_dir(&home, AiToolTarget::Cursor).is_none());
        assert!(skill_install_dir(&home, AiToolTarget::Generic).is_none());
    }

    #[test]
    fn skill_install_dir_returns_paths_for_filesystem_tools() {
        let home = PathBuf::from("/home/user");
        assert!(
            skill_install_dir(&home, AiToolTarget::ClaudeCode)
                .unwrap()
                .ends_with(".claude/skills/nyxid")
        );
        assert!(
            skill_install_dir(&home, AiToolTarget::Codex)
                .unwrap()
                .ends_with(".codex/skills/nyxid")
        );
        assert!(
            skill_install_dir(&home, AiToolTarget::Openclaw)
                .unwrap()
                .ends_with(".openclaw/skills/nyxid")
        );
    }

    #[test]
    fn is_partial_install_returns_false_when_no_skill_dir() {
        let home = temp_home();
        assert!(!is_partial_install(&home, AiToolTarget::ClaudeCode));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn record_attempt_creates_marker_file() {
        let home = temp_home();
        record_attempt(&home);
        assert!(home.join(COOLDOWN_FILE).exists());
        fs::remove_dir_all(&home).unwrap();
    }
}
