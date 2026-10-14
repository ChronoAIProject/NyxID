# Tools and Catalog Manager delivery handoff

Status: incomplete; implementation is uncommitted. This document records the
user's updated acceptance requirements and ordinary Heca session handoff. It
does not modify the original Tools Manager plan or confer publication authority.

## Ownership and constraints

The user now requests Opus 5.5 xhigh as the main PM, GPT-6-Astra xhigh full-access
as the implementation session, and personal review by the coordinating agent.
Use ordinary Heca sessions only; no subagents. This supersedes the previous
GPT-6.1-Sol/no-external-session instruction. Preserve the shared worktree and
`docs/plans/2026-10-13-tools-manager-cli.md`. Do not commit, push, create a PR,
merge, deploy, or use production credentials/services for tests. Do not modify
the coordinator-owned `2026-10-13-tools-manager-review.md`.

Admin alone publishes Tools and configures their audience/prices. Managers
prepare, validate, edit, and pause their own server-marked Tools. All actionable
findings at every severity must be fixed and verified. External gates remain
explicitly pending until actually performed.

## Updated Catalog Manager acceptance

For OAuth catalog services, the CLI must support all six playbook items:

1. Configure the NyxID-managed OAuth client ID and secret without leaking them.
2. Set default scopes matching the vendor's official CLI login scopes; absent
   such a CLI, use the documented minimum for the exposed operations. Presence
   of scopes alone cannot prove this; retain the scope source as evidence.
3. Configure authorization, token, and revocation URLs.
4. Default to `credential_mode: admin`; `both` also supports general connection.
   `user` does not meet this playbook criterion.
5. Set a suitable OpenAPI URL or author explicit catalog operations.
6. Upload a static SVG icon.

Configuration readiness alone is not Verified. The API smoke test and a catalog
Admin's confirmation that an outside user connected their own vendor account
are additional acceptance gates. Local fake-vendor tests may demonstrate the
software flow but cannot establish real-vendor/outside-user verification.
Do not fabricate evidence or test against production to close that gate.

Provider-less shared-credential Tools do not use OAuth clients, provider URLs,
or user OAuth connections. Their equivalents are first shared credential (or
no-auth), curated operations, SVG icon, live audience ACL, Admin publication,
and ordinary-user execution/billing smoke tests. Keep the two contracts explicit.
Non-OAuth providers need explicit applicability rather than invented OAuth URLs.

The original plan already identified two Catalog CLI defects that Tools must
not inherit: active-only row lookup and credential environment access before
dry-run planning. Reconcile the updated Catalog requirements with those paths.

## Existing implementation and evidence

Read AGENTS.md, DESIGN.md, the preserved plan, docs/TOOLS.md,
docs/CATALOG_MANAGER.md, docs/API_DISCOVERY.md, and relevant security/billing
docs. Review the actual diff before treating any implementation claim as true.

Implementation covers authority/live transactional fences, first-credential
freeze, restricted-empty initial audience, sanitized twins, Admin-only
publication, contract demotion/generation, no-auth ACL, redirect suppression,
SVG icons, manifests/variables/wizard/apply/validate/icon, frontend parity, tests,
docs, and an executable loopback fake-vendor E2E. The detailed redacted evidence
is in `2026-10-13-tools-manager-verification.md`.

Recorded completed checks (inspect logs; these are not blanket final signoff):

- CLI run15: 1,567 passed, zero failed, six ignored; includes 18 Tool tests.
  `/tmp/tools-manager-cli-all-target-final15.log`.
- Frontend: 494 files, 5,143 tests passed; build and changed-source lint passed.
  `/tmp/tools-manager-frontend-full-final.log`,
  `/tmp/tools-manager-frontend-build8.log`.
- Workspace Clippy7 passed with warnings denied.
  `/tmp/tools-manager-workspace-clippy7.log`.
- E2E13 passed, one test, 4.68s, on an earlier snapshot.
  `/tmp/tools-manager-e2e13.log`.
- Pool billing seven tests passed; other affected suites are recorded in the
  verification report. Remote CI and model signoffs have not happened.

No full backend green result exists. Run5 selected 7,923 tests and recorded
seven failures before interruption. Three production regressions/fixtures were
corrected, and additional timeout fixtures were adjusted while retaining their
dispatch/cancellation assertions. Auth-device approval passed unchanged in
isolation; denial's outcome and the full-run panic diagnostics remain unknown.
Never classify these as environmental without evidence.

Run5 failure names:

- pool_proxy_billing_unknown_timeout_releases_hold_without_inventing_usage
- pool_proxy_overall_deadline_stops_before_backup_dispatch
- tool_twins_copy_contracts_as_drafts_without_mutating_source_or_copying_secrets
- linked_shared_service_preserves_secrets_and_policy_and_enforces_admin_access
- assistant_oneshot_timeout_cancels_without_retry_and_settles_meter
- approve_pending_row_encrypts_tokens_shortens_expiry_and_audits
- deny_pending_row_sets_terminal_fields_and_audits

Latest corrections: optional twin names inherit/preserve; preflight allows
omitted twin name only; non-Tool credential presence remains hidden from
non-admins; Tool freeze marker is not set/reported on other service kinds.
Assistant one-shot timeout correction has not passed yet.

## Interrupted verification and environment

On this continuation, old PIDs 73475, 73900, 78651, 86931, and owned Mongo PID
71060 no longer exist. Logs end without final results. Treat full run5,
correction suite14, E2E14, and denial investigation as interrupted/pending;
passing individual lines do not establish suite completion.

- `/tmp/tools-manager-backend-full-tests5.log`
- `/tmp/tools-manager-backend-final-corrections14.log`
- `/tmp/tools-manager-e2e14.log`
- `/tmp/tools-manager-auth-device-deny-investigation.log`

Do not touch the other actor's Mongo PID 31485 on port 27017 or their test
processes. Previously owned DB was loopback port 27943, replica set
`tools_manager_local`, dbpath `/tmp/nyxid-tools-manager-db-final`. Check ownership
and health before reuse or start a fresh dedicated instance. Mongo locally is
9.0.2 versus CI target 8.0; Rust locally is 1.96.0 versus CI pin 1.98.1.
The host has 16 GB RAM; avoid overlapping large compilations and index-heavy
database suites. Available disk at handoff was 57 GiB.

Preserved plan SHA256:
`79cf53cf2bd9928003540db7de02f0617e9e98e1f3d376771888b0fd619aec84`.
The source inventory/report hashes must be recomputed after further edits.

## PM deliverables and implementation sequence

Opus should audit the actual code against both acceptance contracts, write a
concrete gap/acceptance matrix and execution plan in a new PM-owned document,
identify any justified applicability exceptions, and avoid expanding to a new
verification product without evidence the playbook requires it. Distinguish CLI
support, automated local proof, and real-service operational confirmation.

Astra should own all implementation changes and verification. Begin with an
evidence audit, recover interrupted local checks efficiently, and implement
the PM's concrete gaps. Keep a detailed redacted report with commands, exit
codes, snapshots, failures and their resolution. The coordinator personally
reviews every changed path and sends fixes back; repeat until every actionable
finding is closed. Do not treat this handoff or creation of sessions as delivery.
