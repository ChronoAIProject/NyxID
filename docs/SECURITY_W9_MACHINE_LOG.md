# W9.4 machine separation — stream D

## D-0 — inventory (PM approved)
- Read required machine/security docs; inventoried setup, context/runtime, assignment, server and UI paths without code changes.
- PM approved D ownership and automatic / require-separated / explicit-shared-legacy contract; existing assignments retain their mode.
- PM requires a verdict after each D-1–D-5 batch and an assignment-path decision table before D-3 implementation; no commit for the read-only inventory.

## D-1 — capability evidence and selector (awaiting PM verdict)
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
