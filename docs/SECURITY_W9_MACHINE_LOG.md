# W9.4 machine separation — stream D

## D-0 — inventory (PM approved)
- Read required machine/security docs; inventoried setup, context/runtime, assignment, server and UI paths without code changes.
- PM approved D ownership and automatic / require-separated / explicit-shared-legacy contract; existing assignments retain their mode.
- PM requires a verdict after each D-1–D-5 batch and an assignment-path decision table before D-3 implementation; no commit for the read-only inventory.

## D-1 — capability evidence and selector (PM requested R1–R3 revisions)
- Implementation commit: `9309a9132b42bc00ff3f3761b86ed3c62d762b72` (six D-owned files); this log follows in a documentation commit.
- Versioned optional evidence selects new separated mode only on complete confirmed support; automatic/required never silently fall back; explicit legacy remains a distinct choice.
- Linux runtime observes ABI/users/config and disposable child seccomp, Landlock and mapped browser namespace probes; macOS reports unsupported; persisted assignments and registration are untouched.
- Rust commands used `CARGO_TARGET_DIR=/tmp/nyxid-w94-stream-d CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`.
- `cargo test --locked -p nyxid-machine context::tests:: -- --test-threads=1`: 12 passed / 0 failed / 0 ignored.
- `cargo test --locked -p nyxid-cli --bin nyxid node::machine::separation::tests:: -- --test-threads=1`: 5 passed / 0 failed / 0 ignored, including native macOS and mocked denial cases; two A-owned warnings.
- `cargo clippy --locked -p nyxid-machine --all-targets -- -D warnings`: exit 0.
- `cargo clippy --locked -p nyxid-cli --bin nyxid -- -D warnings`: exit 101, 22 diagnostics exclusively in A-owned config/encryption/legacy/secret/storage paths; no suppression or edits there.
- Initial `cargo test --locked -p nyxid-cli --lib --features node-proxy-test machine::separation::tests:: -- --test-threads=1`: exit 101 before tests, A-owned storage.rs:967/971 cannot resolve `crate::node::encryption` in test-library wiring.
- Scoped `rustfmt --check --edition 2024 --config skip_children=true` and staged `git diff --check` passed for all six implementation files.
- Linux cfg compilation, actual positive/negative mechanisms and real Chromium/runtime acceptance remain D-5 gates; the namespace probe alone is not browser-health or VM-isolation evidence.
- Read-only SSH discovered running Lima `nyxid-security-validation`: Ubuntu 24.04.5, Linux 7.0.0-38-generic aarch64; no guest mutation/build, browser start or policy change performed.

## D-1-R — R1–R3 revisions (awaiting PM verdict)
- Implementation: `52c959edf392c88d3e2c35122c267ae156f2a7c5`; five D-owned source paths, including one Linux unused-import fix; registration and other streams' files untouched.
- R1: probes explicitly own kill-on-drop children, kill the process group on timeout/error, and wait/reap; the five-second timeout regression verifies ESRCH and ECHILD on macOS and Linux.
- R2: evidence v2 replaces pre-exec namespace inference with the configured Chromium's fresh-profile `chrome://sandbox` diagnostics under both actual browser identities/NNP; four sandbox rows must all confirm; v1 is insufficient.
- Rust commands use `CARGO_TARGET_DIR=/tmp/nyxid-w94-stream-d CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`; scoped rustfmt and diff checks pass.
- macOS: `cargo test --locked -p nyxid-machine context::tests:: -- --test-threads=1` 12/0/0; CLI `node::machine::` 112/0/2 existing manual benchmarks ignored; final separation filter 6/0/0 (passed/failed/ignored).
- macOS strict Clippy: machine `--all-targets` passes; CLI `--bin nyxid` still has 22 known A-owned diagnostics and none in D paths; no lint suppression.
- Linux independent archive `0aa9b193d578647331f9e65bb28b97478c4d75ec` plus exact D revisions at `/tmp/nyxid-w94-d1-r1` (no writable mount/foreign WIP): `cargo build --locked -p nyxid-machine` and CLI `--bin nyxid` pass; strict Clippy for machine `--all-targets` and CLI `--bin nyxid` both pass.
- Linux tests: machine `context::tests::` 12/0/0; CLI separation 5/0/0; process 5/0/0; context-runtime 3/0/0 run as root so DAC/store bodies execute.
- Guest as-is Evidence: v2/linux, ABI=8, separate_users=false, managed_browser_configured=false, remaining mechanism probes=null, apparmor_userns_restricted=true; unavailable: separated_requires_separate_users.
- After documented `setup --machine --computer --separate-users --profile w94 --root /srv/nyxid-w94-workspace` against loopback registration fixture: all five mechanism/config booleans=true, ABI=8, AppArmor restriction=true; rebuilt daemon emits separated.available=true and computer_ready=true.
- Negative checks: snap wrapper and actual Chromium without executable AppArmor attachment both give managed_browser_sandbox=false; restoring the enforcing attachment restores true; /bin/true namespace probe fails even when Chromium succeeds; global sysctl stays 1.
- Guest changes: snapd + Chromium 155.0.8059.79; documented X11/browser prerequisites plus unzip/python3-websockets; setup's users/policies/pinned cua/supervisor/updater; /usr/bin/chromium links to actual snap binary with an enforcing per-executable userns profile. Exploratory standalone Chromium 151 remains in /opt; its unused profile is unloaded. All six fixture services stopped/disabled; no browser-user survivors.
- Remaining: saved_login_ready=false (managed extension handshake failure); browser lifecycle/adversarial acceptance and hosted amd64 remain D-5 gates. D-3 table is proposed only; external-grant constructor lies outside approved ownership. No D-2/D-3 implementation before verdict.
