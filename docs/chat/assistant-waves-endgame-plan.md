# Assistant Waves — Endgame Plan

- **Date:** 2026-08-24 · **Branch:** `feat/assistant-waves` @ `4b15bda5` (PR #1471) · **Supersedes:** `assistant-waves-completion-plan.md` §1–§3, §9 (its §4 Q1–Q3, §7 cuts and §8 v9 scope stand except where §5 below says otherwise)
- **Verified at HEAD before writing:** `action-card.tsx` (1,135 lines; 2 explicit dialog mounts + 2 `AddKeyDialog` paths); `action-registry.ts` (4 rows, 6 `ActionCardParams` variants incl. `unknown`); 19 param Zod schemas (W1 + all 12 W2; zero W4); 33 dialog components on disk; 54 manifest verbs; handler tests keys 15 / services 15 / endpoints 12 / nodes 9 / org 6; `action-card.test.tsx` 34 tests mocking the key dialogs **by module path**.
- `/tmp/fable-audit.md` no longer exists (tmp wiped). F-numbers below follow the completion plan's §5 and the brief; nothing here depends on the lost text.

---

## 0. Two corrections and one new finding

1. **My "~20–24 variants cover 39 verbs" was wrong.** I counted dialog *shapes*. The variant is the verb's normalized-params shape — it carries the per-verb Zod schema, the per-verb summary chips, and the per-verb report resource — so it is one per verb regardless of how many dialogs exist. Parameterisation changes the **dialog count** (11 components serve 27 W4 verbs), not the variant count. See §2.
2. **Parameterisation does not change the binding shape** — it adds one constraint: the shared dialog's `params` prop must be typed per `action`, not `Record<string, unknown>`. That is forced by the next item.
3. **F10 (new, blocks W4 wiring):** `assistant-org-action-dialog.tsx:182`, `assistant-service-account-action-dialog.tsx:131`, `assistant-developer-app-action-dialog.tsx:120` build the effect body as `{ ...params, actionRequestId }` from the raw model-supplied params object. `assertNoSensitiveActionParams` screens values and the org handlers are `deny_unknown_fields` (24 sites), so an injected *unknown* key fails closed — but any *known* body field the dialog does not itself set (`contactEmail`, `avatarUrl`, `slug`, `role` on verbs that don't override it, `ttlHours`, …) flows from the model to the mutation untouched. The fix is free once wiring is table-driven: the registry's `normalize` produces a typed allowlisted variant and the dialog receives only that. Not a production issue today (dormant), but it must land **before** any W4 verb is wired, so it is folded into E4a, not filed separately.

---

## 1. Ordered task list to done

| # | Task | Owner | Files (exclusive while open) | Unblocks | Review gate |
|---|---|---|---|---|---|
| **E0** | **Harness refactor** (§2): descriptor extension, `ACTION_DIALOGS` table, generic mount, migrate `key_create`/`key_rotate`; legacy connect/reauthorize branches untouched | PM, now (disjoint from the in-flight org stream) | `blocks/action-card.tsx`, new `blocks/action-dialogs.tsx`, `lib/assistant/action-registry.ts`, new `action-registry.test.ts` | E2, E4 | 34 card tests + `mock-data`/`scenario-engine`/`wire-replay` tests pass **unmodified**; `npm run build` |
| **E1** | Review + land the in-flight org stream (F7, F9c, 30-verb coverage) | PM review | — | E4a | Every new test names its falsifier; F9c test fails if the factor pin is removed; F7 test fails on byte-`len` |
| **E2** | **Wire Wave 2** — 12 variants, 12 registry rows, 12 dialog bindings, 12 journey tests | Dispatched, single writer, starts after E0 merges | the 3 registry files + `blocks/action-card.wiring.test.tsx` (new; the 34 existing tests stay untouched) + `schemas/assistant-actions.ts` (union only — schemas exist) | E3, E5a, E6 | Journey test per verb asserts dialog props **and** the report resource; registry-completeness test green |
| **E3** | **W2 descriptor freeze pass** — the 12 `params_schema` bodies become byte-pinned by `DeepEquals` at v9; last chance to amend wording/shape | PM, 1 h, during E2 | `handlers/assistant_actions.rs` (descriptor text only) | E6 | Diff reviewed against the 12 dialogs' `paramsSchema`s — the two must agree field-for-field |
| **E4a** | **Wire Wave 4 part 1** — org (8), service_account (5), developer_app (4), notifications (3), `external_key.add_gcp_service_account` (1) = 21 verbs through the 5 parameterised dialogs; **F10** typed `params` per `action`; 21 W4 Zod schemas | Dispatched, single writer, after E1 *and* E2 merge | 3 registry files + the 5 parameterised dialogs + their tests + wiring test | E5b, v10 | Each dialog's body is built from typed fields only (grep: no `...params` spread survives); journey tests |
| **E4b** | **Wire Wave 4 part 2** — account `profile_update`/`revoke_consent` (2), approval configure/enable/disable/revoke_grant (4) = 6 verbs | Same agent as E4a, second PR | 3 registry files + 6 dialog tests + wiring test | v10 | As E4a |
| **E5a** | Consolidated module sweep on a wiped dbpath (all five handler modules + receipts + manifest + frontend assistant surface); record numbers; PR #1471 body rewritten against HEAD | PM, after E2 | PR body, `docs/chat/*` | **Merge #1471** | Numbers in the PR body are reproducible by the commands listed |
| **E6** | **v9 bump = Wave 2** (§4): `PINNED_ACTIONS_BY_REVISION` entry, `ASSISTANT_ACTIONS_REVISION`, golden fixture, dormancy test flipped; file the upstream asks | PM | `handlers/assistant_actions.rs`, fixture | production reachability | §4 gates |
| **E5b** | Second sweep + body refresh after E4b | PM | — | v10 candidate | — |
| **E7** | v10 = Wave 4 curated (27) — bump only after E4b review closes and upstream pins | PM, later | as E6 | — | §4 gates, re-run |

**Hard orderings:** E0 → E2 → E4a → E4b are strictly serial (same three registry files, one index). E1 → E4a (the org stream owns the W4 dialogs until it lands). E3 ∥ E2 (different files). E5a → merge → E6 (bump after merge, from main, so the revision change is a one-line reviewable PR rather than line 60 of a rollup).

### Concurrency schedule (two agent slots, serial PM review)

| Session | Agent A | Agent B | PM |
|---|---|---|---|
| 1 (now) | org stream (in flight) | idle | **E0** build + self-review |
| 2 | fix-ups from E1 review, then done | **E2** | E1 review; merge E0 |
| 3 | idle | E2 fix-ups | E2 review; **E3**; E5a sweep; merge #1471 |
| 4 | **E4a** | idle | **E6** v9 PR + upstream asks; E4a review when it lands |
| 5 | E4b | idle | E4a/E4b review; E5b |

Agent capacity is never the constraint after session 2; every session has at most one PR waiting on the PM plus one agent building the next. That is the shape the bottleneck dictates: **never two PRs queued for review at once.**

---

## 2. Wiring design, concretely

### Variant accounting

| Family | Verbs | Dialogs | New variants | Notes |
|---|---|---|---|---|
| W1 (shipped) | 4 | 2 + `AddKeyDialog` ×2 paths | 0 (5 exist + `unknown`) | connect/reauthorize keep their hand-written branches (popup + watch + scope check) |
| W2 | 12 | 12, typed props | 12 | Zod schemas already exported |
| W4 curated | 27 | 11 (5 parameterised + 2 account + 4 approval, enable/disable being thin wrappers over `toggle`) | 27 | 27 Zod schemas to write; F10 |
| Cut (§5) | 11 | 0 / 2 | 0 | explicit `deferred` rows so the completeness test still covers 54 |

**Total after E4b: 45 variants (44 + `unknown`), 39 dialog bindings, 54 registry rows.** Parameterisation means 21 of the 27 W4 bindings differ only in the `action` literal they pass.

### Descriptor extension (`lib/assistant/action-registry.ts`)

This module is imported by `aevatar-transport.ts`, `wire-replay.ts`, `scenario-engine.ts`, `mock-data.ts` — transport-layer code. **It must not import React components.** The component binding lives in a second, React-side table keyed by the same variant names.

```ts
export type ActionIcon = "service" | "globe" | "shield" | "key" | "org" | "bell" | "app" | "node";
export interface SummaryRow { readonly label: string; readonly value: string; readonly mono?: boolean }

export interface ActionDescriptor<P extends ActionCardParams = ActionCardParams> {
  readonly title: (p: P) => string;
  readonly body: (p: P) => string;
  readonly cta: (p: P) => string;
  readonly risk: ActionRisk;
  /** raw model params → typed variant, or null → `unknown`. Replaces normalizeParams' if-chain. */
  readonly normalize: (raw: unknown) => P | null;
  /** Replaces the ParameterSummary JSX branches; card renders rows generically. */
  readonly summary: (p: P) => readonly SummaryRow[];
  readonly icon: ActionIcon;                       // resolved to lucide inside the card
  readonly busyLabel: "Working" | "Authorizing" | "Connecting";
  readonly assurance: string;                      // the ShieldCheck sentence
  /** Builds the completion report resource from the dialog's onComplete id. */
  readonly resource: (id: string) => ActionResource;
  readonly wiring: "dialog" | "legacy_connect" | "legacy_reauthorize" | "deferred";
  readonly journey: (p: ActionCardParams) => ActionJourney;   // unchanged; `supported` still derives from it
}
```

`ActionJourney` widens from the 5 literals to `ActionCardParams["variant"] | null`. `resolveAssistantAction` becomes `registry[action]?.normalize(request.params) ?? {variant:"unknown"}` — one lookup, no per-verb branches.

### Dialog table (`components/assistant/blocks/action-dialogs.tsx`)

```ts
type DialogVariant = Exclude<ActionCardParams["variant"], "catalog" | "custom" | "service_reauthorize" | "unknown">;
type ParamsOf<V extends DialogVariant> = Extract<ActionCardParams, { variant: V }>;

interface DialogBinding<V extends DialogVariant> {
  readonly Dialog: ComponentType<AssistantDialogProps<unknown>>;   // open / onOpenChange / actionRequestId / params / onComplete(id)
  readonly toProps: (p: ParamsOf<V>) => unknown;                    // typed per row via the mapped type below
}
export const ACTION_DIALOGS: { readonly [V in DialogVariant]: DialogBinding<V> } = {
  key_create: { Dialog: AssistantKeyCreateDialog, toProps: (p) => ({ name: p.name, platform: p.platform, allowedServiceIds: p.allowed_service_ids }) },
  key_rotate: { Dialog: AssistantKeyRotateDialog, toProps: (p) => ({ keyId: p.key_id }) },
  // W2 rows (E2) …
  org_member_update_role: orgBinding("member_update_role"),          // W4 (E4a): helper returns { Dialog: AssistantOrgActionDialog, toProps: p => ({ action, params: p }) }
  …
};
```

The mapped type over `DialogVariant` makes the table **exhaustive** (a variant without a row is a compile error) and **correctly paired** (`toProps` receives exactly its variant). The card's mount collapses to:

```tsx
{!verdict && !unsupported && isDialogVariant(params.variant) ? (() => {
  const b = ACTION_DIALOGS[params.variant];
  return <b.Dialog open={dialogOpen} onOpenChange={setOpen} actionRequestId={block.action_request_id}
                   params={b.toProps(params)} onComplete={(id) => report("completed", descriptor.resource(id))} />;
})() : null}
```

For F10 the five parameterised dialogs change `params: Record<string, unknown>` → `params: OrgActionParams[A]` (a per-action type derived from the new Zod schemas) and build the body from named fields; the `{ ...params }` spread is deleted.

### Migration order (E0 is one PR, two commits)

1. **Commit 1 — data, no structure:** add `normalize`/`summary`/`icon`/`busyLabel`/`assurance`/`resource`/`wiring` to the 4 existing descriptors; card reads `descriptor.summary()` etc. instead of its `params.variant` ladders (`ParameterSummary`, icon ternary, busy label, assurance sentence). No dialog change. Run the 34 tests.
2. **Commit 2 — mount:** add `action-dialogs.tsx` with the two key rows; replace the two explicit `<AssistantKey*Dialog>` mounts with the generic mount. Leave the two `AddKeyDialog` branches exactly as they are. Run the 34 tests.
3. Add `action-registry.test.ts`: `registry_covers_every_manifest_verb` — iterates a checked-in 54-name fixture (`lib/assistant/__fixtures__/assistant-actions-manifest.json`, add it; none exists) and asserts each name resolves to a `dialog` row with an `ACTION_DIALOGS` entry, a `legacy_*` row, or an explicit `deferred` row. Fails when a verb is added without a row, when a row loses its binding, and when a dormant verb is silently wired.

### Why the existing vitest suite proves behaviour preservation

`action-card.test.tsx` mocks `@/components/assistant/assistant-key-create-dialog`, `…-key-rotate-dialog`, and `@/components/dashboard/add-key-dialog` **by module path** (`:81`, `:180`, `:211`), and its assertions read the mock's rendered `data-*` attributes (`actionRequestId`, `name`, `platform`, `service-ids`, `key-id`) and the report emitted by `onComplete`. Because `action-dialogs.tsx` imports from those same paths, the mocks bind through the table unchanged. So: if the generic mount passes the wrong props, wrong `actionRequestId`, or builds the wrong report resource, tests `:720` and `:754` fail; if the copy/summary/icon ladders regress, `:792`, `:933`, `:978`, `:1126`, `:1159` fail; the reauthorize/watch tests (`:408`–`:674`, `:1316`–`:1607`) prove the legacy branches are untouched. **Gate: zero edits to that file.** The transport-side callers (`mock-data.test.ts`, `scenario-engine.test.ts`, `wire-replay.test.ts`) cover `resolveAssistantAction`'s new lookup shape. `npm run build` (tsc -b, `noUncheckedIndexedAccess`) is the type gate, not `tsc --noEmit`.

### Journey test shape (E2/E4, new file, one per verb)

Synthesize the action envelope → `resolveAssistantAction` yields the expected variant → render `ActionCard` with the dialog module mocked by path → assert the mock received `actionRequestId` + every semantic field → click the mock's finish → assert the `action.continue` report carries the right resource variant (`endpoint.endpointId`, `externalKey.externalKeyId`, `serviceAccount.serviceAccountId`, …). Falsifier: delete the registry row (card renders unsupported), break `toProps` (props assertion), or change `resource` (report assertion). The dialogs' own request-body tests (`06b6c6a1` for W2) stay where they are; journey tests do not re-test the POST.

---

## 3. Effort, recalibrated

Measured: one dispatched stream ≈ one working session for several fixes + ~11 falsifiable tests across up to three modules, clean on `fmt`/warnings; the PM reviews ≈ two such PRs per session. Previous plan assumed 2.5–3 agent-days per stream — **3× too pessimistic on build, about right on review.**

| Task | Build | PM review |
|---|---|---|
| E0 harness | 0.5–1 session (PM) | self + 1 h |
| E1 org stream | in flight | 2 h |
| E2 wire W2 | 1 session | 1.5 h |
| E3 freeze pass | 1 h (PM) | — |
| E4a wire W4 pt 1 + F10 | 1–1.5 sessions | 2.5 h |
| E4b wire W4 pt 2 | 0.5 session | 1 h |
| E5a/E5b sweeps + body | 0.5 session each (PM) | — |
| E6 v9 bump + asks | 0.5 session (PM) | 0.5 h |

**Totals: ~3 agent-sessions, ~3 PM-sessions, ~8.5 h serial review. Calendar: v9-ready at the end of session 3–4 (one working week); everything in scope including W4 wiring by session 5–6.** Down from the previous 2.5–3 weeks because the build side was over-estimated and the W4 residue is no longer greenfield. The one thing that can blow this up is the same as before: an agent brief dense in credential terms (E4a's service-account/developer-app secret-display paths). Mitigation unchanged — write those briefs in terms of `evidence-projection-conventions.md`, and if E4a's agent dies on those two dialogs, the PM finishes them (≈ 2 h) rather than re-dispatching.

---

## 4. v9, revisited

**Yes — bump as soon as E2 merges, and only to Wave 2.** Everything that previously gated it on our side is now closed: F3/F6/F8/F9b on services/endpoints, F-class on keys, journeys about to be wired with per-verb tests. What still gates the bump, beyond upstream acceptance:

1. **E2 merged with the journey tests green** — a pinned verb with no card behind it is a card Aevatar renders that NyxID declines as unsupported. That is the only *new* state v9 creates, so it is the one hard gate.
2. **E3 freeze pass** — after v9, the 12 `params_schema` bodies are `DeepEquals`-pinned forever; a mismatch between a descriptor schema and its dialog's `paramsSchema` today is a cosmetic bug, after v9 it is a permanently served contract. One hour, do it before the bump.
3. **Negotiation suite extended** — `PINNED_ACTIONS_BY_REVISION` gets the v9 tuple; golden fixture; the dormancy test flips to "the 12 W2 names are in v9's set and the other 38 are not" (not "everything beyond W1 is dormant"). Fails if a W4 name leaks into the set.
4. **Registry-completeness test agrees** — every v9-pinned name resolves to a `dialog` row, none to `deferred`. Couples the backend pin to the frontend table at test time.
5. **Upstream, in order of value, none blocking our PR:** (a) `SupportedActions` parser entries for the 11 new names + v9 pin-set entry — without this the bump is inert, not harmful; (b) the `?revision=` startup fetch — without it, deploy choreography (NyxID serves v9 composition before Aevatar pins it, which `compose_revision_manifest` already handles); (c) adopt `/authorization` evidence reads — affects shipped W1 today, highest value, *not* a v9 gate; (d) aevatar#3496 disposition.

Not a gate: W4 review state, F7/F9c, the `?mock=1` e2e spec. Bump from `main` after #1471 merges (E5a) so the revision PR is one line plus fixture and reviewable in minutes.

---

## 5. Cuts and restores

**Keep every §7 cut** (W3 journeys, `openclaw.connect`, `account.delete`/`account.mfa_setup` wiring, no new `?mock=1` specs, no test matrices, no org-file split). Re-checked: W3 still has zero dialogs; the two account verbs still have no product case for a chat surface. They get explicit `deferred` registry rows so the completeness test counts them.

**New cut — stop treating W4 wiring as part of "done" for #1471.** Merge after E2 + E5a (hardened backends for all five modules, W2 wired, W4 dialogs dormant and unwired — the same state W2 sat in for weeks, with F10 still on the dormant side). Reasons: the branch is ~60 commits and every session it stays open adds rebase risk and review surface; v9 needs none of W4; E4a is the one tranche with real attrition risk and should not hold W2 hostage. E4a/E4b then land as a fresh PR from `main` targeting v10.

**New cut — drop the "two PRs per tranche" idea for E2.** 12 rows + 12 bindings + 12 tests is one reviewable unit once the harness exists; splitting it only adds a review round-trip, which is the scarce resource.

**Restore — nothing from the previous cuts.** The one thing I considered restoring is `account.mfa_setup` (recovery codes are one-time material but the dialog exists and is tested); still no — one-time material display in a chat card has no precedent on this programme and the settings page already does it with proper friction.

**Not a cut, a tightening:** any test added in E2/E4 that passes with its registry row deleted is rejected in review, same rule as before. The wiring tranches are where coverage padding is most tempting (39 near-identical tests); the falsifier requirement is what keeps them honest.

---

## 6. Definition of done for the programme

- #1471 merged at: all five effect modules hardened with named-falsifier tests; F1–F9 closed (F7/F9c via E1); harness table-driven; W2 wired with journey tests; registry-completeness test pinning all 54 names; PR body reproducible.
- v9 PR merged from `main`; upstream asks (a)–(d) filed with the exact names and pin-set.
- Follow-up PR: E4a (incl. F10) + E4b, v10 candidate.
- Never done, by design: W3 journeys, `openclaw.connect`, `account.delete`/`mfa_setup` wiring — until someone brings demand evidence.
