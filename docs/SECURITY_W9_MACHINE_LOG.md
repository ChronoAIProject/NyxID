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

## D-2 — setup intent and probe cleanup (awaiting PM verdict)
- D-1-R accepted by PM; D-2 implementation: `f5d41ef497b55c7e49bc298a27de4fcd4f80993e` (eight D source files and NYXID_NODE.md); evidence log follows separately.
- `--separation automatic|require-separated|shared-legacy`: new setups record automatic; enabled older profiles preserve absent intent; explicit or stored choices persist; existing assignments are untouched.
- Unsupported automatic/required configurations fail before pairing where known; real post-provision probes gate service startup. Failed intent stays recorded. Registration call/signature and A-owned paths are unchanged.
- Probe groups are swept after successful waits as well as failure/timeout; a drop guard covers cancellation. New successful-parent/grandchild regression checks ESRCH; timeout regression checks ESRCH/ECHILD.
- Typed timeouts survive observation as diagnostic `probe_timed_out`; support reason is `separated_probe_timeout` with five-second/cold-start/rerun guidance. Positive v2 evidence semantics are unchanged; no version bump or timeout increase.
- Rust commands use `CARGO_TARGET_DIR=/tmp/nyxid-w94-stream-d CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`; scoped rustfmt and `git diff --check` pass.
- macOS `cargo test --locked -p nyxid-machine {context,config}::tests:: -- --test-threads=1`: 13+4 passed; CLI `node::machine::{setup,process,separation}::tests::`: 6+5+7 passed (zero failed/ignored).
- Linux same shared filters: 13+4 passed; CLI setup/process/separation: 6+6+6 passed; context-runtime filter run as root: 3 passed (zero failed/ignored).
- Strict Clippy: machine `--all-targets` passes both OSes; CLI `--bin nyxid --tests -- -D warnings` passes Linux; macOS exits 101 with 22 production/7 test diagnostics, all A-owned and none D-owned; no suppressions.
- Linux `cargo build --locked -p nyxid-cli --bin nyxid` passes in independent `/tmp/nyxid-w94-d1-r1` archive plus exact eight D files; SHA256 matches host sources; no writable host mount or foreign WIP copied.
- Real guest setup: absent old intent retained; automatic and required confirm separated; explicit legacy records shared_legacy with full warning; omitted reruns retain automatic/legacy. Six setup cases pass.
- Disposable hanging-browser setup exits 1 with full timeout guidance, retains automatic and leaves no probe PID. Rebuilt daemon advertises require_separated, v2/linux/ABI8, all five mechanism booleans true, probe_timed_out=false, AppArmor restriction=true and support=true. Fixture sleeper removed; fixture daemon/server stopped; six services stopped; no browser-user processes survive.
- Integration: browserless `cli/container/machine-node-only-entrypoint.sh:59` needs C-coordinated informed owner selection forwarded to setup; PM notified, file untouched. Backend/UI assignments remain D-3/D-4; native lifecycle/fill and hosted amd64 remain D-5 gates. Await D-2 verdict.

## D-2-R — informed interactive selection and Docker propagation (awaiting PM verdict)
- Implementation: `ce76f9ad8`; six owned source/entrypoint/doc paths; explicit pathspec commit; registration call/signature and other streams' paths untouched.
- Automatic failures prompt only when stdin and stderr are TTYs, after the observed reason and full node-token/signing-secret/stored-credential/shared-kernel/container-or-VM warning; only `y` selects explicit shared_legacy. No/empty/EOF fail; required never prompts; noninteractive errors name `--separation shared-legacy`.
- Pending pre-pairing intent is private/atomic `machine-separation-choice.json`, without registration credentials; successful registration moves intent to config.toml. Existing unselected/legacy profiles and stored assignments retain their meaning.
- Docker ownership extension: cli/src/cli.rs, cli/src/commands/node.rs, cli/container/machine-node-only-entrypoint.sh. Start/restart forward explicit NYXID_MACHINE_SEPARATION; absent new choice is automatic, never legacy; existing containers retain their choice.
- Docker preflight runs the real probes in a disposable image with the startup seccomp/init/shm settings, no identity/workspace mounts or token; missing/invalid evidence fails closed or requires informed consent. Actual setup probes again. Node-only first setup validates the env choice and forwards it; docs recommend a separate container per agent.
- Rust commands use `CARGO_TARGET_DIR=/tmp/nyxid-w94-stream-d CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`; scoped rustfmt, shell syntax and owned-path diff checks pass.
- macOS and Linux: `cargo test --locked -p nyxid-cli --bin nyxid node::machine::setup::tests:: -- --test-threads=1` 12/0/0 each; rebuilt CLI test binaries `commands::node::tests::docker_` 9/0/0 and exact Docker selection parser 1/0/0 each (passed/failed/ignored).
- Both OSes: `cargo test --locked -p nyxid-machine context::tests:: -- --test-threads=1` 13/0/0; machine `clippy --all-targets -- -D warnings` and CLI `build --bin nyxid` pass. Linux CLI `clippy --bin nyxid --tests -- -D warnings` passes; macOS has known A-owned 22 production/7 test diagnostics, none in D paths, no suppressions.
- `python3 /tmp/nyxid-w94-d2r-live-check.py /tmp/nyxid-w94-stream-d/debug/nyxid`: 13/0 live PTY/non-TTY and Docker-command-fixture cases on each OS; yes records legacy, decline retains automatic, required never prompts, positive evidence forwards automatic/required, existing containers preserve intent. Docker fixture is not engine/runtime evidence.
- Linux `sudo python3 /tmp/nyxid-w94-d2r-guest-extra.py`: 6/0 real setup cases; post-provision probe failure yes/no/required preserves the expected config choice; node-only absent/automatic/required refuse before registration with separated_requires_managed_browser.
- Guest uses fresh committed archive at /tmp/nyxid-w94-d2r plus exactly five revised source/entrypoint files, SHA256 matched to host; no writable shared mount or foreign WIP. Main fixture config unchanged, six services stopped, AppArmor userns restriction remains 1; temporary setup profiles self-cleaned.
- Remaining: actual full/node-only Docker image startup and probe lifecycle require hosted Docker CI via C; native lifecycle/fill and hosted amd64 remain D-5 gates. D-3 is not started; await PM verdict on this revision.

## D-2-C — hosted Docker CLI contract follow-up
- Implementation: `d99acb094`; explicit pathspecs for cli/src/commands/node.rs, cli/src/cli.rs and docs/NYXID_NODE.md only.
- New machine Docker containers explicitly use self-hosted registration (`NYXID_MACHINE_MANAGED=0`) and the reviewed separation choice; machine stop retains the container for start/restart preservation.
- Config is /var/lib/nyxid-machine/node/config.toml, [machine].separation_choice; the Docker CLI never mounts or writes the host profile. Refused fresh selection precedes persistent container/volume creation.
- Linux independent /tmp/nyxid-w94-d2r snapshot: `cargo test --locked -p nyxid-cli --bin nyxid commands::node::tests::docker_ -- --test-threads=1` 9/0/0 (passed/failed/ignored).
- Linux `cargo clippy --locked -p nyxid-cli --bin nyxid --tests -- -D warnings` and `cargo build --locked -p nyxid-cli --bin nyxid` pass; scoped macOS rustfmt and owned diff checks pass.
- `python3 /tmp/nyxid-w94-d3-live-check.py /tmp/nyxid-w94-stream-d/debug/nyxid`: 18/0 live CLI/PTY and Docker-command-fixture cases, including closed stdin, self-hosted env and stop/start/restart preservation.
- These fixture checks are not Docker engine evidence. C's hosted job must verify image startup/TLS, registration, actual anonymous-volume cleanup and no named volumes after refusal; cleanup explicitly removes retained machine containers.
- Registration and stream A's evolving secret-storage implementation are untouched; credential absence must include the complete fresh state/profile root and applicable keychain/vault stores.

## D-3 — assignment defaults, compatibility and consent

- Implementation: `2fe081187`; exactly 34 D paths, including the three PM-approved shared adapters. No registry or foreign-file hunks.
- Immutable `machine-separation-defaults-v1` marker and atomic snapshot freeze enabled-node ceilings and org-admin reachability through the existing ensure_indexes hook; no main/config/db/errors changes.
- Owner cards carry the full legacy warning; new external grants require explicit legacy consent and acknowledgement. API/UI/tool/watch results expose actionable selection_required; watches settle once.
- After PM's build-policy change, every Cargo invocation used the shared tree and CARGO_TARGET_DIR=/tmp/nyxid-shared-target CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0, without --config overrides; locks were awaited.
- `cargo test --locked -p nyxid --no-default-features --bin nyxid-server --no-run`: pass in 13m50s; runtime filters used that shared build's pinned executable against the local Mongo replica set.
- Eight Mongo/runtime filters: access 42, integration 24, external-grant 51, external-service 34, OAuth 12, handler-tools 1, service-tools 2, NyxBot guard 1 = 167 passed / 0 failed / 1 pre-existing ignored dispatch benchmark.
- Initial isolated run was 142 passed / 2 failed / 0 ignored; both fixture failures were repaired (explicit legacy intent and specialist grant_to). No tests were deleted or newly ignored.
- Frontend targeted Vitest: 115 passed / 0 failed / 0 ignored in ten files; TypeScript and scoped ESLint pass. D sources remain unchanged since validation.
- `cargo clippy --locked -p nyxid --no-default-features --bin nyxid-server --tests -- -D warnings`: exit 101, only B-owned audit_runtime/mod.rs:9,11 unused exports and intent_tests.rs:325 field_reassign_with_default (2 production/2 test diagnostics); no D diagnostic.
- `cargo clippy --locked -p nyxid-cli --bin nyxid --tests -- -D warnings`: exit 101, 7 A-owned unused-code diagnostics in secret_snapshot/secret_transaction/storage; no D diagnostic or suppression.
- Pinned shared-target CLI test executable, built after unchanged D CLI inputs: commands::node::tests::docker_ --test-threads=1 passes 9/0/0. D's duplicate Cargo test was stopped only while waiting for the lock, before compilation.
- Full review patch and explicit commit pathset were checked. Actual Docker image/TLS/volume cleanup remains C's hosted job; D-4/D-5 have not started. Awaiting PM D-3 verdict.

## D-4 — setup review, support display and rollout docs

- Implementation: `05cd07136`; exactly 12 approved frontend/doc paths, explicit commit pathspec; no registry or other-stream hunks.
- Setup review preserves automatic/required/legacy intent across native and Docker commands; a Radix empty reset event no longer erases a prefilled choice. Legacy review requires fresh acknowledgement and changing mode clears consent.
- Raw profiles require complete v2 evidence plus authority v2 for confirmed support; old booleans, absent/partial evidence and unknown versions show unknown. Failed prerequisites and five-second probe timeout have distinct actionable messages.
- Machine-level sharing is labelled per agent; existing legacy modes remain visible even with confirmed support. Pending selection stays blocked and contradictory support/reason reports do not enable the separated action.
- Three machine docs cover actual Chromium/AppArmor probes, macOS/browserless limits, explicit intent/config values, immutable cutover snapshot, replica rollout, external legacy consent and shared-kernel residuals; no VM-boundary claim.
- `npm test -- --maxWorkers=2` with machine-setup, machines schema, machine-isolation, machine-settings, machine-activity and machine-capabilities files: 100 passed / 0 failed / 0 ignored across six files.
- Initial targeted run: 95 passed / 1 failed; the new legacy-prefill regression exposed the empty-select reset bug, fixed before the final run. No test removal, ignore, timeout change or lint suppression.
- `npx tsc -b --pretty false`, scoped `npx eslint` on all nine changed frontend files, and `git diff --check`: pass. No Rust changes or Cargo invocation in this batch.
- D-5 read-only reconnaissance: Lima nyxid-security-validation is running, Ubuntu 24.04.5/aarch64, AppArmor userns restriction=1; earlier D checkouts/harness remain. No guest mutation/build. Real context acceptance and hosted amd64 remain D-5 gates; await PM D-4 verdict.

## D-5-CI-0 — explicit Docker fixture selection
- Commit `421d01066`: granted node_docker.rs fixture explicitly accepts shared-legacy; all original CA-mount/seccomp assertions retained and choice propagation asserted.
- Container E2E sets NYXID_MACHINE_SEPARATION=require-separated through setup’s existing environment contract; no entrypoint argument/parser or Dockerfile changes.
- Shared-target cargo test --locked -p nyxid-cli --test node_docker -- --test-threads=1: 6 passed / 0 failed / 0 ignored; rustfmt, node --check and scoped git diff --check pass.
- Hosted container execution and Chromium timeout calibration remain pending; this fixture commit does not establish Docker/runtime acceptance.

## D-5 — native acceptance in progress; browser probe revision pending
- Source commits `001fae5aa` and `7bf31eadb`: native profile traversal preserves private node state; context useradd requests no subordinate UID/GID allocations. Explicit owned pathspecs only.
- Guest archive/committed deltas through 7bf31eadb, native Ubuntu 24.04.5/aarch64/kernel7.0/ABI8; no writable host mount. AppArmor restriction=1 and executable attachment retained.
- Native run: fresh required separate-users setup/evidence, two-context commands/files/reconnect, actual daemon restart/journal/private-file persistence pass; browser cold-start fails; login/browser/display/socket/quarantine/final-renderer acceptance not reached.
- Linux unit filters on traversal revision: context 13 + config 4 + setup 12 + separation 6 + process 6 + context-runtime 4 + context-policy 1 = 46 pass/0 fail/0 ignored; root context-runtime filter on final 7bf31eadb source also passes 4/0/0 (overlap, not additive).
- Full existing adversarial check.py --disposable --native-profile: 44 pass/0 fail, no tool failures; negative controls and documented metadata/shm/pathname-socket residuals retained. Temporary fixture dependency correction imports committed production modules; checked-in Cargo approval pending.
- Strict Linux machine Clippy passes; CLI Clippy fails with 7 A-owned dead-code diagnostics in secret_snapshot/secret_transaction/storage, none D-owned; no suppression.
- Guest mutations: copied archives/cache/ws, installed libnss3-tools/x11-utils/g++/nodejs/npm/cargo/rustc dependencies, created disposable native profiles/role users and m13 test users without subids; diagnostic UID21999 removed; global managed resources refreshed by setup.
- Root-owned /etc/chromium-browser -> /etc/chromium alias exposes the same protected policies to real Snap Chromium. Wrappers restored; fixture daemons/browser processes stopped; no AppArmor/sysctl/subid relaxation.
- Root cause: policy-loaded --dump-dom refuses DevTools, emits no DOM and times out even at 120s. No production bound raised; v3 OS-observation correction proposed to PM, not implemented.
- Prototype fresh-profile timings: 24 positive namespace/NNP/seccomp observations, median0.128s/max0.289s; 4 stricter descendant/executable/additional-filter controls pass, max1.445s. Hosted timings and production calibration remain pending.

## D-5 revision — v3 OS observation, package policies and native acceptance (PM accepted; hosted gates pending)
- Code: e2832737e (approved fixture manifest/lock only); 83f061e34 (v3/R1/R2); follow-ups through 9142c8d2c; docs 70432e193. Every commit uses explicit D pathspecs.
- Actual renderers must match executable, all UIDs/GIDs, NNP, extra seccomp filters, ancestry and distinct user/PID/net namespaces, then survive recheck; group kill/reap is retained on all paths; v1/v2/unknown remain insufficient for new automatic/required selections.
- R1 resolves package-backed Debian/Snap/Chrome policy/native-host layouts; unknown/writable metadata fails closed; the macOS canonical-root and ordinary-login umask fixture regressions pass. The temporary Snap policy alias is removed.
- R2 requires the real secure identity's administrator DevTools refusal and no listener; removing policy produces managed_browser_policy=false/sandbox=null, restoring its exact inode restores support. Timeouts retain separated_probe_timeout (8305b0be8).
- Native denials exposed legacy browser-home access through policy groups (fixed 9e5eae634) and executable ACL loss on reinstall (fixed a128a3fa5); root regressions preserve legacy UID access while denying context UIDs and retaining every resource denial.
- Linux: cargo build --locked -p nyxid-cli --bin nyxid PASS; strict CLI+machine Clippy PASS on 9142c8d2c; root node::machine:: on a128a3fa5 =135/0/0; shared context:: =13/0/0; latest standalone production browser_layout:: =1/0/0 (overlap).
- macOS shared-target cargo test -p nyxid-cli --bin nyxid node::machine:: =126/0/2 existing manual ignores; strict CLI Clippy PASS after awaiting the shared lock. Scoped frontend Vitest =42/0; TypeScript and ESLint PASS.
- Final committed-checkout CLI + bash cli/tests/machine_native_acceptance.sh --disposable (NYXID_TEST_PROBE_SAMPLES=1 smoke rerun): 16/0/0 lifecycle cases; two contexts, replay, private roots, secure/dev/AX, signed login and cross-fill refusal, native/D-Bus/X11 denials, four streams, takeover, quarantine and two restarts.
- Final renderer acceptance observes 2 active secure identities/9 real renderers; context users have no subuid/subgid entries. Positive evidence: v3/linux/ABI8, six required booleans=true, AppArmor restriction=true, timed_out=false, separated.available=true.
- Full checked-in adversarial suite with the locked production-module launcher: 44/0, no tool failures or removed controls; private harness-only mounts refresh fixture /tmp and /dev/shm without deleting old data; documented metadata/shm/public-pathname-socket residual controls remain allowed.
- Production calibration: 100 reports/500 successful mechanisms; policy median/p99/max 409.5/513/626ms, secure 234/417/436ms, developer 240.5/375/456ms. Bound remains 5000ms pending C's actual hosted image/amd64 samples; cold first policy sample=626ms.
- Guest mutations: committed archives/cache, four synthetic w94 profiles/users/roots, package-correct global policies/resources and UID home gates, private adversarial roots. No sysctl/profile relaxation; no writable worktree mount; cleanup confirms zero live processes across 37 fixture UIDs, zero probes/backups, AppArmor sysctl=1.
- C entrypoints: native bash cli/tests/machine_native_acceptance.sh --disposable; image python3 cli/tests/machine_probe_acceptance.py --disposable --binary /usr/local/bin/nyxid --container --samples 100 with normal seccomp/init/512m shm and no token or identity/workspace mounts. Hosted timing/Docker lifecycle remain release gates. NYXID_NODE v3 prose committed as 0f15120ef via PM-directed git add -p; cached diff/check verified only D's 17-line section, leaving A's 38-line OAuth section unstaged; no runtime tests rerun for prose.

## D-5 hosted native browser selection correction (2026-10-20)
- Code c6f812247: prerequisite and native harness share one selector, defaulting explicitly to /usr/bin/google-chrome; installed Chromium shims cannot win discovery. Candidate/resolved/error/selection and policy-directory diagnostics precede refusal.
- Linux setup accepts NYXID_MACHINE_BROWSER, persists that executable and forwards only its selected path through the sudo installer. Invalid explicit paths never fall back; resolved-package policy validation and v3 live checks remain required.
- Hosted prereqs still require /etc/opt/chrome/policies/managed. Explicit native Snap and Debian choices keep their distinct policy paths; package-layout regression covers all three.
- node --test cli/tests/machine_native_browser.test.mjs: macOS 8/0/0 and Lima 8/0/0; existing scripts/test-machine-acceptance-ci.py: 9/0; Node/Python syntax, rustfmt and scoped diff checks pass.
- Shared-target cargo test --locked -p nyxid-cli --bin nyxid node::machine::setup::tests: 15/0/0 (3 new); same executable's browser_layout::tests: 1/0/0. Strict CLI Clippy --all-targets --features node-proxy-test -- -D warnings passes.
- Real Lima selection: /usr/bin/chromium resolves to /snap/chromium/3560/usr/lib/chromium-browser/chrome with Snap policies; hosted Chrome-only check rejects it. Missing default Chrome refuses despite installed Chromium; every candidate resolution is printed.
- Guest mutations: selector/tests copied only to /tmp/nyxid-w94-browser-selection; no browser package, policy, sysctl, profile or context changes. Actual hosted amd64 Chrome/lifecycle/calibration proof awaits PM's next push.
- Selection is documented in MACHINE_NODES.md; focused ENV.md/CLAUDE.md registry documentation is requested from PM and remains untouched pending approval.

## D-5 hosted container observation diagnostics (2026-10-20)
- Code `3f343ee33`: closed browser-observation labels and errno, printed before acceptance assertions; strict v3 predicates, timeout classification and group cleanup retained. Isolation docs now consistently reject v1/v2/unknown evidence for new selection.
- Linux guest checkout: committed `a51ebb71eb50` plus exactly the two diagnostic code files, copied archive with matching SHA-256; no writable host-worktree mount.
- Linux `cargo test --locked -p nyxid-cli --bin nyxid node::machine::browser_probe::tests`: 3 passed/0 failed/0 ignored; CLI binary build and strict CLI all-target Clippy with node-proxy-test pass; logs `/tmp/nyxid-w94-container-diagnostics-linux-{tests,build,alltargets-clippy}.log` in guest.
- macOS `cargo clippy --locked -p nyxid-cli --all-targets --features node-proxy-test -- -D warnings`: reused A's successor, completed 21:50:23 after unchanged D sources; `/tmp/nyxid-w96-cutover-clippy.log`. Earlier run failed only A's two subsequently fixed daemon/cutover lints.
- Actual local image: nyxid-node-machine:0.41.0, Debian Chromium154, Docker29.1.3, Ubuntu24.04.5/aarch64/kernel7.0; docker-default(enforce), AppArmor userns restriction=1. Default capabilities refuse with browser_proc_fds_unreadable/errno13; manual renderer observations confirm nested PIDs/additional filters but protected proc entries are unreadable.
- Disposable supervisor-SYS_PTRACE experiment with six tracing/process-memory syscalls denied passes one strict report/five mechanisms: CommandSeccomp1ms/Landlock2ms/BrowserPolicy1534ms/SecureBrowser157ms/DeveloperBrowser184ms. This is experimental evidence, not shipped runtime acceptance.
- Additional disposable controls: non-root child effective/permitted/inheritable/ambient capabilities all zero, NNP=1; ptrace/process_vm_readv/process_vm_writev/pidfd_getfd/kcmp/process_madvise all return EPERM. Host log `/tmp/nyxid-w94-observer-permission-controls.log`.
- Guest mutations: temporary diagnostic archive/build/logs, read-only fixture mounts and disposable --rm containers only; zero live containers, two pre-existing anonymous volumes unchanged; no sysctl/AppArmor/persistent runtime changes.
- Hosted Docker28/amd64 diagnosis and 100-sample calibration remain gates. Five-second bound retained. Production capability/profile edits require PM's explicit ownership/contract decision; no seccomp/profile/workflow edit in this batch.
- PM's browser-override clarification is committed in MACHINE_NODES.md by `d35a25a5c`: only recognized resolved package layouts, never an arbitrary privileged installer path. ENV.md and CLAUDE.md remain untouched by D.

## D-5 v4 custody consumers and rollout guidance (2026-10-20)
- Code `88d0f2099`: frontend requires current v4 plus credential_custody=true; absent/null/false custody has actionable node_custody_unverified. Earlier v1/v2/v3 and unknown reports cannot select new automatic/required separation; existing legacy assignments retain their mode.
- Scoped frontend Vitest: 72 passed/0 failed; TypeScript build and scoped ESLint pass. Persistent logs: ~/.cache/nyxid-stream-d/recovery/v4-frontend-{tests,tsc,eslint}.log.
- Acceptance `5f29bed1e`: every calibration sample and initial/restored/restarted native report requires v4 and positive custody; all existing policy, renderer, namespace and lifecycle assertions retained.
- Node/Python syntax pass; browser-selector tests 8/0/0; scripts/test-machine-acceptance-ci.py 9/0/0 after restoring PyYAML6.0.3 to D's private cached Python environment (initial system Python lacked yaml).
- Docs `0d6ba246c` and `238a10a84` describe the frozen v4 field, public preflight sentinels and repair guidance. NYXID_NODE used git add -p: only D's evidence hunk staged/reviewed/committed; A's storage/custody prose remained unstaged.
- Production Rust custody implementation and real v4 Linux execution belong to A and remain pending; these consumer checks do not claim that gate. Hosted native/Docker100-sample and lifecycle gates remain pending.

## D-5 no-capability container observation investigation (2026-10-20)
- Production probe unchanged from `3f343ee33`; no capability grant or profile relaxation. PM rejected supervisor SYS_PTRACE because ordinary proc open/read could expose browser memory despite syscall denials.
- Real full image0.41.0/DebianChromium154 on Lima Ubuntu24.04.5/aarch64/kernel7.0, normal init/512m/seccomp/docker-default: status/stat/cgroup/uid_map/gid_map/net tables readable without added capabilities; renderer exe/ns links deny EACCES13, including matching-fsuid reads of nondumpable zygote/renderers.
- Real renderer: UID/GID1001 in all fields, NNP1, Seccomp2, filters2 vs browser1, NSpid94/12/1, CapEff0; restricted UID/GID maps and different net/dev inode from browser. Main browser exe is readable under matching fsuid; its re-execed zygote is not.
- Negative control: unrelated helper named chromium with renderer argv, same UID/GID, NNP1, Seccomp2, filters3 vs parent1, nested PID IDs and CapEff0 retains the parent's network namespace. Four diagnostic assertions pass/0 fail, demonstrating status-only predicates are insufficient.
- Remaining essential proof: exact renderer executable and distinct user/net namespace identities; Name/argv/cgroup do not establish them. UID maps and per-network proc inodes need kernel-contract validation and do not establish executable identity. Policy listener association also currently needs protected fd links.
- Reproduction sources/logs: ~/.cache/nyxid-stream-d/recovery/no-cap-proc-observe.py, no-cap-proc-observe{,-ancestry}.log, status-only-negative.{c,py,log}. C helper built with gcc -O2 -Wall -Wextra -Werror; normal --rm containers, read-only public fixture mounts, no token/identity/workspace mounts.
- Guest mutations: only /tmp/w94-no-cap-* and /tmp/w94-status-only-negative* fixtures; no packages/accounts/policies/sysctls/AppArmor changes. No live containers; Lima slot returned to A. Hosted Docker28/amd64 cause awaits diagnostic output after PM's next push; latest hosted head remains82e7605bb.

## NODE-1 follow-ups and hosted Machine regressions (2026-10-20)
- `d35a25a5c`: committed revocation closes the captured connection before audit admission; durable Narrowing outcome, single-send guard and successor fences retained. Handlers warn with node/operation IDs and stable code on failure; shared audit_principal fixes Relay attribution.
- `426c46232`: both rotation failpoint phases use test_utils::fail_command::Guard; retry, prepared token, transaction identity and audit assertions retained.
- `1e83c2e29`: external HTTP journeys explicitly acknowledge shared_legacy; omitted consent asserts 400/1008, full warning, no grant and unchanged key version. Hosted-specialist matrix preserves defaults only with recorded legacy consent; missing choice requires selection and grants no capabilities.
- Reused C's post-reboot shared executable nyxid_server-483e6416bf263370 (linked22:20:43, after D's final Rust edit22:04:25), own Mongo50264/w8node; no duplicate Cargo build.
- Direct executable w8_node_ --test-threads=1: 23 passed/0 failed/0 ignored. routes::machine services::remote_machine services::machine --test-threads=2: 440 passed/0 failed/3 existing manual benchmarks ignored, 1386.28s; all three hosted journeys and specialist regression pass.
- Reused strict backend all-target both-KMS Clippy, drivers22:05:52/53 after D's source freeze, exit0 at22:10:20. Logs: ~/.cache/nyxid-stream-d/recovery/{w8-node,machine-full}.log and ~/Library/Caches/nyxid-coreb/gates/post-reboot-shared-rerun-{test-no-run,clippy}.log.
- WS credentials authenticate on connection, not every frame; active-node/connection-owner DB fence renewals close a failed immediate disconnect on healthy heartbeat/sweep (default30s; managed5s). DB failures can extend that residual window; no hard bound claimed.
- These gates cover the frozen NODE/fixture sources, not A's subsequent v4 custody changes. Browser ancestry implementation and hosted native/Docker lifecycle gates remain pending; Lima stays stopped under PM's memory-pressure hold.
