# Aevatar chat feature integrate convergence plan

This program closes the NyxID browser-chat gaps against Aevatar `feature/integrate` for NyxID users and the engineers who maintain the integration.
Done means the pinned contract, typed commands, access review, context attachments, lifecycle recovery, and a credentialed producer canary all pass at named NyxID and Aevatar SHAs.
The program excludes Direct Chrono-LLM, Studio workflow chat, voice, channels, dormant Wave-2 actions, `inputParts`, and explicit `agentProfile` selection.
Execute `AC-0` through `AC-7` in order. Stop every phase at merge-ready.

## How to read this

One box is one unit of work. Every box names the evidence that proves it. A nested box is a sub-step of the box above it. Check a box only when its evidence exists as a file, log, screenshot, recording, test run, link, or SHA. The body is a how-to. The appendices explain decisions and preserve evidence.

The program runs `references/playbook-autopilot-stack.md` from the active `heca-mode` skill. Repository policy comes from `CONTRIBUTING.md` and `WORKFLOW.md`. The operator retains merge authority. Every phase stops at merge-ready with `auto_merge` disabled.

Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

## Program checklist

### Arm the program

- [ ] Present this plan and `docs/plans/aevatar-chat-feature-integrate-convergence.md` to the operator, then stop until explicit authorization. Evidence is the operator's go message.
- [ ] Record `AC-0 -> AC-1 -> AC-2 -> AC-3 -> AC-4 -> AC-5 -> AC-6 -> AC-7`, the done predicate, exclusions, merge authority, and repository policy sources. Evidence is the program entry in the decision trail.
- [ ] Read `references/playbook-autopilot-stack.md` at program start and after every resume. Evidence is the skill path and version in the latest status report.
- [ ] Use a standing Heca task for the dependent stack and no team or loop unless execution shows that the coordinator needs one. Evidence is the Heca task id or the recorded reason for changing this choice.

### Assign owners and verifiers

- [ ] Resolve delegated owners and verifiers from `~/.heca/pstack-models.json` against one live provider catalog snapshot. Evidence is an assignment table with provider, model, effort, and spawn mode.
- [ ] Spawn each delegated role with `heca_agent_spawn` and `notify_on_finish` set to `true`. Evidence is the agent id beside its phase id.
- [ ] Give every writer one branch or worktree and hold the dependency graph at the root. Evidence is the branch, worktree, base SHA, and allowed file boundary in the assignment table.
- [ ] Keep each verifier independent from the owner and read-only on the owner's branch. Evidence is the verifier assignment and its exact head SHA.

### Hold repository policy

- [ ] Re-read `CONTRIBUTING.md` and `WORKFLOW.md` before execution. Evidence is a policy note that records the `main` target default, one required approval, explicit security review for security-sensitive work, required `CI Pipeline`, and disabled auto-merge.
- [ ] Keep each phase independently reviewable and buildable. Evidence is a green exact phase head with no planned internal breakage.
- [ ] Preserve the ordered base and head chain after any topology change. Evidence is the refreshed phase table with exact SHAs.
- [ ] Never merge, close, or arm auto-merge without explicit authority and a clean repository-policy verdict. Evidence is the authorization record or the merge-ready handoff.

### Verify exact heads

- [ ] Run repository, live, and warranted risk checks at each exact phase head. Evidence is the command output and real-surface artifact linked from that phase.
- [ ] Send every finding back to the owner and reverify after any head change. Evidence is the new head SHA and replacement verdict.
- [ ] React to Heca finish notifications and report material state changes without polling agents. Evidence is the notification-linked status entry.
- [ ] Hold a throughput checkpoint after `AC-1` and `AC-4`; advance only when the completed phase is green and the next phase still uses its verified contract. Evidence is the checkpoint verdict with the parent and child SHAs.
- [ ] End each phase with a handoff that names what changed, predicate state, open risks, and exact next input. Evidence is the phase handoff in the decision trail.

## Pin the upstream contract and automate drift detection (AC-0)

**Depends on.** None.

**Owner.** A delegated contract-tooling owner on the stack root branch.

**Verifier.** An independent read-only integration-contract verifier at the exact `AC-0` head.

**Files.**

- [ ] Add `tests/fixtures/assistant/aevatar-chat-contract-pin.json` with the audited branch, remote head, effective chat SHA, watched paths, public command set, internal action set, and context-attachment rules. Evidence is the parsed fixture at the phase head.
  - [ ] Watch exactly these upstream paths, namely `agents/Aevatar.GAgents.NyxidChat/`, `apps/aevatar-console-web/src/pages/chat/`, `apps/aevatar-console-web/src/shared/auth/client.ts`, `src/Aevatar.Mainnet.Host.Api/Chat/MainnetChatEndpoints.cs`, `src/Aevatar.Mainnet.Host.Api/Responses/NyxIdIdentityAssertionAuthentication.cs`, `src/Aevatar.AI.ToolProviders.NyxId/Tools/NyxIdRequireServiceTool.cs`, `src/Aevatar.AI.ToolProviders.NyxId/NyxIdActionEvidenceReadPort.cs`, `src/Aevatar.AI.ToolProviders.NyxId/NyxIdMcpOperationCatalogReader.cs`, `src/Aevatar.AI.ToolProviders.NyxId/NyxIdApiClient.cs`, `src/Aevatar.Studio.Hosting/Endpoints/ContentArtifactEndpoints.cs`, and `src/Aevatar.Studio.Hosting/Endpoints/NyxIdLoginFinalizationEndpoints.cs`. Evidence is the fixture's `watched_paths` array.
- [ ] Add `scripts/check-aevatar-chat-drift.py` and `scripts/tests/test_check_aevatar_chat_drift.py`. Evidence is the scoped diff with the checker and its deterministic tests.
- [ ] Add `.github/workflows/aevatar-chat-drift.yml` with weekly and manual triggers. Evidence is the workflow diff and one manual run URL.
- [ ] Update `docs/chat/README.md` to point to the machine-readable pin and its check command. Evidence is the rendered document at the phase head.

**Build.**

- [ ] Make the pin record Aevatar remote head `e5bba2e9719ad5132004b882744caa3875db1123` and effective chat SHA `706ea7cab9d1f882e0fb0f034bb338102b6d5d2b`. Evidence is the exact JSON values in the fixture.
- [ ] Make the checker fetch `feature/integrate`, compare watched chat paths from the effective SHA to remote HEAD, and fail with changed path names when contract-owned files move. Evidence is a passing clean case and a failing synthetic-drift case.
- [ ] Keep NyxID's additive action manifest. Treat `schema_version` as the registry-wide compatibility gate and `revision` as an observability label. Evidence is a fixture assertion that unknown or divergent descriptors do not require an exact revision-wide match.

**You see.**

- [ ] Run the checker against the audited Aevatar clone and observe zero watched-path changes between the effective chat SHA `706ea7cab` and remote HEAD `e5bba2e9`. Evidence is a JSON or text receipt naming both SHAs and an empty changed-path list.

**Verify, repository.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Run `python3 -m unittest scripts.tests.test_check_aevatar_chat_drift` and the documentation and workflow checks used by `CI Pipeline` at the exact head. Evidence is a log headed by `git rev-parse HEAD` and the required check URL.

**Verify, live.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Run `python3 scripts/check-aevatar-chat-drift.py --remote https://github.com/aevatarAI/aevatar.git --branch feature/integrate --pin tests/fixtures/assistant/aevatar-chat-contract-pin.json`. Pass when it resolves the live remote, records the observed head, and reports no unreviewed watched-path drift. Evidence is the command receipt with the fetch timestamp and SHAs.

**Verify, risk.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Point the checker at a temporary repository with one watched-path commit after the effective pin. Pass when the command exits nonzero and prints only the changed watched paths without modifying the NyxID worktree. Evidence is the negative-test transcript and clean `git status --short` output.

**Review gate.** None. This phase changes contract metadata, checks, and documentation only.

**Deliver.**

- [ ] Hand off the merge-ready root PR with `main` as its base, `auto_merge` disabled, one independent verdict, and a green `CI Pipeline`. Evidence is the PR URL and exact head SHA.

## Remove obsolete commands and stale registry assumptions (AC-1)

**Depends on.** `AC-0` is green and its pin is the accepted contract source.

**Owner.** A delegated assistant-boundary owner on a branch based on the exact `AC-0` head.

**Verifier.** An independent read-only API and frontend verifier at the exact `AC-1` head.

**Files.**

- [ ] Edit `backend/src/services/assistant_service.rs` at `AssistantChatCommand`, `PlanResolveCommand`, `RawPlanResolveCommand`, `parse_assistant_chat_command`, and `prepare_assistant_chat_command`. Evidence is a diff with every `plan.resolve` parser and reconstruction path removed.
- [ ] Edit `backend/src/handlers/assistant.rs` at `completions`, `backend/src/routes.rs` at `/assistant/completions`, and their tests. Evidence is a diff that removes the route unless production telemetry proves a current caller and records its closed typed contract.
- [ ] Edit `frontend/src/lib/assistant/chat-api.ts`, `frontend/src/hooks/use-assistant-chat-controls.ts`, `frontend/src/hooks/use-assistant-chat.ts`, `frontend/src/components/assistant/chat-actor-controls.tsx`, `frontend/src/components/assistant/assistant-chat-page.tsx`, `frontend/src/components/assistant/blocks/task-plan-card.tsx`, and their tests. Evidence is a diff with no callable plan-confirmation control.
- [ ] Edit `frontend/src/lib/assistant/chat-task-plan.ts` so historical unknown gate fields are ignored without preserving a callable `ChatPlanGate` state. Evidence is a decoder test for an old snapshot and no outbound command.
- [ ] Edit `backend/src/handlers/assistant_actions.rs`, `tests/fixtures/assistant/aevatar-pinned-actions-by-revision.json`, `docs/chat/02-wire-contract.md`, `docs/chat/06-actions-registry.md`, and `docs/chat/assistant-waves-plan.md`. Evidence is a diff that describes per-action degrade, retries, disabled fallback recovery, and revision observability accurately.

**Build.**

- [ ] Define the canonical public command set as `text`, `input.resolve`, `action.continue`, `approval.resolve`, `task.stop`, `task.steer`, `step.retry`, and `step.skip`. Evidence is a backend contract test and a frontend type test generated from the `AC-0` pin.
- [ ] Reject `plan.resolve` at NyxID's typed boundary and delete its callers and tests in the same phase. Evidence is a 400 contract test and a whole-tree `rg` result with only historical audit or plan references.
- [ ] Remove `/api/v1/assistant/completions` after checking production access evidence. If a live caller exists, stop this phase and replace the route only after its exact request contract is added to the pin. Evidence is the telemetry decision record and either route deletion or a strict typed reconstruction test.
- [ ] Preserve all additive default action descriptors while deleting revision-map assertions that claim a manually synchronized fixture detects upstream drift. Evidence is a manifest golden test plus per-action known, unknown, and divergent descriptor tests.

**You see.**

- [ ] Open a stored task snapshot that contains the retired confirm gate and observe a readable task plan with no Confirm or Reject plan commands. Evidence is a desktop screenshot and the browser network log with no `plan.resolve` request.

**Verify, repository.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Run the Backend gate set and Frontend gate set in Appendix D at the exact head. Evidence is the local logs, the green `CI Pipeline` URL, and matching `git rev-parse HEAD` output.
- [ ] Run `rg -n 'plan\.resolve|resolvePlan|onResolvePlan' backend/src frontend/src tests/fixtures docs/chat`. Evidence is output limited to the convergence plan or explicit historical notes.

**Verify, live.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Send a real `plan.resolve` body through `/api/v1/assistant/chat` and request `/api/v1/assistant/completions` on a staging NyxID instance. Pass when both fail at NyxID with the documented 400 or 404 response and Aevatar receives neither body. Evidence is the HTTP transcript and metadata-only upstream wire log.

**Verify, risk.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Review production route access evidence for `/api/v1/assistant/completions` across the agreed retention window. Pass when zero callers justify deletion, or when a named caller and approved replacement contract stop the phase before deletion. Evidence is the redacted query or dashboard link and the recorded decision.
- [ ] Load old task-plan fixtures with confirm gates. Pass when they remain readable, expose no dead controls, and do not crash the actor reducer. Evidence is the focused Vitest log and screenshot.

**Review gate.** Operator and API-owner review are required because this phase removes visible controls and a published route.

- [ ] Have the operator inspect before-and-after task-plan screenshots and the no-request browser recording. Evidence is the approval note linked to the exact head.
- [ ] Have the API owner inspect the completions usage evidence and route decision. Evidence is the approval note or a recorded blocker linked to the exact head.

**Deliver.**

- [ ] Hand off the merge-ready `AC-1` PR based on the exact `AC-0` head with `auto_merge` disabled. Evidence is the PR URL, base SHA, head SHA, independent verdict, and green required checks.

## Prove access-review reachability and choose its authority model (AC-2)

**Depends on.** `AC-1` is green and the obsolete command paths are gone.

**Owner.** A delegated security and integration owner on a branch based on the exact `AC-1` head.

**Verifier.** An independent read-only security verifier who did not author the probe or decision.

**Files.**

- [ ] Add `scripts/probe-aevatar-chat-authority.mjs` with redacted identity, bearer-capability, delegation-header, consent, and replay probes. Evidence is the script diff and its documented environment contract.
- [ ] Add `docs/chat/aevatar-access-review-authority.md` as the decision record for the deployed authorization-session design. Evidence is the accepted decision record with source and runtime receipts.
- [ ] Update `docs/chat/01-architecture.md` and `docs/API.md` to remove the stale claim that Aevatar authenticates only the Bearer header. Update `docs/ENV.md` only if the accepted design adds configuration. Evidence is the corrected transport-auth sequence and configuration contract tied to the pinned upstream files.

**Build.**

- [ ] Reproduce the current cookie-session bridge with one connected service omitted from the Aevatar session authority. Evidence is a redacted claim summary that records `allow_all_services`, `allowed_service_ids`, and `resources` without a token.
- [ ] Compare three authority designs in the decision record. The candidates are consent-derived delegated restrictions, a complete OAuth browser round trip with a NyxID return channel, and preserved unrestricted forwarding with an upstream reachability explanation. The consent-derived candidate requires three NyxID changes and the record must cost each of them, namely a delegated read allowance for `GET /api/v1/mcp/config` anchored by a live `CatalogDelegationGrant`, a platform-row exemption on the callback paths, and the Aevatar client link. Evidence is a table with threat model, revocation, retry, deployment, and rollback results.
- [ ] Prove whether the bridge bearer can read `GET /api/v1/mcp/config`, the endpoint Aevatar uses for both the access probe and the access-review postcondition. Evidence is the route-policy citation at `backend/src/mw/auth.rs` `delegated_read_denied_path` and `delegated_request_allowed`, the middleware test that pins it, and a local-runtime HTTP receipt with a delegated token.
- [ ] Prove whether a service-restricted delegated token can still complete the Aevatar LLM callback through `/api/v1/llm/gateway/v1` and `/api/v1/proxy/s/chrono-llm-public`. Evidence is a local-runtime receipt for each path with `allow_all_services=false`.
- [ ] Resolve the Aevatar OAuth client identity without guessing. Production shows a registered client named `aevatar` with id `a6ff2946-f02f-4c35-8203-1ec46132b660`, which Aevatar pins as `BackendConsole.OidcClientId`. Decide between an admin-managed link on the `aevatar` catalog row and a configuration value. Evidence is the decision record entry and the field or setting it names.
- [ ] Prefer consent-derived restrictions only if the probe proves chat bootstrap still works, the exact Aevatar OAuth client identity can be resolved without guessing, and an approved service changes the next minted capability. Evidence is the decision predicate and live result for each condition.
- [ ] Keep `forward_access_token` enabled until the selected design proves that Aevatar receives both caller identity and a capability source for tool execution. Evidence is the deployment precondition in the decision record.

**You see.**

- [ ] Produce a seven-row probe receipt for identity only, identity plus capability Bearer, identity plus delegation header, replayed identity `jti`, bridge bearer reading `/api/v1/mcp/config`, restricted token on the LLM gateway path, and restricted token on the proxy slug path. Evidence is a redacted table with HTTP status, upstream code, surface used, and exact deployment version or local SHA.
- [ ] Force or attempt `USER_SERVICE_ACCESS_REQUIRED` for a connected but unauthorized service. Evidence is the emitted `service.access_review` action or a falsified reachability result that names the authority preventing it.

**Verify, repository.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Run the script's offline parser and redaction tests plus the documentation checks at the exact head. Evidence is the local log, clean `git status --short`, and green `CI Pipeline` URL.

**Verify, live.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Run the probe against the same deployed NyxID and Aevatar pair intended for `AC-3`. Pass when identity-only behavior, capability delivery, replay rejection, and access-review reachability are all observed and recorded, or when the phase stops with a falsified candidate and no implementation child starts. Evidence is the redacted probe receipt with both deployment SHAs or image digests.

**Verify, risk.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Decode every probe token only inside the probe process and emit allowlisted claim summaries. Pass when logs contain no JWT, cookie, authorization code, refresh token, API key, or credential value. Evidence is a secret-pattern scan and independent log review.
- [ ] Exercise expiry, `jti` replay, service removal, consent revocation, and cross-user resource substitution. Pass when each case fails closed without changing another user's consent or service authority. Evidence is the negative-case matrix and audit event ids.

**Review gate.** Explicit security review is required because this decision changes which service authority Aevatar receives.

- [ ] Have the security reviewer approve the chosen authority model, revocation behavior, token delivery headers, and rollback order. Evidence is a signed review verdict linked to the exact head and probe receipt.

**Deliver.**

- [ ] Hand off the merge-ready `AC-2` decision and probe PR based on the exact `AC-1` head. Evidence is the PR URL, base SHA, head SHA, independent verdict, and green required checks. No `AC-3` branch starts without the accepted decision.

## Implement service access review end to end (AC-3)

**Depends on.** `AC-2` is green, the live action is reachable, and security approved the authority model.

**Owner.** A delegated full-stack access-review owner on a branch based on the exact `AC-2` head.

**Verifier.** An independent read-only security and browser-flow verifier at the exact `AC-3` head.

**Files.**

- [ ] Edit `frontend/src/schemas/assistant-actions.ts` at `AssistantActionRequest`, `ActionCardParams`, and `ActionResource` to add the exact `service.access_review` variant. Evidence is a strict Zod contract test for `userServiceId`, `serviceSlug`, and `resourceUri`.
- [ ] Edit `frontend/src/lib/assistant/action-registry.ts` and `frontend/src/lib/assistant/chat-action-validation.ts` to recognize, normalize, and recover the action. Evidence is the registry and recovery test diff.
- [ ] Add `frontend/src/components/assistant/assistant-service-access-review-dialog.tsx` and wire it through `frontend/src/components/assistant/blocks/action-card.tsx` and `action-dialogs.tsx`. Evidence is the component and wiring tests at the phase head.
- [ ] Edit `backend/src/handlers/assistant_action_effects_services.rs`, `backend/src/routes.rs`, and the service selected by the `AC-2` decision. Expected service candidates are `backend/src/services/consent_service.rs` and the assistant forward-authority code in `backend/src/handlers/assistant.rs`. Evidence is the accepted decision mapped to exact changed symbols.
- [ ] Edit `frontend/src/hooks/use-assistant-chat-controls.ts` only as needed to report a completed resource shaped as `userService.userServiceId`. Evidence is the exact `action.continue` body test.
- [ ] Update `docs/chat/04-action-cards.md`, `docs/chat/07-testing-and-gaps.md`, and `docs/API.md`. Evidence is documentation of the endpoint, journey, postcondition, and live proof.

**Build.**

- [ ] Parse only schema version 4 action envelopes whose action is `service.access_review` and whose params contain exactly one `serviceAccessReview` object. Evidence is strict valid, missing, extra-field, secret-shaped, and malformed-resource tests.
- [ ] Resolve the target `UserService` under the authenticated owner and recompute its canonical RFC 8707 resource URI server-side. Evidence is a handler-to-service test that rejects caller substitution and slug or URI mismatch.
- [ ] Apply the authority mutation chosen in `AC-2` only after an explicit human confirmation. Implement it in `backend/src/services/consent_service.rs` as a merge that preserves existing `scopes` and `allowed_service_ids` in one atomic update; `grant_consent_with_services` replaces the row and must not be called from this path. The effects handler stays HTTP and DTO. Evidence is before, first apply, retry, revoke, and cross-user database tests.
- [ ] Report only `actionRequestId`, `originTurnId`, `disposition`, and `resource.userService.userServiceId`. Evidence is a serialized continuation body with no secrets or tokens.
- [ ] Continue the same `nyxid-chat-*` actor and wait for Aevatar's postcondition result before presenting completion. Evidence is a reducer test and live wire sequence with stable actor id.

**You see.**

- [ ] Observe an access-review card that says the service is already connected, asks for exact service access, and never falls back to the unsupported-action card. Evidence is desktop and mobile screenshots.
- [ ] Approve and decline separate requests. Evidence is a recording that shows the same conversation resume with completed and declined dispositions.

**Verify, repository.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Run the Backend gate set and Frontend gate set in Appendix D at the exact head. Evidence is the local logs, the green `CI Pipeline` URL, and matching head SHA.

**Verify, live.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Use a staging human cookie session to trigger `USER_SERVICE_ACCESS_REQUIRED`, approve the card, observe a successful Aevatar postcondition, and use the same service on the next turn. Pass when the actor id stays stable and the second service call succeeds without another review. Evidence is the redacted browser recording, network trace, Aevatar event ids, and both exact deployment SHAs.

**Verify, risk.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Try another user's `userServiceId`, a valid id with the wrong slug, an HTTP resource URI, userinfo, query, fragment, encoded path confusion, and an inactive service. Pass when every case fails before authority mutation and creates a metadata-only audit event. Evidence is the adversarial test matrix and database before-and-after snapshot.
- [ ] Retry the same completion across a lost browser response. Pass when one authority change exists, the report remains replay-safe, and Aevatar reaches one terminal postcondition. Evidence is the idempotency transcript and stored consent or grant row.

**Review gate.** Operator and security review are required for the consent interaction and authority change.

- [ ] Have the operator review the desktop and mobile screenshots plus the approve, decline, cancel, error, and return states. Evidence is the operator verdict linked to the exact head.
- [ ] Have the security reviewer verify the mutation, report allowlist, audit metadata, and cross-user denial. Evidence is the security verdict linked to the exact head.

**Deliver.**

- [ ] Hand off the merge-ready `AC-3` PR based on the exact `AC-2` head with no merge or auto-merge action. Evidence is the PR URL, base SHA, head SHA, independent verdicts, and green required checks.

## Add typed context-attachment transport and readback (AC-4)

**Depends on.** `AC-1` is green and the `AC-0` pin still matches the current upstream attachment contract. `AC-4` is built on the exact `AC-1` head in parallel with `AC-2` and `AC-3` because its files are disjoint; the root rebases it onto the `AC-3` head before delivery and reverifies the gates.

**Owner.** A delegated typed-contract owner on a branch based on the exact `AC-1` head.

**Verifier.** An independent read-only boundary and compatibility verifier at the exact `AC-4` head.

**Files.**

- [ ] Edit `backend/src/services/assistant_service.rs` at `TextChatCommand`, `RawTextChatCommand`, `parse_assistant_chat_command`, and `prepare_assistant_chat_command` to add typed context attachments. Evidence is the parser and exact reconstruction diff.
- [ ] Edit `frontend/src/lib/assistant/chat-api.ts` to model `ContextAttachmentReference` and allow it only on first-turn `text`. Evidence is a compile-time and runtime command-contract test.
- [ ] Edit `frontend/src/lib/assistant/chat-types.ts` at `ConversationMeta` and `ChatConversationDetail`. Evidence is typed durable attachment references in list and detail state.
- [ ] Edit `frontend/src/lib/assistant/chat-history-decoders.ts` at `decodeConversationMeta` and `decodeChatConversationDetail`, plus `frontend/src/lib/assistant/chat-actor-state.ts` at `applyCurrentStateResult`. Evidence is readback tests for list and current-state responses.
- [ ] Edit `frontend/src/hooks/use-assistant-chat.ts` at the first-turn send boundary. Evidence is a test that attaches references only when `conversationId` is absent.
- [ ] Update `frontend/src/lib/assistant/__fixtures__/aevatar-chat-history.json`, `frontend/src/lib/assistant/__fixtures__/aevatar-nyxid-chat-stream.sse`, and focused tests. Evidence is fixture coverage of create, reload, and admission errors.

**Build.**

- [ ] Model each attachment as `artifactId`, `revisionMode`, and `pinnedRevisionId`. Evidence is one discriminated type that makes `pinned_revision` require a nonempty revision id and makes `follow_current` omit it on the wire.
- [ ] Enforce a maximum of four unique, nonempty artifact ids at NyxID's boundary. Evidence is focused tests for zero, one, four, five, duplicate, empty, pinned, and follow-current cases.
- [ ] Reject attachments on follow-up text and preserve their create-only, sealed-to-conversation rule. Evidence is a boundary test that never forwards a replacement or removal attempt.
- [ ] Preserve only references in `ConversationMeta`, transcript detail, and actor current state. Evidence is decoder output with no artifact body or backing-object data.
- [ ] Preserve Aevatar's `ATTACHMENT_ADMISSION_DENIED` code and the reasons `not_found`, `access_denied`, `unsupported_kind`, `over_limit`, `pinned_revision_unavailable`, `invalid_request`, `inactive`, and `read_model_unavailable`. Evidence is the error-decoder matrix.

**You see.**

- [ ] Send a first-turn request with one `follow_current` and one `pinned_revision` reference, then reload list and current state. Evidence is a wire transcript that shows the exact normalized request and the same durable references on readback.

**Verify, repository.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Run the Backend gate set and Frontend gate set in Appendix D at the exact head. Evidence is the local logs, green `CI Pipeline` URL, and matching head SHA.

**Verify, live.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Call the real first-turn endpoint with valid artifact references, follow it with a second turn, and read the list and state endpoints. Pass when Aevatar admits the first turn, rejects any replacement attempt, and returns the sealed references on every documented read model. Evidence is the redacted HTTP and SSE transcript with NyxID and Aevatar SHAs.

**Verify, risk.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Send duplicate ids, five ids, missing pinned revision ids, unsupported modes, extra fields, and follow-up attachments. Pass when NyxID or Aevatar returns the pinned typed error and no conversation is created or mutated. Evidence is the negative-case matrix and list or state proof.
- [ ] Inspect all logs and wire-capture output. Pass when only ids, modes, reason codes, byte counts, and statuses appear, with no artifact content. Evidence is a secret and content-pattern scan signed by the verifier.

**Review gate.** None. This phase adds transport and readback only; it does not expose attachment controls yet.

**Deliver.**

- [ ] Hand off the merge-ready `AC-4` PR based on the exact `AC-3` head after the root-owned rebase, with no merge or auto-merge action. Evidence is the PR URL, base SHA, head SHA, independent verdict, and green required checks.

## Add server-scoped artifact discovery and new-chat selection (AC-5)

**Depends on.** `AC-4` is green and its attachment types are the only browser-to-backend representation.

**Owner.** A delegated full-stack attachment-UX owner on a branch based on the exact `AC-4` head.

**Verifier.** An independent read-only security, accessibility, and browser verifier at the exact `AC-5` head.

**Files.**

- [ ] Edit `backend/src/services/assistant_service.rs` to add a server-owned content-artifact path builder derived from `AuthUser.user_id`. Evidence is a path test that has no caller-supplied `scopeId` input.
- [ ] Edit `backend/src/handlers/assistant.rs` to add a bounded artifact-list handler with dedicated safe response structs, and edit `backend/src/routes.rs` to mount `GET /api/v1/assistant/context-artifacts`. Evidence is the handler, DTO, and route test diff.
- [ ] Add `frontend/src/schemas/assistant-context-attachments.ts` and `frontend/src/hooks/use-assistant-context-artifacts.ts`. Evidence is strict response parsing and TanStack Query tests.
- [ ] Add `frontend/src/components/assistant/context-attachment-picker.tsx` and wire it through `frontend/src/components/assistant/chat-composer.tsx`, `assistant-chat-page.tsx`, and `use-assistant-chat.ts`. Evidence is the component, composer, and send tests.
- [ ] Update `docs/chat/05-frontend-ui.md`, `docs/chat/07-testing-and-gaps.md`, and `docs/API.md`. Evidence is the new endpoint, new-chat-only interaction, and safe discovery contract in the rendered docs.

**Build.**

- [ ] Derive `/api/scopes/{AuthUser.user_id}/content-artifacts` on the server and never accept a browser scope segment. Evidence is a route test that substitutes another user id and still forwards only the authenticated id.
- [ ] Return only artifact id, title, kind, lifecycle status, current revision id, and safe revision metadata needed for pin selection. Never return inline content, backing object keys, content hashes unless required for display, access lists, owner internals, or provenance. Evidence is an exact response serialization test.
- [ ] Filter the picker to active `text`, `markdown`, and `structured_document` artifacts and cap pages and aggregate bytes. Evidence is pagination, cap, unsupported-kind, and malformed-upstream tests.
- [ ] Show attachment selection only for a new draft. Limit selection to four unique artifacts and provide `follow_current` or `pinned_revision` controls. Evidence is component state-machine tests.
- [ ] Freeze the selected references when the first turn starts and hide editing after actor adoption. Evidence is a send-and-adopt test with stable references and no follow-up control.

**You see.**

- [ ] Open a new chat and select, change, and remove allowed artifacts without shifting the composer or covering adjacent controls. Evidence is desktop and mobile screenshots for empty, loading, selected, maxed, error, and pinned-revision states.
- [ ] Send the first turn and observe the attachment summary become read-only after actor adoption. Evidence is a browser recording and network body.

**Verify, repository.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Run the Backend gate set and Frontend gate set in Appendix D at the exact head. Evidence is the local logs, green `CI Pipeline` URL, wizard-bundle freshness result when its source closure changes, and matching head SHA.

**Verify, live.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Use a staging human cookie session to list real artifacts, select both revision modes, start a real Aevatar chat, and reload it. Pass when only owned or readable artifacts appear, the first turn succeeds, and the sealed reference summary survives reload. Evidence is the browser recording, screenshots, redacted network trace, and deployment SHAs.

**Verify, risk.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Attempt caller-supplied scope injection, another user's page token and artifact id, unsupported kind selection, stale current revision, and oversized upstream pages. Pass when every case fails closed or returns an empty safe result without content disclosure. Evidence is the adversarial matrix and metadata-only audit ids.
- [ ] Run keyboard-only and screen-reader checks at mobile and desktop widths. Pass when focus order, labels, mode controls, removal, errors, and the four-item limit are understandable and no text overlaps. Evidence is an accessibility report, screenshots, and recording.

**Review gate.** Operator review is required because this phase changes the composer and new-chat workflow.

- [ ] Have the operator inspect the named desktop and mobile states and the full selection-to-send recording. Evidence is the operator verdict linked to the exact head.
- [ ] Have the security reviewer inspect the scoped handler and safe DTO. Evidence is the security verdict linked to the exact head.

**Deliver.**

- [ ] Hand off the merge-ready `AC-5` PR based on the exact `AC-4` head with no merge or auto-merge action. Evidence is the PR URL, base SHA, head SHA, independent verdicts, and green required checks.

## Fix lifecycle liveness and recoverable state (AC-6)

**Depends on.** `AC-5` is green and the readback types include context attachments.

**Owner.** A delegated frontend lifecycle owner on a branch based on the exact `AC-5` head.

**Verifier.** An independent read-only concurrency and browser-state verifier at the exact `AC-6` head.

**Files.**

- [ ] Edit `frontend/src/lib/assistant/chat-history-api.ts` at `deleteConversation` and its tests to observe typed delete completion. Evidence is a bounded poll state machine for accepted, not-found, timeout, and abort cases.
- [ ] Edit `frontend/src/hooks/use-assistant-chat.ts` and `frontend/src/components/assistant/assistant-sidebar.tsx` to retain an accepted deletion as deleting until state returns `not_found`. Evidence is hook and sidebar tests for no row resurrection.
- [ ] Edit `frontend/src/lib/assistant/chat-stream-orchestrator.ts` and its tests so valid Aevatar keepalive frames reset the progress watchdog without becoming visible content. Evidence is a virtual-time test beyond 120 seconds.
- [ ] Edit `frontend/src/lib/assistant/chat-session-state.ts`, `chat-history-decoders.ts`, `chat-actor-state.ts`, `runtime-event-semantics.ts`, and their tests for terminal refresh and recoverable action or authorization state. Evidence is reload and settlement test coverage.
- [ ] Edit `docs/chat/03-stream-protocol.md`, `docs/chat/05-frontend-ui.md`, and `docs/chat/07-testing-and-gaps.md` to record the exact recovery boundary. Evidence is documentation that distinguishes durable state, live-only events, and upstream-owned omissions.

**Build.**

- [ ] Treat typed DELETE 202 as accepted work, follow the returned or canonical state URL with bounded jittered backoff, and finish only at `not_found`. Evidence is an explicit deleting state and timeout error in the domain type.
- [ ] Treat `aevatar.nyxid_chat.keepalive` as stream progress for liveness only. Evidence is a watchdog test that neither times out nor appends a message for keepalive-only intervals.
- [ ] Refresh current state and conversation metadata after terminal stream settlement and action postcondition settlement. Evidence is a test that terminal task, attention, attachment, and action state update without a page reload.
- [ ] Restore pending `service.connect` and `service.access_review` cards from actor current state after reload; the access-review half depends on `AC-3`. Evidence is a reload test with an actionable card and the original action identity.
- [ ] Probe whether public transcript `operations` or current state can reconstruct `MEDIA_CONTENT` and generic `nyxid.authorization.required`. If neither can, record the upstream ownership, file or link a pinned upstream issue, and do not add NyxID shadow persistence. Evidence is the response capture and documented ownership decision.

**You see.**

- [ ] Delete a real conversation and observe a stable deleting state followed by permanent removal. Evidence is a recording with the 202 response, state polling, terminal 404 or `not_found`, and refreshed list.
- [ ] Reload while an access-review card is pending and after a terminal action result. Evidence is a recording that shows the actionable card and final status restored without duplicate controls.

**Verify, repository.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Run the Frontend gate set in Appendix D and focused fake-timer lifecycle tests at the exact head. Evidence is the local logs, green `CI Pipeline` URL, and matching head SHA.

**Verify, live.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Run a real Aevatar turn that emits keepalives for more than 120 seconds, delete a real typed conversation, reload a pending access-review card, and settle one action. Pass when no client stop is sent during keepalives, deletion ends only at absence, and available durable state restores exactly once. Evidence is the browser recording, redacted network trace, event ids, and deployment SHAs.

**Verify, risk.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Race delete against list refresh, route selection, component unmount, network loss, and an active stream. Pass when active-stream deletion remains refused, accepted deletion cannot resurrect a row, abort stops polling, and timeout leaves a retryable state. Evidence is the deterministic race-test log and browser recording.
- [ ] Feed malformed and high-frequency keepalives. Pass when only a valid normalized keepalive resets liveness, no attacker-controlled event suppresses timeout, and timer count stays bounded. Evidence is the fake-timer and malformed-frame test log.

**Review gate.** Operator review is required because this phase adds a deleting state and changes reload behavior.

- [ ] Have the operator inspect delete success, timeout, retry, pending-card reload, and long-turn recordings at desktop and mobile sizes. Evidence is the operator verdict linked to the exact head.

**Deliver.**

- [ ] Hand off the merge-ready `AC-6` PR based on the exact `AC-5` head with no merge or auto-merge action. Evidence is the PR URL, base SHA, head SHA, independent verdict, and green required checks.

## Add credentialed producer proof and close the contract (AC-7)

**Depends on.** `AC-6` is green and all product paths required by the done predicate exist.

**Owner.** A delegated release-verification owner on a branch based on the exact `AC-6` head.

**Verifier.** An independent read-only release verifier who checks the exact stack tip and live receipts.

**Files.**

- [ ] Extend `frontend/scripts/verify-aevatar-action-wake.mjs` into a redacted producer-contract runner or split it into focused modules under `frontend/scripts/aevatar-chat-canary/`. Evidence is a script diff that covers the final matrix and preserves the existing empty-action wake.
- [ ] Add `frontend/e2e/live-aevatar.spec.ts` and a non-mock Playwright project in `frontend/playwright.config.ts`. Evidence is a test that never appends `mock=1` and requires an explicit credentialed staging configuration.
- [ ] Add `.github/workflows/aevatar-chat-canary.yml` with secret-backed scheduled and manual runs against a controlled staging pair. Evidence is the workflow diff and one successful run URL.
- [ ] Update `docs/chat/README.md`, `docs/chat/07-testing-and-gaps.md`, and the `AC-0` contract pin only after re-auditing the remote. Evidence is final documentation and pin values tied to the canary run.

**Build.**

- [ ] Cover first and follow-up text, input, approval, stop, steer, retry, skip, empty `actions`, `service.connect`, `service.access_review`, `key.create`, `key.rotate`, context attachments, state reload, and delete observation. Evidence is a machine-readable canary result with one row per capability.
- [ ] Record NyxID head SHA, Aevatar source or image SHA, effective chat pin, timestamps, actor ids, turn ids, action ids, status codes, and safe reason codes. Evidence is the allowlisted JSONL schema and one validated receipt.
- [ ] Fail instead of skip when the credentialed workflow is armed but a secret, seed, producer condition, or deployment version is missing. Evidence is negative workflow tests for every prerequisite.
- [ ] Keep the mock Playwright suite as deterministic regression coverage and label it as mock evidence only. Evidence is the existing suite result next to the separate live result.
- [ ] Re-run the capability matrix in Appendix E and mark a row complete only when its repository and live evidence exists at the exact stack tip. Evidence is the final matrix with links and no unsupported completeness claim.

**You see.**

- [ ] Open the canary run summary and see one green row per done-predicate capability plus exact source pins. Evidence is the workflow artifact and run URL.
- [ ] Open the non-mock Playwright recording and see the real staging host, real network calls, access review, attachment selection, reload, and delete completion. Evidence is the trace, screenshots, and recording.

**Verify, repository.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Run the Frontend gate set, the `AC-0` drift checker, the canary parser tests, and the plan checker in Appendix D at the exact head. Evidence is the local logs, green `CI Pipeline` URL, and matching stack-tip SHA.

**Verify, live.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Run the credentialed producer workflow against the controlled staging NyxID and Aevatar pair. Pass when every required row is green, no row is skipped, the browser uses no mock transport, and the receipt names both exact deployments. Evidence is the workflow URL, JSONL receipt, Playwright trace, screenshots, and recording.

**Verify, risk.** Tests alone are not sufficient verification. A phase is verified only when its repository and live checks have evidence and every warranted risk-specific check has evidence.

- [ ] Scan workflow logs and artifacts for JWTs, cookies, authorization codes, refresh tokens, API keys, credential payloads, and artifact bodies. Pass when the scan is empty and artifacts expose only allowlisted metadata. Evidence is the scanner log and independent verifier signature.
- [ ] Change one pinned command, action, or watched upstream path in a temporary fixture. Pass when drift or canary validation turns red and identifies the exact changed capability. Evidence is the deliberate-failure run and restored clean run.

**Review gate.** None. This phase adds test and documentation infrastructure and does not change the product interaction.

**Deliver.**

- [ ] Hand off the merge-ready `AC-7` stack tip with the root-to-tip PR order and no merge or auto-merge action. Evidence is every PR URL, exact base and head SHA, independent verdict, green `CI Pipeline`, live canary URL, and final capability matrix.

## Close the program

- [ ] Check every box above only after its named evidence exists. Evidence is the completed plan and its linked artifacts.
- [ ] Return the Autopilot Stack final report to the operator. Evidence is the root-to-tip PR order, exact bases and heads, one verdict per link, live canary receipt, and every parked item with its reason.

## Appendix A. Prototype evidence

The audit settled what source inspection could settle. It did not have credentials for a live NyxID-to-Aevatar producer run. `AC-2` therefore owns the remaining deployed-auth experiment.

| Question | Artifact | Result | Remaining proof |
|---|---|---|---|
| Does NyxID derive Aevatar scope from the authenticated user | `backend/src/handlers/assistant.rs` at `forward` and `backend/src/services/assistant_service.rs` path builders | Yes. The browser cannot submit `scopeId` for chat, history, state, or delete | Repeat for the new artifact-list route in `AC-5` |
| Can a normal cookie session naturally trigger service access review | `backend/src/mw/auth.rs` session `AuthUser`, `build_forward_authorization`, and `TokenRestrictionClaims::from_auth_user` | Probably not. Session auth currently has `allow_all_services` true, an empty id list, and no resource list, which the bridge copies into its delegated token | Run the controlled deployed probe in `AC-2` |
| Does identity assertion remove the need for a capability | Aevatar `NyxIdIdentityAssertionAuthentication.cs` and `NyxIdChatEndpoints.Streaming.cs` | No. The identity header becomes authoritative for caller identity, while streaming still extracts a Bearer or delegation token for NyxID capability | Verify the four-row header matrix in `AC-2` |
| Are context attachments part of the public typed browser contract | Aevatar `NyxIdChatPublicEndpoints.cs`, `NyxIdChatEndpoints.Streaming.cs`, and `ConversationContextAttachmentAdmission.cs` | Yes. They are create-only, maximum four, and durable by reference | Exercise valid and rejected requests against the deployed pair in `AC-4` |
| Does Aevatar require exact action-registry revisions | Aevatar `NyxIdAssistantActionRegistry.cs` after `0b8ec500087331c3d12819b532e7dfa29e740fb4` | No. `schema_version` gates the registry. Revision labels are observational. Descriptor failures degrade per action | Automate watched-path drift detection in `AC-0` |
| Is current browser proof live | `frontend/e2e/helpers.ts` and `frontend/scripts/verify-aevatar-action-wake.mjs` | No. Playwright always adds `mock=1`; the producer script only proves an empty action wake when manually credentialed | Add the secret-backed canary and non-mock browser run in `AC-7` |
| Can the bridge bearer read the endpoint Aevatar probes for access | `backend/src/mw/auth.rs` at `delegated_read_denied_path` and `delegated_request_allowed`; Aevatar `NyxIdRequireServiceTool.cs` at `InspectCurrentBearerServiceAccessAsync` and `NyxIdApiClient.cs` at `GetMcpConfigAsync` | No. `/api/v1/mcp` is a denied class for every delegated GET, so the probe returns access denied and Aevatar reports `SourceStale`, never `USER_SERVICE_ACCESS_REQUIRED` | Local-runtime receipt in `AC-2` |
| Does a restricted token break chat bootstrap | `backend/src/handlers/proxy.rs` at the legacy `DownstreamService` scoped denial and `execute_admin_proxy`; Aevatar `appsettings.json` `GatewayEndpoint` | Probably on the proxy slug path; the LLM gateway path checks only scope | Receipts for both callback paths in `AC-2` |
| Is the Aevatar OAuth client resolvable | Production `/users/me/consents` shows client `aevatar` id `a6ff2946-f02f-4c35-8203-1ec46132b660`; Aevatar `appsettings.json` `BackendConsole.OidcClientId` | Yes as a registered client; NyxID has no link from the catalog row to it | Decide the link in `AC-2` |

Focused baseline evidence at NyxID `52402f6a5510478b3601636a25572055f895c973` is 75 passing frontend tests across eight files and 35 passing Rust `assistant_service` tests. The Rust run filtered out 5,674 tests. These are regression baselines, not integration proof.

`npm ci` installed 608 packages and reported 21 audit findings. The report contained 1 low, 11 moderate, 8 high, and 1 critical finding. No audit fix ran. Dependency remediation is outside this program unless a finding affects a changed assistant path.

## Appendix B. Alternatives rejected

| Alternative | Decision | Reason |
|---|---|---|
| Declare parity from focused unit and mock Playwright tests | Rejected | They do not exercise a credentialed Aevatar producer or deployed auth headers |
| Copy Aevatar Console's OAuth card before probing reachability | Rejected | NyxID's cookie session and proxy-minted capability differ from Aevatar Console's resource-scoped OAuth browser session |
| Keep the unrestricted cookie bridge and add only a visual access-review card | Rejected | The action may remain unreachable and approval may not change the next capability token |
| Shrink the default NyxID action manifest to current Aevatar executables | Rejected | Current Aevatar tolerates unknown and divergent descriptors per action; additive descriptors do not widen its executable set |
| Keep a manual per-revision fixture as the upstream drift guard | Rejected | It stays green when upstream changes until a human edits it |
| Keep `plan.resolve` as a compatibility adapter | Rejected | Current Aevatar classifies it as unsupported, and no current actor source emits a confirmation gate |
| Leave `/assistant/completions` as arbitrary pass-through | Rejected | It bypasses typed reconstruction and has no proven current in-repository caller |
| Accept browser-supplied artifact `scopeId` | Rejected | Scope belongs to the authenticated server boundary, not the client request |
| Persist media or authorization payloads in a new NyxID shadow store | Rejected | It duplicates Aevatar actor ownership and risks storing content that the current public read model intentionally omits |
| Add `inputParts` and explicit `agentProfile` selection now | Rejected | Aevatar Console omits both in its browser chat, and NyxID's current product boundary is prompt plus durable context references |
| Deliver one rollup PR | Rejected | Auth, contract, UI, and liveness failures would be harder to isolate and independently verify |

## Appendix C. Risks

| Risk | Owner phase | Check | Failure signal |
|---|---|---|---|
| Upstream changes after the audit | `AC-0`, `AC-7` | Fetch and compare watched paths from the effective pin | Nonzero drift check with changed path names |
| Identity works but Aevatar has no usable NyxID capability | `AC-2` | Four-row header matrix and real tool callback | Stream 401 or tool authorization failure |
| Access review is unreachable from a cookie session | `AC-2` | Connected but unauthorized live probe | No `USER_SERVICE_ACCESS_REQUIRED` or unrestricted token claims |
| Access review widens another user or service | `AC-3` | Recompute owner and resource server-side; adversarial substitutions | Consent or grant diff outside the exact owner and service |
| Consent succeeds but the next token remains stale | `AC-3` | Mint after approval and verify Aevatar postcondition | Repeated access-review action or unverified postcondition |
| Attachments disclose cross-tenant content | `AC-4`, `AC-5` | Reference-only DTO and cross-user probes | Body, backing key, access list, or foreign artifact in response |
| Follow-up text mutates sealed attachments | `AC-4` | Create-only parser and live replacement probe | Changed attachment list on current state |
| Delete 202 is mistaken for completion | `AC-6` | Poll state until `not_found` | Sidebar row reappears after accepted delete |
| Keepalive-blind watchdog stops a valid long turn | `AC-6` | More than 120 seconds of keepalive-only liveness | Client sends `task.stop` or aborts the stream |
| History cannot restore live media or a generic auth blocker | `AC-6` | Inspect transcript operations and actor current state | Required state exists only in an expired SSE event |
| Canary leaks credentials | `AC-7` | Allowlisted JSONL and secret scanner | Token, cookie, code, key, credential, or artifact body match |
| A mock run is reported as live | `AC-7` | Separate non-mock Playwright project and strict prerequisites | `mock=1`, installed mock handler, or skipped credentialed row |
| Removing completions breaks an external caller | `AC-1` | Production access evidence before deletion | Named current caller within the agreed retention window |

## Appendix D. Links and reading list

Repository policy and required gates live in `CONTRIBUTING.md`, `WORKFLOW.md`, and `.github/workflows/ci.yml`. Pull requests target `main` by default. A dependent phase PR targets its verified parent branch until the parent lands, then the root coordinator retargets it. At least one approval is required. Security-sensitive phases also require explicit security review. `CI Pipeline` is the required status. Auto-merge stays disabled.

The Backend gate set is the following command group.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run -p nyxid --profile ci
cargo build -p nyxid --features aws-kms
cargo build -p nyxid --features gcp-kms
cargo build -p nyxid --features aws-kms,gcp-kms
```

The Frontend gate set is the following command group.

```bash
npm --prefix frontend run lint
npm --prefix frontend run test
npm --prefix frontend run build
cargo test -p nyxid-cli --test wizard_bundle_freshness
```

Run the plan checker with the following command.

```bash
node /Users/chronoai/.codex/skills/heca-mode/references/check-plan.mjs docs/plans/aevatar-chat-feature-integrate-convergence.md
```

Core NyxID reading is `docs/chat/README.md`, `docs/chat/01-architecture.md`, `docs/chat/02-wire-contract.md`, `docs/chat/03-stream-protocol.md`, `docs/chat/04-action-cards.md`, `docs/chat/05-frontend-ui.md`, `docs/chat/06-actions-registry.md`, and `docs/chat/07-testing-and-gaps.md`.

Core Aevatar reading at the audited clone is listed below.

| Concern | Source |
|---|---|
| Public commands and attachments | `agents/Aevatar.GAgents.NyxidChat/NyxIdChatPublicEndpoints.cs` and `NyxIdChatEndpoints.Streaming.cs` |
| Attachment admission | `agents/Aevatar.GAgents.NyxidChat/ConversationContextAttachmentAdmission.cs` and `NyxIdChatLifecycleFacade.cs` |
| Artifact discovery contract | `src/Aevatar.Studio.Hosting/Endpoints/ContentArtifactEndpoints.cs` and `src/Aevatar.Studio.Application.Abstractions/Studio/Contracts/ContentArtifactContracts.cs` |
| Access-review production | `agents/Aevatar.GAgents.NyxidChat/NyxIdChatBrowserActions.cs` and `NyxIdChatConversationAguiFrameBuilder.cs` |
| Action registry | `agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistry.cs` |
| Browser reference | `apps/aevatar-console-web/src/pages/chat/chatActorState.ts`, `ChatActorControls.tsx`, `index.tsx`, and `apps/aevatar-console-web/src/shared/auth/client.ts` |
| Identity assertion | `src/Aevatar.Mainnet.Host.Api/Responses/NyxIdIdentityAssertionAuthentication.cs` |

Owners must read the active `heca-mode` skill, `references/playbook-autopilot-stack.md`, `pstack-architect`, `pstack-tdd`, `pstack-unslop`, `pstack-no-comments`, and `pstack-technical-writing` before editing. Security verifiers must also use `pstack-interrogate`. UI owners must exercise the real browser surface and attach screenshots or recordings.

The external source is [Aevatar feature/integrate](https://github.com/aevatarAI/aevatar/tree/feature/integrate). Relevant upstream commits are [service access review](https://github.com/aevatarAI/aevatar/commit/0c5d4fbdb8e50037f78c5faa5e631d94e8dd30d7), [typed context attachments](https://github.com/aevatarAI/aevatar/commit/a850250d9c7822420bf4b07594b15397d80e71cd), [attachment readback and reasons](https://github.com/aevatarAI/aevatar/commit/676d577fd381e1b3dae227579b435662d6d49a44), [plan confirmation removal](https://github.com/aevatarAI/aevatar/commit/bf0536bf1d0f118e5fc50e4faf1fa19703796add), and [per-action registry degrade](https://github.com/aevatarAI/aevatar/commit/0b8ec500087331c3d12819b532e7dfa29e740fb4).

## Appendix E. Capability matrix

Verdicts use five values. `complete` means the current Aevatar contract has a NyxID implementation and focused tests. `partial` means a real path exists with a known gap. `missing` means the claimed browser experience requires the capability and NyxID lacks it. `obsolete` means NyxID still exposes behavior Aevatar removed. `not in scope` means the capability belongs outside this browser-chat program.

| Capability | Current verdict | Evidence and gap | Target phase |
|---|---|---|---|
| Human cookie authorization boundary | complete | Human-only middleware rejects API key, service-account, delegated, and relay auth | Preserve in every phase |
| Transport identity assertion and capability bearer | partial | Identity assertion is implemented, but the unrestricted cookie bridge may make access review unreachable and Aevatar still needs a capability | `AC-2`, `AC-3` |
| Typed command reconstruction | partial | Strict reconstruction exists, but includes obsolete `plan.resolve` and omits context attachments | `AC-1`, `AC-4` |
| SSE and AG-UI normalization | complete | The normalizer handles the current named actor facts and AG-UI frames | Preserve in `AC-6` |
| Actor and session identity | complete | First-run adoption and follow-up identity checks reject actor changes | Preserve in `AC-7` |
| First and follow-up turns | complete | First text omits conversation id; later text binds the adopted actor | Live proof in `AC-7` |
| Conversation list, history, state, and delete | partial | List, transcript, and state exist; delete treats 202 as completion and transcript operations are ignored | `AC-6` |
| Input, approval, stop, steer, retry, and skip | complete | Typed paths exist with state-version fences | Live proof in `AC-7` |
| `service.connect` | complete | Implemented card, continuation report, and postcondition path; no credentialed producer receipt | Live proof in `AC-7` |
| `key.create` and `key.rotate` | complete | Implemented dialogs and safe reports; no credentialed producer receipt | Live proof in `AC-7` |
| Action reports and postconditions | complete | Existing action paths wait on typed Aevatar continuation and postcondition state | Refresh and live proof in `AC-6`, `AC-7` |
| `service.reauthorize` | partial | NyxID advertises and renders it, while current Aevatar knows but does not execute it | Document in `AC-1`; no parity claim |
| `service.access_review` | missing | Unknown-action recovery is decline-only | `AC-2`, `AC-3` |
| Context-attachment transport | missing | Strict `RawTextChatCommand` rejects the field | `AC-4` |
| Context-attachment discovery and UX | missing | No safe artifact route or new-chat picker exists | `AC-5` |
| `plan.resolve` | obsolete | Backend and frontend still parse and send a command Aevatar rejects | `AC-1` |
| Registry deployment semantics and docs | partial | Tests and docs still encode obsolete exact-revision behavior | `AC-0`, `AC-1` |
| Keepalive liveness | partial | Valid keepalives do not reset the 120-second watchdog | `AC-6` |
| Media restoration | partial | `MEDIA_CONTENT` renders live but is not restored from current NyxID history decoding | Classify and close available recovery in `AC-6` |
| Authorization-card restoration | partial | Some actor cards restore from state; live-only generic authorization events do not | `AC-3`, `AC-6` |
| Credentialed producer proof | missing | Playwright uses `mock=1`; the manual script covers only empty action wake | `AC-7` |
| Direct engine durability | not in scope | Direct Chrono-LLM is a separate memory-only engine | Excluded |
| `inputParts` | not in scope | Aevatar Studio uses it, but current Aevatar Console browser chat and NyxID's product boundary do not | Excluded |
| Explicit `agentProfile` selection | not in scope | Optional upstream field with no current NyxID browser requirement | Excluded |
| Assistant feature-flag route enforcement | partial | The flag hides navigation but does not block the page or API | Product-policy decision outside Aevatar contract parity |

## Appendix F. Audit pins and independent review

| Item | Value |
|---|---|
| NyxID worktree | `/Users/chronoai/Library/Application Support/heca/worktrees/ea204fe1/swift-mesa` |
| NyxID branch | `add-test` |
| Audited NyxID HEAD | `52402f6a5510478b3601636a25572055f895c973` |
| `origin/main` observed during audit | `bba9682c9ab1bf166f7676982b61b7cf1995f50e` |
| Merge base | `52402f6a5510478b3601636a25572055f895c973` |
| Aevatar clone | `/tmp/aevatar-integrate.R1n4oF/repo` |
| Aevatar branch HEAD | `d53d150817c17f06e9e58b069d2da9ec41196900` |
| Effective last changed chat SHA | `b0b3738ed9477513edbfc6b9a6a75f14a592dbf4` |
| Prior NyxID documentation pin | `e7ba2e6eb` |
| Heca invocation | `01a060f1-08a4-7a00-9ef0-98b960983381` |
| Independent Grok agent | `01a060f1-08aa-7390-86c8-7c8bb67c1a04` |
| Independent provider | `grok` |
| Independent model | `grok-4.6` |
| Independent effort | `x_high`, requested as `xhigh` |
| Independent mode | `bypassPermissions` |
| Independent final state | `idle` |
| Independent final activity | sequence `5628`, output event id `707723` |
| Watched chat paths after effective SHA | No changes through Aevatar branch HEAD |
| NyxID stack base (`origin/main` at program start) | `4bc00e1a103b94ad7847aa776b50bfa87572c68d` |
| Aevatar branch HEAD at program start | `e5bba2e9719ad5132004b882744caa3875db1123` |
| Effective chat SHA at program start | `706ea7cab9d1f882e0fb0f034bb338102b6d5d2b` (tool-catalog materialization only; browser contract files unchanged since `b0b3738e`) |
| Operator go | 2026-09-02, this session, Fable restricted to planning |

The independent review answered no to the completeness question. Its additional validated findings were accepted for delete 202 observation, keepalive liveness, untyped completions forwarding, lack of credentialed producer proof, and the identity-versus-capability distinction. Its feature-flag finding is a product-policy issue, not an Aevatar contract defect. Its recommendation to shrink the default action manifest was rejected because current upstream code degrades unknown or divergent descriptors per action.

No product code changed during the audit. `frontend/node_modules` was installed with `npm ci` and remains ignored. No live NyxID-to-Aevatar producer test ran because the required deployment credentials and controlled producer state were unavailable.

## Appendix G. Pinned wire facts

The canonical public command set is shown below.

```text
text
input.resolve
action.continue
approval.resolve
task.stop
task.steer
step.retry
step.skip
```

The internal Aevatar access-review action uses the following parameters.

```text
action = service.access_review
userServiceId
serviceSlug
resourceUri
```

The context-attachment request shape is shown below.

```text
contextAttachments[]
  artifactId
  revisionMode = follow_current | pinned_revision
  pinnedRevisionId
```

The attachment set is create-only and sealed to the conversation. It contains at most four unique nonempty artifact ids. `pinned_revision` requires `pinnedRevisionId`. `follow_current` clears it. Allowed kinds are `text`, `markdown`, and `structured_document`. Public list and current-state reads expose references, not bodies. Admission uses `ATTACHMENT_ADMISSION_DENIED` and the reason set pinned in `AC-4`.

Aevatar delete returns HTTP 202 with `status` set to `accepted` and a `stateUrl`. The client must observe state until `not_found`. Aevatar sends keepalive events every 15 seconds. NyxID's 120-second progress watchdog must count a valid keepalive as liveness without rendering it.

## Appendix H. Principles that changed this plan

| Principle | Concrete choice |
|---|---|
| Prove It Works | The done predicate requires live browser and producer receipts, exact-head evidence, a real remote drift check, and deployment SHAs. Mock Playwright does not close a capability row |
| Sequence Work into Verifiable Units | The stack removes drift and obsolete paths before auth, actions, attachments, UX, liveness, and release proof. A dependent phase starts only after its parent has a green exact-head verdict |
| Guard the Context Window | The audit read targeted symbols and commit deltas, preserved summarized evidence here, and avoided copying complete source files into the plan |
| Never Block on the Human | Reversible scope and phase decisions are fixed now. Operator attention is reserved for explicit UI review, security authority, and merge gates |
| Foundational Thinking | `AC-0` establishes contract types and drift detection. `AC-4` establishes the attachment data shape before the picker uses it |
| Boundary Discipline | NyxID reconstructs typed Aevatar bodies, derives scope and resource identity server-side, and returns a dedicated reference-only artifact DTO |
| Build the Lever | `AC-0` replaces manual fixture synchronization with a rerunnable drift checker. `AC-7` adds a rerunnable credentialed contract canary |
| Subtract Before You Add | `AC-1` removes `plan.resolve`, the unsafe unused completions route, and stale revision assumptions before new state is added |
| Migrate Callers Then Delete Legacy APIs | Every `plan.resolve` caller, parser, control, fixture, test, and current-contract document moves or disappears in one phase |
| Model the Domain | Access review, attachment revision mode, deleting state, typed errors, and canary results use explicit variants and state transitions instead of generic objects or booleans |

## Appendix I. Open decisions and stop conditions

`AC-2` must choose the deployed authorization-session model. The current recommendation is consent-derived delegated restrictions, but only if the live probe proves bootstrap, client identity, mutation, revocation, and next-token behavior. A failed predicate changes the decision before product code starts.

`AC-1` must inspect production access evidence before removing `/api/v1/assistant/completions`. A current external caller pauses deletion until its owner and strict typed contract are known. Repository search alone cannot disprove external use.

`AC-6` must classify media and generic authorization recovery from the public Aevatar read models. If upstream exposes no durable source, the owner files or links an upstream issue and documents the boundary. NyxID does not create a shadow content store to manufacture parity.

The `experimental:ai-assistant` flag currently hides navigation only. Whether it must also block the route and backend API is a NyxID product-policy choice. It does not block this Aevatar contract program and needs a separate operator decision.

Any Aevatar watched-path drift, failed security review, missing credentialed staging prerequisite, changed phase head after verification, or red `CI Pipeline` voids the current verdict. The affected phase remains unready and no dependent phase advances.

## Appendix J. Decisions taken at program start and live-environment constraints

The operator delegated these decisions by authorizing execution on the reviewer's recommendation.

| Decision | Choice | Reversible by |
|---|---|---|
| Authority model candidate | Consent-derived delegated restrictions with the three enabling NyxID changes, falsifiable by the `AC-2` probe rows | `AC-2` decision record |
| Aevatar client identity | Admin-managed link from the `aevatar` catalog row to the registered OAuth client, chosen in `AC-2` | `AC-2` decision record |
| Consent mutation semantics | Merge, never replace, in `consent_service`; the OAuth consent page's replace behavior is filed as a separate issue | `AC-3` review |
| `/api/v1/assistant/completions` | Delete. The route sits on the human-only router, which rejects API keys, service accounts, delegated, and relay tokens, and no CLI, SDK, mobile, or frontend caller exists. Route-level telemetry does not exist, so router policy is the access evidence | `AC-1` review |
| `experimental:ai-assistant` enforcement | Out of scope; filed as a separate issue | Operator |
| Delegated routing | `~/.heca/pstack-models.json` with every `claude-fable` selection replaced by the next entry in its chain, because the operator restricted Fable to planning | Operator |

These live-environment constraints were observed at program start.

- No staging NyxID or Aevatar pair exists. The production pair is NyxID `https://nyx-api.chrono-ai.fun` and Aevatar `https://aevatar-console-backend-api.aevatar.ai`.
- Aevatar validates NyxID tokens against production JWKS, so a changed local NyxID cannot be exercised against real Aevatar before deployment. Deployment is operator authority.
- `dotnet` is not installed, so Aevatar cannot run locally. Docker is not running; a local single-node replica-set mongod on `127.0.0.1:27017` serves backend tests.
- Each phase names its live surface honestly. The surfaces are production read-only probes where no NyxID change is required, the local NyxID runtime with real HTTP for changed NyxID behavior, and the credentialed canary in `AC-7` designed to run after deployment. A mock run is never reported as live.
