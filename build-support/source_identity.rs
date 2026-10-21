//! Shared build-time source identity. No network or third-party build dependency.
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn git(directory: &Path, arguments: &[&str]) -> Option<String> {
    let result = Command::new("git")
        .current_dir(directory)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(arguments)
        .output()
        .ok()?;
    if !result.status.success() {
        return None;
    }
    Some(String::from_utf8(result.stdout).ok()?.trim().to_owned())
}

fn commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn epoch(value: &str) -> Result<u64, &'static str> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|c| c.is_ascii_digit())
    {
        return Err("source epoch must be canonical decimal seconds");
    }
    let value = value.parse::<u64>().map_err(|_| "invalid source epoch")?;
    if value > 253_402_300_799 {
        return Err("source epoch exceeds supported date range");
    }
    Ok(value)
}

fn input(name: &str) -> Result<Option<String>, &'static str> {
    match env::var(name) {
        Ok(value) if value.is_empty() => Err("explicit source identity inputs cannot be empty"),
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(_) => Err("source identity inputs must be UTF-8"),
    }
}

fn watch(path: &Path) {
    let text = path.to_str().expect("source paths must be UTF-8");
    assert!(
        !text.chars().any(char::is_control),
        "source paths cannot contain control characters"
    );
    println!("cargo:rerun-if-changed={text}");
}

fn watch_git(directory: &Path) {
    // Linked worktrees have a .git FILE. Resolve metadata via Git rather than
    // assuming package-local .git directories. Refs may also be packed.
    for name in ["HEAD", "index", "packed-refs", "refs"] {
        if let Some(path) = git(directory, &["rev-parse", "--git-path", name]) {
            let path = directory.join(path);
            if path.exists() {
                watch(&path);
            }
        }
    }
    if let Some(root) = git(directory, &["rev-parse", "--show-toplevel"]) {
        let root = Path::new(&root);
        let result = Command::new("git")
            .current_dir(root)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .args([
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "--full-name",
                "-z",
            ])
            .output()
            .expect("list source inputs");
        assert!(result.status.success(), "could not enumerate source inputs");
        // Watching the actual files also catches edits made without git add.
        // Cargo's ignored build outputs/node_modules are absent from this list.
        for value in result.stdout.split(|&b| b == 0).filter(|s| !s.is_empty()) {
            let path = std::str::from_utf8(value).expect("source paths must be UTF-8");
            watch(&root.join(path));
        }
    }
}

pub fn emit() {
    for name in ["NYXID_GIT_HASH", "SOURCE_DATE_EPOCH", "TARGET"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let directory =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    // file!() may already start with the package's repository-relative path.
    // Cargo watches build.rs and its Rust module dependencies automatically;
    // adding a second, guessed path can point to a nonexistent app/app/... and
    // force every build to rerun, concealing missing source watches.
    let package = env::var("CARGO_PKG_NAME").expect("Cargo package name");
    let version = env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    let target = env::var("TARGET").expect("Cargo target");
    for value in [&package, &target] {
        assert!(
            !value.is_empty()
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c)),
            "invalid package/target identity"
        );
    }
    assert!(
        !version.is_empty()
            && version
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-+.".contains(&c)),
        "invalid package version identity"
    );
    let explicit = input("NYXID_GIT_HASH").expect("invalid source commit input");
    if let Some(value) = &explicit {
        assert!(
            commit(value),
            "source commit must be full lowercase hexadecimal"
        );
    }
    let supplied_epoch = input("SOURCE_DATE_EPOCH")
        .expect("invalid source epoch input")
        .map(|value| epoch(&value).expect("invalid source epoch"));
    let git_commit = git(&directory, &["rev-parse", "HEAD"]).filter(|v| commit(v));
    let (resolved_commit, resolved_epoch, state) = if let Some(head) = git_commit {
        watch_git(&directory);
        let date = git(&directory, &["show", "-s", "--format=%ct", "HEAD"])
            .and_then(|v| epoch(&v).ok())
            .expect("Git commit epoch");
        if let Some(value) = &explicit {
            assert_eq!(value, &head, "supplied source commit differs from checkout");
        }
        if let Some(value) = supplied_epoch {
            assert_eq!(value, date, "supplied epoch differs from commit");
        }
        let status = git(
            &directory,
            &["status", "--porcelain", "--untracked-files=normal"],
        )
        .expect("could not determine source state");
        (
            head,
            Some(date),
            if status.is_empty() {
                // A cached build script cannot observe every newly created
                // untracked input used by another package's build script.
                // Never certify a working tree as clean from this cache.
                // Official producers validate/export exact source separately.
                "git-unverified"
            } else {
                "git-dirty"
            },
        )
    } else if let Some(value) = explicit {
        (
            value,
            Some(supplied_epoch.expect("exported source requires commit epoch")),
            "exported",
        )
    } else {
        assert!(
            supplied_epoch.is_none(),
            "epoch without a source commit cannot identify exported source"
        );
        ("unknown".into(), None, "unknown")
    };
    let display = if state == "git-dirty" {
        format!("{resolved_commit}-dirty")
    } else {
        resolved_commit.clone()
    };
    println!("cargo:rustc-env=NYXID_GIT_HASH={display}");
    println!("cargo:rustc-env=TARGET={target}");
    let date = resolved_epoch
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".into());
    let json = format!(
        "{{\"schema\":\"nyxid.build-identity.v1\",\"package\":\"{package}\",\"package_version\":\"{version}\",\"target\":\"{target}\",\"commit\":\"{resolved_commit}\",\"source_date_epoch\":{date},\"source_state\":\"{state}\"}}"
    );
    let marker = format!("NYXID_BUILD_IDENTITY_V1\0{json}\0NYXID_BUILD_IDENTITY_END\0");
    let generated = format!(
        r#"
#[used]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__nyxid_id"))]
#[cfg_attr(target_os = "linux", unsafe(link_section = ".nyxid.identity"))]
pub static NYXID_BUILD_IDENTITY: [u8; {}] = {:?};
pub fn retain() {{ std::hint::black_box(&NYXID_BUILD_IDENTITY); }}
"#,
        marker.len(),
        marker.as_bytes()
    );
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"))
            .join("nyxid_build_identity.rs"),
        generated,
    )
    .expect("write source identity");
}
