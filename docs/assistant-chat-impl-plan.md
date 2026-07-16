# AI Assistant Chat v1 — Implementation Plan (mock-data pass)

- **Status:** v2 — CONVERGED (PM: Fable · Dev lead/review round 1: GPT-5.6-Sol, verdict ITERATE → amendments folded in below). This version is authoritative for implementation.
- **Branch:** `ai-assistant-marketplace-pivot` (main merged 2026-07-16, includes feature-flag system #1171). Baseline verified green: `npm run build` ✓, 1717 frontend tests ✓.
- **References (binding):**
  - `docs/assistant-chat-prd.md` v7 — block schemas §5.5, stream events §5.6, rendering rules §5.7. Mock data must be shaped **exactly** like §5.5 (required-nullable fields included) so the later API pass is a transport swap.
  - `mockups/nyxid-assistant-shell.html` — visual reference (chat view only for this pass).
  - `DESIGN.md` + `frontend/src/app.css` — app.css tokens win where DESIGN.md is stale.

## 1. Goal & scope

Ship the assistant **chat page** into the real frontend, gated behind the `experimental:ai-assistant` feature flag, powered entirely by client-side mock data behind a real transport interface.

**In scope**
- Feature flag `experimental:ai-assistant` end to end (backend registry + frontend catalog + two-layer route guard + nav gating, desktop **and** mobile nav).
- Assistant shell at `/assistant`: top bar (logo, `assistant / {chat title}` crumb, theme toggle, profile), assistant-own left sidebar (New chat, Chats list, Studio link back to `/dashboard`, profile footer). Responsive: off-canvas sidebar / mobile takeover below `md`, safe-area composer padding, card wrapping, top-bar truncation.
- Chat history: mock conversation list, switching, active highlight. **Known PRD divergence, accepted by PM:** PRD v1 floor is one auto-created conversation (§5.0); multi-conversation list/create is v1.1 (§9.6). Calvin requires chat history, so the mock implements the v1.1 surface; the API pass either lands C1 v1.1 list/create or scopes down. Documented here so nobody "fixes" it silently.
- Transcript renderer for the 5 PRD block types (`text`, `connect_card`, `run`, `approval_card`, `artifact`) from final-state mock history, plus an **unsupported-content shell** for unknown block types / newer schema versions (§5.0 versioning rule).
- Composer: send appends the user message and plays a scripted mock turn with progressive text streaming; `turn.status` drives composer/stop state; **Stop actually cancels** (see §5 cancellation). No model selector (PRD R1); keep the disclaimer line.
- Local interactivity: Approve/Deny on the approval card (decision-branched, see §5); other card buttons are visual no-ops with "wired in the API pass" tooltips.

**Out of scope (later passes)**
- Run inspector, Plugins/marketplace page, Artifacts library page, Aevatar/API integration, real streaming transport, approvals wiring, admin-shell banner, admin feature-flag write surface. Plugins will later map onto AI services (`/keys` catalog); skills sourcing TBD with Calvin.
- Persistence: mock state resets on reload. Acceptable.

## 2. Feature flag

- **Backend** `backend/src/services/feature_flag_service.rs`: add to the production `FEATURE_FLAGS` registry:
  ```rust
  FeatureFlagDef {
      key: "experimental:ai-assistant",
      description: "AI Assistant chat surface (mock-data preview).",
      default_enabled: false,
      org_manageable: true,
  }
  ```
  **Test registry requirement:** `find_flag("experimental:ai-assistant")` must also resolve under `cfg(test)` — either duplicate the entry alongside `example_ui` in the test registry, or restructure so production flags are shared and `example_ui` is appended for tests. Sol-A picks the cleaner mechanical option; backend tests must be able to look up the shipped key (`feature_flag_service.rs:48`).
- **Rollout reality (decided):** personal-flag resolution applies default + global + platform-user overrides only (`feature_flag_service.rs:193`); the only writable override routes today are org-scoped (`routes.rs:751`) and the admin UI is local mock state. **v1 enablement paths: (a) dev/demo = `?mock` mode; (b) production = manual `feature_flag_overrides` row (global or per-user) inserted by an operator — runtime toggle, no deploy.** Document this in the PR description. An admin write surface for global/user overrides is a separate follow-up track, not this PR.
- **Frontend** `frontend/src/lib/feature-flags.ts`: `AI_ASSISTANT: "experimental:ai-assistant"`.
- **Type-narrowing fallout:** `FeatureFlag` narrows from `string` to the exact union. `src/hooks/use-feature-flag.test.tsx` has **five** `"example_ui"` usages plus fixture arrays (`use-feature-flag.test.tsx:47,94,…`) — migrate them all coherently (use the real key or a typed cast at the fixture boundary). Audit any other `useFeature`/`<Feature>` call sites and `admin-feature-flags.tsx` for literals that stop compiling.
- **Mock mode:** `frontend/src/lib/mock-data.ts` — add `enabled_features: ["experimental:ai-assistant"]` to `MOCK_USER.capabilities`.

## 3. Architecture & file map

Route lives **outside `DashboardLayout`** as a child of `rootRoute` (pattern: `sshTerminalRoute`, `router.tsx:244-254`) — full `h-dvh` layout, own scroll management.

```
# Sol-A — platform integration
backend/src/services/feature_flag_service.rs        # registry entry + test-registry visibility (§2)
frontend/src/lib/feature-flags.ts                   # AI_ASSISTANT key
frontend/src/hooks/use-feature-flag.test.tsx        # narrowing migration (all 5 usages + fixtures)
frontend/src/lib/mock-data.ts                       # MOCK_USER.enabled_features
frontend/src/lib/assistant-availability.ts          # shouldRedirectFromAssistant({isLoading,user}) — mirrors billing-availability.ts
frontend/src/components/assistant-route-guard.tsx   # reactive guard mirroring billing-route-guard.tsx (+ test, same idiom as billing-route-guard.test.tsx)
frontend/src/pages/lazy.ts                          # AssistantPage lazy export
frontend/src/router.tsx                             # assistantRoute: beforeLoad (auth mirror + flag) + component wrapped in AssistantRouteGuard
frontend/src/components/dashboard/sidebar.tsx       # flag-gated "Assistant" entry above MAIN_NAV (purple-bordered, sparkles icon, NEW pill)
frontend/src/components/layout/dashboard-layout.tsx # MobileNav renders nav independently (dashboard-layout.tsx:481,593) — same flag-gated entry there

# Sol-B — entire assistant surface
frontend/src/types/assistant.ts                     # blocks/messages/conversation/events/transport (§4)
frontend/src/lib/assistant/transport.ts             # AssistantTransport interface + mock implementation wiring
frontend/src/lib/assistant/mock-data.ts             # mock store (authoritative), conversations, scripted turns, injectable clock
frontend/src/lib/assistant/stream.ts                # cursored turn-event reducer (§5)
frontend/src/hooks/use-assistant.ts                 # useConversations/useConversation/useSendMessage/useCancelTurn/useDecideApproval
frontend/src/pages/assistant.tsx                    # AssistantPage (created FIRST as a compiling skeleton — see §8 seam rule)
frontend/src/components/assistant/assistant-shell.tsx      # h-dvh frame; useApplyTheme(); responsive off-canvas sidebar
frontend/src/components/assistant/assistant-sidebar.tsx
frontend/src/components/assistant/chat-thread.tsx           # incl. unsupported-block shell
frontend/src/components/assistant/chat-composer.tsx         # send/stop per turn.status; safe-area padding
frontend/src/components/assistant/blocks/{text-block,connect-card,run-card,approval-card,artifact-block}.tsx
```

Tests (vitest, colocated; per-file `createWrapper()` with fresh `QueryClient({retry:false})`, `useAuthStore.setState(...)`):
- Sol-A: `assistant-availability.test.ts`, `assistant-route-guard.test.tsx`, fixed `use-feature-flag.test.tsx`; backend flag lookup test if the registry restructure warrants it.
- Sol-B: `stream.test.ts` (incl. duplicate + out-of-order cursor delivery), `use-assistant.test.tsx` (send → streamed reply; cancel mid-turn; **fake timers**: `vi.useFakeTimers()` + `advanceTimersByTimeAsync` inside `act`, restore real timers + reset singleton store + dispose QueryClient per test — mixing `waitFor` with unadvanced fake timers hangs happy-dom), `approval-card.test.tsx` (approve AND deny branches, click throttle), `text-block.test.tsx` (security: raw HTML escaped, non-https/mailto links inert, `rel="noopener noreferrer"`, no remote images), unknown-block shell test.

## 4. Types contract (`types/assistant.ts`) — PRD §5.5/§5.6 exact

Full shapes, no field omissions or optionality downgrades:
- `connect_card` includes `icon_url`, `device_user_code`, `device_verification_url` (nullable, **present**) — PRD :285-:305.
- `run` step `service_slug`, `artifact_id`, `approval_request_id` are required-nullable, not optional — PRD :311-:314.
- **Every `TurnEvent` carries `cursor: number`** (per-turn monotonic; PRD :352). Reducer state stores `lastCursor` and drops `cursor <= lastCursor` (at-least-once delivery discipline), even though local scripts happen to be ordered — the reducer exists for API compatibility.
- `AssistantMessage { id, role, schema_version: number, blocks, created_at }` — renderer treats `schema_version !== 1` and unknown `block.type` as the unsupported-content shell, never a crash/drop.
- `Conversation { id, title, created_at, last_message_at }` (list shape is the acknowledged v1.1 divergence).

```ts
export interface AssistantTransport {
  listConversations(): Promise<Conversation[]>;
  createConversation(): Promise<Conversation>;
  getHistory(conversationId: string): Promise<{ conversation: Conversation; messages: AssistantMessage[]; has_more: boolean }>;
  sendMessage(conversationId: string, content: string, onEvent: (e: TurnEvent) => void): TurnHandle; // TurnHandle: { turnId, cancel(): void }
  decideApproval(conversationId: string, blockId: string, approved: boolean): Promise<void>;
}
```
The mock implements this; the API pass swaps the implementation (list/create pending the v1.1 decision). Hooks depend only on the interface.

## 5. Mock layer & state ownership

- **Authoritative state lives in the mock transport** (module-singleton store inside `lib/assistant/mock-data.ts`, reset hook exported for tests). React Query caches are projections: every mutation/event updates the store first, then writes **both** the detail view (`{conversation, messages, has_more}` — C1 §5.1 shape) and the conversation-list view via **immutable functional `queryClient.setQueryData` updaters**. No divergence between Map and cache on refetch/focus.
- **Conversations (3, mirroring the mockup sidebar):**
  1. **"Failed Stripe payments digest"** — showcase, final-state history: user text → assistant text + `connect_card` (stripe, `connected`, steps done, footer per mockup) → status `text` ("✓ Stripe connected — charges:read granted · credential sealed in NyxID's vault") → assistant text + `run` (`awaiting_approval`, 2/3, step 3 `waiting`) + `artifact` (failed-payments-2026-07-13.md, preview) + `approval_card` (`decision: null`, `approval_mode: "per_request"`).
  2. **"Rotate GitHub deploy key"** — completed run (2/2) + closing text.
  3. **"Weekly usage report"** — text + artifact + text.
- **Clock:** store takes an injectable `now()` (default `Date.now`); `expires_at` on the pending approval is computed at store init (~14 min out) so demos show a sane countdown and tests pin time.
- **Scripted turns:** generic `TurnEvent[]` reply for sends (streamed text via several cursored `block.delta`s → `block.completed` → `message.completed` → `turn.completed`), played with `setTimeout` cadence (~80-150 ms/event) through a **cancellable handle** (clears pending timers).
- **Cancellation lifecycle (PRD Stop-flow):** `useCancelTurn` → transport cancels the script, patches open blocks to terminal states, emits their `block.completed`, then `turn.completed {status:"cancelled"}`. Concurrent sends while a turn is active are rejected (mirrors C1 `409 turn_active`).
- **Approval decision branching:** Approve → card `decision:"approved"`, `decision_channel:"web"`, run step 3 → `done`, run `completed`, 3/3. Deny → card `denied`, step 3 → `failed`, run `failed`; the write step is **not** marked done. Buttons disabled while pending + ≥750 ms throttle (PRD D4).
- **New chat:** `createConversation()` → `local-{n}`; first send titles it from the first ~40 chars (crumb + list update through the store).
- **Text rendering:** markdown subset per PRD §5.7. Check existing deps first (docs/blog pages may already ship a markdown renderer) — if a suitable lib with strict config exists in `package.json`, reuse it; **otherwise write a tiny in-repo renderer** (bold, italic, inline code, `https:`/`mailto:` links with `rel="noopener noreferrer"`, line breaks; everything else escaped literal). **No new npm dependencies** — lockfile churn triggers unrelated CI surface and vetting requirements.
- `// TODO(api-pass)` markers at every transport swap point.

## 6. Routing & gating (two-layer, billing pattern)

```ts
const assistantRoute = createRoute({
  path: "/assistant",
  getParentRoute: () => rootRoute,
  validateSearch: (s) => ({ c: typeof s.c === "string" ? s.c : undefined }),
  beforeLoad: ({ location }) => {
    // Layer 1 (synchronous snapshot):
    // 1. auth — mirror dashboardLayout.beforeLoad semantics (DEV ?mock init, return_to deep-link,
    //    redirect /login) with a comment pointing at router.tsx:259; do NOT refactor the original.
    // 2. flag — shouldRedirectFromAssistant({ isLoading, user }) → redirect /dashboard
  },
  component: () => (<AssistantRouteGuard><AssistantPage /></AssistantRouteGuard>),
});
```
- **Layer 2:** `AssistantRouteGuard` — store-subscribing component guard mirroring `billing-route-guard.tsx:13` (renders nothing while loading; `navigate({to:"/dashboard", replace:true})` when loading settles with the flag absent). Without it, the `beforeLoad` snapshot misses the loading→loaded transition.
- `shouldRedirectFromAssistant({ isLoading, user })` → `!isLoading && !(user?.capabilities?.enabled_features ?? []).includes(FEATURE_FLAG.AI_ASSISTANT)`; never redirects while `isLoading`.
- **Nav entries (both):** desktop `sidebar.tsx` above `MAIN_NAV`; **and** the independent `MobileNav` in `dashboard-layout.tsx` (:481, :593). Both gated by `useFeature(FEATURE_FLAG.AI_ASSISTANT)`. Command palette: skip this pass.
- Assistant sidebar footer "Studio" → `/dashboard`.

## 7. Design (mockup → app tokens)

Dark default; light via `html.theme-light`. **Theme tokens/utilities only, no hex literals.** `AssistantShell` calls `useApplyTheme()`.

| Mockup | App implementation |
|---|---|
| bg `#0D0D0D` / card `#171717` | `bg-background` / `bg-card` |
| hairlines `.08/.15` | `border-hairline` / `border-hairline-strong` |
| overlays `.03/.06` | `bg-overlay` / `bg-overlay-strong` |
| purple CTA gradient (New chat, send) | `Button variant="primary"` (`nyx-gradient-vivid`) |
| accent `#A672FB` (active icons, assistant avatar) | `nyx-secondary-400` idiom (match sidebar active state) |
| warning/success/error card tints | tokens with `/10 · /15 · /30` alpha steps |
| radii 6/10/12/14 | `rounded-sm/md/lg/xl` |
| type scale | 13px nav+chat body · 12px UI · 11px meta · 10px badges/run-head · 9px group labels |
| top bar / sidebar | h-[52px], logo zone w-[200px]; sidebar w-[200px] desktop, off-canvas < md |
| thread | max-w-[680px] centered, 26px avatar + body rows — **not** bubbles |

Reuse `components/ui/*` where they fit; bespoke card internals are local JSX. Approve = green-tint, Deny = red-tint. No `console.log`.

## 8. Parallel split (heca agents, same worktree — revised per Sol-Lead)

| Agent | Workstream | Files |
|---|---|---|
| **Sol-A — platform integration** | Everything in the Sol-A block of §3: flags (both registries), rollout documentation, narrowing fixes, MOCK_USER, availability helper + reactive guard + tests, lazy/router, desktop sidebar + MobileNav entries | no assistant-surface files |
| **Sol-B (Sol-Lead) — assistant surface** | Everything in the Sol-B block of §3: types, transport, mock store, reducer, hooks (incl. cancel), page, shell, sidebar, thread, composer, blocks, responsive behavior, all their tests | no platform files |
| **Sol-R — review** | After A+B settle: contract-shape audit vs PRD §5.5/§5.6/§5.7, security/rendering audit, full gates (§9), browser QA at desktop + narrow mobile width | — |

**Seam rule (only coupling point):** Sol-B's **first edit** is a compiling skeleton `pages/assistant.tsx` exporting `AssistantPage`, so Sol-A's lazy/router wiring type-checks against it. The only shared symbols are that lazy export and `FEATURE_FLAG.AI_ASSISTANT`. Neither agent edits the other's files; if a seam problem appears, stop and flag the PM rather than crossing ownership.

PM verifies at each seam: post-A/B integration → post-review → browser QA (`?mock`) before commit.

## 9. CI / quality gates (all before commit)

1. `cd frontend && npm run build` (the real type gate: `tsc -b`, `noUncheckedIndexedAccess`).
2. `npm run lint`, `npm test`.
3. `cargo test` (backend), `cargo fmt --check`; clippy if repo-standard.
4. **Wizard bundle — conditional:** freshness hashes only the recorded wizard module graph + extras (`cli/tests/wizard_bundle_freshness.rs:62`, `cli/src/wizard/bundle-meta/index.manifest`); the planned files aren't in it. Run `cargo test -p nyxid-cli wizard_bundle_freshness` to confirm; regenerate (`npm --prefix frontend run build:wizard` + commit `cli/src/wizard/`) only if it fails.
5. Conventional commits; never commit to main; PR targets `main`.

## 10. Workspace section (scope addition, Calvin 2026-07-16)

The assistant sidebar gains a **WORKSPACE** group above CHATS, per the mockup: Plugins, Artifacts (trailing mono count), Approvals (amber count badge), Devices & Nodes, Activity.

**Clickable in v1: Plugins and Approvals** (Calvin 2026-07-16, twice amended). Artifacts, Devices & Nodes, and Activity render exactly per the mockup — icons, labels, live counts from the mock store — but are **disabled**: non-navigating, muted/disabled treatment per DESIGN.md, tooltip "Coming soon".

- **Approvals** (`/assistant/approvals`) — "a way to check on what needs to be done": "Waiting on you" = pending approval cards across all conversations (decidable inline via the `ApprovalCard` block component), plus a **History** section styled after the Studio `approval-history.tsx` page (desktop table / mobile cards: request + source-conversation link, service, decision badge, channel, when) fed by seeded historical decisions in the mock store. Amber sidebar badge = pending count, live. Deny semantics (review-hardened): denying a gate fails its step, skips the run's other open steps, cancels sibling pending cards, and failed runs are terminal.
- **Plugins runs on real data** (Calvin 2026-07-16): Connectors "Added" derives from `GET /keys` (deduped to **one card per connected service** — multiple credentials show a connection count; join key `catalog_service_slug`), "Available to add" = `GET /catalog` minus connected; tiles use the shared `ServiceIcon` brand glyphs (same set as the AI Services page, initial fallback for custom services); Connect → `/keys?slug=` deep link, Manage → key detail (single) or `/keys` (multi). Skills tab = the real Ornn skills (`lib/assistant/skills.ts`, from the ornn repo SKILL.md frontmatter), session-mock install state; "Add your own skill" stays disabled — the API pass wires it to ornn-api `POST /skills/pull` (already NyxID-permission-gated, see the skills investigation in the PR description).

- **Plugins** (`/assistant/plugins`) — the mockup marketplace: header + subtitle, search, Connectors/Skills tabs, "Added" / "Available to add" sections, cards (tile initial, name, category, description; Connected badge + Manage for added; meta + Connect/Install primary button for available; "Add your own skill" dashed card). Mock catalog mirrors the mockup: added = OpenAI, Anthropic, Stripe, Lark Bot; available = GitHub (oauth), Context7 (mcp · 12 tools), Report Writer (skill), Repo Triage (skill), Postgres (runs on-node), OpenClaw Gateway (self-hosted). Connect/Install flips the card into Added in local mock state. `// TODO(api-pass)`: this maps onto AI services (`/catalog` + `/keys`).
- The **Artifacts library view is deferred** (mock artifact content stays for in-chat artifact blocks only).

**Ownership seam:** the Plugins view, sidebar group, and mock-store extensions are Sol-B files (`components/assistant/plugins-view.tsx`, sidebar, `lib/assistant/*`). The `/assistant/plugins` route registration in `router.tsx` stays Sol-A/PM-owned (a `createRoute` sibling reusing the same guard wrapper) — Sol-B must NOT edit `router.tsx`; export the component and state the exact desired route shape in the handoff note.

## 11. Resolved decisions (were §10 open questions)

1. Mock transport = direct mock module behind `AssistantTransport` (must demo without `?mock` for flag-enabled users).
2. §5.6 reducer with cursors + dedup + immutable functional `setQueryData` updaters + cancellable scripts.
3. `?c=` search param; absent → default conversation. Multi-conversation is the documented v1.1 divergence.
4. Revised split (§8): assistant surface single-owner.
5. Auth guard: mirror-and-comment, no refactor of the dashboard guard.
6. API-pass readiness = transport interface (§4) + C1-shaped detail cache (§5) + cursor/resume discipline + unsupported-block shell. List/create API is the one acknowledged gap.
