# Chat Assistant — Rework Spec (Monday)

**Status:** planned rework, not started. Captured 2026-07-18. Owner: Calvin.
**One-line:** the chat bug is fixed by a **config change** (the live aevatar row
now matches the `eanz17/nyxid-chat` reference contract); the **code bridge** we
built along the way is dead/divergent and should be **reverted** so the feature
is the aligned `/assistant` mount + the reference-contract config, nothing else.

This is the execution plan. Do NOT touch it before Monday. Related docs:
`CHAT_SPEC.md` (how it works today), `CHAT_ASSISTANT_SPECS.md` (change history).

---

## 0. TL;DR of what we learned

- The 401/500 saga was chasing the wrong fix. Prod Aevatar **already** honors the
  reference contract: it authenticates via `X-NyxID-Identity-Token`
  (`aud: urn:aevatar:api`) and calls NyxID back via `X-NyxID-Delegation-Token`.
- The prod `aevatar` catalog row had **drifted** off the contract. Fixing the row
  made chat work with the currently-deployed code, **no code deploy needed**.
- The bridge (`build_forward_authorization` minting a bearer into `Authorization`)
  is now **dead code** (gated on `forward_access_token`, which is now `false`) and
  is a **divergence** from the reference + platform architecture.
- Verified working end-to-end on prod: **CLI/Bearer** and the **real React FE**
  both produce real answers and a complete tool execution (web search → real
  headline+URL rendered in the UI), no 401.

---

## 1. The fix that is already live (config, keep it)

The live prod `aevatar` `downstream_services` row was set to the reference
contract (via admin API `PUT /api/v1/services/{id}`):

```json
{
  "forward_access_token": false,
  "inject_delegation_token": true,
  "identity_propagation_mode": "jwt",
  "identity_jwt_audience": "urn:aevatar:api",
  "delegation_token_scope": "proxy:*"
}
```

- **Gotcha:** the admin API only accepts `delegation_token_scope` ∈
  `{llm:proxy, proxy:*, llm:status}` — **`proxy` is rejected**. Use `proxy:*`
  (grants REST proxy, which the Aevatar → NyxID `/proxy/s/{slug}` callbacks need).
- **Rollback snapshot** (pre-change) if ever needed:
  `{forward_access_token:true, inject_delegation_token:false,
  identity_jwt_audience:"", delegation_token_scope:"llm:proxy"}`.
- Service id: `a8e4314c-3fb2-4e1d-ac2a-08f6ac4b86ed`, base_url
  `https://aevatar-console-backend-api.aevatar.ai`.

With this row, a cookie session (or any caller) hits `/api/v1/assistant/*` →
`execute_proxy` injects the identity + delegation headers → Aevatar validates
identity, uses the delegation token for its LLM/proxy callbacks. No forwarded
bearer, no bridge.

**Decision for Monday:** confirm we keep this live config (it's what makes chat
work). It is field-for-field the reference contract.

---

## 2. The rework: revert the dead bridge (code)

Non-breaking, contained, but a dormant divergence (codex + 4683 tests confirm).
Delete it so the shared JWT validator / config carry no assistant-specific
tombstone and there is no accidental reactivation path.

**Safe to delete now:** no DB migration; marker tokens are stateless JWTs, never
persisted; the marker generator is already unused; prod row is
`forward_access_token:false` so nothing mints one; the cut-4 marker drain window
(max TTL 300s + skew, conservatively 10 min) has long passed.

### 2.1 `backend/src/handlers/assistant.rs`
- Remove `needs_forward_token_bridge`, `resolve_forward_scope`,
  `build_forward_authorization` and their `crypto::jwt` / `models` / `mw::auth`
  imports (`generate_delegated_access_token`, `TokenRestrictionClaims`,
  `MCP_DELEGATION_TOKEN_TTL_SECS`, `DownstreamService`, `PROXY_SCOPE`,
  `scope_allows_rest_proxy`, `AuthMethod`).
- Make `forward`'s `request` non-`mut`; remove the conditional `Authorization`
  overwrite + the `assistant_delegation_token_minted` trace.
- Remove the three bridge test blocks: `bridge_mints_only_for_cookie_sessions...`,
  `forward_authorization_is_a_delegated_proxy_token_for_aevatar`,
  `forward_authorization_falls_back_to_proxy_when_row_scope_is_insufficient`.
- **KEEP:** the route handlers, `assistant_service` path builders,
  `synthetic_request`, create-poll materialization, composite delete, SSE
  streaming, and the `execute_proxy` call. (Synthetic materialization/delete
  requests still work — `execute_proxy` builds identity/delegation from the
  verified `AuthUser`, no incoming bearer required.)

### 2.2 `backend/src/crypto/jwt.rs`
- Remove `Claims.assistant_forward` and every `assistant_forward: None`
  constructor entry (8 literals).
- Remove `generate_assistant_forward_access_token` (the `#[allow(dead_code)]`
  tombstone).
- Remove the `verify_token` marker rejection branch.
- Remove marker tests: `assistant_forward_token_*`, `decode_as_downstream`
  helper, and the `assistant_forward` assertions in
  `normal_token_generators_leave_assistant_forward_unset`.
- **KEEP `generate_delegated_access_token` unchanged** — the standard proxy
  `inject_delegation_token` path still uses it (that IS the live mechanism now).

### 2.3 `backend/src/config.rs` + test literals
- Remove `AppConfig.jwt_assistant_forward_ttl_secs` (field, `Debug`, env parse,
  default).
- Remove the one-line `jwt_assistant_forward_ttl_secs: 300,` from the 7 test
  literals: `test_utils.rs`, `crypto/aes.rs`, `crypto/apple_client_secret.rs`,
  `crypto/local_key_provider.rs`, `handlers/channel_webhooks.rs`,
  `services/channel_relay_service.rs`, `services/social_auth_service.rs`, plus
  the jwt/config test fixtures.

### 2.4 Docs
- Remove `JWT_ASSISTANT_FORWARD_TTL_SECS` from `CLAUDE.md` and `docs/ENV.md`.
- Remove the "Assistant Forward Token" paragraph from `docs/API.md` and the
  compatibility note from `docs/DEVELOPER_GUIDE.md`.
- Rewrite `docs/CHAT_SPEC.md` around the **deployed identity + standard
  delegation** contract (remove bridge / resilience-floor / fallback-warning /
  Authorization-mint text; §1's config table + the security boundary stay).
- Keep the bridge story in `docs/CHAT_ASSISTANT_SPECS.md` as **historical /
  reverted** only — no "current behavior" phrasing.

---

## 3. PR history to reconcile Monday

- `#1200` (merged, `dee0b7e9`) — marker-token bridge. **Superseded.**
- `#1201` (merged, `186d5c31`) — delegated-token bridge. **This is what's
  deployed.** The revert removes it.
- `#1202` (OPEN, branch `verify-chat-connection`) — resilience floor + docs +
  `CHAT_SPEC`. **Close/supersede** with the revert PR (or convert #1202 into the
  revert). Its `CHAT_SPEC.md` + the config learning are worth keeping.

**Monday sequence:** open the bridge-revert PR off `main` → keep `/assistant`
mount + reference-contract config → merge → deploy. No behavior change for users
(the bridge is already inert); the deploy just removes dead code.

---

## 4. Verified evidence (so we don't re-litigate)

- Real prod Aevatar, reference-contract row, **CLI/Bearer**: full turn,
  `RUN_FINISHED`, real answer, 0 `RUN_ERROR`.
- Real **React FE** against prod (Bearer-injected vite harness = cookie path):
  loaded `/assistant` authenticated, typed in the composer → streamed answer;
  **complete tool execution** (web search → real headline + Google News URL
  rendered). Screenshot `/tmp/fe-assistant-working.png`,
  `/tmp/fe-tool-exec.png`. FE assistant tests: 28 pass.
- Broker audit for a FE turn: `POST chat/completions` via `/proxy/s/{llm}`
  attributed to the user (delegation token reaches the LLM).
- Backend: 4683 tests, clippy, fmt green. Codex scan: NO-BREAKING-CHANGES, live
  behavior CONTAINED, alignment = keep mount / remove bridge.

## 5. Known gaps to fold into the rework (not blockers to chat working)

- **G-hang / G6:** a slow/unresponsive tool (observed: an Ornn skill-search)
  hung the turn — Aevatar produced no output, no terminal frame, and the FE
  composer stayed **stuck disabled** with no error/retry. NyxID kills a silent
  SSE stream at 60s (`PROXY_STREAM_IDLE_TIMEOUT_SECS`) but emits **no error
  frame**, and the FE doesn't recover a hung turn. Fix: idle-timeout error frame
  + FE hung-turn recovery.
- **G1 / TD-7 (frontend):** AG-UI normalizer renders text/`RUN_ERROR`/
  `RUN_FINISHED` only; approval/authorization/tool frames are dropped, and
  `decideApproval` JSON-parses an SSE response. Basic Q&A + tool *results* render;
  approval *cards* and tool-activity cards do not.
- **G2 / TD-1:** `execute_proxy` still runs per-user node routing + rejects an
  inactive legacy Aevatar `UserServiceConnection` — "works for everyone but me".
  Strict admin-target execution mode.
- **G4:** delegation token is 300s; `/delegation/refresh` expects `act.sub` to be
  an active OAuth client, so a run whose first callback lands after 5 min may
  fail. (Immediate callbacks fine.)
- **G5 / TD-10:** no `runs/{runId}:resume` — workflow approvals can't resume.
- Doc: recommend `delegation_token_scope: "proxy:*"` (NOT `proxy`, which the
  admin API rejects).
