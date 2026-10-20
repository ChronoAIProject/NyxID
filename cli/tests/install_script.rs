#[cfg(not(unix))]
#[test]
fn installer_requires_unix_shell() {
    eprintln!("install_script: Unix shell harness is not applicable on this platform");
}

#[cfg(unix)]
mod unix {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
        path::{Path, PathBuf},
        process::{Command, Output},
    };

    struct Harness {
        root: tempfile::TempDir,
        home: PathBuf,
        bin: PathBuf,
    }
    fn executable(path: &Path, contents: &str) {
        fs::write(path, contents).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    impl Harness {
        fn new(home_name: &str) -> Self {
            let root = tempfile::tempdir().unwrap();
            let home = root.path().join(home_name);
            let bin = root.path().join("stubs");
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(&bin).unwrap();
            fs::create_dir(root.path().join("tmp")).unwrap();
            executable(
                &bin.join("uname"),
                "#!/bin/sh\ncase \"$*\" in -m) echo x86_64;; *) echo \"${TEST_OS:-Linux}\";; esac\n",
            );
            executable(
                &bin.join("curl"),
                r#"#!/bin/sh
[ "${TEST_MODE:-}" != download_failed ] || exit 22
out=
while [ "$#" -gt 0 ]; do
  case "$1" in -o) shift; out="$1";; esac
  shift
done
[ -n "$out" ] || exit 99
cp "$TEST_ROOT/fake-installer" "$out"
"#,
            );
            fs::write(
                root.path().join("fake-installer"),
                r#"#!/bin/sh
set -eu
[ "${TEST_MODE:-}" != run_failed ] || exit 1
[ "${NYXID_CLI_NO_MODIFY_PATH:-}" = 1 ] || exit 99
# The real 0.30.0 precedence, including inherited controls, is deliberate.
dir="${NYXID_CLI_INSTALL_DIR:-${CARGO_DIST_FORCE_INSTALL_DIR:-${NYXID_CLI_UNMANAGED_INSTALL:?}}}"
printf '%s\n' "$dir" > "$TEST_ROOT/staging-path"
mkdir -p "$dir"
[ "${TEST_MODE:-}" != missing ] || exit 0
cp "$TEST_ROOT/fake-nyxid" "$dir/nyxid"
[ "${TEST_MODE:-}" != not_executable ] || exit 0
chmod 755 "$dir/nyxid"
"#,
            )
            .unwrap();
            fs::write(root.path().join("fake-nyxid"), "#!/bin/sh\ncase \"${TEST_MODE:-}\" in bad_version) echo unknown;; bad_binary) exit 1;; *) echo 'nyxid 0.37.1';; esac\n").unwrap();
            executable(
                &bin.join("cargo"),
                r#"#!/bin/sh
printf '%s\n' "$*" >> "$TEST_ROOT/cargo-calls"
[ "$1" != --version ] || { echo 'cargo fixture'; exit 0; }
while [ "$#" -gt 0 ]; do
  if [ "$1" = --root ]; then shift; dir="$1"; fi
  shift
done
mkdir -p "$dir/bin"
cp "$TEST_ROOT/fake-nyxid" "$dir/bin/nyxid"
chmod 755 "$dir/bin/nyxid"
"#,
            );
            executable(
                &bin.join("rustup"),
                "#!/bin/sh\necho invoked >> \"$TEST_ROOT/rustup-calls\"\nexit 99\n",
            );
            Self { root, home, bin }
        }
        fn run(&self, env: &[(&str, &str)]) -> Output {
            let mut command = Command::new("/bin/bash");
            command
                .arg(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../skills/nyxid/scripts/install.sh"),
                )
                .env_clear()
                .env("HOME", &self.home)
                .env("SHELL", "/bin/bash")
                .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
                .env("TEST_ROOT", self.root.path())
                .env("TMPDIR", self.root.path().join("tmp"))
                // These must not escape unmanaged staging.
                .env(
                    "NYXID_CLI_INSTALL_DIR",
                    self.root.path().join("wrong-app-dir"),
                )
                .env(
                    "CARGO_DIST_FORCE_INSTALL_DIR",
                    self.root.path().join("wrong-global-dir"),
                );
            for (key, value) in env {
                command.env(key, value);
            }
            let output = command.output().unwrap();
            assert_eq!(
                fs::read_dir(self.root.path().join("tmp")).unwrap().count(),
                0,
                "staging not cleaned"
            );
            assert!(!self.root.path().join("wrong-app-dir").exists());
            assert!(!self.root.path().join("wrong-global-dir").exists());
            output
        }
        fn assert_install(
            &self,
            output: Output,
            root: &Path,
            active: &Path,
            rc: &str,
            source: bool,
        ) {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let target = root.join("v0.37.1/nyxid");
            assert_eq!(fs::read_link(active).unwrap(), target);
            assert_eq!(
                fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                0o755
            );
            assert_eq!(
                Command::new(active)
                    .arg("--version")
                    .output()
                    .unwrap()
                    .stdout,
                b"nyxid 0.37.1\n"
            );
            let contents = fs::read_to_string(self.home.join(rc)).unwrap();
            assert!(
                contents.contains(active.parent().unwrap().to_str().unwrap()),
                "{contents}"
            );
            assert_eq!(contents.matches("# NyxID CLI").count(), 1);
            assert_eq!(self.root.path().join("cargo-calls").exists(), source);
            assert!(!self.root.path().join("rustup-calls").exists());
        }
        fn defaults(&self) -> (PathBuf, PathBuf) {
            (
                self.home.join(".local/share/nyxid/versions"),
                self.home.join(".local/bin/nyxid"),
            )
        }
    }

    #[test]
    fn installer_default_paths() {
        let h = Harness::new("home");
        let (root, active) = h.defaults();
        h.assert_install(h.run(&[]), &root, &active, ".bashrc", false);
    }
    #[test]
    fn installer_custom_home() {
        let h = Harness::new("custom home");
        let (root, active) = h.defaults();
        h.assert_install(h.run(&[]), &root, &active, ".bashrc", false);
    }
    #[test]
    fn installer_custom_install_root_only() {
        let h = Harness::new("home");
        let (_, active) = h.defaults();
        let root = h.root.path().join("custom versions");
        h.assert_install(
            h.run(&[("NYXID_INSTALL_ROOT", root.to_str().unwrap())]),
            &root,
            &active,
            ".bashrc",
            false,
        );
    }
    #[test]
    fn installer_custom_active_symlink_only() {
        let h = Harness::new("home");
        let (root, _) = h.defaults();
        let active = h.root.path().join("custom bin/tool");
        h.assert_install(
            h.run(&[("NYXID_ACTIVE_SYMLINK", active.to_str().unwrap())]),
            &root,
            &active,
            ".bashrc",
            false,
        );
        assert!(!h.home.join(".local/bin/nyxid").exists());
    }
    #[test]
    fn installer_both_custom_paths() {
        let h = Harness::new("home");
        let root = h.root.path().join("versions");
        let active = h.root.path().join("bin/tool");
        h.assert_install(
            h.run(&[
                ("NYXID_INSTALL_ROOT", root.to_str().unwrap()),
                ("NYXID_ACTIVE_SYMLINK", active.to_str().unwrap()),
            ]),
            &root,
            &active,
            ".bashrc",
            false,
        );
    }
    #[test]
    fn installer_xdg_data_home() {
        let h = Harness::new("home");
        let (_, active) = h.defaults();
        let data = h.root.path().join("xdg");
        h.assert_install(
            h.run(&[("XDG_DATA_HOME", data.to_str().unwrap())]),
            &data.join("nyxid/versions"),
            &active,
            ".bashrc",
            false,
        );
    }
    #[test]
    fn successful_installer_with_invalid_artifact_never_falls_back() {
        for (mode, message) in [
            ("missing", "missing or not executable"),
            ("not_executable", "missing or not executable"),
            ("bad_version", "could not determine"),
            ("bad_binary", "failed to run"),
        ] {
            let h = Harness::new("home");
            let output = h.run(&[("TEST_MODE", mode)]);
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains(message));
            assert!(!h.root.path().join("cargo-calls").exists());
            assert!(!h.root.path().join("rustup-calls").exists());
        }
    }
    #[test]
    fn failed_download_or_installer_uses_versioned_source_fallback() {
        for mode in ["download_failed", "run_failed"] {
            let h = Harness::new("home");
            let (root, _) = h.defaults();
            let active = h.root.path().join("custom/tool");
            h.assert_install(
                h.run(&[
                    ("TEST_MODE", mode),
                    ("NYXID_ACTIVE_SYMLINK", active.to_str().unwrap()),
                ]),
                &root,
                &active,
                ".bashrc",
                true,
            );
        }
    }
    #[test]
    fn installer_rerun_replaces_existing_symlink_regular_file_and_same_version() {
        for legacy in [true, false] {
            let h = Harness::new("home");
            let (root, active) = h.defaults();
            fs::create_dir_all(active.parent().unwrap()).unwrap();
            if legacy {
                executable(&active, "#!/bin/sh\necho 'nyxid 0.1.0'\n");
            } else {
                symlink("/missing/old/nyxid", &active).unwrap();
            }
            h.assert_install(h.run(&[]), &root, &active, ".bashrc", false);
            h.assert_install(h.run(&[]), &root, &active, ".bashrc", false);
        }
    }
    #[test]
    fn installer_path_line_follows_active_directory_for_each_shell() {
        for (shell, os, rc) in [
            ("/bin/zsh", "Darwin", ".zshrc"),
            ("/bin/bash", "Darwin", ".bash_profile"),
            ("/bin/fish", "Linux", ".config/fish/config.fish"),
            ("/bin/sh", "Linux", ".profile"),
        ] {
            let h = Harness::new("home");
            let (root, _) = h.defaults();
            let active = h.root.path().join("elsewhere/tool");
            h.assert_install(
                h.run(&[
                    ("SHELL", shell),
                    ("TEST_OS", os),
                    ("NYXID_ACTIVE_SYMLINK", active.to_str().unwrap()),
                ]),
                &root,
                &active,
                rc,
                false,
            );
        }
    }
    #[test]
    fn installer_relocation_failure_is_fatal_without_source_fallback() {
        let h = Harness::new("home");
        let blocked = h.root.path().join("blocked");
        fs::write(&blocked, "file").unwrap();
        let active = blocked.join("nyxid");
        let output = h.run(&[("NYXID_ACTIVE_SYMLINK", active.to_str().unwrap())]);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("could not create install directories")
        );
        assert!(!h.root.path().join("cargo-calls").exists());

        let h = Harness::new("home");
        let (root, active) = h.defaults();
        fs::create_dir_all(root.join("v0.37.1/nyxid")).unwrap();
        let output = h.run(&[]);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("versioned binary path is a directory")
        );
        assert!(!active.exists());
        assert!(!h.root.path().join("cargo-calls").exists());
    }
}
