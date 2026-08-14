# Live implementation review — Chrono ephemeral-agent POC (Opus, adversarial)

Reviewer: Opus · adversarial implementation reviewer, live during Sol's work
Branch: `feat/assistant-ephemeral-agent-poc` · worktree `zen-fox` · base `origin/main 9f3d38a6`
Target: merged POC surface (`assistant_direct_agent_poc*`) + Sol's extension diff
Specs: `docs/chat/chrono-llm-ephemeral-agent-plan.md` (v0, shipped) · `docs/chat/chrono-ephemeral-agent-poc-extension-plan.md` · `…extension-plan.review-sol.md`

- **Pass 1** — against merged main, pre-diff. Complete.
- **Pass 2 (final)** — against Sol's diff across 10 files (backend engine, tools, prompt, prompt asset;
  frontend transport, shared type, RunCard, two fixtures, transport test). Frontend `result_preview` lockstep
  has landed and is verified. Live read-only Ornn evidence supplied by the owner is incorporated.
- Fixes for **P2 / P3 / P10 / M6** were relayed to Sol and are in flight; they are listed as open until the
  code shows them.
- No build was run: `cargo check` takes the shared `target/` lock and would stall Sol's loop. All findings are
  static, with file:line evidence.

## Axis status (final)

| Axis | Pass 1 | Pass 2 |
| --- | --- | --- |
| Plan phase has no tools / never dispatches | FAIL (B1, B2) | **PASS** |
| Native loop executes typed reads in user context | PASS | PASS |
| No Aevatar runtime / no guessed / fixed / UserManaged fallback | PASS | **PASS** — zero `aevatar` references anywhere in the POC |
| No arbitrary URL/path/method execution | PASS | PASS |
| Skills bounded, source-provenanced with content SHA, untrusted | PARTIAL FAIL | **PASS** for provenance; residuals P3, P10 (fix in flight) |
| Result previews bounded / scrubbed / useful / no raw args | none existed | **PASS** for bound + key-scrub + render; **value-scrub open (M6)**, **projection leaks open (L2)** |
| Context budgeting preserves a final Report | FAIL (M1) | **PASS** — compaction + reserve, with a real test |
| Session auth, CSRF, flag gating, route behavior | PASS | PASS — untouched |
| Tests prove dispatch + phase boundaries | FAIL | **PARTIAL** — much stronger, but nothing drives `execute()` (P11) |
| Disposable POC, no durable machinery | PASS | PASS |

**Overall: the two BLOCKERs are closed and the diff is materially stronger than the plan it implements.**
Remaining blocking-grade work is L1 + L2 (both ~2-line projection fixes, both live-informed), plus the
in-flight P2/P3/P10/M6.

---

# Pass 2 — resolved by the diff (re-verified, not taken on trust)

**B1 resolved.** `build_hop_body` adds `tools` / `tool_choice` / `parallel_tool_calls` only under
`ToolMode::Enabled`; Plan and Report bodies carry none of them. The test was strengthened from
`assert_eq!(plan["tool_choice"], "none")` to asserting *absence* of all three fields for both bodies. This is
the structural fix, not the advisory one.

**B2 resolved, more strictly than proposed.** `PlanningTransition::ExecuteBatch` is deleted;
`planning_transition` returns `Err(RunError::Upstream)` when a plan hop carries **any** tool call *or*
`finish_reason == "tool_calls"` — the non-empty check is independent of the finish reason, so a `stop` hop
that still emitted calls is caught. `plan_phase_tool_call_is_rejected_before_any_tool_dispatch` asserts zero
`tool_dispatches` and no appended message. The dispatch path is gone, not merely guarded.

**M1 resolved — well.** `compact_context_for_hop` runs before every Execute and Report hop, targeting
`MAX_UPSTREAM_BODY_BYTES − REPORT_CONTEXT_RESERVE_BYTES` (448 − 64 = 384 KiB), looping on
`compact_oldest_complete_tool_exchange`. That helper is protocol-correct: it compacts only an
`assistant(tool_calls)` whose replies are **all present, adjacent, and `tool_call_id`-matched**; it rewrites
rather than deletes (every `id` preserved, so no orphaned `tool_call_id`); it replaces `arguments` with `"{}"`
and bodies with `compacted_from_model_content`. `ContextOverflow` fires only when nothing compactable
remains. `report_compaction_preserves_complete_tool_exchange_groups` builds 20 oversized exchanges and asserts
the body drops under target, the injected `secret_argument` text is gone, and every call still has its
matching adjacent reply.

**M2 resolved.** Any non-`stop` plan finish is now `Err(RunError::Upstream)`, so `length` / `content_filter`
can no longer yield `done{status:"completed"}` with no answer.

**M3 / M4 resolved.** `skill_document()` emits `source`, `id`, `version`, `content_sha256`,
`content_truncated`, with the digest in the fence header; bundled skills reach the model through the same
tool-result path and the same fence, with build-pinned `BUNDLED_SKILL_VERSION`. `sha2` / `hex` were already
backend deps — no manifest change. The hardcoded prod `ORNN_DEMO_SKILL_GUID` is gone entirely, replaced by
the run-local observed set.

**M5 / P1 resolved.** Backend `result_preview` (2 KiB, derived from the already-scrubbed `body`, never from
`arguments`) now has full frontend lockstep: `result_preview: z.string().max(2048)` in the `.strict()` schema,
`result_preview?: string | null` added as **optional** on the shared `RunContentBlock` step (so the Aevatar
transport keeps typechecking), `<details><pre>` rendering in `RunCard`, both fixtures updated, and a test that
actually renders the card and asserts the preview text is visible. The bound is safe in both directions: the
backend truncates by UTF-8 bytes and Zod's `.max()` counts UTF-16 units, and UTF-8 bytes ≥ UTF-16 units for
all inputs, so a validly-truncated preview can never be rejected. React escapes the `<pre>` content — no
`dangerouslySetInnerHTML`, so untrusted skill text rendered here cannot inject markup.

**P4 resolved.** Oversized arguments are no longer a context hazard: the exchange is compactable (the
synthetic `invalid_args` reply completes the pair) and compaction runs before the next hop, so the blob never
reaches Chrono. The compaction test asserts the argument text is absent afterwards.

**P12 mostly resolved.** `result_preview_is_redacted_and_independently_bounded` is a genuine test: it builds a
real `ModelToolResult` containing `accessToken` and a nested `x-api-key`, and asserts both are absent from the
preview while a safe value survives, plus the byte bound and the truncation marker. Residual is cosmetic —
see P12′ below.

**P8's core assumption verified by the owner's live read-only probe.** `data.items[].guid` is a lowercase
canonical UUID, so the strict `canonical_uuid` filter is compatible with the live registry, and
`find_result_array`'s `data.items` special case is the branch that fires (the top-level pass skips `data`
because it is an object, leaving exactly one candidate). The demo-killing form of P8 is closed. What remains
is the field-ordering hazard, now L1.

**N4 / Sol N1 resolved.** `agent-poc.md` states the protocol as "Understand preflight -> Plan -> Execute ->
Report" and names Understand as a server-owned inventory preflight; phase strings were rewritten to match the
post-B1 reality ("Plan and Report deliberately declare no tools").

---

# Pass 2 — open findings

## L1 — MAJOR: `id`-before-`guid` preference can silently drop every live Ornn row

**Evidence.**

```rust
// backend/src/services/assistant_direct_agent_poc/tools.rs:1094-1095
let selected = selected_string(object, &["id", "guid", "skillId"])?;
let id = canonical_uuid(&selected)?;
```

`selected_string` returns the first key that **exists as a string**; `canonical_uuid` is then applied to that
one value only, and `filter_map` drops the row on failure. The owner's live probe confirms the canonical UUID
lives in **`guid`**. If a row also carries an `id` field holding anything non-canonical — a numeric string, a
slug, a composite key — `selected` binds that value, canonicalization fails, and the row is discarded **even
though a valid `guid` is present two keys later**.

**Consequence.** `matches` becomes empty, `count: 0`, the observed set stays empty, and
`nyx_get_skill{source:"ornn"}` can never pass its own gate. The failure is silent and indistinguishable from
"no skills matched" — the demo's Ornn beat dies with a plausible-looking empty result. The existing test
cannot catch it: `ornn_search_admits_only_canonical_uuid_ids` uses rows with a single `id` key.

**Fix (2 lines).** Take the first candidate that *canonicalizes*, not the first that exists:

```rust
let id = ["id", "guid", "skillId"]
    .iter()
    .find_map(|key| object.get(*key).and_then(serde_json::Value::as_str).and_then(canonical_uuid))?;
```

Extend the test with a row shaped `{"id":"42","guid":"<canonical-uuid>","name":"…"}` asserting it is admitted
and that `observed_ornn_skill_ids` contains the guid.

## L2 — MAJOR: a scalar `creator` bypasses the projection allowlist, and the live response carries creator email

**Evidence.**

```rust
// backend/src/services/assistant_direct_agent_poc/tools.rs, project_creator
match creator {
    serde_json::Value::String(_) | serde_json::Value::Number(_) => creator.clone(),   // verbatim passthrough
    serde_json::Value::Object(creator) => serde_json::json!({
        "id":   selected_field(creator, &["id", "userId", "user_id"]),
        "name": selected_field(creator, &["name", "displayName", "display_name"]),
    }),
    _ => serde_json::Value::Null,
}
```

The object branch is a correct allowlist and deliberately omits email. The scalar branch defeats it: any of
`creator` / `createdBy` / `created_by` arriving as a bare string is copied verbatim. The owner's live probe
confirms the search response carries **creator email and permission metadata**.

**Consequence.** `{"creator":"alice@example.com"}` reaches the model context *and*, now that
`result_preview` ships, the browser. `scrub_credentials` cannot help — it filters credential *key names*, and
`creator` is not one. This is the exact "minimal projection" requirement the owner restated.

**Fix.** Delete the scalar passthrough: map `String`/`Number` to `Value::Null`, or admit it only when it
canonicalizes as an id. Test: a row with `{"creator":"alice@example.com"}` and one with
`{"creator":{"name":"Alice","email":"alice@example.com"}}` — assert the serialized projection contains no
`@`. Consider the same assertion over the whole projected row, since `description` / `accessReason` are also
free third-party text that now reaches the browser.

## M6 — MAJOR (fix in flight): scrubbing is key-name-only; values reach the model *and now the browser*

`scrub_credentials` removes object entries whose **key** canonicalizes to one of 17 names; it never inspects
values, and a non-JSON body becomes an unexamined `Value::String`. So `{"message":"use bearer sk-live-…"}`, a
`text/plain` token, or `{"headers":{"X-Custom-Auth":"…"}}` pass through. `result_preview` gives that gap a
second consumer — the UI — where previously only the model saw it.

Note the new test reinforces the *key* property only (`accessToken`, `x-api-key`), so it should not be read as
covering values. Fix: add a value-side pass for high-signal shapes (`sk-[A-Za-z0-9]{16,}`,
`ghp_`/`gho_`/`github_pat_`, `nyx_[a-z]{3}_`, `eyJ…\.` JWTs) applied to the string arm too, and restate the
guarantee precisely: *NyxID never sends its own injected credentials; downstream-returned secret-shaped
strings are best-effort redacted.*

## P2 — MAJOR (fix in flight): `nyx_search_skills` reports Ornn `not_connected` when it was never queried

```rust
let mut ornn_status = "not_connected";
let remaining = requested_limit.saturating_sub(matches.len());
if remaining > 0 && let Some(service) = registry.ornn_service() { ornn_status = "failed"; … }
```

When `remaining == 0` the branch is skipped and the status stays `"not_connected"` regardless of whether an
executable `ornn-api` service exists. `remaining` reaches 0 easily because `bundled_skill_matches`
substring-matches the **entire skill body** (`skill.body.to_ascii_lowercase().contains(&query)`) across
38 KB of bundled prose (16,434 / 11,314 / 10,492 bytes), so a common query — `"api"`, `"nyxid"`, `"service"` —
fills all three slots; with `limit: 3` Ornn is never contacted and is reported disconnected. The model then
states a false live-state fact, which no prompt rule can catch because NyxID's own tool produced it.
Fix: distinguish `"not_queried"` from `"not_connected"`, and match slug/label before falling back to body.

## P3 — MINOR (fix in flight): skill truncation is deliberate but not self-describing

`skill_content_is_capped_without_changing_full_content_digest` pins the choice: `content_sha256` is the digest
of the **full** body while the delivered `content` is truncated at `MAX_SKILL_CONTENT_BYTES = 8 KiB`. All
three bundled bodies exceed that cap, so `nyx_get_skill{bundled}` always returns a partial skill — `nyxid.md`
loses roughly half — while prompt rule 3 tells the model "if neither an injected skill nor the connected
operation catalog documents an endpoint, you do not know it". Raising the constant is not a safe one-liner:
the envelope truncation in `from_response` replaces the structured body with a flat `Value::String`, which
would destroy the provenance fields. Fix: keep the cap, add `content_bytes_total`,
`content_bytes_delivered`, and `delivered_sha256` so the delivered bytes are independently verifiable.

## P10 — MAJOR (fix in flight): the audit trail has no skill identity

`tool_audit_data` carries `ornn_skill_version` and nothing else about the skill — no source, no id, no digest
(`ToolCompletion.skill_version: Option<&'a str>` is the only channel). The model context now has full
provenance while the audit — the only durable artifact of a deliberately ephemeral run — cannot answer "which
skill text did this run consume?". Fix: widen `ToolCompletion` to carry `SkillProvenance { source, id,
version, content_sha256, content_bytes_delivered }`; `skill_document` already computes every value.

## P11 — MAJOR: nothing drives `execute()`; phase-boundary tests remain helper-level

`run.execute()` is called from exactly one place — `run()` — and no test calls it. There is no scripted stub
upstream anywhere in the POC (no `TcpListener` / `axum::serve` / `Router::new` in either POC file), unlike the
sibling `handlers/assistant_direct.rs`, which has one. `plan_phase_tool_call_is_rejected_before_any_tool_dispatch`
calls `accept_plan_hop` directly — a pure function that could not dispatch even if the boundary were broken —
so its `tool_dispatches == 0` assertion is tautological.

**Consequence.** The suite would stay green if `execute()` were rewired to call `execute_tool_batch` from the
plan branch again, because no test observes the run loop's real sequencing. The brief's criterion ("tests
prove actual dispatch and phase boundaries, not only helper behavior") is the one axis still unmet.

**Fix.** One scripted-upstream integration test in the pattern of `handlers/assistant_direct.rs:246-343`:
(1) a plan hop emitting a tool call ⇒ run terminates, `tool_dispatches == 0`; (2) plan → execute → report
happy path ⇒ exact frame order and exactly one `done`. `TestDispatchObserver` already exists and is threaded
through both dispatch sites; it just needs a caller that exercises the loop.

## P6 — MINOR: the Report-phase asymmetry moved rather than closed

Undeclared tool calls are now fatal in the Plan hop *and* in the **natural** Report hop
(`if !final_hop.tool_calls.is_empty() { return Err(RunError::Upstream) }`), but remain benign in the
**forced** Report hop, which still calls `append_skipped_tool_messages` and breaks into a normal `done`. Both
are `AgentPhase::Final` + `ToolMode::Disabled` — identical stimulus, identical phase, opposite terminality,
decided only by which branch produced them. Pick one; if the forced path is deliberately lenient so a
budget-exhausted run still yields an answer, say so in a comment, because the natural path's `Err` now reads
as the intended rule.

## Remaining MINORs

- **P12′** — the frame-level leak test still hand-feeds `result_preview: "{\"executed\":false}"`. The
  redaction property is properly pinned in `tools.rs` now, but the frame test cannot detect a future rewiring
  of `ToolCompleted.result_preview` to a non-scrubbed source. Build that frame's preview from a real
  `ModelToolResult` so the wiring, not just the helper, is covered.
- **P13** — a truncated plan maps to `RunError::Upstream`, whose fixed message is "The Chrono stream failed."
  Chrono did not fail. The `error.code` enum is closed, so keep the code and fix the message
  ("The plan phase ended before a usable plan.").
- **P14** — `REPORT_CONTEXT_RESERVE_BYTES` reserves *request* bytes; model output does not consume them. What
  it actually buys is headroom for exchanges appended between one compaction and the next Report body — sound,
  but worth naming accurately. Sizing: one maximal exchange is a 16 KiB envelope plus JSON re-escaping
  (tool content is a JSON string containing serialized JSON, so quote-dense bodies roughly double) ≈ 33 KiB,
  so 64 KiB covers about two. Rename to `POST_COMPACTION_HEADROOM_BYTES` or document the derivation.
- **Fixture wire fidelity** — both happy-path fixtures lost their trailing blank line, so they end
  `data: [DONE]\n` while `send_done()` emits `data: [DONE]\n\n`. The terminal marker is now exercised only
  through `flushSseBuffer` (EOF path), not `drainSseBuffer` (mid-stream path). Restore the blank line on at
  least one fixture so both paths stay covered and the fixture matches the wire.
- **FE test drift** — `frontend/src/lib/assistant/direct-agent-poc.test.ts:302` still asserts
  `"tool":"ornn_get_skill"`, a name the backend can no longer emit. The FE schema accepts any string so it
  passes, but it documents a dead contract. (The `ornn_search_skills` references in
  `aevatar-transport.test.ts` are the Aevatar engine's own labels — leave them alone.)
- **N1** — `result_bytes` is the pre-truncation size while `truncated: true`; defensible, undocumented.
- **N2** — `find_string_field` recurses into every nested object looking for any key named `version`, so a
  nested dependency version can win. Scope it to the top-level document or to the object that produced
  `SKILL.md`.
- **N5** — platform discovery admits any active http row with `requires_user_credential:false` and category
  ≠ `provider` (`mcp_service.rs:720-782`) — no visibility or `internal` exclusion — so an internal platform
  row publishing typed GETs is callable on the *platform's* credential. Inherited from the MCP catalog and
  partly gated by PR-A's `authorize_master_credential`, but the POC hands the reach to a model rather than to
  a configured agent. Needs an owner decision; a `service_category == "internal"` filter in `ReadOnlyRegistry`
  is three lines.

---

# Watch-list (unchanged, still not present)

- **W1 — Aevatar fallback.** Extension §11/E5 proposes fixed descriptors resolved against a *UserManaged*
  `aevatar` row. The brief forbids a guessed/fixed/UserManaged fallback; Sol's B3 agrees. Still absent —
  BLOCKER on arrival. Canonical `Platform` catalog only.
- **W2 — mutation allowlist.** A `(slug, endpoint_name)` tuple does not bind the operation that executes
  (`logical_operation_name` is `slug__endpoint.name`, and endpoint rows are re-materialized by spec sync). Any
  write must key on `endpoint_id` + method + path + contract digest, re-checked against the freshly loaded
  catalog immediately before dispatch. Ships empty until then.
- **W3 — single-predicate discipline.** `is_poc_operation_eligible` guards both advertise time and execute
  time. If E1's rename splits that into two functions, the strongest wall in the design is gone.
- **W4 — account tools.** Watch for raw model structs in results (`Node` carries token material) and for
  `is_connected` derived from persisted status rather than `NodeWsManager`.
- **W5 — budget increases** (8→12 / 8→10) are now safe to land: compaction exists.

---

# Build state / gates

Backend stale references are cleared. Remaining: the FE test's dead tool name (above). Gates to run before
green: `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D warnings`;
`cargo nextest run -p nyxid --profile ci` (needs replica-set Mongo + `NYXID_TEST_DATABASE_URL`);
`cargo test -p nyxid billing_route_coverage_smoke`; `python3 scripts/check-backend-docker-embeds.py`;
`cd frontend && npm run lint && npm run test && npm run build`. The FE test's new
`@testing-library/react` import is already a devDependency (`frontend/package.json:75`) and neither
`package.json` nor the lockfile is modified, so the Wizard Bundle Freshness check is not implicated.

---

# Confirmed invariants (attacked, survived; re-verified where the diff touched them)

1. **The loop executes typed tools in the authenticated user's context.** `execute_endpoint` →
   `mcp_service::execute_tool` with `McpExecContext{api_key_id: None, allow_all_nodes: true}` and the session
   user as both actor and billing principal. Credentials are resolved inside `execute_tool`, after model
   context is built — "Chrono never sees NyxID-injected credentials" is structural, not prompt-based.
2. **No arbitrary URL/path/method.** Method and path come from the typed `McpToolEndpoint`;
   `is_poc_operation_eligible` rejects the generic-proxy endpoint and requires
   `derive_verb_from_method(...) == ApprovalVerb::Read`, and the *same function* guards advertise time and
   execute time. Arguments are twice validated — closed key sets in `validate_tool_arguments` plus the typed
   schema in `validate_operation_arguments`, whose keyword allowlist **fails closed** on any assertion it
   cannot enforce (`pattern`, `const`, `nullable`, union `type` arrays), with `format` documented as
   annotation-only. Ornn remains two server-built fixed descriptors; the model supplies no path.
3. **Ornn fetch requires an in-run observation.** `observed_ornn_skill_ids` is populated only from a projected
   search result, enforced before fetch, and the one-fetch cap sits after that check. Blind GUID probing is
   closed, and the hardcoded prod GUID is gone.
4. **Deny rules target the catalog identity.** `enforce_deny_only` routes through
   `mcp_approval::approval_target_for_tool` before `evaluate_deny_only` — the defect I raised against the v0
   plan is fixed in shipped code and untouched by the diff.
5. **Session-only auth before any work.** The `AuthMethod::Session` check precedes permit acquisition and body
   read, with a test asserting an access token is rejected before a permit is taken. The route sits in the
   human-only router, so API-key / SA / delegated / relay callers are rejected by the mount's layers.
6. **CSRF posture intact.** Session cookies are `SameSite=Lax` (`handlers/auth.rs:217-228`), so a cross-site
   POST carries no cookie, and the JSON content type forces a preflight. The diff adds no cookie, origin, or
   bearer fallback.
7. **Billing classification is route-attested, not self-attested.** The hop request copies the policy the
   mounted route attached ("copied, never reconstructed or self-attested"), the route is in
   `BILLING_ROUTE_INVENTORY` (`route_inventory.rs:148`), and it is exercised end-to-end in
   `billing_route_coverage_smoke` (`billing_integration_tests.rs:376-417`). Untouched.
8. **Ephemerality holds.** Run state lives only in `RunContext`; the diff adds `observed_ornn_skill_ids` to
   that same struct, so it dies with the request. No collections, no persistence, metadata-only audit.
9. **Budget arithmetic is off-by-one-free, including the new always-separate Report hop.** Execute hops run
   only while `llm_calls ≤ max−2` (`should_force_final` fires at `llm_calls + 1 >= max`), so the Report hop
   always finds `llm_calls < max` at `chrono_hop`'s guard. Maximum total upstream calls = `MAX_LLM_CALLS`
   on both the natural and forced paths.
10. **Cancellation is properly threaded.** The handler captures `request_cancellation` and clones
    `ClientConnectionCancellation` into every synthesized hop; in-flight tools settle to `outcome_uncertain`
    before any terminal frame, asserted by `assert_started_tools_settle_before_terminal`.
11. **Transport decoding is bounded and protocol-strict.** `MAX_SSE_EVENT_BYTES = 256 KiB`,
    `MAX_HOP_DECODED_BYTES` tied to the request cap, `finish_reason` + usage + `[DONE]` all required at EOF,
    duplicate `[DONE]` and post-terminal frames rejected. Tool-call reassembly correctly ignores empty-string
    `name` continuation fragments — the exact upstream quirk pinned by the v0 probes.
12. **The frontend settles every terminal path.** `settle()` closes both text blocks, maps every
    `active`/`waiting` step to `done`/`failed`/`skipped`, completes the run block, and emits `turn.completed`
    — reached from HTTP error, missing body, truncated stream, timeout, network error, protocol error, and
    cancel. The "forever-spinning step" failure mode in the older `direct-transport.ts` does not exist here.
13. **Frame contract is fail-loud on drift.** Unknown frame *types* are ignored (forward-compatible) while a
    known type with an unknown *field* calls `protocolError` and fails the turn. That asymmetry is the right
    default and is why the `result_preview` lockstep mattered; it should be stated in the frame contract so
    the next field addition is not attempted piecemeal.

---

# Final fix priority

| # | Item | Cost |
| --- | --- | --- |
| 1 | **L1** — select the first Ornn id candidate that *canonicalizes*, not the first that exists; add the `{"id","guid"}` row to the test | XS |
| 2 | **L2** — drop the scalar `creator` passthrough; assert no `@` in the projected row | XS |
| 3 | **M6** (in flight) — value-side redaction now that previews reach the browser; restate the guarantee | S |
| 4 | **P2** (in flight) — `not_queried` vs `not_connected`; match slug/label before body | XS |
| 5 | **P10** (in flight) — thread `SkillProvenance` into the audit | XS |
| 6 | **P11** — one scripted-upstream `execute()` test proving plan-phase zero-dispatch and frame order | S |
| 7 | **P3** (in flight) — self-describing skill truncation (`delivered_sha256`, byte counts) | XS |
| 8 | **P6** — unify forced vs natural Report handling of undeclared tool calls, or comment the split | XS |
| 9 | Fixture trailing newline; FE test dead tool name; P12′ frame-level preview wiring | XS |
| 10 | **P13, P14, N1, N2** — message/naming/reporting nits | S |
| 11 | **N5** — owner decision on internal platform rows in the model's reach | S |

Items 1–2 are the only ones I would gate the demo on; both are two-line changes with live evidence behind
them. Everything else is either in flight, test-depth, or an owner decision.
