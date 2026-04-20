# CLI Wizard v2

- **Status:** Proposed
- **Owner:** CLI team
- **Tracks:** NyxID#351 (original wizard issue — v2 supersedes the approach attempted in PR #358)
- **Supersedes:** `~/Desktop/docs/cli-wizard.md` (historical, out-of-repo) and the implementation in NyxID PR #358
- **Scope of this doc:** Phase 1 (docs only). Phase 2 (implementation) is gated on this doc being approved and merged. Phase 2 will open a fresh PR against `NyxID#351` (or a new follow-up issue if the team prefers a clean slate).

---

## 1. Why

Every time the NyxID CLI prints a secret — API key, node registration token, TOTP seed, SSH CA cert, agent API key — that secret lands in whatever process is driving the CLI. When the driver is an LLM coding agent (Claude Code, Codex, OpenClaw, Cursor), the secret enters the model's context window, gets prompt-cached, and becomes a persistent leak surface.

The v2 wizard removes that leak:

- Secrets are rendered in a local browser page served by the CLI on `127.0.0.1:<ephemeral>`.
- The terminal sees a single `Opening http://…` line and the CLI's exit status.
- Scripted and CI invocations are unchanged — the wizard only fires under the rules in §3.1.

This is the one outcome v2 must deliver. Everything else is how.

---

## 2. Commands Affected

The wizard edits commands that **already exist**. It does not introduce a new top-level command group.

| Credential                | Existing command               | Backend API                                    | First-ship? |
|---------------------------|--------------------------------|------------------------------------------------|:-----------:|
| AI service / API provider | `nyxid service add [slug]`     | `POST /api/v1/keys`                            |      ✅      |
| SSH service               | `nyxid service add-ssh`        | `POST /api/v1/keys` (ssh)                      |             |
| NyxID agent API key       | `nyxid api-key create`         | `POST /api/v1/api-keys`                        |             |
| API key rotation          | `nyxid api-key rotate <id>`    | `POST /api/v1/api-keys/{id}/rotate`            |             |
| Node registration         | `nyxid node register-token`    | `POST /api/v1/nodes/register-token`            |             |
| Node rotation             | `nyxid node rotate-token <id>` | `POST /api/v1/nodes/{id}/rotate-token`         |             |
| MFA / TOTP                | `nyxid mfa setup` + `verify`   | `POST /api/v1/mfa/setup` + `.../verify-setup`  |             |
| OpenClaw setup            | `nyxid openclaw setup`         | `POST /api/v1/keys`                            |             |
| Channel bot               | `nyxid channel-bot register`   | `POST /api/v1/channel-bots`                    |             |

v2.0 ships `service add` only. The remaining rows are later PRs that reuse the framework unchanged — see §5.

---

## 3. User Flow — `nyxid service add`

### 3.1 Invocation rules

The wizard fires only when **all** of these are true:

- none of the scripted-flow flags are set: `--credential`, `--credential-env`, `--oauth`, `--device-code`, `--custom`, `--auth-method`, `--auth-key-name`, `--scope`, `--org`, `--openapi-spec-url`
- stdout is a TTY
- `--output` is not `json`
- the environment is not headless — none of:
  - `NYXID_NO_WIZARD` set to any value (explicit opt-out)
  - `SSH_CONNECTION` or `SSH_TTY` set (SSH session — no local browser)
  - on Linux: both `DISPLAY` and `WAYLAND_DISPLAY` unset (no X/Wayland)

The `slug`, `--label`, `--via-node`, and `--endpoint-url` flags are **prefill-compatible** — they pre-populate the wizard form instead of disabling it.

Any other invocation runs the existing non-interactive path **unchanged**. Scripted, CI, and SSH users see no behavior change from the pre-wizard CLI.

### 3.2 Terminal output

The CLI streams concise status to the terminal while the user is in the browser, so both surfaces stay in sync. The terminal is never silent for more than one phase.

**While the wizard runs:**

```
$ nyxid service add
→ Opening http://127.0.0.1:54213/wizard … (Ctrl-C to cancel)
  Waiting for browser …
  Connected. Complete the steps in your browser.
  Creating service 'work-openai' …
```

Each line is appended as the wizard progresses. No carriage-return trickery; simple append-only output so it scrolls predictably inside tmux / screen recording / `script`.

**On success, the terminal prints the same confirmation summary the browser showed** (so the user never has to switch focus back to the browser to find the proxy URL):

```
✓ Service 'work-openai' created.
  Slug:      work-openai
  Proxy URL: https://auth.nyxid.dev/api/v1/proxy/s/work-openai/
  Agents:    all (no binding)

  Next:  curl $PROXY/v1/models -H "Authorization: Bearer $NYX_KEY"

$
```

**On cancel (user closes the tab or hits Ctrl-C):**

```
✗ Wizard cancelled. No service was created.
```

**On timeout (5 min inactivity):**

```
✗ Wizard timed out after 5 minutes. No service was created.
  Tip: use the scripted form for CI / non-interactive setups:
       nyxid service add <slug> --credential-env OPENAI_KEY --label <label>
```

**On API error (mid-flow failure):** the browser shows an inline error and allows retry; the CLI stays running and does not print to the terminal unless the wizard is ultimately cancelled or completed.

See §3.7 for the full FE↔CLI handoff contract (how the browser signals completion, how the CLI detects tab-close, how the browser tab behaves after the user clicks Done).

### 3.3 Browser — Step 1: pick a service

```
┌────────────────────────────────────────────────────────────────┐
│  Add an AI service                                         1/4 │
│                                                                │
│  🔍 [ search services...                                   ]   │
│                                                                │
│  Simple setup                                                  │
│  ╭──────────────────╮ ╭──────────────────╮ ╭────────────────╮  │
│  │ [icon]           │ │ [icon]           │ │ [icon]         │  │
│  │ OpenAI           │ │ Anthropic        │ │ Google Gemini  │  │
│  │ paste key        │ │ paste key        │ │ paste key      │  │
│  ╰──────────────────╯ ╰──────────────────╯ ╰────────────────╯  │
│                                                                │
│  Advanced                                                      │
│  ╭──────────────────────────────────────────────────────────╮  │
│  │ Custom / self-hosted …                                   │  │
│  │ For OpenClaw, Ollama, Lark, OAuth flows, or any          │  │
│  │ endpoint that isn't a simple bearer-token API.           │  │
│  ╰──────────────────────────────────────────────────────────╯  │
│                                                                │
│                                                [ Cancel ] [→]  │
└────────────────────────────────────────────────────────────────┘
```

**v2.0 scope (Option C):** the "Simple setup" grid lists only catalog entries where the wizard's SimpleKey form works — catalog entries with `provider_type = "api_key"`, `auth_method = "bearer"`, and `requires_gateway_url = false`. Everything else (OAuth, device code, token exchange, self-hosted, custom endpoints) lives behind the single "Custom / self-hosted" card, which routes to the power-user form (§3.4b).

Populated from `GET /api/v1/catalog`. Cards show `name`, icon, and a one-line hint. `provider_type` is hidden from the card because all Simple-setup entries are the same type.

Total steps is now **1 → 2 → 2.5 → 3** (catalog → credential → agent bindings → confirmation), rendered as `1/4`, `2/4`, `3/4`, `4/4` in the header so users see a consistent progress count whether or not they have agents to bind.

### 3.4 Browser — Step 2: credential

v2.0 ships two credential forms. The one shown depends on which card the user picked on Step 1.

#### 3.4a SimpleKey form (the v2.0 happy path)

Shown when Step 1 picked a Simple-setup card (OpenAI, Anthropic, Gemini, etc. — all simple bearer-token APIs).

```
┌────────────────────────────────────────────────────────────────┐
│  Connect OpenAI                                            2/4 │
│  ← back                                                        │
│                                                                │
│  Label                                                         │
│  [ work-openai                                             ]   │
│  ↑ shown everywhere in the CLI and web UI                      │
│                                                                │
│  API key                                                       │
│  [ sk-•••••••••••••••••••••••••••••••••••••••••  👁         ]   │
│  ↑ paste from platform.openai.com/api-keys                     │
│                                                                │
│  This will create:                                             │
│    · an endpoint      (where NyxID sends requests)             │
│    · a stored key     (encrypted at rest)                      │
│    · a routing rule   (slug: work-openai)                      │
│                                                                │
│                                                  [ Connect ]   │
└────────────────────────────────────────────────────────────────┘
```

Design notes:

- **Label is required** at the backend (`backend/src/handlers/keys.rs:132` — `CreateKeyRequest.label` is not optional). The field is pre-filled from the catalog entry's `name` lowercased and slugified. User can edit.
- **API-key input is `type="password"`** with an optional eye-toggle for show/hide. `autocomplete="off"` so the browser doesn't offer to save it. `spellcheck="false"`.
- **The three-line "This will create" panel** explains the unified-key-service model in plain language. Users understand NyxID is creating three records (endpoint + key + service) not one.
- **Provider-specific hint** under the API key field — copy lives in the catalog entry as `credential_hint` (new optional field, defaults to generic "paste your API key").
- **No advanced section in v2.0.** Node routing, endpoint override, custom auth method all live in the Custom form (3.4b).

On **Connect**, the browser POSTs `/api/proxy/api/v1/keys` with body `{service_slug, credential, label}` and the `X-Wizard-CSRF` header. The CLI's proxy adds the NyxID bearer server-side and forwards to `unified_key_service::create_key`.

#### 3.4b Custom / self-hosted form (power-user escape hatch)

Shown when Step 1 picked the "Custom / self-hosted" card. This form exposes the full flag surface of `nyxid service add` as form inputs — it is the visual equivalent of the scripted CLI, not a curated UX. v2.0 ships it intentionally rough; polish is follow-up work (see §12).

```
┌────────────────────────────────────────────────────────────────┐
│  Custom service                                            2/4 │
│  ← back                                                        │
│                                                                │
│  Label                              [ my-openclaw           ]  │
│  Catalog slug (optional)            [ llm-openclaw          ]  │
│  Endpoint URL                       [ https://openclaw.…    ]  │
│  API key / credential               [ ••••••                ]  │
│  Auth method   [ bearer ▾ ]    (bearer | header | query | basic | none) │
│  Auth key name (for header/query)   [ Authorization         ]  │
│  ▸ Route through a node                                        │
│  ▸ OpenAPI spec URL                                            │
│                                                                │
│                                                  [ Connect ]   │
└────────────────────────────────────────────────────────────────┘
```

v2.0 design constraints for this form:
- **No OAuth / device-code flow support.** Those flows require backend polling and placeholder-key contracts (see §12.10) that v2.0 punts. Users who need OAuth use the existing scripted form: `nyxid service add <slug> --oauth`.
- **No multi-field token-exchange support** (e.g. Lark's `app_id` + `app_secret`). Same reason — token-exchange handling is deferred.
- **Required fields:** label, endpoint URL, credential. Everything else optional with sensible defaults.
- The form submits the same `POST /api/proxy/api/v1/keys` as SimpleKey — the backend is the same, only the form surface is wider.

Catalog entries that would require OAuth / device-code / token-exchange are **visible in search** but cards are disabled with a tooltip: *"This provider needs OAuth — use `nyxid service add <slug> --oauth` in your terminal for now. Wizard support is coming in a follow-up PR (see §9.1)."*

### 3.5 Browser — Step 2.5: agent bindings (new — feature (3))

After the key is successfully created, **and only if the user has one or more existing NyxID API keys (`nyxid_ag_...`)**, the wizard shows an optional binding step. This ties the new credential to specific agents so each coding LLM uses the right API key without manual `nyxid api-key bind` commands.

```
┌────────────────────────────────────────────────────────────────┐
│  ✓ Connected OpenAI — which agents should use this key?    3/4 │
│                                                                │
│  ● All agents (default — any NyxID key routes to this service) │
│                                                                │
│  ○ Specific agents only:                                       │
│    ☐ coding-agent      claude-code   created 2d ago            │
│    ☐ chat-assistant    openclaw      created 5d ago            │
│    ☐ batch-processor   codex         created 1w ago            │
│                                                                │
│  Why bind?                                                     │
│  Per-agent credentials isolate blast radius — one agent can    │
│  use a throwaway key, another can use your production key.     │
│  Change any time with `nyxid api-key bind`.                    │
│                                                                │
│                                          [ Skip ]  [ Continue ]│
└────────────────────────────────────────────────────────────────┘
```

Design notes:

- **Step is skipped entirely** if the user has zero agent API keys. In that case, Step 3 (confirmation) shows a small tip: *"Create an agent API key with `nyxid api-key create` and bind it later with `nyxid api-key bind`."*
- **Default is "All agents"** — matches current behavior where any NyxID API key can proxy any service the user owns. Choosing this skips the binding writes entirely.
- **Selecting specific agents** POSTs one `/api/v1/api-keys/{agent_key_id}/bindings` request per selected agent, with body `{user_service_id, user_api_key_id, label}`. Uses the existing `AgentServiceBinding` model (CLAUDE.md §9 — "Agent Isolation") — no new backend work.
- Skipping this step is a first-class choice. "Skip" and "Continue" sit side-by-side, equal weight. Continue requires a selection (either "All agents" or at least one specific agent).
- Fetched via `GET /api/v1/api-keys` on step entry (added to the proxy allowlist in §4.3).

### 3.6 Browser — Step 3: confirmation

```
┌────────────────────────────────────────────────────────────────┐
│  ✓ Service 'work-openai' created                           4/4 │
│                                                                │
│  Slug:       work-openai                                       │
│  Proxy URL:  https://auth.nyxid.dev/api/v1/proxy/s/work-       │
│              openai/v1/chat/completions                        │
│  Agents:     all (no binding)                                  │
│                                                                │
│  Try it:                                                       │
│    curl $PROXY/v1/models -H "Authorization: Bearer $NYX_KEY"   │
│                                                                │
│  [ Copy proxy URL ]    [ Copy curl ]                           │
│                                                                │
│                      [ Done — return to terminal ]             │
└────────────────────────────────────────────────────────────────┘
```

The button copy is deliberately "Done — return to terminal" not just "Done". It is the primary handoff cue.

- **Slug may include an auto-generated suffix** (e.g. `work-openai-2`) if the user already has a `work-openai` service. Backend autogenerates via `unified_key_service.rs:27`; the wizard displays whatever the backend returned.
- **Raw API key is never re-displayed.** The user just pasted it; we don't echo it back.
- **On "Done":** the browser sends `POST /api/proxy/complete` to the CLI server. The CLI prints its terminal summary (§3.2), cleans up, and exits `0`. The browser page then shows a final "You can close this tab" message (see §3.7 for why `window.close()` isn't used).

### 3.7 FE ↔ CLI handoff contract

The browser wizard runs in a separate OS window from the terminal where the user invoked the CLI. v2.0 makes the handoff explicit so the user is never stranded wondering which surface to look at.

**Protocol (browser → CLI):**

The browser page has three lifecycle pings it sends to the local server:

| Ping                           | When                                        | CLI reaction                                |
|--------------------------------|---------------------------------------------|---------------------------------------------|
| `POST /api/proxy/heartbeat`    | Every 10 s while page is visible            | Resets the inactivity timer                 |
| `POST /api/proxy/cancel`       | On `beforeunload` (tab close / navigate)    | CLI prints "✗ Wizard cancelled", exits 1    |
| `POST /api/proxy/complete`     | User clicks "Done — return to terminal"     | CLI prints success summary, exits 0         |

All three require the `X-Wizard-CSRF` header (same token as proxy requests).

**Protocol (CLI → browser):**

The page long-polls `GET /api/proxy/status` every 2 s. Response shape:
```json
{ "state": "running" | "completing" | "shutdown", "message": "…" }
```
When the server sees `complete`, it transitions to `shutdown` and the page renders the "You can close this tab" final view.

**Why not `window.close()`?** Modern browsers block `window.close()` on tabs the user navigated to manually (even when opened via `open::that()`, the browser treats the tab as user-initiated). v2.0 does not try. The tab shows a clear "You can close this tab and return to your terminal" message after completion; the browser's close-tab button does the rest.

**Timeout semantics:**

- **Inactivity timeout: 5 min.** No heartbeat for 5 consecutive minutes → CLI prints timeout message, exits 1, cleans up any partially-created resources it can (see §12.5 for placeholder-key cleanup debt).
- **Overall timeout: 30 min.** Hard ceiling — catches the case where a user leaves the tab open and walks away. Same exit message as inactivity timeout.
- **Ctrl-C in the terminal:** CLI sends `POST /api/proxy/shutdown` to its own server (not strictly necessary, just cleaner) and exits. Browser page detects `shutdown` state on its next status poll and shows "The CLI was cancelled. Close this tab."

**What the user sees end-to-end — happy path:**

```
Terminal                             │  Browser
─────────────────────────────────────┼─────────────────────────────────
$ nyxid service add                  │
→ Opening http://127.0.0.1:… …       │  [ opens, Step 1: pick service ]
  Waiting for browser …              │
  Connected.                         │  [ user picks OpenAI, Step 2 ]
  Creating service 'work-openai' …   │  [ user pastes key, hits Connect ]
                                     │  [ Step 3: agent bindings, "All" ]
                                     │  [ Step 4: confirmation ]
                                     │  [ user clicks "Done — return …" ]
✓ Service 'work-openai' created.     │  "You can close this tab."
  Slug:      work-openai             │
  Proxy URL: https://auth.nyxid.dev… │
                                     │
$                                    │
```

**What the user sees end-to-end — cancel path:**

```
Terminal                             │  Browser
─────────────────────────────────────┼─────────────────────────────────
$ nyxid service add                  │
→ Opening http://127.0.0.1:… …       │  [ opens, Step 1 ]
  Waiting for browser …              │
  Connected.                         │  [ user hits ⌘W / closes tab ]
✗ Wizard cancelled. No service was   │
  created.                           │
                                     │
$                                    │
```

### 3.8 Headless / scripted fallback

- `--terminal`, or `SSH_CONNECTION` set, or `DISPLAY` unset on Linux, or stdout non-TTY → prompt-based renderer using `rpassword` (already a CLI dep). Same step definitions, text-only rendering, no browser.
- `--output json` → non-interactive; requires all fields as flags; prints the existing JSON shape; never binds a port.

---

## 4. Architecture

```
cli/src/wizard/
├── mod.rs             entry: run_flow(flow_id, args) -> Result<Value>
├── runtime.rs         step engine + typed Context map
├── server.rs          axum on 127.0.0.1:0, CSRF check, proxy allowlist
├── renderer/
│   ├── browser.rs     opens browser, serves embedded SPA
│   ├── terminal.rs    rpassword + stdin, same step trait
│   └── json.rs        non-interactive, flag-driven
├── assets/            embedded via rust-embed — all bytes served from
│   │                  127.0.0.1. Zero remote resources (see 4.0).
│   ├── wizard.html
│   ├── wizard.js
│   └── wizard.css
└── flows/
    ├── mod.rs         flow registry
    └── ai_key.rs      (v2.0 — only flow shipped)
```

### 4.0 Visual parity with the frontend — served locally

**Decision:** the wizard's browser UI *visually mirrors* the existing frontend `AddKeyDialog` (see `frontend/src/components/dashboard/add-key-dialog.tsx`) — same layout, same copy, same catalog grid, same form shape, same color palette — but **every byte is hand-rolled vanilla HTML/CSS/JS served locally from `127.0.0.1:<port>`**. No React, no shadcn/ui runtime, no fetch-from-frontend-origin, no redirect pattern.

**Why not redirect to the frontend?** Considered and rejected: serving the wizard from the real frontend origin would put API-key form inputs on a page we don't control at render time (subject to whatever CSP / third-party assets the frontend pipeline ships that week). For a form whose whole reason-to-exist is secret handling, we need the page bytes frozen at CLI-release time and served from `127.0.0.1` with a strict CSP we own. Visual parity gives us the UX; local serving gives us the trust boundary.

**Source of visual truth:**
- Step 1 catalog grid → mirrors `CatalogGrid()` in `add-key-dialog.tsx:223-322` (cards, badges, search input)
- Step 2a SimpleKey form → mirrors the "Form" step of `AddKeyDialog` for simple bearer catalog entries
- Step 2b Custom form → mirrors the custom-endpoint branch
- Step 2.5 agent bindings → mirrors `BindingsCard` in `bindings-card.tsx:71-359` (switch + add/remove rows)
- Step 3 confirmation → mirrors `AddKeyDialog`'s success state
- Tailwind class names and spacing are copied directly; the CSS bundle in `assets/wizard.css` is a hand-extracted subset of the frontend's Tailwind output containing only the classes actually used by these screens.

**Rule:** whenever the frontend's `AddKeyDialog` visual changes, the CLI wizard's HTML/CSS follows in a parallel PR. Tracked as low-priority maintenance debt in §12.

**Rule:** no remote fetches from the wizard page ever. No Google Fonts. No CDN. No external analytics. All fonts are either system-fallback stacks or self-hosted bytes embedded in `assets/`. CSP (see §12.1) enforces this at runtime.

### 4.1 Step types

- `Choice { source, prompt, next_fn }` — catalog picker (Step 1)
- `Input { fields, validate_fn, skip_if }` — form (Steps 2, 2.5). `skip_if` is an optional predicate over `Context` that skips the step entirely (used for "no agent keys → no binding step").
- `Auto { method, path, body_fn }` — proxied API call, result merged into `Context` (on Connect click in Step 2, on Continue click in Step 2.5)
- `Display { template, continue_label, primary_cta }` — non-secret rendering (Step 3). `primary_cta: "Done — return to terminal"` drives the handoff messaging.
- `Confirm { summary_fn }`
- `External { open_url_fn, wait_for }` — OAuth / device redirects (*not used in v2.0* — Custom form punts these flows to the scripted CLI; see §3.4b)

The `Context` is a typed `serde_json::Map` threaded between steps. `body_fn` and `template` read from it via JSON paths. `skip_if` reads from it to branch.

**v2.0 flow composition** (AI-keys flow):
```rust
WizardFlow {
    steps: vec![
        Choice  { source: Catalog, /* Step 1 */ },
        Input   { fields: SimpleKeyOrCustom, /* Step 2 */ },
        Auto    { method: POST, path: "/api/v1/keys", /* create */ },
        Input   { fields: AgentBindings,
                  skip_if: |ctx| ctx["agent_keys"].as_array().unwrap_or(&vec![]).is_empty(),
                  /* Step 2.5 */ },
        Auto    { method: POST, path: "/api/v1/api-keys/:id/bindings",
                  body_fn: /* one request per selected agent */ },
        Display { /* Step 3, primary_cta = "Done — return to terminal" */ },
    ],
    allowlist: /* see §4.3 */,
}
```

### 4.2 Security — fixing the v1 gaps

> **Note:** this table covers framework-level fixes v2.0 ships with. Deeper hardening — locked-down page with strict CSP, Origin/Host enforcement, typed route validation, explicit threat model, placeholder-key cleanup, etc. — is listed as explicit debt in §12 and scheduled for follow-up PRs. Do not read this table as the final security model.

The historical spec at `~/Desktop/docs/cli-wizard.md` lists 13 blocking gaps. v2 closes the framework-level ones on day one:

| v1 gap                        | v2 fix                                                                                                    |
|-------------------------------|-----------------------------------------------------------------------------------------------------------|
| #1 Server-spawn race          | `axum::Server::bind` resolves port; serve future spawned and awaited ready *before* `open::that()`         |
| #2 No headless detection      | Renderer chosen from `--output`, `--terminal`, `SSH_CONNECTION`, `DISPLAY`, `stdout.is_terminal()`         |
| #3 `--output json` bypass     | JSON renderer short-circuits before the server binds                                                      |
| #4 Non-generic completion     | `CompletionResult = serde_json::Value`; flows shape their own output                                      |
| #5 Hardcoded display rendering| `Display` step uses a `template` string + Context paths                                                   |
| #6 Path-only proxy allowlist  | Allowlist is `Vec<(Method, &'static str)>`, enforced per request                                          |
| #7 Token in `/api/config` JSON| CSRF token injected server-side into `<meta name="wizard-csrf">` in the HTML template. No JSON exposes it. |
| #8 JS cleanup on completion   | On `Done`: `stepResults = null; creds = null;` and server `204` then shutdown                             |
| #9 QR for MFA                 | Deferred to the MFA-flow PR (not part of v2.0)                                                            |
| #10–13 Per-command wiring     | Addressed flow-by-flow in follow-up PRs (§5)                                                              |

The NyxID bearer token lives only in CLI process memory. The browser never sees it. The proxy handler attaches it to the outbound request server-side. The CSRF token is checked constant-time on every `POST/PUT/DELETE` to `/api/proxy/*`.

### 4.3 Proxy allowlist — AI-keys flow (v2.0)

Two kinds of endpoints: **NyxID backend proxy routes** (forwarded upstream with the user's bearer token attached) and **CLI-local lifecycle routes** (served by the CLI's own axum server, not forwarded). Only the first class is gated by the proxy allowlist.

**Forwarded to NyxID backend:**

```
GET  /api/v1/catalog
GET  /api/v1/catalog/:slug
GET  /api/v1/api-keys            ← new: lists user's NyxID API keys for Step 2.5
POST /api/v1/keys                ← creates UserEndpoint + UserApiKey + UserService
POST /api/v1/api-keys/:id/bindings  ← new: creates AgentServiceBinding for Step 2.5
```

**Handled locally by the CLI (no allowlist gate, but still CSRF-required):**

```
GET  /api/proxy/status       ← long-poll for handoff state
POST /api/proxy/heartbeat    ← page visibility heartbeat
POST /api/proxy/cancel       ← beforeunload / tab close
POST /api/proxy/complete     ← user clicked "Done — return to terminal"
```

Any other method+path on `/api/proxy/api/v1/...` → `403 Forbidden`.

**Not included in v2.0 allowlist** (deferred with their respective flows):
- OAuth start/callback routes (`/providers/{id}/connect/oauth`) — Custom form punts OAuth to scripted CLI; see §12.10 for correct route paths when the OAuth flow PR lands.
- Device-code polling (`/providers/{id}/connect/device-code/poll`) — same deferral.
- `/nodes` — node routing is part of the Custom form only; Custom form doesn't implement it in v2.0 either. Listed here for the framework follow-up PR.

---

## 5. Scalability — Adding the Next Flow

Each row of the table in §2 becomes one follow-up PR with the same five-step recipe:

1. Create `cli/src/wizard/flows/<id>.rs` — declare `steps` + `allowlist` + `required_overrides`.
2. Register in `flows/mod.rs`.
3. Edit the relevant existing command handler (e.g. `cli/src/commands/api_key.rs::create`) to branch on `interactive_mode(&args)` → `wizard::run_flow(...)`.
4. Optionally add a `nyxid wizard <id>` alias mapping.
5. Add an integration test in `cli/tests/wizard_<id>.rs`.

**Framework changes required for steps 2–5 must be zero.** If a flow needs a new step type or a new renderer affordance, that is its own framework-PR first, separate from the flow PR.

---

## 6. Command Surface

- **Primary:** `nyxid service add` (edited — wizard fires under §3.1 rules)
- **Alias:** `nyxid wizard ai-key` (new thin dispatcher; `nyxid wizard --help` lists every registered flow for discoverability)
- **New flag on `service add`:** `--terminal` (forces prompt renderer). `--output json` already exists.

All existing flags on `service add` continue to work unchanged.

---

## 7. Non-Interactive Contract (unchanged)

`nyxid service add` in scripted mode behaves **exactly** as it does today. The following forms bypass the wizard entirely:

```
nyxid service add llm-openai --credential-env OPENAI_KEY --label work-openai
nyxid service add llm-openai --oauth
nyxid service add --custom --endpoint-url https://api.example.com/v1 --credential-env KEY
nyxid service add llm-openai --output json --credential-env OPENAI_KEY
```

The wizard is a pure addition for bare `nyxid service add` in a TTY. No flag semantics change.

---

## 8. Verification (Phase 2)

1. **Browser happy path.** `nyxid service add` opens the browser, user picks OpenAI, pastes key, optionally binds one agent, clicks "Done — return to terminal", lands on terminal summary. `nyxid service list` shows the new row. Proxy returns `/v1/models`.
2. **No-agents path.** Same as (1) but on a fresh account with no NyxID API keys. Step 2.5 is skipped automatically; the tip about `nyxid api-key create` appears on Step 3.
3. **Custom form path.** Pick "Custom / self-hosted" on Step 1. Fill in endpoint URL + credential + auth method. Creates a service that proxies to the custom endpoint.
4. **Cancel via tab close.** Open wizard, close the browser tab on Step 2. Terminal prints `✗ Wizard cancelled.` and exits `1` within 2 s.
5. **Cancel via Ctrl-C.** Open wizard, Ctrl-C the CLI. Browser tab's next status poll sees `shutdown` and renders "The CLI was cancelled."
6. **Inactivity timeout.** Open wizard, leave tab open idle for 5+ min. Terminal prints `✗ Wizard timed out` and exits `1`.
7. **Back-compat.** `nyxid service add llm-openai --credential-env OPENAI_KEY --label work-openai` completes without browser, identical output to today.
8. **Terminal fallback.** `--terminal` or SSH session drives the same flow via `rpassword` prompts. Agent-binding step surfaces as a numbered prompt; `--output json` skips it entirely unless `--bind-agent` flags are passed.
9. **JSON.** `--output json` prints the existing JSON shape, no browser, no wizard.
10. **Alias.** `nyxid wizard ai-key` reaches the same flow as bare `nyxid service add`.
11. **Leak audit.** `script -q /tmp/session.log nyxid service add` — no `sk-…` or `nyxid_…` bytes in the recorded session log.
12. **Unit tests.** Runtime iteration, Context merging, `skip_if` predicate eval, allowlist rejection (`POST /api/v1/admin/users` → 403), CSRF mismatch → 403, renderer selection matrix, heartbeat/cancel/complete lifecycle transitions.

---

## 9. Out of Scope

This section is deliberately specific so reviewers can tell what v2.0 is *not* promising. Each item says what it is, why it's deferred, and how it lands later.

### 9.1 Wizardifying the other commands in §2

**What:** `service add-ssh`, `api-key create`, `api-key rotate`, `node register-token`, `node rotate-token`, `mfa setup`+`verify`, `openclaw setup`, `channel-bot register`.

**Why not in v2.0:** Each is a one-file flow against the same framework (see §5), but bundling them all into one PR makes review harder and delays real-world feedback on the framework itself. We want the AI-keys flow in production, observed, and iterated on before mass-converting the rest.

**Lands as:** One PR per row of §2, in a sequence the CLI team picks based on user demand. Each PR is ~200–400 LoC: a new `flows/<id>.rs`, an `interactive_mode()` branch in the existing handler, an integration test.

### 9.2 QR code rendering

**What:** Rendering the TOTP seed as a scannable QR code in the browser page.

**Why not in v2.0:** Only the MFA flow needs it — AI-keys users paste a key they already have; no QR required. Shipping QR support (a QR library dep + template plumbing) with v2.0 would be dead code until the MFA flow lands.

**Lands as:** Part of the `mfa` flow PR. Preferred approach: `qrcode` crate → SVG → inlined into the Step 2 `Display` template. No server-side image hosting, no data-URL blob that could leak via browser cache.

### 9.3 `ratatui` TUI renderer

**What:** A full-screen terminal UI (like `tig`, `k9s`) as a third renderer alongside `browser` and `terminal`.

**Why not in v2.0:** `ratatui` + `crossterm` add meaningful weight and another renderer-matrix to test. The step engine is designed renderer-agnostic, so a TUI renderer can be added later as a pure addition (new file in `renderer/`, no changes to flows or runtime). Headless users in v2.0 get `rpassword` + plain prompts, which matches the rest of the CLI.

**Lands as:** Optional future PR — `cli/src/wizard/renderer/tui.rs` plus a `--tui` flag. Not on the roadmap unless users ask.

### 9.4 `service update` / `service rotate-credential` / rotation commands

**What:** Wizardifying the edit/rotate verbs, not just the create verb. Affects `service update`, `service rotate-credential`, `api-key rotate`, `node rotate-token`.

**Why not in v2.0:** Rotation flows are **not** structurally identical to creation — they start from an existing record, need to display the current label/slug, should confirm before overwrite, and (for api-key/node tokens) the new secret is the only one that needs browser rendering. This deserves its own flow shape (probably an extra `Summary` step type or a refined `Confirm`), and we don't want to bake it in until we've run one rotation flow end-to-end.

**Lands as:** A single PR that covers `api-key rotate` + `node rotate-token` together (both are identical structurally: confirm → POST → display new token), with `service rotate-credential` added in a follow-up once the rotation step pattern is settled.

### 9.5 Any backend API change

**What:** New or modified endpoints on the Rust backend.

**Why not in v2.0:** v2 is a CLI-only delivery. It calls existing endpoints (`/api/v1/catalog`, `/api/v1/catalog/:slug`, `/api/v1/keys`, `/api/v1/nodes`, `/api/v1/providers/:slug/oauth/start`+`callback`). Backend team is uninvolved.

**Lands as:** If a future flow needs a backend change (e.g. MFA might want a dedicated `challenge` endpoint), that's carved off as a backend PR blocking the relevant flow PR — not bundled with wizard framework work.

### 9.6 Telemetry and usage analytics

**What:** Server-side tracking of wizard runs, completion rates, or step drop-offs.

**Why not in v2.0:** We're introducing a local HTTP server and a browser page that holds user secrets. Adding analytics at the same time muddies the trust story ("does the wizard phone home with my API key?"). Any telemetry comes later, separately, after the trust model is simple to audit.

**Lands as:** Not planned. If added later, would be opt-in, aggregate-only, and documented separately.

### 9.7 Windows and container environment support

**What:** Running the wizard on Windows, inside Docker/devcontainers, or inside a Codespace.

**Why not in v2.0:** Browser-based flow assumes `open::that()` can launch a browser that can reach `127.0.0.1:<port>` on the same machine. Inside a container or over SSH-forwarded sessions, that assumption breaks. v2.0 tests and supports macOS and native Linux. Other environments fall through to the `rpassword` prompt renderer (which *does* work everywhere). Windows native and container-mode are validated in follow-up work.

**Lands as:** Follow-up testing matrix. Most container cases Just Work via the `--terminal` fallback; the remaining fix is headless-detection tuning.

### 9.8 Wizard UI theming, branding, i18n, accessibility audit

**What:** NyxID brand assets in the browser page; localization; WCAG audit; dark-mode toggle.

**Why not in v2.0:** The browser page is utilitarian — minimal inline CSS, system fonts, high contrast by default. Pulling in a brand asset pipeline, a locale system, or a full a11y audit would balloon the PR and isn't required for the core safety outcome. The CSS is structured so these can be layered later without touching flows.

**Lands as:** Separate UX-polish PRs once v2.0 has real users.

---

## 10. Decision Record (why this shape)

Captured here instead of a separate ADR because the repo's `docs/` doesn't currently use an ADR layout.

**Decision:** Build v2 from scratch as a declarative step engine behind a shared wizard runtime in the CLI crate. Wizardify **existing commands** (`service add` first), not a new `keys` group. Browser-by-default for interactive TTY invocations; `rpassword` prompts for headless; `--output json` for scripting.

**Alternatives considered and rejected:**

- *Patch v1 in place.* Each of the 13 gaps in the historical spec costs roughly as much as building v2; the patched result would still be per-command bespoke HTML with no scalability story.
- *`ratatui` TUI first.* Adds a substantial dep and a second renderer before validating the step engine. Deferred to a follow-up PR.
- *Introduce a new `nyxid keys` command group.* Splits the natural verb-object naming already in use (`service add`, `api-key create`, `node register-token`). Rejected in favor of editing existing commands.
- *Ship the framework with all flows at once.* Larger PR, longer time to first real-world feedback, more coupled rollback. Rejected in favor of framework + one real flow, then iterate.

**Consequences:**

- Secrets never enter terminal / LLM context on interactive invocations.
- Every existing scripted/CI invocation keeps working unchanged.
- Adding the next flow is a ~one-file PR against a stable framework.
- The CLI gains four new deps (`axum`, `tower`, `tower-http`, `rust-embed`). `axum` is already a workspace dep on the backend side; the others are lightweight.
- One new top-level CLI subcommand (`nyxid wizard`) and one new flag on `service add` (`--terminal`).

---

## 12. Known Debt — Follow-up PRs

v2.0 deliberately ships an incomplete security and UX story so the first flow lands fast. The items below are **valid gaps** surfaced during independent review (see NyxID#351 review notes). Each is accepted as debt and scheduled into a concrete follow-up PR. None are blockers for v2.0; none are quietly waved away.

### 12.1 Locked-down wizard page (strict CSP, no third-party assets)

**Gap:** §4.2 treats CSRF-in-meta as the main defense. If the wizard page has any XSS vector, or loads any remote asset (font, icon, analytics pixel), the meta token and any form field become readable. The real defense is a locked-down static page with zero remote resources and a strict Content Security Policy.

**Debt:** Hard-ban remote resources. Emit a strict CSP header on every wizard response:
```
Content-Security-Policy:
  default-src 'none';
  script-src 'self';
  style-src 'self' 'unsafe-inline';
  img-src 'self' data:;
  connect-src 'self';
  form-action 'none';
  frame-ancestors 'none';
  base-uri 'none';
```

**Lands in:** security-hardening PR, **before any flow beyond AI-keys ships.**

### 12.2 Origin/Host enforcement on proxy

**Gap:** Allowlist is method+path only. An attacker exploiting DNS rebinding or a malicious local page that discovers the ephemeral port can still reach the proxy with a valid CSRF token if the user is tricked into visiting it. `Host` and `Origin` must match the exact loopback origin the server is serving.

**Debt:** Proxy middleware rejects requests whose `Origin` header does not match `http://127.0.0.1:<port>` exactly. Missing `Origin` is also rejected (prevents curl-style exploitation by local processes).

**Lands in:** same security-hardening PR as 12.1.

### 12.3 Parameter-normalized route matching + per-route body validation

**Gap:** `Vec<(Method, &'static str)>` is too coarse for paths with params. Over time, convenience will widen it until the proxy is effectively a tiny unaudited reverse proxy.

**Debt:** Replace the allowlist with a typed shape:
```rust
struct ProxyRoute {
    method: Method,
    path_template: &'static str,  // e.g. "/api/v1/catalog/:slug"
    body_validator: Option<fn(&serde_json::Value) -> Result<()>>,
}
```
Body validation is per-route (struct deserialize with a whitelist of fields before forwarding).

**Lands in:** framework PR, before the second flow lands.

### 12.4 Explicit threat model

**Gap:** The doc implies broader "safe" guarantees than v2.0 actually provides. Hostile browser extensions, hostile local processes, crash dumps, DevTools recordings, browser autofill, and the user's clipboard/password manager are all out of scope — but that is not stated plainly.

**Debt:** Add a top-level "Threat Model" section that enumerates:
- **In scope:** terminal transcript leakage, LLM context-window leakage, scripted non-interactive behavior preservation.
- **Out of scope:** hostile browser extensions, compromised browser, ptrace / crash dumps / process memory reads, user's own clipboard and password manager, shoulder-surfing.
- **Assumptions:** the user's browser and OS user account are trusted; `127.0.0.1` is only reachable by the same OS user.

**Lands in:** doc-only update paired with the security-hardening PR.

### 12.5 Placeholder key cleanup on abandoned OAuth / device-code

**Gap:** The existing `service add` creates a pending `/keys` record *before* the user completes OAuth / device-code (see `cli/src/commands/service.rs:701, 715, 919`). If the user abandons the wizard mid-flow, the placeholder sits in the DB forever. v2 inherits this — closing the tab now orphans a record instead of erroring silently in the terminal.

**Debt:** Two options, pick one:
- (a) Wizard cancel hook calls `DELETE /api/v1/keys/{id}` for any pending key it created.
- (b) Server-side TTL: `pending` keys older than N minutes auto-delete via a background sweep (mirrors how approval requests and registration tokens already work in the codebase).

**Lands in:** backend-side cleanup task — tracked as its own issue, independent of the CLI wizard work.

### 12.6 Browser memory is hygiene, not mitigation

**Gap:** §4.2 #8 ("JS cleanup on completion") is framed as a security fix. It is not. Extensions, DevTools, heap dumps, and browser autofill all retain the secret regardless of what the wizard's JS does. The pasted API key sits in the DOM input field's history, autofill store, and possibly the browser's form-save prompt.

**Debt:** Doc-only: relabel row #8 in the §4.2 table as *hygiene*, not security. Real mitigation is the Threat Model statement from 12.4 — "the browser is trusted."

**Lands in:** same doc-only update as 12.4.

### 12.7 Headless detection beyond `$DISPLAY`

**Gap:** `$DISPLAY` is X11-only. Wayland sessions expose `$WAYLAND_DISPLAY`. Containers, Codespaces, and devcontainers each have their own signals (`/.dockerenv`, `$CODESPACES`, `$REMOTE_CONTAINERS`). The current detection will wrongly open a browser inside a headless environment and hang.

**Debt:** Expanded detection, with a **fail-closed default**: when any signal is ambiguous, fall back to `--terminal` rather than attempting to open a browser. Signals to check: `$WAYLAND_DISPLAY`, `$CODESPACES`, `$REMOTE_CONTAINERS`, `$CONTAINER`, existence of `/.dockerenv`, platform-specific probes on macOS and Windows.

**Lands in:** first follow-up flow PR (whichever ships next). Cheap once there is a real-world testbed.

### 12.8 Browser UX edge cases (tab close, reload, multi-tab, duplicate submit, port conflict)

**Gap:** v2.0 doc does not spec what happens in these cases, which are routine for a local-server browser flow.

**Debt:** Each case gets an explicit behavior in Phase 2, with behavior tests in `cli/tests/wizard_ai_key.rs`:
- **Tab close / browser crash:** CLI times out after 5 min of inactivity, exits `1`, cleans up any placeholder key (ties to 12.5).
- **Reload mid-flow:** wizard restarts from Step 1; no server-side session persistence. Prior paste is discarded.
- **Multi-tab:** second tab connects to the same server and observes the same wizard state (single-session).
- **Duplicate submit:** Connect button disables on click. Backend rejects duplicate slugs cleanly.
- **Port conflict:** retry ephemeral bind up to 3 times, then fail with a clear error message pointing at `--terminal`.

**Lands in:** Phase 2 implementation PR (the one that builds the framework and AI-keys flow).

### 12.9 `DisplayOnce` step type for one-time-secret flows

**Gap:** Current `Display` step has no notion of "this is the only time you'll see this — acknowledge before moving on." Future rotation / MFA / node-token / SSH-cert flows need it. Pretending otherwise (§5 "framework changes must be zero") is the claim Codex called fantasy, fairly.

**Debt:** New step type:
```rust
DisplayOnce {
    template: &'static str,
    copy_hint: Option<&'static str>,
    require_ack: bool,   // disabled Continue until checkbox confirms "I've saved this"
    download_filename: Option<&'static str>,  // offer download for file-ish secrets (SSH cert)
}
```
Renderer: disabled Continue button, checkbox "I've saved this and understand I can't retrieve it later," optional download button for file secrets.

**Lands in:** first rotation flow PR (likely `api-key rotate` + `node rotate-token` together — both are structurally identical).

### 12.10 Factual corrections Codex flagged

**Gaps** (each would mislead Phase-2 implementers):
- §4.3 proxy allowlist lists `POST /providers/:slug/oauth/start` and `.../oauth/callback`. Real routes are `GET /api/v1/providers/{id}/connect/oauth` and `POST /api/v1/providers/{id}/connect/device-code/poll` (see `backend/src/handlers/user_tokens.rs:264, 644, 694`).
- §3.4 marks `Label` as optional. `CreateKeyRequest.label` is required at the backend (`backend/src/handlers/keys.rs:132`); the current CLI auto-fills from catalog/user input.
- §3.5 shows a confirmation slug matching the label. Slugs are auto-generated and may receive a collision suffix (`unified_key_service.rs:27`).
- §3.4 OAuth/device-code flow skips the "create placeholder → open auth → poll until active" contract (`cli/src/commands/service.rs:701, 715, 919`). Current behavior is a three-step pattern, not single-button redirect.
- §3.1 and §7 disagree about when the wizard fires (§3.1: no slug + no credential + TTY etc.; §7: "bare `nyxid service add`" only). What happens with `--oauth` / `--device-code` / `--custom` / `--via-node` with no slug is unspecified.
- §9.5 ("no backend changes") is too absolute. MFA will likely need a dedicated challenge endpoint; the claim should be "no backend changes required for AI-keys flow."

**Debt:** These are documentation errors, not design errors. Each is a one-line edit and should be fixed before the Phase 2 PR is opened (so the implementer is not working from wrong routes / wrong contracts). Tracked as a **doc fix-up PR** immediately after Phase 1 merges, not as implementation debt.

**Lands in:** small doc-fix PR between Phase 1 and Phase 2.

---

## 11. Phase Plan

| Phase | Scope                                                    | PR        |
|-------|----------------------------------------------------------|-----------|
| **1** | This doc (`docs/CLI_WIZARD_V2.md`). No code.             | _current_ |
| **2** | Framework + `ai-key` flow. `nyxid service add` wizardified. Back-compat preserved. | next |
| 3+    | One flow per PR, in the order listed in §2.              | later     |

Phase 2 is not authorized by this doc; it is planned separately once Phase 1 merges.
