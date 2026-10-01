# Machine nodes: implementation and validation

This report preserves the implementation and round-1 measurements. For the
subsequent merge with 0.39.0 main, see [merged-tree validation](MACHINE_NODES_MERGE_VALIDATION.md).

This records the implementation on `nyxbot/machine-nodes`, based on `fa96a28a`,
without changing the repository's 0.38.1 version. The binding design is
[MACHINE_NODES.md](MACHINE_NODES.md); operator instructions are in
[NYXID_NODE.md](NYXID_NODE.md#machine-access-for-nyxbot-and-specialists).

## Decisions and acceptance coverage

| Decision | Implementation | Evidence |
|---|---|---|
| D1 | Independent, default-off shell/files/computer profile on existing nodes. | Shared config legacy/default tests; backend capability authorization matrix. |
| D2 | Profile-local CLI enable/disable/status; roots, limits, root opt-in; capability reporting and node-side enforcement. | `config::tests`, `machine_authority_owner_guest_org_membership_offline_and_capabilities`, production container capabilities and non-root command execution. |
| D3 | Owner-turn chat keys only; live personal/org-admin access; specialist grants beside `grants`; permission requests; owner-only confirmation settings. | `machine_mcp_tools_are_only_discovered_and_called_by_owner_chat_keys`, `machine_specialist_permission_is_explicit_durable_and_revocable`, authority matrix, grants-form and machine-settings frontend tests. |
| D4 | Native command/job/file/attachment/computer tools, compact paginated output and attachment-backed screenshots; no git tool. | Real CLI transport tests; jobs/files tests; `machine_screenshots_use_owner_attachments_with_magic_and_turn_limits`; result-size/pagination tests. |
| D5 | Mandatory signed request envelopes, digest-bound parameters, timestamp/nonce replay guard; bounded binary streams and cross-replica dispatch. | `signatures_bind_every_authority_input_and_reject_replay`, cross-replica desktop and gateway upload tests; real signed container requests. |
| D6 | Clean child environments; descriptor-anchored paths; atomic writes; process-group cancellation; retained bounded jobs; streaming attachments. | `children_do_not_inherit_node_environment_and_reject_reserved_input`, file symlink/cwd-swap/atomic-write tests, jobs cancellation/timeout and head/tail retention tests, gateway reconnect lifecycle test, 4 MiB container transfer roundtrip. |
| D7 | Verified cua 0.30.4 release through MCP stdio, public contract filtering, telemetry off, no perception installation, explicit permission mode. | Fake MCP stdio test, platform/checksum pins, actual Linux driver in container; macOS RAM-capture test and real permission probe. |
| D8 | One-command setup, profile-aware daemon, machine Docker image, Publish Images matrix and production-image test; web setup and node details. | Setup URL tests; frontend setup/settings tests; production image build and container test. Existing CLI suite covers retained registration/daemon commands. |
| D8a | Interactive machine TTY and Windows containers remain explicit follow-ups. | Documented scope; no partial implementation. |
| D9 | Metadata-only machine audit; redacted secret types, zeroizing buffers; no command/file/input/frame logging. | Saved-login storage/tool-result/audit tests; signed container test sweeps logs and tool results; shared redaction tests. |
| D10 | Stable machine capability/grant/path/job/card/driver/control/login errors, with existing node offline/timeout errors. | Authorization and confirmation tests assert error variants; frontend warning states; authoritative code mapping in `backend/src/errors/mod.rs` and CLAUDE.md. |
| D11 | Additive defaulted model fields and capability negotiation; legacy node behavior retained. | Shared legacy config tests; existing backend and CLI regression suites. |
| D12 | Operator, protocol, NyxBot, environment and project-rule documentation. | Documentation links in this report; full changed-file inventory below. |
| D13 | Prefilled NyxBot setup links; HMAC pairing, atomic owner approval, single-use page-only tokens; capability change-stream watch and durable wake. | Setup pairing races, page grant atomicity, no-token tool/card test, durable watch dedupe, actual change-stream timing test; frontend setup review and live progress tests. |
| D14 | Per-job loopback tokens; node-signed requests tied to server-issued live jobs; live proxy authority; server-side GitHub Basic injection; per-process git rewrites. | Live-token/expiry/reconnect tests, foreign-node/runtime/conversation binding tests, specialist service-scope test, streaming/cancellation tests, 100 MiB download benchmark and real smart-HTTP clone/fetch/pull/push fixture. |
| D15 | On-demand desktop, owner takeover/hand-back, full agent lockout, durable wake, human-only browser WS, session-scoped binary relay, bounded changed frames. | Controller race/recovery and acknowledged-revision tests, real browser-upgrade authorization test, delegated-route denial test, cross-replica relay test, frame-budget test, frontend panel tests; container owner typing, lockout, hand-back and fresh agent observation. |
| D16 | Write-only encrypted owner/org logins; specialist grants; TOTP; signed force-installed extension/native host; exact origin/field checks; password pinning; single-user opt-in. | Storage/human-only tests; specialist/org/grant/card matrix; RFC 6238 vector; opt-in test; policy-generation and filesystem protection tests; real container HTTPS username/password/TOTP sign-in, mismatch refusal, DevTools/javascript blocking, copy refusal, native socket isolation and encoded-output sweep. |
| D17 | No machine queries for unrelated MCP callers; batched machine reads; one indexed job binding; bounded transfers/output; repeatable latency/throughput/frame/wake benchmarks. | Mongo command-monitor tests with 64 nodes, actual CLI loopback benchmark, streaming/git benchmark, container/macOS desktop benchmark, setup change stream test. Measurements and host limitations are recorded below. |

The linked-chat acceptance path shares the existing owner-turn chat-key and
action-card machinery with web chat. Automated tests exercise those authority,
card, delivery and watch boundaries. They do not send messages to a real
Telegram account or authenticate to a real GitHub account. Git and sign-in tests
use local trusted fixtures with generated credentials, including username,
password and TOTP in the production managed browser.

## Acceptance checks

| Acceptance criterion | Automated coverage |
|---|---|
| NyxBot-led setup and no credential in chat | `machine_setup_tools_and_owner_cards_never_contain_registration_credentials`, pairing approve/deny races, atomic grant application, durable watch dedupe, actual change-stream wake benchmark; setup page tests cover review and live progress. |
| Owner commands, jobs, files, git, services and attachments | Production CLI through real loopback WS; command cancellation/output tests, anchored file operations, job-token reconnect/expiry tests, real smart-HTTP clone/fetch/pull/push plus service proxy fixture; screenshot attachment ownership/magic/turn limits and container file transfers. |
| Specialist grant and guest exclusion | `machine_specialist_permission_is_explicit_durable_and_revocable`, `machine_authority_owner_guest_org_membership_offline_and_capabilities`, MCP discovery/call authority matrix and live specialist gateway scope. |
| Computer use | Production-image test uses the pinned driver under Xvfb for observation, clicks, typing and screenshots. |
| Owner takeover and hand-back | Durable controller race/recovery tests, exact revision fencing, owner takeover shell/file/computer lockout and wake note, browser WS authorization, cross-replica relay; frontend panel and real container input/hand-back tests. |
| Saved login sign-in and privacy | Actual HTTPS username/password/TOTP sign-in in container Chromium; force-installed extension, policy denial, origin/field mismatch, password pinning, copy refusal, OS/socket isolation, encoded-output scrubbing and secret sweeps; backend storage/grant/org/card/opt-in tests and Saved logins form tests. |
| Confirmation | `machine_confirmation_is_bound_to_parameters_and_consumed_once`, observation-versus-change classification and saved-login per-use confirmation. |
| Compatibility and all decisions | CLI/backend/frontend regression suites plus the D1–D17 table above; existing versions remain unchanged. |
| Performance | Indexed lookup/query-count tests, actual CLI loopback timing, 100 MiB gateway/git comparison, desktop scenarios and change-stream wake measurement. |

## Review round 1

All references below are executable regression tests unless labelled as a
formatting/build check. The container test is `cli/tests/machine_container_e2e.mjs`.

| Item | Change | Proof |
|---|---|---|
| 1 | Exec declares only needed service IDs/slugs; jobs persist declarations, gateway rejects other slugs, environment/cards/audit use the declarations. | `machine_gateway_uses_live_specialist_scope_and_server_credentials`, `machine_declared_services_are_bound_to_job_card_and_audit`, `gateway_tokens_are_job_bound_and_completion_survives_reconnect_until_ack`. |
| 2 | Controller epochs immediately stop agent admission, cancel cua/file operations and kill job process groups; late results are discarded. Capture/input have independent paths. | `takeover_cancels_a_thirty_second_cua_action_and_discards_its_result`, `queued_agent_results_are_fenced_at_enqueue_after_takeover`; container stalls cua and a 5 MiB upload, measures takeover ≤150 ms and verifies cancellation/lockout. |
| 3 | Setup/status/Nodes explicitly warn that non-isolated shell commands can read node credentials and token; persistent Not isolated badge; container/separated VM recommendation. | `keeps the non-isolated shell warning visible to readers`, `does not label separated machines as non-isolated`, and the setup command/review test; CLI status/enable implementation reviewed with the production setup flow. |
| 4 | Chromium user/PID namespace sandbox with the shipped Moby-derived seccomp profile, no extra capabilities; generated setup and CLI Docker command include the profile. | Container verifies renderer NoNewPrivs, Seccomp, nested NSpid, browser-only uid mapping and absence of `--no-sandbox`; public seccomp asset freshness test. |
| 5 | Every dropped-privilege Linux child sets PR_SET_NO_NEW_PRIVS. | Container agent command asserts `NoNewPrivs: 1` from `/proc/self/status`; renderer check also passes. |
| 6 | Both gateway hops preserve Content-Encoding and Content-Length, allowing SDK/git automatic decoding without buffering or disabling compression. | Regular `machine_gateway_git_clone_fetch_pull_push_and_sdk_preserve_gzip` uses gzip SDK and git fixtures. |
| 7 | Server catalog inference protocol and git HTTP metadata generate the signed per-job environment; node has no service-slug mappings. Git excludes platform credentials. | `declarations_and_environment_follow_catalog_metadata_not_slugs`, auth/listing parity test, real git fixture. |
| 8 | Gateway and direct API-key extraction share `api_key_auth_user`, including auto-connected/platform allowlist expansion and key purpose. | `machine_gateway_and_direct_api_key_auth_have_identical_effective_authority`. |
| 9 | Gateway service discovery reuses the shared catalog/MCP owner/membership resolver and platform ACL, filtered by effective key scope. | The same parity test checks ordinary, auto-connected and platform rows and platform git exclusion; existing shared-resolver ACL suite. |
| 10 | Runtime carries typed machine errors; incidental error wording cannot select a public code. | `runtime_errors_use_types_never_incidental_words`. |
| 11 | X11/XFixes capture and independent XTest input on Linux; ScreenCaptureKit and independent human cua input on macOS; 30 Hz JPEG dirty rectangles with bounded bandwidth and recovery. | Container fps/latency/idle benchmark; `dirty_rectangle_has_a_base_and_idle_sends_nothing`, `large_displays_preserve_input_coordinates_with_bounded_frames`, frontend rectangle validation and `paints ordered dirty rectangles and requests a full frame when a base is missing`; exact macOS benchmark below. |
| 12 | Embedded cua contract is parsed once via LazyLock. | `changes_confirm_all_commands_and_only_mutating_computer_tools` and fake-driver public-contract tests. |
| 13 | PR CI runs extension tests/CRX freshness and path-filtered container e2e. Git correctness is a regular backend test; only timing workloads remain ignored. | Signed CRX deterministic ZIP/signature/package test; regular gzip/git test; CI aggregator requires machine-container job when selected. |
| 14 | Prettier formatting and extracted setup, desktop controls and saved-login card components. | Frontend tests, lint, type-check and production build. |
| 15 | Username is text with autocomplete off; styled origins textarea gives per-line validation. | `shows metadata only, replaces write-only secrets and clears the form after saving`, `shows a validation message for each invalid origin line`. |
| 16 | Expanded backend macros, select branches and item spacing in machine handlers/services and proxy additions. | Rust formatting, clippy and backend regression chunks. |
| 17 | Updated design, operator/protocol instructions, chat08/chat09, CLAUDE and this report for declarations, warnings, sandbox, compression, metadata and native desktop. | Commands, measurements and test mapping in this report. |

## Validation results

Rust validation used 1.98.1. The complete backend chunks use two test threads
and skip only `curation_concurrent_writers_and_shared_budget`, as requested.
Chunk filters can overlap, so their counts are not a unique-test total. Logs remain in
`target/` for review; infrastructure-invalidated runs are excluded.

| Check | Result | Log |
|---|---|---|
| Backend handlers chunk | 1,971 passed, 0 failed; 173.39s | `machine-review-handlers.log` |
| Backend services chunk | 3,855 passed, 5 ignored, 0 failed; 387.87s | `machine-review-services.log` |
| Backend remainder chunk | 1,136 passed, 0 failed; 51.93s | `machine-review-rest.log` |
| CLI and shared machine crate | 1,335 passed, 2 ignored; includes all CLI integration tests and 11 shared-crate tests | `machine-review-cli-final.log` |
| Workspace clippy, all targets, `-D warnings` | Rust 1.98.1 passed; 10m 49s including Cargo lock wait. Cargo separately reports the existing dependency future-incompatibility notice for `proc-macro-error2` | `machine-review-clippy-final2.log` |
| Rust formatting and whitespace | Workspace fmt, explicitly included runtime file, and `git diff --check` passed | Commands below |
| Frontend tests | 4,008 passed across 396 files, 166.44s; two workers to bound memory | `machine-review-frontend-final2.log` |
| Frontend lint | Passed: 0 errors, 29 existing warnings | `machine-review-lint-final2.log` |
| Frontend type-check | `npx tsc -b` passed | `machine-review-tsc-final2.log` |
| Frontend production build | Passed; ordinary bundle-size warnings; emitted seccomp asset matches the embedded CLI profile | `machine-review-frontend-build.log` |
| Production Linux arm64 image | Built successfully | `machine-review-image-final3.log` |
| Container browser/driver end to end | Passed; extension/freshness tests: 5 passed | `machine-review-e2e-final3.log`, `machine-review-e2e-build-final3.log`, `machine-review-filler-final.log` |
| Backend timing tests | All 4 passed: real node exec (2.39s), streaming/git (7.13s), change-stream wake (0.51s) and signed dispatch (0.76s) | `machine-review-exec-benchmark.log`, `machine-review-gateway-benchmark.log`, `machine-review-setup-benchmark.log`, `machine-review-dispatch-benchmark.log` |
| macOS RAM-only capture volume | Passed, 1 test, 3.32s | `machine-ram-capture-test.log` |

The production image manifest is
`sha256:06f295a23286eae9275f0f7f39e3a7a12c96544a1d92d1c19d75755773e40669`;
the test image manifest is
`sha256:23a1e8ac5b0e66d279e130db7034a65f6296664b9f915c4c44a494ad17fcd1c6`.
Linux amd64 publication is wired in CI but was not built locally. No version
was changed, and no commit or push was made. Temporary containers used `--rm`;
the machine-specific build caches were pruned and `target/machine-validation`
was removed. The final images remain available for review.

## Performance measurements

Measured on 2026-10-01. The backend timing tests use the same Rust debug test
binary as the complete suites, run sequentially after this worktree's builds
and tests. No competing Rust builds or tests were running when the measurements
started. This is a shared development host, not a dedicated benchmark machine.

| Measurement | Direct / baseline | Through NyxID |
|---|---:|---:|
| Exec overhead, actual CLI runtime over loopback WS, command runtime excluded | Budget: ≤ 50 ms p95 | **9.968 ms p50 / 34.132 ms p95** |
| 100 MiB download | 0.196 s / 510.99 MiB/s | **0.178 s / 561.80 MiB/s** |
| 12 MiB git repository clone | 0.656 s | **0.703 s** |
| Capability report → durable NyxBot wake, real change stream | Budget: < 10 s | **13.90 ms** |
| In-process signed dispatch, synthetic node response | — | 2.336 ms p50 / 2.897 ms p95 |

Exec timings retain 100 samples after 10 warmups. The gateway benchmark uses
local HTTPS fixtures and also verifies smart-HTTP clone/fetch/pull/push with
server-side credential injection. Direct runs first; these numbers describe
this fixture and run, not Internet throughput. Setup timing begins when the
connected node reports its capabilities; CLI download, installation and human
approval time are excluded. The result proves the change-stream path without
waiting for the sweep.

The four backend timing logs are named in the results table. They were run
from the already-built test binary with the same filters and harness flags
shown below, avoiding repeated compilation of unchanged code.

Container desktop measurements use the final production Linux arm64 image
under Docker Desktop at 1280×800. This run passed while the shared host was
compiling; it is not a hardware-independent guarantee.

| Scenario | Changed frames/s | Frame bytes/s | Sample |
|---|---:|---:|---|
| Idle | 0 | 0 | 0 frames / 5.069s |
| Typing | 30.192 | 209,319 | 194 actions, 151 frames / 5.001s |
| Scrolling | 29.374 | 2,046,233 | 217 actions, 147 frames / 5.004s |

Owner input-to-frame latency: **28.560 ms p50 / 63.259 ms p95** across ten
alternating scroll inputs, measured from input submission to receipt of the next
changed frame on the loopback node socket. This excludes browser JPEG decoding
and canvas painting; ordered painting and delta recovery have frontend tests.
Takeover while cua is stopped and a 5 MiB upload is stalled: **0.772 ms**,
including its protocol acknowledgement. Both operations are cancelled and the
late agent result is refused. A 4 MiB file transfer round trip took **204.487 ms**.
Source: `target/machine-review-e2e-final3.log`.

Native X11 capture uses in-process XGetImage with XFixes cursor compositing,
on a separate connection from XTest input. The 30 Hz
encoder compares 64-pixel tiles and merges changed tiles into one JPEG dirty
rectangle (quality 70). This keeps text updates small without decoder startup,
while full-window scrolling stays under the 2 MiB/s cap. Missing delta bases
request a full frame. No PNG decode or accessibility traversal is on the human
capture path, and idle pixels produce no payload. Agent operations retain cua.

The test also proves Chromium renderer namespaces/seccomp, agent NoNewPrivs,
the signed extension and native-host isolation, a real username/password/TOTP
sign-in, origin/field refusal, pinning/copy protection and no-secret sweeps.

## Reproduce

Use Rust 1.98.1 and a replica-set MongoDB. Run backend chunks sequentially:

```sh
export CARGO_INCREMENTAL=0
export NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27020/?directConnection=true'
cargo +1.98.1 test -p nyxid --bin nyxid-server -- handlers:: --test-threads 2 --skip curation_concurrent_writers_and_shared_budget
cargo +1.98.1 test -p nyxid --bin nyxid-server -- services:: --test-threads 2 --skip curation_concurrent_writers_and_shared_budget
cargo +1.98.1 test -p nyxid --bin nyxid-server -- --skip handlers:: --skip services:: --skip curation_concurrent_writers_and_shared_budget --test-threads 2
cargo +1.98.1 test -p nyxid-cli -p nyxid-machine
cargo +1.98.1 fmt --all -- --check
rustup run 1.98.1 rustfmt --edition 2024 --check cli/src/node/machine/runtime.rs
git diff --check
cargo +1.98.1 clippy --workspace --all-targets -- -D warnings
```

In `frontend/`, run `npm run lint`, `npm test`, `npx tsc -b`, and `npm run build`.
Run `node --test cli/tests/machine_filler.test.mjs` from the repository root.
Wizard sources are unchanged. Run each benchmark alone after builds/tests stop:

```sh
cargo +1.98.1 test -p nyxid --bin nyxid-server -- machine_loopback_exec_performance --ignored --nocapture --test-threads 1
cargo +1.98.1 test -p nyxid --bin nyxid-server -- machine_gateway_streaming_and_git_performance --ignored --nocapture --test-threads 1
cargo +1.98.1 test -p nyxid --bin nyxid-server -- machine_setup_change_stream_wakes_thread_without_sweep --nocapture --test-threads 1
cargo +1.98.1 test -p nyxid --bin nyxid-server -- machine_exec_dispatch_performance --ignored --nocapture --test-threads 1
docker build -f cli/Dockerfile.machine -t nyxid-node-machine:local .
docker build -f cli/tests/Dockerfile.machine -t nyxid-machine-e2e:local .
docker run --rm --shm-size=256m --security-opt seccomp=cli/resources/machine-container/seccomp.json nyxid-machine-e2e:local
```

The container test uses the production CLI, signed extension, native host and
pinned cua binary at 1280×800. Its output contains timings and frame byte counts;
it does not write screenshots. Idle means a settled page with an unfocused text
field. Typing and scrolling are continuous acknowledged input; scrolling
reverses every six actions to avoid measuring an idle page boundary. Alternating
scroll events then measure input-to-frame latency. Action counts accompany frame
counts.

The macOS benchmark requires a logged-in desktop, Google Chrome and Screen
Recording/Accessibility permission for the app launching the test:

```sh
NYXID_MACHINE_BENCH_CUA="$HOME/.nyxid-node/cua/cua-driver-rs-0.30.4-darwin-universal/cua-driver" \
  CARGO_INCREMENTAL=0 cargo +1.98.1 test -p nyxid-cli --bin nyxid -- \
  macos_desktop_performance --ignored --nocapture --test-threads 1
```

For a named profile, set the driver path under `~/.nyxid-node/profiles/NAME/`
instead. Install computer support with `nyxid node machine enable --computer`
first if needed. The test opens an isolated 1280×800 Chrome window on the main
desktop, keeps captures in RAM, and prints the same scenario/fps/bytes/actions
table as the container plus input p50/p95. The streamed canvas follows the
screen size, capped at 1920×1200, so keep other windows idle during measurement.

The validation host is an Apple M2 with 16 GiB RAM running macOS 27.0.
Docker Desktop reports 8 CPUs and 8,321,798,144 bytes of VM memory; it hosts
MongoDB and the Linux arm64 machine image. Concurrent repository builds caused substantial swap
activity. Earlier service and handler runs were invalidated when the shared
MongoDB container was recreated: failures began with `UnexpectedEof` and
database write-probe errors. Final reruns are recorded in the results table;
the invalidated runs are not counted as validation.

The real macOS permission probe returned `screen_recording=false` and
`accessibility=false`. The desktop benchmark consequently refused capture/input;
macOS fps, bandwidth and input latency have not been measured on this host.
Enable both permissions for the app launching the node, restart it, and run the
command above on a logged-in desktop. The actual RAM-only capture-volume test
passed; generated macOS managed-policy tests also run in the CLI suite.

## Verified external release

The installer pins `cua-driver-rs-v0.30.4`, source commit
`bf6c76786d938070f4ecf1e44004752f69f518b8`, released 2026-09-28.
All three actual release archives were downloaded and hashed, and compared to
the release's `checksums.txt`. The metadata is retained in
`cli/resources/cua/release.json`; temporary release diagnostics were removed.

Release assets:
`https://github.com/trycua/cua/releases/download/cua-driver-rs-v0.30.4/`

| Asset | SHA-256 |
|---|---|
| `cua-driver-rs-0.30.4-darwin-universal.tar.gz` | `9c75a186f89352fb522dc67791575f8c9e8081a38795af2706e103d41fa72be4` |
| `cua-driver-rs-0.30.4-linux-arm64.tar.gz` | `21d00fa2fafe889e48a4e497fba95e6cd03de027753fc8799d5cf0695c30a8a1` |
| `cua-driver-rs-0.30.4-linux-x86_64.tar.gz` | `84445347ceb3039034ce30577b3b7c19a1f0c1f67639423f9da3a71be0418f90` |
| `checksums.txt` | `e9089053ef9421b52cdc0f617fdc94db4644c12b795baf40429cb69dce068b29` |

NyxID's signed filler extension is pinned to ID
`gakifgdopgimcpebaoociibbmogoegjh`, package SHA-256
`6936c2d6d8d94e2938ed2136bae2d61f4bdbdfaac6e83f2b594398d6f52cbc05`.
The source, manifest, signed CRX and package pin live together in
`cli/resources/machine-browser/`. Its packaging script creates an ephemeral
signing key in memory; a package change requires updating the package and ID
pins together. No private signing key is stored in the repository.

## Changed files

Full worktree inventory (original implementation plus this review round):

- `.github/workflows/ci.yml`
- `.github/workflows/publish-images.yml`
- `CLAUDE.md`
- `Cargo.lock`
- `Cargo.toml`
- `backend/Cargo.toml`
- `backend/Dockerfile`
- `backend/build.rs`
- `backend/src/billing_integration_tests.rs`
- `backend/src/db.rs`
- `backend/src/errors/mod.rs`
- `backend/src/handlers/admin_anonymous_endpoints.rs`
- `backend/src/handlers/admin_nodes.rs`
- `backend/src/handlers/api_keys.rs`
- `backend/src/handlers/assistant_action_effects_nodes.rs`
- `backend/src/handlers/assistant_action_effects_services.rs`
- `backend/src/handlers/assistant_group_tests.rs`
- `backend/src/handlers/assistant_team.rs`
- `backend/src/handlers/assistant_team_tests.rs`
- `backend/src/handlers/delegation.rs`
- `backend/src/handlers/machine_desktop.rs`
- `backend/src/handlers/machine_gateway.rs`
- `backend/src/handlers/machine_mcp_tests.rs`
- `backend/src/handlers/machine_setup.rs`
- `backend/src/handlers/machine_tools.rs`
- `backend/src/handlers/mcp_config_routes_tests.rs`
- `backend/src/handlers/mcp_transport.rs`
- `backend/src/handlers/mod.rs`
- `backend/src/handlers/node_admin.rs`
- `backend/src/handlers/node_agent.rs`
- `backend/src/handlers/node_ws.rs`
- `backend/src/handlers/nyxbot.rs`
- `backend/src/handlers/nyxbot_tests.rs`
- `backend/src/handlers/proxy.rs`
- `backend/src/handlers/public_mcp.rs`
- `backend/src/handlers/public_proxy.rs`
- `backend/src/handlers/saved_logins.rs`
- `backend/src/handlers/services.rs`
- `backend/src/handlers/ssh_tunnel.rs`
- `backend/src/models/assistant_agent.rs`
- `backend/src/models/downstream_service.rs`
- `backend/src/models/machine_desktop.rs`
- `backend/src/models/machine_job.rs`
- `backend/src/models/machine_setup.rs`
- `backend/src/models/mod.rs`
- `backend/src/models/node.rs`
- `backend/src/models/saved_login.rs`
- `backend/src/mw/auth.rs`
- `backend/src/routes.rs`
- `backend/src/services/admin_user_service.rs`
- `backend/src/services/anonymous_endpoint_service.rs`
- `backend/src/services/api_key_scope_service.rs`
- `backend/src/services/assistant_acknowledgement_service.rs`
- `backend/src/services/assistant_agent_credential_service.rs`
- `backend/src/services/assistant_authority_tests.rs`
- `backend/src/services/assistant_live.rs`
- `backend/src/services/assistant_team_service.rs`
- `backend/src/services/assistant_team_tools.rs`
- `backend/src/services/credential_push_service.rs`
- `backend/src/services/destination_routing_tests.rs`
- `backend/src/services/google_auto_activation_tests.rs`
- `backend/src/services/key_service.rs`
- `backend/src/services/machine_desktop_service.rs`
- `backend/src/services/machine_gateway_service.rs`
- `backend/src/services/machine_integration_tests.rs`
- `backend/src/services/machine_service.rs`
- `backend/src/services/machine_setup_service.rs`
- `backend/src/services/machine_tools.rs`
- `backend/src/services/machine_transport_tests.rs`
- `backend/src/services/mcp_service.rs`
- `backend/src/services/mod.rs`
- `backend/src/services/node_dispatch.rs`
- `backend/src/services/node_dispatch_tests.rs`
- `backend/src/services/node_fanout_resolver.rs`
- `backend/src/services/node_metrics_service.rs`
- `backend/src/services/node_owner_service.rs`
- `backend/src/services/node_pending_credential_service.rs`
- `backend/src/services/node_routing_service.rs`
- `backend/src/services/node_service.rs`
- `backend/src/services/node_ws_manager.rs`
- `backend/src/services/notification_service.rs`
- `backend/src/services/org_service.rs`
- `backend/src/services/provider_service.rs`
- `backend/src/services/proxy_service.rs`
- `backend/src/services/saved_login_service.rs`
- `backend/src/services/unified_key_service.rs`
- `backend/src/services/user_service_service.rs`
- `backend/src/test_utils.rs`
- `cli/Cargo.toml`
- `cli/Dockerfile.machine`
- `cli/Dockerfile.node`
- `cli/build.rs`
- `cli/container/entrypoint.sh`
- `cli/resources/cua/release.json`
- `cli/resources/machine-browser/background.js`
- `cli/resources/machine-browser/content.js`
- `cli/resources/machine-browser/filler.crx`
- `cli/resources/machine-browser/manifest.json`
- `cli/resources/machine-browser/package.json`
- `cli/resources/machine-browser/policy.js`
- `cli/resources/machine-container/LICENSE`
- `cli/resources/machine-container/README.md`
- `cli/resources/machine-container/seccomp.json`
- `cli/scripts/package-machine-filler.mjs`
- `cli/src/cli.rs`
- `cli/src/commands/node.rs`
- `cli/src/node/config.rs`
- `cli/src/node/machine/browser.rs`
- `cli/src/node/machine/commands.rs`
- `cli/src/node/machine/cua.rs`
- `cli/src/node/machine/desktop.rs`
- `cli/src/node/machine/desktop_bench.rs`
- `cli/src/node/machine/files.rs`
- `cli/src/node/machine/gateway.rs`
- `cli/src/node/machine/jobs.rs`
- `cli/src/node/machine/memory_capture.rs`
- `cli/src/node/machine/mod.rs`
- `cli/src/node/machine/native_desktop.rs`
- `cli/src/node/machine/native_desktop_linux.rs`
- `cli/src/node/machine/native_desktop_macos.rs`
- `cli/src/node/machine/process.rs`
- `cli/src/node/machine/runtime.rs`
- `cli/src/node/machine/setup.rs`
- `cli/src/node/machine/transfer.rs`
- `cli/src/node/mod.rs`
- `cli/src/node/proxy_executor.rs`
- `cli/src/node/proxy_upload.rs`
- `cli/src/node/ws_client.rs`
- `cli/src/node_proxy_test_lib.rs`
- `cli/tests/Dockerfile.machine`
- `cli/tests/machine_container_e2e.mjs`
- `cli/tests/machine_filler.test.mjs`
- `docs/ENV.md`
- `docs/MACHINE_NODES.md`
- `docs/MACHINE_NODES_VALIDATION.md`
- `docs/NODE_PROXY_PROTOCOL.md`
- `docs/NYXID_NODE.md`
- `docs/chat/08-nyxagent-engine.md`
- `docs/chat/09-nyxbot-orchestrator.md`
- `frontend/public/machine-seccomp.json`
- `frontend/src/components/assistant/assistant-chat-page.tsx`
- `frontend/src/components/assistant/machine-desktop-panel.test.tsx`
- `frontend/src/components/assistant/machine-desktop-panel.tsx`
- `frontend/src/components/assistant/machine-grant-picker.tsx`
- `frontend/src/components/assistant/nyxbot-agent-details.tsx`
- `frontend/src/components/assistant/nyxbot-agent-forms.test.tsx`
- `frontend/src/components/assistant/nyxbot-agent-forms.tsx`
- `frontend/src/components/dashboard/sidebar.tsx`
- `frontend/src/components/shared/machine-settings.test.tsx`
- `frontend/src/components/shared/machine-settings.tsx`
- `frontend/src/components/ui/textarea.tsx`
- `frontend/src/hooks/use-machines.ts`
- `frontend/src/hooks/use-saved-logins.ts`
- `frontend/src/lib/machine-desktop.test.ts`
- `frontend/src/lib/machine-desktop.ts`
- `frontend/src/pages/admin-usage.router.test.tsx`
- `frontend/src/pages/lazy.ts`
- `frontend/src/pages/machine-desktop.tsx`
- `frontend/src/pages/machine-setup.test.tsx`
- `frontend/src/pages/machine-setup.tsx`
- `frontend/src/pages/node-detail.tsx`
- `frontend/src/pages/nodes.tsx`
- `frontend/src/pages/saved-logins.test.tsx`
- `frontend/src/pages/saved-logins.tsx`
- `frontend/src/router.tsx`
- `frontend/src/schemas/assistant-nyxagent.ts`
- `frontend/src/schemas/machines.ts`
- `frontend/src/schemas/saved-logins.ts`
- `frontend/src/types/nodes.ts`
- `machine/Cargo.toml`
- `machine/resources/cua-tools.json`
- `machine/src/binary.rs`
- `machine/src/config.rs`
- `machine/src/desktop.rs`
- `machine/src/gateway.rs`
- `machine/src/lib.rs`
- `machine/src/signing.rs`
- `machine/src/text.rs`
