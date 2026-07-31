# Implementation Notes

## Completion

- W1-W8 and the 2026-08-01 Opus/Fable rework are implemented. There are no known incomplete work items.
- No binding-spec conflict required a behavioral deviation from `docs/chat-aevatar-dev-parity-spec.md`.
- The read-only Aevatar reference used for console parity was `origin/dev` at `bbd906eb503a126c1a4b6a9ff67952cc819ccdd4`. It was inspected with `git show origin/dev:<path>` only and was not modified or checked out.

## Rework Changes

- Workflow creates now run identity recovery after a normal terminal frame with no `aevatar.chat.context`, after header/network ambiguity, after stream truncation, and after abort. Empty normal-EOF recovery fails closed while the conversation remains on its `workflow-pending-*` identity.
- Every create-recovery entry point uses the stored pending conversation identity instead of `deliveryStarted`. `RUN_STARTED` run-actor ids are not compared with Chat History turn ids; successful recovery adopts the Chat History turn id. Workflow cancellation starts recovery before considering the run-actor id, while typed/actor stop behavior remains unchanged.
- Non-retryable reservation-refresh failures surface `history_refresh_failed`. Retryable 5xx refresh failures still consume the bounded refresh attempts and ultimately surface the original `CHAT_HISTORY_RESERVATION_UNAVAILABLE`, matching the console retry loop.
- Workflow context scope comparison is enforce-when-known: a present local user id must match, while an unhydrated auth store skips the comparison. This intentionally differs from the console's always-known scope pending a live NyxID production capture.
- The Chat History list drain now degrades on non-JSON or missing-`conversations` pages, preserves the repeated-cursor hard error, sends `Accept: application/json` on every synthetic page request, and stops before accumulated page bodies exceed 8 MiB.
- Valid but unsupported conversation-id prefixes now return a not-found-shaped backend error. The negative 32,769-character trimmed prompt boundary and backend create-recovery 404 passthrough are explicitly tested.
- Session identity is now explicitly pinned across a same-conversation history re-read and a within-conversation reservation retry, in addition to the existing remint-on-reopen test.

## Guard Verification

- The W1.6 source guard now checks real builder fragments such as `conversations/{conversation_id}/approve` and verifies that typed-family detail resolves outside scoped Chat History.
- During development, a temporary `conversations/{conversation_id}/approve` production-source probe was inserted. The guard failed with `per-conversation command route /approve must not return`; the probe was then removed and the same test passed. The scratch probe is not present in any commit.

## Verification Notes

- `cargo test` passed with a disposable unauthenticated MongoDB on the test harness's supported CI port (`127.0.0.1:27017`): main backend target `4911 passed`, all remaining workspace targets and doctests passed.
- `cargo test --workspace assistant` passed before the full gate: backend target `46 passed` plus the matching CLI assistant test.
- Frontend gates passed: both production builds completed, Vitest reported `194 passed` files / `2323 passed` tests, and lint reported 0 errors / 23 existing repository-wide warnings.
- `cargo test -p nyxid-cli --test wizard_bundle_freshness` passed. The wizard bundle was not rebuilt because the changed assistant transport files remain outside its committed dependency graph.
- `npm ci` was needed in the first implementation round because frontend dependencies were absent. It did not change dependency declarations or the lockfile; its package-audit findings were pre-existing.
- The frontend build still emits the repository's existing generated-CSS `text-destructive` pseudo-class warning. The touched frontend files pass focused ESLint.
- No dependencies were added, no push was performed, and the upstream Aevatar worktree remained read-only.
