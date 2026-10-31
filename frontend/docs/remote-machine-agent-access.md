# Remote Machine Agent access frontend handoff

## Latest focused repair: Q1 save ordering (2026-10-24)

This finite repair owns only the shared capability-save form, its associated
frontend regression checks and this handoff. The current consumers remain
Machine Access (`HostedCapabilities`) and Agent Details (`MachineCapabilities`),
which retain the same form instance across access refreshes. The basis is
GoalArtifact v3 success criteria 2/7 and proposal §§6.1/11.1: consecutive direct
capability saves, including an explicit all-off policy, must use current owner
authority without altering membership, mode, saved-login scope or application
evidence. No compatibility path is added.

Repair plan and ownership:

1. Reproduce Q1 in a new form regression using the actual `useNyxBotAgents`,
   `useMachineAccess`, `useSetMachineAccess`, owner mutation guard and TanStack
   Query client. Defer only the HTTP responses to control query completion.
2. Repair `src/components/assistant/machine-capabilities.tsx` using the existing
   reactive form defaults rather than the completed mutation's submitted
   snapshot. Preserve form identity, dirty gating and the existing owner review.
3. Verify both refresh orders, an intervening newer live policy, consecutive
   all-off submission and draft/revert behavior; run affected components/hooks,
   frontend build/typecheck and lint, then record exact outcomes here.

Independent reproduction completed before the production edit. With machine
access completing before the Agent list, the actual-hook test displayed policy
revision 8 after saving revision 7, then observed an all-off PUT carrying
`expected_revision: 7` instead of 8. The same test's reverse completion order
passed. The transport fixture enforces revision CAS and returns 409 for the
stale request; it is client regression evidence, not real backend acceptance.

The repair uses `form.reset()` after the awaited mutation. `useAppForm` already
tracks server access through its reactive `values`, so resetting to its current
defaults clears dirty state without restoring the submitted stale revision,
capabilities or saved-login IDs. There is no remount, revision arithmetic,
query/mutation contract change or new authorization mechanism.

Focused verification completed (all commands run from `frontend/` unless noted):

```sh
npm test -- src/components/assistant/machine-capabilities-ordering.test.tsx
npm test -- src/components/assistant/machine-capabilities-ordering.test.tsx src/components/assistant/machine-capabilities.test.tsx src/components/assistant/machine-context-review.test.tsx src/components/assistant/nyxbot-agent-details.test.tsx src/components/machines/remote-machine-agent-access.test.tsx src/hooks/use-machine-access.test.tsx src/hooks/use-agent-owner-mutation.test.tsx src/hooks/use-machine-context-review.test.tsx
npm run build
npm run lint
npx eslint src/components/assistant/machine-capabilities.tsx src/components/assistant/machine-capabilities-ordering.test.tsx
# From the repository root:
git diff --check -- frontend/src/components/assistant/machine-capabilities.tsx frontend/src/components/assistant/machine-capabilities-ordering.test.tsx frontend/docs/remote-machine-agent-access.md
```

- Before repair, the initial two-case ordering run exited 1: **1 failed, 1
  passed**, with the exact stale-revision assertion described above.
- After repair, all **3 actual-hook ordering tests passed**, exit 0. They cover
  both query completion orders and an additional manual status refresh to live
  revision 9/new saved-login scope while the mutation's revision-8 response is
  still waiting for the Agent list. The next all-off save uses revision 8 or 9
  respectively and reaches revision 9 or 10 without reload. They also check
  stable form identity, clean/dirty/reverted drafts, retained login restriction
  and absence of fabricated authority/environment-applied status.
- The affected suite passed **116 tests in 8 files**, exit 0, no skips. It
  includes existing review/confirmation, independent flags/gates, save errors,
  capability ceiling/browser dependencies, membership, owner-switch guard and
  status-polling coverage. Hooks and their query/mutation behavior are unchanged.
- The first build exited 2 because the new test used an incomplete `User` cast
  (`TS2352`). Replacing that fixture with a complete `satisfies User` value fixed
  the validation error. The final ordering run again passed **3/3**, exit 0.
  Final `npm run build` passed, exit 0: incremental `tsc -b`, both production
  Vite builds, legal prerender and mock-footprint assertion. Existing
  `text-destructive` CSS pseudo-class and bundle-size diagnostics remain.
- Full lint passed, exit 0, **0 errors and 33 existing warnings**; the final
  changed-file lint passed, exit 0, with no diagnostics. Scoped diff whitespace
  validation passed, exit 0.

Required local repair obligations: **none remaining**. No Rust command was
needed or run, and no backend/CLI/runtime/deploy/integration-evidence file was
edited by this repair. The tests use real frontend hooks and a deferred HTTP
fixture; they establish client ordering behavior only. Browser fixture tests,
real mounted backend, hosted/full-image v4 contexts, CSI/PVC replacement,
amd64/browser sandbox, controller/Node/client authentication, physical stopping
and paid billing acceptance were not run here. Their existing real-environment
gates and limits below remain in force. This is assignment completion evidence,
not formal review, release approval or whole-GoalArtifact satisfaction.

Scope: frontend outcomes of both phases of
`docs/REMOTE_MACHINE_AGENT_SEPARATION_AND_AUTHORIZATION_PROPOSAL.md` and
GoalArtifact v2. This worker owns `frontend/` only and reused the terminal
phase-one implementation. Backend/shared configuration, runtime, image,
renderer and the shared implementation plan are peer-owned. The approved
design needs no further permission. The latest “不需要处理兼容” instruction is
authoritative: current contracts only, with no bridge or migration work.

## Implementation plan

- [x] Add failing component regressions for independent capability/context flags in Access and Details, specific edit gates, partial/all-off saved grants, and approval/delivery/expiry/cleanup wording.
- [x] Update `src/components/assistant/machine-capabilities.tsx` to share those projections and gates without changing the direct capability-save API or fabricating hosted configuration evidence. Preserve `useAppForm`, revision, capability ceiling, browser dependencies and saved-login restrictions.
- [x] Update `src/components/machines/remote-machine-agent-access.tsx` and the Details callers to supply independent flags and membership. Display known Node connectivity separately from authorization.
- [x] Cover cleanup-only recovery polling in `src/hooks/use-machine-access.test.tsx`, then update the hook if needed. Polling remains bounded and observation-only.
- [x] Run affected component/hook tests, frontend typecheck/build and lint. Record exact outcomes and limits below.

## Phase-two implementation plan (2026-10-24, frontend worker)

Ownership is `frontend/` only. Backend, shared configuration, runtime, image,
renderer and shared plan files are peer-owned. The complete proposal is approved;
the latest “不需要处理兼容” instruction supersedes its compatibility paragraphs.
No delegation, git lifecycle or live/paid operations are part of this assignment.

1. Add schema/intent and form regressions for the three hosted choices, explicit
   `remote-machine-separation-v1` acceptance, canonical capability ordering and a
   complete immutable create body across reload, quote failure and retry. Update
   `schemas/machines.ts`, `lib/remote-machine-intent.ts` and the hosted form.
   Reject incomplete new intents; introduce no old-request bridge.
2. Pass the selected Specialist to a focused Access entry after creation, with
   membership-only/all-tools-off wording independent of flags. Own the existing
   setup/search/detail/Access frontend callers and their fixtures.
3. Add regressions for persisted owner prepare/review/commit, server-authored
   exact tool/login scope, expiry, stale/conflicting rereview, account switching
   and no client `Selection` in prepare or commit. Implement schemas/hooks and a
   shared review component used by Access and Agent Details. Capability-only
   changes keep the direct PUT and `useAppForm` path.
4. Parse the actual hosted `machine_configuration` view and Agent assignment
   revision separately. Show current server-applied/pending evidence, target
   config revision/digest/generation, authority delivery/cleanup and the server's
   environment result without inferring success from approval, save or time.
   Keep Machine Ready separate from Agent availability, existing pause/delete
   and accounting, and use only the runtime's documented retained/temporary data
   policy. No whole-machine reconfiguration UI is added.
5. Run affected schema/intent/hook/component tests, frontend build/lint, focused
   Playwright fixture checks and e2e TypeScript. Record exact commands/results,
   interface limitations and real-runtime evidence gaps here before handoff.

## Implemented phase-two outcomes

- Hosted creation presents Automatic, Require separated and Shared legacy.
  Separation acceptance is explicit under `remote-machine-separation-v1` and
  independent of product/data/paid terms. Changing the choice, changing the
  policy or refreshing preflight clears separation acceptance. Require
  separated requires the current account context flag; Automatic explains
  pending owner resolution and never selects shared access implicitly.
- Creation intent contains name, canonical sorted unique capability ceiling,
  Specialist, choice, policy versions, affirmative consent, profile, free/paid
  selection and accepted quote. A lost response or reload reuses the full body
  and idempotency key. Incomplete new create intents fail before persistence or
  dispatch. Rejected requests need explicit reset and review. Creation uses one
  `create` scope and does not read the former alternate creation scope.
- Selected Specialist navigation carries its bounded ID to Machine Access and
  opens that Agent's configuration. Assignment is membership only, with all
  tools and saved logins off regardless of flags. Permission changes remain a
  separate direct PUT. Membership removal, partial/all-off grants, capability
  ceilings and other service/login grants retain their existing behavior.
- Access and Agent Details share `MachineContextReview` through the existing
  capability form. Prepare sends mode/version/affirmative acknowledgement only;
  it does not send a browser `Selection`, tools or login IDs. A successful GET of
  the persisted review is required before confirmation. The page displays the
  server-authored Agent/machine/mode/policy/revision, full tool and saved-login
  scope including `null` versus `[]`, additions/removals, warning, storage note,
  expiry and hosted target revision/digest/generation/credential epoch.
- Confirmation is bound to the displayed stored snapshot. Commit sends `{}` to
  the review ID. Stale/expired/failed/409 responses require prepare and a fresh
  review/checkbox. Current configuration loss/digest/generation drift or lost
  separation support disables confirmation and clears prior acceptance.
  Unsaved capability edits, owner maintenance/read failures and pending current
  hosted configuration also prevent mode submission. Account changes clear the
  owner component and the existing owner mutation guard prevents stale writes.
  The server's first-party human guard and transactional fences remain the
  authorization authority; the frontend adds no alternate token classifier.
- `MachineApplicationStatus` keeps Agent assignment revision, Agent policy
  revision, current hosted configuration version/revision/digest/runtime
  generation, authorization application and environment application distinct.
  The final applied label requires both server observations, applied current
  configuration with no reason, and any reviewed target match. Missing/unknown
  observations, approval, save, elapsed time or authority ACK cannot create it.
  Existing authority delivery/expiry/quarantine diagnostics remain separate.
  Read-only status polling is bounded and has explicit manual refresh.
- Overview labels Machine Ready independently of Agent availability, explains
  current per-Agent/default semantics, and retains existing Pause/Delete and
  billing flows. Agent failure does not imply pause, refund, deletion, physical
  stop or charge finality. Creation and policy text follow the runtime's stated
  retained legacy/separated workspaces and protected journal, with temporary
  HOME/browser/download/tmp and no file/cookie migration. This is a disclosure
  of the documented contract, not a claim of successful hosted recovery.

## Concrete interfaces used

Source was inspected directly in `backend/src/handlers/remote_machine.rs`,
`backend/src/handlers/machine_access.rs`,
`backend/src/services/machine_context_review_service.rs`,
`backend/src/services/remote_machine_configuration.rs`,
`backend/src/models/{remote_machine,machine_access}.rs`, `backend/src/routes.rs`
and `machine/src/managed.rs`. Runtime storage policy comes from
`deploy/remote-machine/HOSTED_SEPARATION_SUBSTRATE.md`.

All paths below are relative to `/api/v1`:

| Interface | Frontend use |
| --- | --- |
| `POST /remote-machines` | Existing lifecycle/idempotency flow plus required `separation_choice`, `separation_policy_version`, `acknowledge_separation` |
| Entitlement/quote policy | Required separation version/three choices/warning and `shared_environment_mode=per_agent`; existing pricing gates unchanged |
| `GET /remote-machines[/{node}]` | Required current safe configuration v2 and Specialist membership states, including `not_assigned` |
| `GET /assistant/nyxagent/agents/{agent}/machines` | Separate assignment/policy revisions, membership, both server flags/reasons, current `context_policy_version`, support, authority and configuration observations |
| `PUT /assistant/nyxagent/agents/{agent}/machines/{node}` | Existing capability-only revision CAS; no context-mode field added |
| `POST /assistant/nyxagent/agents/{agent}/machines/{node}/context-prepare` | `{mode, policy_version: "machine-context-mode-v1", acknowledge_separation: true}` only |
| `GET /assistant/nyxagent/agents/{agent}/machines/{node}/context-reviews/{review}` | Read persisted typed action scope and current applicability/application observations |
| `POST /assistant/nyxagent/agents/{agent}/machines/{node}/context-reviews/{review}/commit` | Empty JSON object; server applies the stored action through configure |

Managed configuration and bootstrap payload semantic version 2 are independent
of envelope version 1, authority version 2 and separation evidence version 4.
Frontend never receives bootstrap credential material and does not derive or
author a managed configuration digest.

## Verification

The earlier phase-one baseline was 180 tests in six files, three fixture
Playwright cases, build/e2e TypeScript and lint (0 errors, 33 existing warnings).
It is historical evidence for that slice, not whole-goal or hosted acceptance.

Final affected tests: **437 passed in 24 files**, exit 0, no skips:

```sh
DEBUG_PRINT_LIMIT=0 npm test -- src/schemas/machines.test.ts src/schemas/remote-machine-separation.test.ts src/lib/remote-machine-intent.test.ts src/lib/machine-search.test.ts src/hooks/use-machine-access.test.tsx src/hooks/use-machine-context-review.test.tsx src/hooks/use-machines.test.tsx src/hooks/use-remote-machine-admission.test.tsx src/components/assistant/machine-capabilities.test.tsx src/components/assistant/machine-context-review.test.tsx src/components/assistant/machine-application-status.test.tsx src/components/assistant/nyxbot-agent-details.test.tsx src/components/machines src/components/shared/machine-isolation.test.tsx src/pages/machine-setup.test.tsx src/pages/machines.test.tsx
```

Final fixture browser checks: **6 detail cases plus 2 creation cases passed**,
both commands exit 0:

```sh
npx playwright test --config=playwright.machine-detail.config.ts --grep 'context opt-in|assignment survives|owner stored review|Machine Ready with Agent'
npx playwright test --config=playwright.remote-machine.config.ts --grep 'free creation'
```

Detail checks cover 1440px/320px partial saved File scope with capability editing
off, membership surviving a failed capability save, a 409 followed by a fresh
all-off stored review and exact empty-body commit, and Machine Ready with
separated Agent support unavailable while fees/Pause/Delete remain visible.
Creation checks cover 1440px/390px independently required policy checkboxes,
canonical complete request/key, cleared acknowledged intent and operation
navigation. Browser layout assertions check horizontal overflow; captured
320px owner-review and 390px creation screenshots were visually inspected.

New regressions were observed failing before fixes for missing create consent,
stored review contracts, disappearing support, missing/current-config drift,
retained acceptance after a digest change, and incomplete create persistence.
Existing fixture expectations were updated to the current new contract,
including membership `not_assigned`, the single create scope and canonical
capability ordering. No old-format acceptance path was added.

Screenshot QA additionally found checked creation acknowledgements retaining
earlier validation errors after a capability edit. Both acknowledgement
controls now use `useAppForm.setValue` for immediate dirty/touch/validation
updates. The new regression failed before the fix; the final unit and browser
runs verify that those errors clear before submission.

Final `npm run build` passed, exit 0: `tsc -b`, both Vite production builds,
legal prerender and mock-footprint assertion. It retained the existing CSS
`text-destructive` pseudo-class and bundle-size diagnostics. Final
`npm run lint` passed, exit 0: **0 errors and 33 existing warnings**, with no new
warning added. `npx tsc --project tsconfig.machine-detail-e2e.json --noEmit`
and root-directory `git diff --check -- frontend` passed, exit 0.

No Rust invocation, real backend/Node/runtime execution, cluster mutation or
paid-resource operation was required or performed for these frontend checks.

## Remaining integration and evidence limits

The required owner prepare/read/commit interfaces exist in current source; no
alternate backend was invented. Backend/shared/runtime final checks and any
late contract drift remain integration-owned. This frontend worker ran no Rust
commands and changed no backend/shared/runtime files, branch, worktree or git
lifecycle state.

The HTTP safe configuration view exposes only version/revision/digest/runtime
generation and a summary `state`/`reason` (currently pending/applied). It does
not expose separate configuration transport/delivery stages or the raw actual
runtime/connection/credential-epoch/observation-time evidence record. The UI
renders the supplied summary and reasons. Detailed actual evidence/stage
inspection remains unavailable without an additional peer-owned interface;
the UI never manufactures those fields from Agent ACKs or timing.

All browser HTTP/WebSocket calls are intercepted with the backend proxy aimed
at `127.0.0.1:9`. These checks establish client behavior only, including request
bodies, conflict handling and responsive layout. They do not exercise a mounted
real backend, real Node or paid runtime.

Formal acceptance remains unverified for two concurrent hosted full-v4 contexts
with Shell/File-only ceilings and Browser/Computer denial; actual worker-role
cross-context/legacy/credential/socket custody; protected UID supply/exhaustion
under the final image; original CSI/PVC replacement and pause/resume with
workspace and generation/tombstone continuity; temporary-state reconstruction;
arm64/amd64 browser sandbox under the intended RuntimeDefault/AppArmor policy;
stale lease/login rejection; live controller/Node/client authentication;
physical stopping and real billing. Peer local Docker/native/renderer tests
are not substituted for these scenarios. Runtime support/readiness gates stay
server/runtime-owned and fail closed. No deployment, production, cluster or
paid-resource operation was performed by this worker.
