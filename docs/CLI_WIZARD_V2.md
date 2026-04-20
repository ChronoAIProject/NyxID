# CLI Wizard v2

- **Status:** Shipped on branch `worktree-cli-wizard-v2`. Pending push + PR.
- **Owner:** CLI team
- **Tracks:** NyxID#351 (supersedes the approach attempted in PR #358)
- **Supersedes:** `~/Desktop/docs/cli-wizard.md` (historical, out-of-repo)

This document is a *current-state* spec — it reflects what's in the branch, not what was originally proposed. Sections that were speculative in earlier revisions have been replaced by a description of the actual implementation.

---

## 1. Why

Every time the NyxID CLI prints a secret — API key, node registration token, TOTP seed, SSH CA cert, agent API key — that secret lands in whatever process is driving the CLI. When the driver is an LLM coding agent (Claude Code, Codex, OpenClaw, Cursor), the secret enters the model's context window, gets prompt-cached, and becomes a persistent leak surface.

The v2 wizard removes that leak:

- Secrets are entered in a local browser page served by the CLI on `127.0.0.1:<ephemeral>`.
- The terminal sees a single `Opening http://…` line, streamed progress, and the CLI's exit status.
- Scripted and CI invocations are unchanged — the wizard only fires under the rules in §3.1.

This is the one outcome v2 must deliver. Everything else is how.

---

## 2. Commands Affected

The wizard edits commands that **already exist**. It does not introduce a new top-level command group.

| Credential                | Existing command               | Backend API                                    | v2.0  |
|---------------------------|--------------------------------|------------------------------------------------|:-----:|
| AI service / API provider | `nyxid service add [slug]`     | `POST /api/v1/keys`                            |  ✅   |
| SSH service               | `nyxid service add-ssh`        | `POST /api/v1/keys` (ssh)                      |       |
| NyxID agent API key       | `nyxid api-key create`         | `POST /api/v1/api-keys`                        |       |
| API key rotation          | `nyxid api-key rotate <id>`    | `POST /api/v1/api-keys/{id}/rotate`            |       |
| Node registration         | `nyxid node register-token`    | `POST /api/v1/nodes/register-token`            |       |
| Node rotation             | `nyxid node rotate-token <id>` | `POST /api/v1/nodes/{id}/rotate-token`         |       |
| MFA / TOTP                | `nyxid mfa setup` + `verify`   | `POST /api/v1/mfa/setup` + `.../verify-setup`  |       |
| OpenClaw setup            | `nyxid openclaw setup`         | `POST /api/v1/keys`                            |       |
| Channel bot               | `nyxid channel-bot register`   | `POST /api/v1/channel-bots`                    |       |

v2.0 ships `service add` only. `service add` covers every flow shape in the catalog — paste-key, header-auth, no-auth, self-hosted with gateway URL, multi-field token exchange, OAuth (with optional client_id/secret sub-step), and device-code with refresh. The other rows are later PRs (§10).

---

## 3. User Flow — `nyxid service add`

### 3.1 Invocation rules

The wizard fires only when **all** of these are true:

- none of the scripted-flow flags are set: `--credential`, `--credential-env`, `--oauth`, `--device-code`, `--custom`, `--auth-method`, `--auth-key-name`, `--scope`, `--org`, `--openapi-spec-url`
- stdout is a TTY
- `--output` is not `json`
- `NYXID_NO_WIZARD` is unset (explicit opt-out)
- `SSH_CONNECTION` and `SSH_TTY` are both unset (SSH sessions can't open a local browser)
- on Linux: at least one of `DISPLAY` / `WAYLAND_DISPLAY` is set

The `slug`, `--label`, `--via-node`, and `--endpoint-url` flags are **prefill-compatible** — they pre-populate the wizard's Step 2 fields instead of forcing the scripted path. When the slug points at a catalog entry, the wizard auto-advances to Step 2 pre-selected.

Any other invocation runs the existing non-interactive path **unchanged**.

### 3.2 Terminal output

The terminal streams concise status while the user is in the browser.

**While the wizard runs:**
```
$ nyxid service add
→ Opening http://127.0.0.1:54213/wizard … (Ctrl-C to cancel)
  Waiting for browser …
```

**On success**, the terminal prints the summary (user never has to switch back to the browser to find the proxy URL):
```
✓ Service 'work-openai' created.
  Slug:      work-openai
  Proxy URL: https://nyx-api.chrono-ai.fun/api/v1/proxy/s/work-openai/

  Next:
    curl https://nyx-api.chrono-ai.fun/api/v1/proxy/s/work-openai/<api-path> -H "Authorization: Bearer $NYX_KEY"
  Example: append /v1/models for OpenAI-compatible providers.
```

A `BEL` (`\x07`) + OSC-9 growl notification is emitted at completion / cancel / timeout so terminals like iTerm2, WezTerm, and Kitty pop a notification. Plain terminals just see a dock-bounce.

**On cancel (tab close or Cancel button):** `✗ Wizard cancelled. No service was created.`, exit 1.

**On timeout (30-min overall ceiling):** `✗ Wizard timed out. No service was created.`, exit 1, with a hint pointing at the scripted form.

### 3.3 Browser — Step 1: pick a service

All 29 catalog entries are shown in one grid. A small type badge in the top-right of each card indicates the flow shape (`OAuth`, `Device code`, `SSH`) so the user knows what they're picking before committing. No greyed-out / disabled cards — every card is clickable; clicking a shape the wizard doesn't drive (SSH at time of writing) shows a fallback notice with a copyable scripted-CLI command.

Grid layout: exactly 2 columns, 2 rows visible at once (= 4 cards at a time). Extra cards scroll inside the grid via a 2×2 viewport cap at `min(132px, 18vh)` per row. The page itself never grows beyond one viewport.

Search input filters cards live by slug / name substring.

A single "Custom / self-hosted…" card lives in an Advanced section below the main grid. It's a planned power-user escape hatch; v2.0 surfaces it but punts its form to a follow-up (clicking it shows a notice with the scripted command today).

The header at the top-left is the NyxID brand wordmark: 36 px SVG mark + "NyxID" in DM Serif Display (embedded WOFF2, served from `127.0.0.1`, no remote fonts).

### 3.4 Browser — Step 2: credential (shape-dispatched)

Step 2 renders dynamically based on the catalog entry's shape. Every form shares a **label** field on top (required, pre-filled from the slug, overridable via `--label` prefill) and a primary purple left-bar accent on the one field that matters most for that shape.

| Shape           | Detection                                                | Form |
|-----------------|----------------------------------------------------------|------|
| `paste-key`     | default — bearer / header / path / query / bot_bearer   | Label + API key (password input, show/hide toggle, paste hint from catalog `api_key_instructions` / `api_key_url` / `documentation_url`) |
| `gateway-url`   | `requires_gateway_url === true` (e.g. OpenClaw)         | Label + required Gateway URL + API key |
| `token-exchange`| `token_exchange_credential_fields` non-empty            | Label + N fields from the catalog spec (`app_id`, `app_secret`, etc., each tagged secret/visible) |
| `no-auth`       | `requires_credential === false`                         | Label + "1-click connect — no credential needed" panel |
| `oauth`         | `provider_type === "oauth2"`                            | Two sub-steps (see below) |
| `device-code`   | `provider_type === "device_code"`                       | Device-code panel with code + URL + Copy / Refresh / Open |
| `ssh`           | `service_type === "ssh"`                                | Fallback notice with `nyxid service add-ssh` copy-command |

#### OAuth — sub-step A (conditional)

When the catalog entry's `credential_mode` is `user` or `both`, the wizard inserts a pre-sign-in step asking for the user's OAuth app credentials:

- Client ID (required)
- Client Secret (required, password input, purple primary accent)
- "📖 How to create an OAuth app" link pulled from the catalog entry's `documentation_url`

On Connect, `PUT /api/v1/providers/:provider_id/credentials` with `{client_id, client_secret, label}`. On success the form re-renders as sub-step B. Back button returns from B → A so the user can edit.

When `credential_mode === "admin"` or `system`, sub-step A is skipped — the wizard jumps straight to sub-step B.

#### OAuth — sub-step B (sign-in)

A short info panel ("Sign in with X") + the same docs link. On Connect:
1. `POST /api/v1/keys` to create a placeholder (status `pending_auth` if the admin hasn't pre-authorized; `active` immediately if `credential_mode=admin`).
2. If the placeholder came back `active` (admin-mode), skip straight to Step 3 confirmation.
3. Otherwise, `GET /api/v1/providers/:provider_id/connect/oauth?redirect_path=/keys/:key_id` → returns `{authorization_url}`.
4. `window.open(authorization_url, "_blank")` in a new tab so the wizard tab stays alive.
5. Poll `GET /api/v1/keys/:key_id` every 2 s until `status === "active"` (5-min deadline).

#### Device-code

`POST /api/v1/providers/:provider_id/connect/device-code/initiate` returns `{user_code, verification_uri, state, interval}`. The panel renders with:

- Purple-accented info panel with the user code in a monospace badge
- **Copy** button (tiny, subtle-purple styling)
- **↻ Refresh code** button (bumps `deviceCodeGen`, re-initiates, restarts polling without leaving Step 2)
- **Open** button (opens the verification URI in a new tab — NOT auto-opened; user clicks when ready)

The wizard polls `POST /api/v1/providers/:provider_id/connect/device-code/poll` with `{state}` at the interval the backend returns. On `slow_down`, it bumps the interval. On `expired`, the panel flips to a "Code expired — Get a new code" state; the outer promise stays pending so the user can refresh in place. On `denied`, the outer promise rejects with a clear error. On `complete` / `authorized` / `access_token`, the wizard fetches `GET /api/v1/keys/:key_id` and advances to Step 3.

**10-min overall ceiling** per session (more generous than OAuth's 5-min because device code naturally takes longer — you're entering a code on another device).

### 3.5 Browser — Step 3: confirmation

```
┌────────────────────────────────────────────────────────────────┐
│  ✓ Service created                                             │
│                                                                │
│  Slug:        work-openai                                      │
│  Label:       work-openai                                      │
│  Proxy URL:   https://nyx-api.chrono-ai.fun/api/v1/proxy/s/   │
│               work-openai/             [ Copy ]                │
│                                                                │
│  Try it:                                                       │
│  curl <proxy-url>/<api-path> \                                 │
│    -H "Authorization: Bearer $NYX_KEY"                         │
│  # e.g. <api-path> = v1/models for OpenAI-compatible providers │
│  [ Copy curl ]                                                 │
│                                                                │
│                       [ Done — return to terminal ]            │
└────────────────────────────────────────────────────────────────┘
```

- The slug may have a collision-suffix appended by the backend (`work-openai-2` etc.); the wizard displays whatever `POST /keys` returned.
- The raw API key is never re-shown.
- **Copy proxy URL** and **Copy curl** buttons carry the subtle-purple secondary-button styling (border 22% primary, text 40% primary, hover bumps to 80% / primary).
- **Done — return to terminal** is the single primary CTA.

On Done click: `POST /api/proxy/complete` → the CLI prints its terminal summary (§3.2), rings the bell, shuts down the local server, exits 0. The browser tab then shows a full-screen **green overlay** ("Service complete — it is safe to close the browser now") with the user's terminal-switching hint.

### 3.6 FE ↔ CLI handoff contract

**Browser → CLI (all require `X-Wizard-CSRF` header):**

| Endpoint                          | Fires when                                | CLI reaction                             |
|-----------------------------------|-------------------------------------------|------------------------------------------|
| `POST /api/proxy/heartbeat`       | Every 3 s while tab is visible            | Resets the inactivity timer              |
| `POST /api/proxy/cancel`          | Cancel button clicked                     | Print "✗ Wizard cancelled", exit 1       |
| `POST /api/proxy/cancel-unload`   | `beforeunload` via `navigator.sendBeacon` | Same as cancel (no-op if busy — see below) |
| `POST /api/proxy/complete`        | User clicks Done                          | Print success summary, exit 0            |
| `POST /api/proxy/abandon-placeholder` | User cancels or closes tab with a pending OAuth/device-code key in flight | Server-side GET-then-conditional-DELETE of the placeholder key |
| `GET /api/proxy/status`           | Not currently used (reserved)              | Returns `{state: "running", uptime_s}`   |

**Heartbeat-based disconnect detection:** the browser sends heartbeats every 3 s. On two consecutive failures (~6 s), the browser shows a full-screen **amber "Wizard disconnected"** overlay telling the user to close the tab and re-run `nyxid service add`. Covers the case where the user Ctrl-C'd the CLI, closed the terminal, or the CLI crashed — instead of sitting there looking functional, the tab signals the user to move on.

**Tab-close during an in-flight POST:** `handle_cancel_unload` refuses to shut the server down while `in_flight_mutations > 0`, so a mid-flight `POST /api/v1/keys` completes cleanly rather than racing a server shutdown and producing an orphan.

**CLI watchdog (server-side):** on the CLI's end, `HEARTBEAT_STARTUP_GRACE = 8 s` + `HEARTBEAT_DEAD_AFTER = 22 s`. If no heartbeat arrives for 22 s after the startup grace period, the CLI declares the browser dead and exits with the cancel message.

**Overall ceiling:** 30 min from server start. Catches walked-away tabs.

**Ctrl-C in the terminal:** gracefully shuts down the axum server, the browser's next heartbeat fails twice → disconnected overlay.

### 3.7 Access-token refresh

The proxy attaches the CLI's cached access token to every forwarded request. When the backend returns `401 token_expired` (access tokens have a 15-minute TTL per `JWT_ACCESS_TTL_SECS`), the wizard's proxy transparently:

1. Reads the saved refresh token for the CLI profile via `crate::auth::read_saved_refresh_token_for`.
2. `POST {base}/api/v1/auth/refresh` with `{refresh_token}`.
3. On 200 success: persists the new `{access_token, refresh_token}` pair via `crate::auth::save_tokens_for` (so subsequent terminal `nyxid` commands pick them up too), updates the in-memory token, retries the original request with the new bearer, and returns that response to the browser.
4. On refresh failure (no saved refresh token, refresh endpoint 4xx, refresh token itself expired): falls through to the original 401, which the browser surfaces as a clear re-login message.

Mirrors `ApiClient::try_refresh_token` in `cli/src/api.rs` exactly. Concurrent 401s from parallel proxy requests serialize on a mutex around the token so only one `/auth/refresh` call fires.

### 3.8 Headless / scripted fallback

Scripted use (`--credential-env`, `--oauth`, `--device-code`, `--output json`, etc.) bypasses the wizard entirely and hits the existing scripted path in `cli/src/commands/service.rs`. Byte-identical to pre-wizard behavior.

SSH / no-display invocations fall through the §3.1 gate and run the scripted path's existing `rpassword`-backed prompts (no new renderer added in v2.0).

---

## 4. Architecture

### 4.1 Actual file tree

```
cli/src/wizard/
├── mod.rs                    entry: run_ai_key_wizard(auth, prefill)
├── server.rs                 axum server (bind, proxy, lifecycle, refresh)
└── assets/                   embedded via rust-embed, served from 127.0.0.1
    ├── wizard.html           single-page shell + all overlays
    ├── wizard.js             ~1500 lines of hand-rolled vanilla JS
    ├── wizard.css            design-token-driven CSS
    ├── nyxid-logo.svg        brand mark (2.8 KB)
    └── fonts/
        ├── dm-serif-display-400.woff2  wordmark / titles (17 KB)
        └── OFL.txt                     license attribution
```

No separate runtime / renderer / flows crates. The whole flow is one `async fn` in `mod.rs` that builds a `ProxyContext`, hands it to `server::run_flow`, and waits on a `WizardOutcome` channel. `server.rs` is the entire server + proxy + refresh logic. `wizard.js` is the entire UI.

**Trade-off:** deliberately *not* the declarative step-engine described in earlier drafts. The ai-key flow is intricate enough (seven shape branches, two OAuth sub-steps, device-code refresh, overlay state machine) that a declarative engine would have been all abstraction, no leverage. Adding the next wizard flow will require duplicating some JS / adding new allowlist routes — accepted cost.

### 4.2 Visual parity with the frontend

The wizard's browser UI *visually mirrors* the frontend `AddKeyDialog` (`frontend/src/components/dashboard/add-key-dialog.tsx`) — same card grid, same sub-steps, same badge pattern, same color palette — but every byte is hand-rolled vanilla HTML/CSS/JS served locally from `127.0.0.1:<port>`.

Shared design tokens:
- Primary: `#8b5cf6` (Void 500) / `#7c3aed` (Void 600 in light mode) — matches the frontend's `--color-primary`
- Wordmark: `#c4b5fd` (Void 300)
- Font: DM Serif Display 400 (wordmark + Step 2 title)
- Body: system sans-serif stack (no body-font embed)

**Why not redirect to the frontend?** Serving the wizard from the frontend origin would put API-key form inputs on a page we don't control at render time (subject to whatever CSP / third-party assets the frontend pipeline ships). For a form whose whole reason-to-exist is secret handling, we need the page bytes frozen at CLI-release time, served from `127.0.0.1` with a strict CSP we own.

### 4.3 Security

| Surface                               | Implementation |
|---------------------------------------|----------------|
| Same-origin                           | Everything served from `http://127.0.0.1:<ephemeral>`. No remote fetches from the page. |
| CSP                                   | `default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; font-src 'self'; form-action 'none'; frame-ancestors 'none'; base-uri 'none'`. Emitted on every response from `server.rs::base_security_headers`. |
| CSRF                                  | 32-byte random token minted per session, substituted into `<meta name="wizard-csrf">` server-side. Required on every `POST/PUT/PATCH/DELETE` to `/api/proxy/*`. Constant-time compared. |
| Origin enforcement                    | Mutating proxy requests require `Origin: http://127.0.0.1:<bound_port>` exactly (port-literal match, not prefix). GET requests relax this because Chrome omits Origin on same-origin GET. `Host` header is also validated. |
| Proxy allowlist                       | Typed `ProxyRoute { method, path_template, body_fields }` structs. Path parameters (`:slug`, `:key_id`, `:provider_id`) match any non-empty segment; everything else must match literally. Query strings forwarded untouched. |
| Body whitelist                        | `body_fields` on each route enumerates permitted top-level JSON keys. Unknown keys → 400 Bad Request. Prevents a compromised wizard page smuggling privileged fields (`target_org_id`, `identity_propagation_mode`, SSH flags) through `POST /keys`. |
| Bearer token scope                    | Lives only in CLI process memory (wrapped in `Arc<tokio::sync::Mutex<String>>` so refresh can mutate it). Browser never sees it. |
| Access-token refresh                  | On upstream 401, proxy transparently refreshes via `/api/v1/auth/refresh` using the saved refresh token for the profile, persists the new pair to `~/.nyxid/`, retries once. §3.7. |
| Placeholder cleanup                   | `POST /keys` responses with `status=pending_auth` are tracked server-side. On Cancel / unload / CLI shutdown, each tracked key gets a GET-then-conditional-DELETE — revokes only if still pending, can't accidentally delete a key that just flipped to active. |
| In-flight mutation guard              | `cancel-unload` refuses to shut the server down while a mutating upstream request is open. Prevents tab-close-mid-POST races. |
| Upstream timeout                      | reqwest client has `connect_timeout(10s)` + total `timeout(60s)`. Slow backend can't strand the user. |

**Threat model:**

In scope:
- Terminal transcript leakage (stdout piping, `script` recording, `tmux` scrollback)
- LLM context-window / prompt-cache leakage
- Preservation of existing scripted / CI invocations

Out of scope (explicitly):
- Hostile browser extensions with DOM access — they can read the CSRF meta tag and any input
- Compromised browser or OS user account
- Process memory reads (ptrace, crash dumps)
- User's own clipboard / password manager / browser autofill prompts
- Shoulder-surfing

Assumption: the user's browser and OS user account are trusted. `127.0.0.1` is only reachable by the same OS user.

### 4.4 Proxy allowlist

**Forwarded to NyxID backend** (all require CSRF, mutating methods also require exact-origin match, all body payloads validated against the route's `body_fields` whitelist):

```
GET  /api/v1/catalog
GET  /api/v1/catalog/:slug
POST /api/v1/keys              body: service_slug, credential, label,
                                     endpoint_url, slug, auth_method,
                                     auth_key_name, openapi_spec_url
GET  /api/v1/keys/:key_id
PUT  /api/v1/providers/:provider_id/credentials        body: client_id, client_secret, label
GET  /api/v1/providers/:provider_id/connect/oauth
POST /api/v1/providers/:provider_id/connect/device-code/initiate
POST /api/v1/providers/:provider_id/connect/device-code/poll  body: state
```

**Handled locally by the CLI** (no upstream call, still CSRF-required):

```
POST /api/proxy/heartbeat
POST /api/proxy/cancel
POST /api/proxy/cancel-unload
POST /api/proxy/complete
POST /api/proxy/abandon-placeholder    body: key_id
GET  /api/proxy/status
```

`DELETE /api/v1/keys/:key_id` is **intentionally not** in the allowlist — placeholder cleanup goes through the local `abandon-placeholder` endpoint, which does the GET-then-conditional-DELETE server-side so an accidentally-active key can't be revoked.

Any other `method+path` on `/api/proxy/*` → `403 Forbidden`.

---

## 5. Command Surface

- **Primary entry:** `nyxid service add` (edited). Wizard fires under §3.1 rules.
- **Prefill flags:** `<slug>` positional, `--label`, `--via-node`, `--endpoint-url` all pre-populate the form. The wizard auto-advances to Step 2 when a valid catalog slug is prefilled.
- **No new subcommands.** The earlier `nyxid wizard` alias was removed — the natural verb-object command is the only entry point.
- **No new flags on `service add`.** Everything that used to work still works.

---

## 6. Non-Interactive Contract

`nyxid service add` in scripted mode behaves **exactly** as it did before the wizard landed. These forms all bypass the wizard:

```
nyxid service add llm-openai --credential-env OPENAI_KEY --label work-openai
nyxid service add llm-openai --oauth
nyxid service add llm-openai --device-code
nyxid service add --custom --endpoint-url https://api.example.com/v1 --credential-env KEY
nyxid service add llm-openai --output json --credential-env OPENAI_KEY
nyxid service add | cat      # piped — not a TTY
NYXID_NO_WIZARD=1 nyxid service add
ssh remote 'nyxid service add'
```

The wizard is a pure addition for bare-ish `nyxid service add` in a local TTY.

---

## 7. Verification

Manual tests (run each against prod `https://nyx-api.chrono-ai.fun` after `nyxid login`):

1. **Bare happy path.** `nyxid service add` → Step 1 catalog (29 cards), pick OpenAI, paste disposable key, Done. Terminal prints summary, service appears in `nyxid service list`.
2. **Prefill.** `nyxid service add llm-anthropic --label test-anth` opens directly on Step 2 for Anthropic with `test-anth` in the label field.
3. **OAuth (user-mode credentials).** `nyxid service add api-github` → sub-step A (Client ID + Client Secret + docs link) → PUT `/providers/:id/credentials` → sub-step B (Sign in with GitHub) → new tab opens, user authorizes, wizard polls → Step 3.
4. **OAuth (admin-mode).** `nyxid service add llm-openai-codex` → POST /keys returns `status=active` immediately → wizard skips device-code entirely → Step 3 confirmation.
5. **Device code + refresh.** For a device-code provider with `credential_mode=user` (none in prod today but hypothetically), code panel appears → click ↻ Refresh code → new code issued, polling restarts cleanly.
6. **Disconnect detection.** Open wizard, Ctrl-C the CLI. Within ~6 s the browser shows the amber ⚠ Wizard disconnected overlay.
7. **Token refresh.** Let the wizard sit until the access token expires (15 min). The next form action triggers `/auth/refresh` transparently; user sees no 401 banner. Terminal stderr logs `[wizard] refreshed expired access token`.
8. **Cancel via tab close.** Close the wizard tab. CLI prints `✗ Wizard cancelled.`, exits 1 within ~22 s (heartbeat watchdog).
9. **Cancel via Cancel button.** Click Cancel on Step 1 → CLI exits 1 immediately.
10. **Back-compat.** `nyxid service add llm-openai --credential-env OPENAI_KEY --label work-openai` runs the scripted path, byte-identical to pre-wizard.
11. **JSON.** `nyxid service add --output json` bypasses the wizard and errors on missing fields as before.
12. **Leak audit.** `script -q /tmp/session.log nyxid service add` → `grep -E 'sk-|nyxid_' /tmp/session.log` returns nothing (terminal never sees the secret).

Automated tests:

- `cli/src/wizard/server.rs` unit tests: allowlist routing, body-whitelist validation, origin enforcement, CSRF mismatch → 403.
- A pty-driven harness (`NYXID_WIZARD_NO_OPEN=1 python3 pty.fork exec nyxid`) spins up the wizard locally, curls every allowlisted route, asserts headers. Run during dev via the hand scripts in this repo's history (not yet extracted into a proper test file).

---

## 8. Out of Scope

v2.0 deliberately excludes:

1. **Wizardifying the other commands** in §2 (SSH, api-key, node-token, MFA, openclaw, channel-bot). Each is a separate follow-up PR.
2. **ratatui TUI renderer.** Would add deps + second renderer matrix before validating the current flow. Text-prompt fallback lives in the existing scripted path.
3. **`service update` / `service rotate-credential` flows.** Rotation needs a "one-time secret display" UX (acknowledge-before-proceed + optional download) that doesn't exist yet.
4. **Windows native support.** Not tested. `open::that()` claims to work but CSP / font / CSRF surface hasn't been validated there.
5. **Containers / Codespaces / devcontainers.** `§3.1` gate detects SSH + missing DISPLAY/WAYLAND, so containerized invocations fall through to the scripted path. Explicit container-native support is future work.
6. **Telemetry / usage analytics.** Intentionally not phoning home during a secret-handling flow.
7. **i18n / a11y audit / theming.** The page is utilitarian; polish comes after real users.

---

## 9. Decision Record

**Decision:** Build a single hand-rolled wizard behind one `nyxid service add` entry point. Browser-by-default for interactive TTY invocations; existing scripted paths for everything else. Served locally from the CLI binary so the trust boundary is a local loopback, not a cross-origin redirect.

**Alternatives considered and rejected:**

- *Redirect to the frontend's `/keys` page with a callback URL.* Would give us the full frontend UX for free, but puts secret inputs on a page we don't control at render time. The whole point of the wizard is tightening the trust boundary, not loosening it.
- *Declarative step engine with a typed `WizardStep` enum.* Tried the shape on paper — the ai-key flow has enough shape-branches and sub-step state (OAuth credentials, device-code refresh, overlay state machine) that a declarative engine was all abstraction, no leverage. Hand-rolled JS won for this milestone. If the next flow (api-key rotation) can reuse 70 %+ of the form scaffolding, we'll factor it into shared helpers at that point — not ahead of a real second flow.
- *ratatui TUI as a first-class renderer.* Adds a second full renderer matrix. Punted to a future PR once the browser UX has real miles.
- *New `nyxid keys` command group.* Splits the natural verb-object naming already in use. Rejected in favor of editing `service add`.

**Consequences:**

- Secrets never enter the terminal / LLM context on interactive invocations.
- Every existing scripted / CI invocation keeps working unchanged.
- CLI binary grew by ~40 KB for the embedded font + logo + rust-embed overhead.
- Four new deps in the CLI crate: `axum`, `tower`, `tower-http`, `rust-embed`, `constant_time_eq` (axum is already a workspace dep on the backend side; others are lightweight).
- 1-2 full working days of additional polish was spent on UX iteration (grid sizing, wordmark font, button accents, disconnect detection, token refresh) *after* the happy path was working. Future flows will benefit from that shared baseline.

---

## 10. Known Debt / Follow-up PRs

Items accepted as debt, not shipping blockers. Each is a concrete follow-up.

### 10.1 Windows + container matrix

Not tested on Windows, Docker, Codespaces, or devcontainers. The §3.1 gate should catch most container cases via missing DISPLAY/WAYLAND on Linux, but Windows + Codespaces need explicit validation. **Lands in:** a follow-up testing PR once someone has access to all three environments.

### 10.2 `DisplayOnce` step type for rotation flows

Future rotation / MFA / node-token / SSH-cert flows need "secret shown exactly once, acknowledge-before-continue, optional download-as-file" semantics that today's confirmation step doesn't express. **Lands in:** first rotation flow PR (probably `api-key rotate` + `node rotate-token` together).

### 10.3 QR code rendering for MFA

TOTP setup needs the seed rendered as a scannable QR in addition to the text code. Ships with the MFA flow PR. Proposed: `qrcode` crate → inlined SVG in the `Display` template.

### 10.4 Telemetry opt-in

Currently zero. If we add it, must be aggregate-only, opt-in, and documented separately from the secret-handling trust story.

### 10.5 Test harness extraction

The pty-driven validation script lives in shell snippets in commit messages / dev notes rather than a proper `cli/tests/wizard_*.rs`. Should be extracted before adding the second wizard flow so regressions get caught automatically.

### 10.6 Custom / self-hosted form

The "Custom / self-hosted…" card on Step 1 currently shows a fallback notice pointing users to the scripted CLI for custom endpoints. Users wanting non-catalog HTTP endpoints have to `nyxid service add --custom …` for now. **Lands in:** a separate UX PR once there's demand.

### 10.7 Agent-binding step

Earlier drafts proposed a post-creation "bind to specific NyxID API keys" step. Not built in v2.0 — default behavior (any NyxID key routes to any of the user's services) is good enough for most users, and per-agent isolation is handled by the existing `nyxid api-key bind` command. **Lands in:** a later PR if demand emerges.
