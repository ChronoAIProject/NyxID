# W5–W9 implementation worker log

This file records worker changes and executed evidence. It does not replace the
PM-owned execution/review ledgers, and a worker test pass is not a PM review
verdict. Baseline: `f96cf877834445d4976f62691aa192e534535c1f`, branch
`security-review-remediation-report`. The source review report is preserved.

## Runtime and authorization

The actual initial Codex turn context recorded `gpt-6.1-sol`, `xhigh`,
`danger-full-access`, approval `never`, provider `heca_gateway`; PM independently
verified it and authorized implementation. No other workers or reviewer agents
were spawned. Work is local; no publication, deployment, push or merge is part of
this authorization.

Required `CLAUDE.md`, the full source assessment and execution/review documents
were read before coding. Relevant subsystem documents are read before changes.
PM owns checklist/review status and reviews each bounded commit personally.

## W6 verifier and explicit source updates — host verified, PM container review pending

Implemented SET-authenticated Rekor time, exact DSSE body/certificate/signature
binding, Merkle proof and signed checkpoint verification, historical CT/Rekor
key validity and Fulcio trust/anchor validity at event time, GitHub OIDC issuer
and exact workflow/ref identity, bounded JSON/material inputs and restricted
bundle redirects. Production TUF authentication remains mandatory.

Public logged NyxID v0.15.0, v0.40.0 and v0.71.0 producer fixtures are retained
with sources/digests in `cli/tests/fixtures/sigstore/README.md`. All original
fixtures are v0.3, `dsse/0.0.1`; older promise-container policy cases are explicit
transformations. Rekor v2/RFC3161-only bundles remain unsupported and fail closed.

Source updates now require explicit `--from-source`, resolve a real release to a
full immutable commit, use Cargo `--rev` and `--locked`, validate the compiled
version, activate atomically, and persist a source marker/update hold. Missing
releases/assets cannot install default-branch HEAD. The verification-bypass flag
is removed; source builds cannot replace the trusted scheduled controller.

Executed intermediate evidence (superseded by final checks below): initial full
build completed in6m24s, with18 passes/7 fixture failures/1 ignored. Those failures
identified the unsupported assumption that every TUF log ID is SHA256(SPKI):
Rekor v2 uses a note-key ID. Root parsing now retains TUF's format-specific IDs,
and supported Rekor v1 explicitly checks its own ID contract. Subsequent updater
runs passed64 and68 tests with no failures, each leaving only the explicitly
ignored live TUF check. Expanded cryptographic cases assert relevant error
contexts and include independent recursive RFC6962 paths for sizes1..33.

Validation root:
`/var/folders/y2/9pj294vd0qjfp88k419b5j8r0000gn/T/nyxid-w5w9-validation-3moeqwm0`.
Cargo uses its `target` directory, jobs2, incremental off, dev/test debug info0.
Final host commands (same Cargo environment above):

- `cargo test --locked -p nyxid-cli --bin nyxid update -- --nocapture`:
  169 passed, 0 failed, 4 ignored, 1445 filtered (`w6-update-final.log`).
- `cargo test --locked -p nyxid-machine-updater -- --nocapture`: library
  120 passed, 0 failed, 1 ignored; binary 78 passed, 0 failed, 4 ignored;
  doctests 0 (`w6-machine-final.log`). These share verifier source; the counts
  describe separate consumers and are not unique test counts.
- `cargo test --locked -p nyxid-cli --bin nyxid github_tuf_mirror_verifies_root_rotation_expiry_and_target_hash -- --ignored --nocapture`:
  1 passed, 0 failed (`w6-live-tuf-final.log`). This actually refreshed the
  GitHub mirror and checked the authenticated target using persisted state.
- `cargo clippy --locked -p nyxid-machine-updater --all-targets -- -D warnings`:
  passed after factoring the validity-interval return type into an alias
  (`w6-updater-clippy-final.log`).
- `cargo check --locked -p nyxid --bin nyxid-server`: passed
  (`w6-backend-check-final.log`).
- `cargo fmt --all -- --check` and `git diff --check`: passed.

Ignored-test accounting: the live TUF test was explicitly executed above;
`companion_process` is a child-process entrypoint, not standalone acceptance.
`real_container_migration_signed_upgrade_and_rollback` and
`real_companion_handoff_and_rollback` require the Docker harness
`cli/tests/machine_updater_e2e.sh`; PM owns their execution on this stable source
against the immutable released machine supervisor. They remain pending and are
not counted as passes.

The signed mini-TUF tests exercise restart/concurrent process locking, signed
still-unexpired timestamp/snapshot rollback, expiry, legitimate role rotation,
embedded bootstrap advancement ahead/behind cached lineage, incompatible forks,
corrupt/missing initialized state, authenticated legacy cache migration and a
nonroot read-only installation with a writable private state volume. Native,
companion and server state paths are absolute/private/durable, with no ephemeral
fallback. The server registry token is bounded and zeroizing.

Compiler future-compatibility warning: `proc-macro-error2` 2.0.1 reexports a
private `proc_macro` extern crate (E0365, rust-lang/rust#127909). This is recorded
for W7 dependency remediation, not hidden and not counted as a W6 failure.
`w7-future-incompat-inventory.log` retains Cargo's report.

## Remaining authorized batches

W5 production CSP/header/runtime-origin delivery and browser matrix; W7 pinned,
locked and scanned builds, shipped-artifact SBOMs and mandatory official manifest
signing; W9 all new-key creation defaults, readiness, real OS identity/key-service
boundaries and resumable full secret migration; W8 strict attribution, complete
critical-operation durability, independent protected witness, prefix retention,
key lifecycle and scoped API/UI. Major W8 code follows a concrete operation and
architecture proposal sent to PM. No scope row is closed by this log.
