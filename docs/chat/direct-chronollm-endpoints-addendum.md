# Direct Chrono-LLM — Endpoint Selector Addendum (spec v4)

Status: implementation-ready. Branch: `chat-chronollm-direct` (extends PR #1426).
Supersedes the flag + hardcoded-slug parts of `direct-chronollm-spec.md` v3.2;
everything else in v3.2 (transport drain-through, billing gate BE-6, rate
limiter, identity reset, prompt composition) stays as built.

Owner directives (2026-08-12):
1. "Gate this behind the aevatar wire pass through flag." → retire the
   unreleased `experimental:direct-chat-engine`; gate the whole surface
   (backend routes + the new gear panel) behind
   `experimental:aevatar-chat-wire-log`.
2. "Add a gear cog beside the same icon to trigger a panel allowing us to
   configure the endpoint used. There will be 3: aevatar, my direct pass
   through, another pass through."
3. Third endpoint: "we are working on using codex as a managed service, but
   we just need to leave it empty for now. These envs should be configured
   via typed enum and config."
4. Persistence: "saved server side by config. FE should expect the same
   shape; if different we will apply transformers on the FE."

## 1. Flag consolidation

- **Remove** `DIRECT_CHAT_ENGINE_FLAG` from
  `feature_flag_service::FEATURE_FLAGS` + `PLATFORM_*` lists + the flag-key
  tests, and `FEATURE_FLAG.DIRECT_CHAT_ENGINE` from
  `frontend/src/lib/feature-flags.ts`. It never shipped (default-off), so no
  migration or DB cleanup is needed. Grep the tree; delete every reference.
- All backend direct routes and the FE gear/panel + engine selection now gate
  on `feature_flag_service::AEVATAR_CHAT_WIRE_LOG_FLAG_KEY`
  (`experimental:aevatar-chat-wire-log`). Rename the backend helper
  `require_direct_chat_enabled` → `require_advanced_chat_enabled` (checks the
  wire-log flag). FE: the gear button and engine selection read
  `useFeature(FEATURE_FLAG.AEVATAR_CHAT_WIRE_LOG)`.
- Net: the wire-log flag is now the single "advanced chat" gate. With it off,
  behavior is byte-identical to today (Aevatar only, no gear, no direct).

## 2. Typed endpoint enum + config (backend)

New typed enum — the single source of truth for what endpoints exist:

```rust
// services/assistant_endpoints.rs (new)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChatEndpointKind { Aevatar, Direct, Secondary }
```

Each endpoint's concrete config comes from `AppConfig` (env), NOT hardcoded
in logic. Add to `AppConfig` (all `parse_*_env`, defaults shown):

```text
ASSISTANT_DIRECT_ENABLED     = true
ASSISTANT_DIRECT_SLUG        = chrono-llm-public
ASSISTANT_DIRECT_LABEL       = "Chrono LLM (direct)"
ASSISTANT_SECONDARY_ENABLED  = false        # empty for now (future: Codex)
ASSISTANT_SECONDARY_SLUG     = ""            # unset -> endpoint disabled
ASSISTANT_SECONDARY_LABEL    = "Secondary pass-through"
```

Resolution (`assistant_endpoints::resolve_config(&AppConfig) -> Vec<ChatEndpointConfig>`):

```rust
pub struct ChatEndpointConfig {
    pub kind: ChatEndpointKind,
    pub label: String,
    pub enabled: bool,                 // secondary: false unless slug set
    pub transport: EndpointTransport,  // Aevatar | Passthrough
    pub slug: Option<String>,          // pass-through target (None for Aevatar)
    pub models: Vec<DirectModel>,      // pass-through picker options (empty for Aevatar)
    pub skills: Vec<DirectSkillMeta>,  // pass-through picker options
}
```

- **Aevatar**: always enabled, `transport: Aevatar`, no slug/models/skills.
- **Direct**: enabled per env, `transport: Passthrough`, slug/label from env,
  models = existing `DIRECT_MODELS`, skills = existing `DIRECT_SKILLS`.
- **Secondary**: `enabled = ASSISTANT_SECONDARY_ENABLED && !slug.is_empty()`.
  Disabled by default → surfaces in the catalog as a present-but-disabled
  option ("empty for now"). When later enabled, reuses the same pass-through
  machinery + prompt composition; models default to `[gpt-5.5]` unless a
  future env supplies them, skills = `DIRECT_SKILLS`.

`DIRECT_LLM_SLUG` const is deleted; the completions handler resolves the slug
from the selected endpoint's config (SSRF-safe: client picks a `kind`, server
maps kind → configured slug; an unknown/disabled kind → 400).

## 3. Per-user selection persistence

New model `models/assistant_chat_preference.rs` (`COLLECTION_NAME =
"assistant_chat_preferences"`, `_id = user_id`):

```rust
pub struct AssistantChatPreference {
    pub id: String,                     // = user_id (UUID string)
    pub active_kind: ChatEndpointKind,  // default Aevatar
    pub direct_model: Option<String>,
    pub direct_skill_slug: Option<String>,
    pub secondary_model: Option<String>,
    pub secondary_skill_slug: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
```

Absent row → defaults (active = Aevatar). Selections are validated on write
against the endpoint's configured `models`/`skills` (unknown → 400).

## 4. Endpoints (all gated on the wire-log flag)

```
GET /api/v1/assistant/chat-config
  -> { endpoints: [ChatEndpointView...], active: kind, selections: {...} }
PUT /api/v1/assistant/chat-config
  { active: kind, direct_model?, direct_skill_slug?, secondary_model?, secondary_skill_slug? }
  -> the same GET shape (validated; disabled/unknown kind or option -> 400)
```

`ChatEndpointView` (uniform shape the FE renders directly — directive 4):

```jsonc
{ "kind": "aevatar"|"direct"|"secondary",
  "label": "...",
  "enabled": true,
  "transport": "aevatar"|"passthrough",
  "models": [ { "id","label","default" } ],   // [] for aevatar
  "skills": [ { "slug","label" } ] }           // [] for aevatar
```

- The old `GET /assistant/direct/skills` and `/models` routes are **removed**;
  their data is folded into `chat-config` per endpoint (one round-trip for the
  panel). Update the FE hooks accordingly.
- `POST /api/v1/assistant/direct/completions` gains a required `endpoint` field
  (`ChatEndpointKind`); the handler:
  1. wire-log flag check (404 when off),
  2. resolve the endpoint config; reject if not a `Passthrough` transport or
     `!enabled` (400 — Aevatar is never served here; it has its own route),
  3. rate limiter (unchanged),
  4. validate model/skill against THAT endpoint's config,
  5. resolve slug from config, rebuild body (prompt composition unchanged),
     `execute_admin_proxy` to the configured slug.
  The client cannot supply a slug or URL — only a `kind` the server maps.

Billing route inventory entry (BE-8) and the BE-6 usage-capture union gate
are unchanged (the target slug is still a token-metered admin service).

## 5. Frontend

### 5.1 Gear panel

- Add a gear-cog `Button` (lucide `Settings` / `Cog`, `variant=ghost
  size=icon`) **beside** the wire-log `Network` button in the same header
  action cluster (`AssistantWireLogAction` neighbourhood, `assistant.tsx`
  `HeaderActions`). Gated behind the same `AEVATAR_CHAT_WIRE_LOG` flag (render
  both or neither).
- Clicking opens a right-side `Sheet` (mirror the wire-log panel's
  Sheet/Tooltip pattern), titled "Chat endpoint". Contents from
  `GET /assistant/chat-config`:
  - a radio/list of the 3 endpoints (label + a "disabled" affordance for
    `secondary` while empty — shown, not selectable);
  - for the active pass-through endpoint, the **model + skill pickers**
    (moved out of the composer into this panel);
  - selecting an endpoint or changing model/skill issues `PUT` and
    optimistically updates.
- FE consumes the uniform `ChatEndpointView[]`; a `normalizeEndpoint()`
  transformer maps each raw view to the internal shape so a future
  differently-shaped endpoint only needs a transformer, not new UI
  (directive 4).

### 5.2 Engine selection now driven by the persisted active endpoint

- `engine` = `active === "aevatar" ? "aevatar" : "direct"` (both `direct` and
  `secondary` use `DirectAssistantTransport`). Read `active` from the
  chat-config query; fall back to `"aevatar"` while loading or if the flag is
  off. The 60s refetch + reload both re-resolve it.
- `DirectAssistantTransport` sends the active `endpoint` kind (+ model/skill)
  on each turn; the transport is pointed at whichever pass-through the config
  names. Conversation-id prefix stays `direct-` for BOTH pass-through kinds
  (they share the transport and store); the engine router is unchanged.
- Remove the composer's inline model/skill pickers (now in the gear panel);
  keep the "Direct model chat — not saved" banner, shown whenever `active` is
  a pass-through.
- Mock/flag-off: unchanged — wire-log flag off (or `?mock`) → no gear, engine
  forced `aevatar`, byte-identical to today.

## 6. Deletions / retirements (FI-007)

- `experimental:direct-chat-engine` (both sides), `DIRECT_CHAT_ENGINE_FLAG`.
- `DIRECT_LLM_SLUG` const (replaced by config).
- `GET /assistant/direct/skills`, `GET /assistant/direct/models` (folded into
  chat-config) + their FE hooks.
- The composer's inline picker wiring (moved to the panel).

## 7. Tests

Backend:
- `resolve_config` env matrix (direct enabled/disabled, secondary
  empty→disabled vs slug-set→enabled, labels).
- chat-config GET/PUT: defaults when no row; validation (disabled/unknown
  kind, unknown model/skill → 400); persisted round-trip.
- completions `endpoint` routing: direct kind → chrono slug; aevatar kind →
  400 (wrong route); secondary-disabled → 400; unknown → 400. Keep the
  fixture-based billing smoke, now driving `endpoint: "direct"`.
- flag-off → 404 on chat-config + completions.

Frontend:
- gear panel: renders from chat-config, secondary disabled, selecting
  persists (PUT called), model/skill change persists; hidden when flag off.
- engine selection from `active`: aevatar vs direct transport; mid-flip
  routing (existing engine-router tests still pass with the new source).
- normalizeEndpoint transformer unit test.

## 8. Out of scope (future)

- The Secondary/Codex target itself (env stays empty; UI shows it disabled).
- Free-form endpoint URLs (never — SSRF; the enum+config allowlist is the
  boundary).
- Per-org / admin-global default selection (per-user only for v1).
