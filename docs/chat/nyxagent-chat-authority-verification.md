# NyxAgent chat authority: implementation and verification

Round 3 implements D10–D15 on top of the existing uncommitted engine work.
No commit was created. The table lists files changed or extended for this round;
previous engine and review-round changes remain in the worktree.

## File-by-file changes

| File | Change |
| --- | --- |
| `backend/src/models/assistant_acknowledgement.rs` | New persistent acknowledgement record, BSON timestamps and redacted Debug. |
| `backend/src/models/assistant_agent_credential.rs` | Conversation identity on encrypted credential rows; legacy compatibility. |
| `backend/src/models/assistant_conversation.rs` | Ask/Full access mode with Ask default for legacy documents. |
| `backend/src/models/mod.rs` | Register the acknowledgement model. |
| `backend/src/services/assistant_acknowledgement_service.rs` | Transactional requests and decisions, permission grants, denial retention, expiry, canonical digests and one-time action consumption. |
| `backend/src/services/assistant_account_tools.rs` | Closed 22-tool native account inventory, schemas, owner checks, safe projections, service dispatch, mode gates and audits. |
| `backend/src/services/assistant_access_mode_service.rs` | Atomic mode/key changes, live-turn refusal, retained individual grants and mode audit. |
| `backend/src/services/assistant_agent_credential_service.rs` | Per-conversation provisioning/replacement, encrypted rotation adoption, latest valid models credential and child revocation audits. |
| `backend/src/services/assistant_nyxagent.rs` | Transactional first-turn provisioning, draft mode grammar, credential index migration and transactional conversation deletion. |
| `backend/src/services/key_service.rs` | Session-aware key deletion and binding cleanup; rotation preserves chat mode and resets individual acknowledgements. |
| `backend/src/services/channel_routing_service.rs` | Reject assistant keys during route creation, updates and runtime resolution. |
| `backend/src/services/admin_user_service.rs` | Acknowledgement cascade during account deletion. |
| `backend/src/services/mcp_service.rs` | Internal virtual tool source, kept outside proxy execution. |
| `backend/src/services/mcp_approval.rs` | Handle internal tools without treating them as proxied service operations. |
| `backend/src/services/exact_service_approval_service.rs` | Handle internal tool sources in exact-service authorization. |
| `backend/src/services/mod.rs` | Register authority services and integration tests. |
| `backend/src/services/assistant_authority_tests.rs` | Lifecycle, concurrency, all 22 tools, ownership, audits, migration, rollback, deletion, mode and assistant-key boundary tests. |
| `backend/src/services/assistant_nyxagent_tests.rs` | Extend engine tests for conversation credentials and closed access-mode request grammar. |
| `backend/src/handlers/assistant_nyxagent.rs` | Acknowledgement/mode routes and safe conversation, history and index DTOs. |
| `backend/src/handlers/assistant_nyxagent_tests.rs` | Human/flag/owner gates, decisions, conflicts, DTOs, key links and mode audits. |
| `backend/src/handlers/mcp_transport.rs` | Discover all connected services, annotate chat_access, enforce acknowledgement gates, dispatch native tools and audit every chat MCP request. |
| `backend/src/handlers/mcp_chat_authority_tests.rs` | Authenticated MCP listing/refusal/Allow/retry/Full-mode tests against a real local upstream. |
| `backend/src/handlers/api_keys.rs` | Batch owner-scoped assistant conversation links into key metadata. |
| `backend/src/mw/auth.rs` | Declare the internal assistant:account scope; public key scope assignment remains closed. |
| `backend/src/routes.rs` | Mount human-only acknowledgement and access-mode routes. |
| `backend/src/billing_integration_tests.rs` | Update the assistant egress fixture to expect scoped nyxid_ag_ keys. |
| `frontend/src/schemas/assistant-nyxagent.ts` | Acknowledgement, pending-count and access-mode wire schemas. |
| `frontend/src/lib/assistant/nyxagent-transport.ts` | History cards, explicit decisions, identity fencing, mode changes and draft-only initial mode submission. |
| `frontend/src/lib/assistant/nyxagent-transport.test.ts` | Transport, history ordering, identity, decisions and mode regression tests. |
| `frontend/src/lib/assistant/nyxagent-http-fixtures.ts` | Durable acknowledgement and access-mode browser scenarios. |
| `frontend/src/hooks/use-assistant-nyxagent.ts` | Decision/mode mutations, 750 ms decision throttle and active-or-pending history polling. |
| `frontend/src/hooks/use-assistant-nyxagent.test.tsx` | Polling, explicit decisions, throttling and cache refresh tests. |
| `frontend/src/components/assistant/nyxagent-acknowledgement-card.tsx` | Per-kind Allow/Deny cards and compact decided records. |
| `frontend/src/components/assistant/nyxagent-acknowledgement-card.test.tsx` | Explicit decisions, status rendering and failure handling. |
| `frontend/src/components/assistant/nyxagent-mode-selector.tsx` | Mode choices, Full-access confirmation and disabled/error states. |
| `frontend/src/components/assistant/nyxagent-mode-selector.test.tsx` | Confirmation, cancellation, failure and active-turn behavior. |
| `frontend/src/components/assistant/assistant-chat-page.tsx` | Inline cards, composer focus after decisions, Mode selector and Full-access header badge. |
| `frontend/src/components/assistant/chat-message.tsx` | Inline transcript rendering hook for acknowledgement cards. |
| `frontend/src/components/assistant/assistant-engine-sidebar.tsx` | Format the extended engine sidebar consistently. |
| `frontend/src/stores/assistant-draft-store.ts` | Remember the new-draft mode with per-user isolation. |
| `frontend/src/stores/assistant-draft-store.test.ts` | Mode persistence and identity reset tests. |
| `frontend/src/components/dashboard/api-key-table.tsx` | Hide assistant keys by default; add the display toggle and simplify the copy callback. |
| `frontend/src/components/dashboard/api-key-table.test.tsx` | Hidden-by-default and toggle coverage. |
| `frontend/src/pages/api-key-detail.tsx` | Show the owning assistant chat link. |
| `frontend/src/pages/api-key-detail.test.tsx` | Verify assistant chat metadata and navigation. |
| `frontend/src/types/api.ts` | Optional assistant_conversation_id on API key responses. |
| `frontend/src/pages/channel-bots.tsx` | Exclude assistant keys from device channel route selection. |
| `frontend/src/pages/channel-bot-detail.tsx` | Exclude assistant keys from bot route selection. |
| `frontend/e2e/nyxagent-authority.spec.ts` | Allow/retry, Deny, account/action cards, hidden keys, Full access and remembered drafts. |
| `frontend/e2e/nyxagent.spec.ts` | Select profiles by accessible name alongside the new Mode selector. |
| `frontend/e2e/channel-platform-fixtures.ts` | Shared descriptor catalog matching the channel UI contract. |
| `frontend/e2e/managed-onboarding-fixtures.ts` | Serve channel platform descriptors to affected channel tests. |
| `frontend/e2e/channel-whatsapp.spec.ts` | Use descriptor-driven labels and keep screenshots inside Playwright output directories. |
| `frontend/e2e/channel-telegram-new.spec.ts` | Use descriptor-driven labels, repository-local screenshots and distinguish request polling from summary refreshes. |
| `frontend/src/lib/assistant/chat-stream-orchestrator.ts` | Release the stopped stream without awaiting sidebar refresh, preventing a silently dropped immediate follow-up. |
| `frontend/src/hooks/use-assistant-chat.test.tsx` | Regression test with a stalled sidebar refresh after Stop. |
| `cli/src/wizard/assets/index.html` | Rebuilt embedded wizard bundle for the updated shared draft store dependency. |
| `cli/src/wizard/bundle-meta/index.hash` | Regenerated wizard source freshness hash. |
| `cli/src/wizard/bundle-meta/index.manifest` | Regenerated wizard dependency manifest. |
| `CLAUDE.md` | Per-conversation secret-storage exception, internal scope, route-agent restriction, acknowledgement routes and modes. |
| `docs/ENV.md` | Confirm no new environment variables and describe conversation authority. |
| `docs/chat/08-nyxagent-engine.md` | Normative per-conversation credentials, acknowledgements, exact native inventory, mode, audit and UI contracts. |
| `docs/chat/07-testing-and-gaps.md` | Authority/mode coverage and Stop regression coverage. |
| `docs/chat/nyxagent-chat-authority-plan.md` | Retain D10–D15 as the design record superseded by the normative engine document. |

## Decision trace

| Decision | Principal files | Tests proving the contract |
| --- | --- | --- |
| D10 | `assistant_agent_credential_service.rs`, `assistant_nyxagent.rs`, `key_service.rs`, `channel_routing_service.rs`, key DTO/table/detail | `concurrent_provisioning_is_stable_encrypted_and_scoped_for_mcp_and_llm`; `models_select_latest_valid_credential_and_conversation_delete_revokes_only_its_key`; `rotation_adopts_the_successor_without_minting_or_disclosing_another_key`; `deleting_a_conversation_revokes_children_clears_bindings_and_audits_after_commit`; `legacy_conversation_defaults_to_ask_and_owner_unique_credentials_migrate`; key table/detail tests |
| D11 | `assistant_acknowledgement.rs`, `assistant_acknowledgement_service.rs`, `mcp_transport.rs`, `handlers/assistant_nyxagent.rs` | `acknowledgement_service_allow_is_atomic_scoped_and_versions_the_key`; `acknowledgements_deny_expire_and_reask_only_after_a_new_user_message`; `action_acknowledgements_bind_arguments_key_conversation_and_are_single_use`; `acknowledgement_history_keeps_pending_and_only_twenty_decided_without_arguments`; `chat_mcp_lists_ungranted_tools_and_allow_retries_execute_without_bypassing_denial` |
| D12 | `assistant_account_tools.rs`, `mcp_service.rs`, `mcp_transport.rs`, `mcp_approval.rs`, `exact_service_approval_service.rs` | `native_inventory_is_closed_and_schemas_exclude_secret_inputs`; `every_native_tool_requires_a_real_conversation_key_and_audits_refusals`; `every_native_tool_requests_account_acknowledgement_and_preserves_owner_scope`; `native_account_inventory_executes_existing_services_and_audits_every_tool`; `native_key_tools_hide_another_owners_existing_key_even_with_full_access`; `assistant_keys_cannot_self_widen_bind_or_become_route_agents` |
| D13 | `nyxagent-transport.ts`, `use-assistant-nyxagent.ts`, acknowledgement card, chat page, key table/detail and HTTP fixtures | Transport/hook/card/table/detail Vitest suites; `nyxagent-authority.spec.ts` service Allow/retry, Deny/refusal, account/action cards, focus, persistence and hidden-key navigation flows |
| D14 | `CLAUDE.md`, `docs/ENV.md`, `08-nyxagent-engine.md`, `07-testing-and-gaps.md`, retained design record | Full verification results below. Version remains 0.24.0; no new environment variables. Original upstream contract/source trace remains in `08-nyxagent-engine.md`. |
| D15 | `assistant_conversation.rs`, `assistant_access_mode_service.rs`, mode handler/route, MCP gates, mode selector, draft store | `access_mode_switch_is_owner_scoped_fenced_and_preserves_acknowledged_services`; `full_draft_provisions_full_authority_and_rotation_and_replacement_preserve_mode`; `native_account_full_access_executes_every_tool_without_cards_and_audits_the_mode`; `full_access_cannot_modify_chat_keys_or_use_them_as_route_agents`; mode selector and draft-store Vitest suites; repeated Full-access deletion/browser draft test |

## Verification

All final commands exited 0. The excerpts below are copied verbatim from their
actual output. Rust commands ran at the worktree root; npm/Playwright commands
ran in `frontend/`.

`cargo fmt --check`: no output, exit 0.

`cargo clippy --all-targets -- -D warnings`:

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2m 03s
```

`NYXID_TEST_DATABASE_URL=mongodb://127.0.0.1:27019 cargo test -p nyxid --bin nyxid-server -- --test-threads 2`:

```text
test result: ok. 6173 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 405.62s
```

`cargo test -p nyxid-cli` (all unit, integration and doc-test summaries):

```text
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1233 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.70s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 21.37s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.62s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

`npm run lint`:

```text
✖ 27 problems (0 errors, 27 warnings)
```

`npm run test`:

```text
 Test Files  335 passed (335)
      Tests  3368 passed (3368)
   Start at  22:23:57
   Duration  75.95s (transform 22.51s, setup 28.65s, import 113.83s, tests 190.21s, environment 126.71s)
```

`npm run build`:

```text
✓ built in 979ms
✓ built in 56ms
Mock scenario footprint assertion passed; credential-accept output is present.
```

`npm run build:wizard`:

```text
✓ built in 548ms
install-wizard-bundle: wrote cli/src/wizard/bundle-meta/index.manifest (111 files)
install-wizard-bundle: wrote cli/src/wizard/bundle-meta/index.hash (9a211cbaa8ec…)
```

Repeated browser command:

```sh
npx playwright test e2e/nyxagent.spec.ts e2e/nyxagent-authority.spec.ts e2e/new-chat.spec.ts e2e/chatting.spec.ts e2e/history.spec.ts e2e/switching.spec.ts e2e/channel-whatsapp.spec.ts e2e/channel-telegram-new.spec.ts --repeat-each 4
```

```text
  172 passed (3.5m)
```

`git diff --check`: no output, exit 0. Build directories, Playwright output and
verification logs do not appear in git status; the rebuilt embedded wizard is an
intentional source artifact.

Earlier verification exposed an obsolete billing fixture key prefix, stale channel
platform fixtures, a polling assertion counting a separate summary refresh, and a
real immediate-send-after-Stop race in the existing actor chat. These were fixed.
A browser run affected by live formatting and a resource-contended frontend test
run were discarded and rerun to completion with stable source files. No tests were
skipped, and no failed final command remains.

## Completion

D10–D15 are complete. Nothing is deferred. Changes remain uncommitted in this
worktree; the NxyAgent reference clone was not modified.
