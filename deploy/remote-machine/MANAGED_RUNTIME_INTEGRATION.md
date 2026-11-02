# Managed runtime new contract integration

Historical finite-worker handoff. The source gaps and counts below describe that
worker's snapshot. Final storage/allocation and ordinary context-generation
integration, exact current contracts and verification are tracked in
[the final integration evidence](../../docs/REMOTE_MACHINE_SEPARATION_INTEGRATION_EVIDENCE.md).
Do not treat the historical producer-gap statements as the final source status.

Scope: this worker owns CLI bootstrap/config/runtime/protocol integration and tests. Backend/shared machine contracts and frontend remain peer-owned. This is implementation evidence, not hosted acceptance or deployment approval.

## Plan recorded before implementation

1. Test semantic payload v2 installation for all three separation choices, exact configuration revision/digest, canonical ceilings, strict missing/unknown versions and malformed payload rejection before credentials/config persistence. Use the shared `managed::BootstrapPayload` and `RequiredConfiguration` directly.
2. Persist the sealed configuration identity in NodeConfig, validate the actual local machine config exactly against it on restart, send payload version in both workload prepare/bootstrap requests, and reject ordinary local changes. No legacy defaults or compatibility bridge.
3. Bind the installed configuration and live managed runtime in the actual WSS parent. Report actual validated configuration/digest/current generation only after binding; keep this separate from Agent authority ACK and full v4 separation probes.
4. Install the protected store in the parent dispatcher only with authenticated original PVC UID and explicit fresh-allocation versus recovery authority; use the live daemon runtime UUID. Deny managed native-store fallback and quarantine ACK without protected store recovery. Preserve retained UID/workspace/generation/tombstones and reject profile reconstruction without an existing authorized generation advance.
5. Run focused bootstrap/config/report/dispatch regression tests, macOS and Linux incremental CLI checks. Keep hardened filesystem fixture evidence separate from formal two-context Chromium/selected-cluster evidence.

## Source-contract gap at intake

`machine/src/managed.rs::BootstrapPayload` currently contains payload_version, credentials, RequiredConfiguration and configuration_digest only. `Binding` and authenticated `auth_ok` also contain no original PVC UID and no authoritative allocation/recovery fact. `backend/src/services/remote_machine_bootstrap.rs::bootstrap` constructs that exact shared payload. Environment values, path existence, credential epoch and generation are not substitutes for a fresh-allocation fact. This worker will not invent that authority or modify peer-owned shared/backend contracts. A small producer addition is required before the actual daemon can open the hosted retained store.

A restarted daemon must also obtain an increased context generation through the existing signed Agent Authority contract before recreating ephemeral profiles. CLI must never increase it on behalf of owner consent.

## Implemented integration

- Prepare and bootstrap requests explicitly request semantic payload v2. The
  recipient decodes the shared strict payload, validates current configuration
  version, positive revision, canonical ceilings and exact digest before creating
  credential storage or persisting the installed config. All three approved
  separation choices are installed directly; no old payload default remains.
- `NodeConfig.managed_configuration` retains the exact configuration report.
  Managed restart checks require its runtime generation to match the sealed
  binding and compare the entire actual machine config against the configuration
  derived from that report. Shell/File ceilings retain full infrastructure while
  Browser/Computer API permissions remain false.
- The real WSS parent binds that actual configuration only after the server
  acknowledges the exact managed binding and the signed runtime window validates.
  `MachineProfile.managed_configuration` reports it alongside the live daemon
  UUID. Configuration installation, Agent authority ACK and complete v4 evidence
  remain separate checks.
- The parent installer opens `Store::open_hosted` with a supplied original-volume
  binding, explicit allocation/recovery decision and its own live daemon UUID;
  it installs the store in the same dispatcher that provisions and quarantines
  contexts. It refuses unbound parents and volume substitution. There is no
  production caller until the producer supplies authenticated storage authority.
- Missing hosted storage makes managed separated support unavailable and denies
  context dispatch and quarantine ACK. A pending cleanup still durably fences the
  authority revision. The native temporary journal cannot stand in for the
  protected retained store after restart.
- Retained workspace/UID and generation/tombstones remain predecessor substrate
  responsibilities. Integration tests exercise reopening that store in a new
  daemon, refusing former-generation profile reuse, preserving a real dropped-UID
  workspace marker, accepting only a supplied newer authority generation, and
  denying quarantined authority. They do not issue real owner consent.

## Exact remaining producer obligations

1. Deliver authenticated **original PVC UID** and an explicit **fresh allocation
   versus recovery decision** to the Node bootstrap/runtime consumer. The current
   sealed `BootstrapPayload`, `Binding` and authenticated `auth_ok` contain neither.
   `RuntimeTarget.pvc_uid` belongs to the independent provider/deadline control
   path and is not Node bootstrap authority. Once the existing authority produces
   these facts, the daemon must retain them in protected config and call the
   parent installer after live WSS binding and before reporting support. An
   environment variable, empty directory, generation/epoch number or client
   assertion cannot authorize a new journal.
2. Before ephemeral browser reconstruction after daemon replacement, deliver a
   greater context generation through the existing signed Agent Authority path.
   Current backend admission reuses the persisted generation; `configure` advances
   it for mode/saved-login changes. Automatic runtime-recovery generation issuance
   is not supplied. The CLI keeps same-generation recovery denied and cannot
   manufacture an approval or increment the generation itself.

Backend/shared sources were inspected directly; this worker did not add guessed
fields, change their types, or implement an alternate orchestrator. These gaps
prevent production separated execution; local fixtures do not satisfy hosted
rollout acceptance.

## Local verification

Every Rust invocation and the disposable Docker stage explicitly uses
`CARGO_INCREMENTAL=1`. Host checks reuse
`/tmp/nyxid-hosted-substrate-target`; Linux checks reuse
`/tmp/nyxid-hosted-linux-target`. No cache was cleaned.

| Command | Actual result |
|---|---|
| `CARGO_INCREMENTAL=1 CARGO_TARGET_DIR=/tmp/nyxid-hosted-substrate-target cargo test -p nyxid-cli --bin nyxid node::managed_runtime::tests -- --nocapture` | macOS: 20 passed, none ignored. Includes all choices, strict malformed semantic payload rejection before identity persistence, exact revision/digest and restart generation/config identity. |
| `CARGO_INCREMENTAL=1 CARGO_TARGET_DIR=/tmp/nyxid-hosted-substrate-target cargo test -p nyxid-cli --lib --features node-proxy-test` | macOS: 218 passed, 2 existing ignored; runtime/authority/custody regressions and actual-config report refusal. |
| `CARGO_INCREMENTAL=1 CARGO_TARGET_DIR=/tmp/nyxid-hosted-substrate-target cargo test -p nyxid-cli --bin nyxid managed_handshake_refuses_missing_or_stale_fence_before_runtime_start -- --nocapture` | macOS: 1 passed; neither missing nor stale authenticated binding starts the runtime. |
| `CARGO_INCREMENTAL=1 CARGO_TARGET_DIR=/tmp/nyxid-hosted-substrate-target cargo check -p nyxid-cli --bin nyxid --lib --features node-proxy-test` | macOS library and binary checks passed. |
| `sh cli/tests/managed_runtime_integration/run.sh` | Linux arm64, exit 0: 3 parent/config/quarantine tests passed with `--include-ignored`; 1 actual authenticated WSS configuration-report test passed with `--ignored`; all 20 managed-bootstrap tests passed; CLI library and binary offline/locked checks passed. Every Cargo subprocess explicitly sets `CARGO_INCREMENTAL=1`. The fixture keeps read-only root, no-new-privileges and exactly CHOWN/SETUID/SETGID/KILL. |
| Separate `sh -n cli/tests/managed_runtime_integration/run.sh` and `sh -n deploy/remote-machine/tests/full-machine-image-smoke.sh`; Python `ast.parse` of the smoke fixture | Passed syntax checks. The full-image smoke was updated to v2 and the protected retained mount layout, but was **not executed**. |
| `CARGO_INCREMENTAL=1 rustfmt --edition 2024 --check` on all worker-owned Rust files; `git diff --check -- cli deploy/remote-machine` | Passed. |

The semantic-choice and malformed-payload regressions were observed failing
before installation was corrected. The hardened parent test then exposed a
restart defect: root without `DAC_OVERRIDE` cannot open an existing worker-owned
0700 HOME for reading. A disposable four-capability syscall reproduction
confirmed the boundary. The runtime now pins and validates the directory with
`O_PATH`, takes ownership through that descriptor using existing `CHOWN`, then
reopens it before restoring the private role ownership. This adds no capability
and retains no-follow checks and supervisor-owned parent gates. The three
focused Linux parent/config/quarantine tests passed after the fix.

The final Linux binary check reports three dead-code warnings for the pending
production installer and its two volume-binding helpers. They are deliberately
not suppressed: only the trusted fixture can supply the missing producer facts
today. Cargo also reports the existing upstream future-incompatibility warning
for `proc-macro-error2` 2.0.1. An earlier Linux attempt returned dependency E0463
errors; the subsequent run reused the same target without cleaning and compiled
those dependencies. A first integration compile also caught the sibling
dispatcher-store visibility error, corrected with `pub(super)`.

The final direct source recheck still finds the exact shared v2 configuration
types and sealed producer described above; neither bootstrap nor `auth_ok` has
the missing storage authority. No backend/shared type was edited by this worker.

## Acceptance limitations

The disposable Linux image supplies immutable accounts and the actual CLI, but
has no Chromium/CMA/Heca. Its browser probe correctly remains unavailable;
complete evidence v4 is still required. Real filesystem reopening and dropped-UID
access are fixture evidence, not original-PVC/CSI or browser acceptance.

Still unverified: the formal full hosted image; two simultaneous complete-v4
contexts including secure/developer browsers and real stale-login/lease checks;
selected Pod RuntimeDefault/AppArmor observability; amd64; actual original-CSI-PVC
replacement and pause/resume; real owner/backend/external-client authentication;
controller, stop observations and Billing. No production or paid resource was
mutated. The two producer obligations above remain code-contract gaps in
addition to these environment acceptance limits.

## Later-source first-dispatch repair

The integration evidence now has a separate round1 ARCH-1/TEST-1 addendum:
[changed-source checks and remaining gates](../../docs/REMOTE_MACHINE_SEPARATION_INTEGRATION_EVIDENCE.md#later-source-round1-repair-arch-1-and-test-1-2026-10-24).
The hardened runner hands all three homes to their startup worker UIDs at
`0700`, then exercises actual signed File/Shell dispatch through the retained
parent. Private homes are pinned with `O_PATH`, ownership is taken through that
same descriptor using existing CHOWN, and mode/UID ACL work reads the pinned
inode. The test asserts the four-capability ceiling, NNP and read-only root.
Its supplied prerequisite evidence is test data; missing browser resources still
refuse execution and cannot select shared mode.

This local kernel's tmpfs rejects POSIX ACLs. The runner uses an anonymous
Docker disk volume for `/home`, removed with its disposable container, so the
real browser UID ACL and policy-group denial can be exercised. This changes only
the test fixture; production ACL, browser, full-v4 and deployment gates remain.
The full-image smoke separately checks actual advertised support against its
authenticated installed retained store and v4 evidence, independent of the
fixture's shared selection. Its local consumer tests are not a full-image run.
