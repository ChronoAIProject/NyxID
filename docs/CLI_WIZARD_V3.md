# CLI Wizard v3 — DisplayOnce

- **Status:** Shipped on branch `feature/cli-wizard-v3-display-once`. Pending push + PR.
- **Owner:** CLI team
- **Builds on:** [`CLI_WIZARD_V2.md`](./CLI_WIZARD_V2.md) — read v2 first; v3 reuses every v2 invariant.
- **Tracks:** v2 §10.2 follow-up (`DisplayOnce` step type for rotation flows).

This document is a *current-state* spec — it reflects what's in the branch.

---

## 1. Why

Two CLI commands have always printed a freshly-generated secret straight to stdout:

```
$ nyxid api-key rotate coding-agent
Key rotated!
New Key: nyxid_ag_2f31a7…  (save this -- shown only once)

$ nyxid node rotate-token edge-tokyo
Node token rotated.
New Token: nyx_nauth_8c1d…  (save this -- shown only once)
```

When the driver is an LLM coding agent, those secrets land in the model's context window and prompt cache — the same leak surface v2 closed for `service add`. v3 extends the v2 wizard primitive to cover both rotation flows.

The new shape is **DisplayOnce**: the BACKEND generates the secret, the wizard renders it once in the browser (masked, click-to-reveal, copy, download as `.txt`), the user clicks "I have saved this", and the CLI exits with a no-secret summary line.

This document is also honest about what v3 does NOT promise (see §3.5).

---

## 2. Commands Affected

| Credential                | Existing command               | Backend API                                    | v3.0  |
|---------------------------|--------------------------------|------------------------------------------------|:-----:|
| API key rotation          | `nyxid api-key rotate <id>`    | `POST /api/v1/api-keys/{id}/rotate`            |  ✅   |
| Node token rotation       | `nyxid node rotate-token <id>` | `POST /api/v1/nodes/{id}/rotate-token`         |  ✅   |
| NyxID agent API key       | `nyxid api-key create`         | `POST /api/v1/api-keys`                        |       |
| Node registration         | `nyxid node register-token`    | `POST /api/v1/nodes/register-token`            |       |
| Channel bot               | `nyxid channel-bot register`   | `POST /api/v1/channel-bots`                    |       |
| MFA / TOTP                | `nyxid mfa setup` + `verify`   | `POST /api/v1/mfa/setup` + `.../verify-setup`  |       |

v3.0 ships the two rotation flows. The other rows are follow-up PRs:

- **v3.1:** `api-key create` (needs a scope picker — see [v3.0 follow-up notes](#10-known-debt--follow-up-prs)) + `node register-token` + `channel-bot register`.
- **v3.2:** `mfa setup` (also needs the §10.3 QR-code work flagged in v2).

---

## 3. User Flow — `nyxid api-key rotate` and `nyxid node rotate-token`

### 3.1 Invocation rules

The wizard fires only when **all** of these are true:

- `--output` is not `json` (TTY caller, not a script)
- stdout is a TTY
- `crate::wizard::is_wizard_eligible()` returns true:
  - `NYXID_NO_WIZARD` is unset
  - `SSH_CONNECTION` and `SSH_TTY` are both unset
  - on Linux: at least one of `DISPLAY` / `WAYLAND_DISPLAY` is set

These mirror v2 §3.1. Any other invocation runs the existing scripted path **byte-identical to pre-wizard behavior** — `--output json`, piped stdout, `NYXID_NO_WIZARD=1`, SSH sessions, headless containers all keep the old in-terminal output.

### 3.2 Terminal output

While the wizard runs, the terminal sees concise status (same as v2):

```
$ nyxid api-key rotate coding-agent
→ Opening http://127.0.0.1:54213/wizard?resource_id=…&display_name=coding-agent … (Ctrl-C to cancel)
  Waiting for browser …
```

**On success** — the secret is NEVER in this output:

```
✓ API key 'coding-agent' rotated. New value was shown in the browser.
  ID: 5e2c1f3a-…
  The previous key is now revoked.
```

For node rotation:

```
✓ Node 'edge-tokyo' token rotated. New auth token + signing secret were shown in the browser.
  ID: 8d12-…
  Restart the node agent with the new credentials:
    nyxid node rekey --auth-token <token-from-browser> --signing-secret <hex-from-browser>
  The previous token is now revoked.
```

The placeholders in the rekey line are intentional — we won't restate the secret here. The user copies it from the browser tab.

**On cancel / timeout** — the messaging is honest about server-atomic rotation:

```
✗ Rotation cancelled.
  If the new API key value was shown in the browser, the rotation already happened on the server.
  If you saved it, you're done. If not, run `nyxid api-key rotate <id>` again to issue a fresh value.
```

This is the load-bearing UX wrinkle (see §3.5).

### 3.3 Browser — Step 1: confirm rotate

```
┌────────────────────────────────────────────────────────────────┐
│  Rotate API key 'coding-agent'                                 │
│                                                                │
│  The current value will stop working immediately. The new      │
│  value will be shown once on the next screen — make sure you   │
│  have somewhere to save it (password manager, vault, paper)    │
│  before continuing.                                            │
│                                                                │
│  ID  │ 5e2c1f3a-…                                              │
│                                                                │
│                       [ Cancel ]   [ Rotate now ]              │
└────────────────────────────────────────────────────────────────┘
```

Clicking "Rotate now" POSTs `/api/v1/api-keys/:id/rotate` (or `/api/v1/nodes/:id/rotate-token`) through the local proxy. Empty body (the body validator rejects any non-empty body for empty-body routes). On success, the response is held in browser memory and the panel transitions to DisplayOnce.

### 3.4 Browser — Step 2: DisplayOnce

```
┌────────────────────────────────────────────────────────────────┐
│  🔑  Save the new API key                                      │
│                                                                │
│  This is the only time you'll see the new value. Copy it or    │
│  download it before clicking acknowledge — once acknowledged,  │
│  this page won't show it again.                                │
│                                                                │
│  NEW API KEY                                                   │
│   •••••••••••••••••••••  [ Reveal ]  [ Copy ]                 │
│                                                                │
│  [ Download as .txt ]                                          │
│                                                                │
│  ⚠ Once you click I have saved this, this page won't show     │
│    the value again. Your old key is already revoked on the     │
│    server.                                                     │
│                                                                │
│            [ I have saved this — close ]                       │
└────────────────────────────────────────────────────────────────┘
```

For node rotation, two secret rows render (`auth_token` and `signing_secret`); the `.txt` download bundles both with a header line and the `nyxid node rekey ...` template line so the user can paste it directly.

**Display security:**

- Secret values live in JS-local closure variables. The DOM holds a `•` mask string in its text node until the user clicks Reveal; revealing flips the text content to the real value.
- **Auto-remask on `blur` and `visibilitychange → hidden`.** The moment the user can't see the page, the mask returns. There is no time-based auto-remask while the page stays focused (would just annoy users transcribing).
- **Download via `Blob` + `URL.createObjectURL()` + `revokeObjectURL()`.** `data:` URLs leak into browser history / crash logs / referrer surfaces; Blob handles are revocable and same-origin opaque.
- The secret is rendered as a `<code>` element, not `<input type="password">`, so password managers don't try to capture it as a saved credential.

### 3.5 Honesty about the contract

v3 deliberately ships a weaker security contract than the original draft:

- **The rotation is server-atomic.** Once the rotate POST succeeds, the old value is dead and the new value exists on the server. We cannot undo this from the CLI.
- **The CLI cannot enforce "ack before shutdown."** If the JS tab dies, the heartbeat watchdog fires before the user clicks ack, or the user closes the tab without saving — the rotation already happened. The CLI prints a cancel summary that says so plainly.
- **The heartbeat watchdog is a heuristic, not a guarantee.** Rotation flows use `HEARTBEAT_DEAD_AFTER_ROTATION = 60s` (vs 22s for ai-key). That gives users time for a quick alt-tab into a password manager. >60s of heartbeat silence still triggers cancel — at which point the secret is still rendered in the browser tab and recoverable from there until the user closes it, but the CLI exits with a "cancelled" status.

The defenses we DO have are about leak prevention, not interaction safety:

- The `/api/proxy/complete` body for rotation flows is parsed through a typed Rust struct (`RotationAckPayload`) with `#[serde(deny_unknown_fields)]`. Any field beyond `acknowledged` and `resource_id` is rejected with 400 server-side, so the browser cannot smuggle the secret back through the completion path.
- The terminal summary printer uses an explicit field allowlist (`id`, `name`, `message`) — never `Debug`-formats the payload, never reads anything secret-shaped.
- The body-field validator on rotation routes rejects any non-empty body bytes (rotation endpoints take no JSON).

### 3.6 FE ↔ CLI handoff contract

v3 reuses v2's lifecycle endpoints as-is. Same heartbeat (every 3s, watchdog-killable), same `/cancel` and `/cancel-unload`, same CSP / CSRF / Origin pin / body whitelist. The only changes are:

- **`POST /api/proxy/complete` body shape is per-flow.** AiKey flow: untyped `Value` (existing behavior). Rotation flows: typed `RotationAckPayload` with `#[serde(deny_unknown_fields)]`.
- **`HEARTBEAT_DEAD_AFTER_ROTATION = 60s`** for rotation flows; `HEARTBEAT_DEAD_AFTER = 22s` for ai-key (unchanged).

No new endpoints. No `WizardPhase` server-side state machine.

---

## 4. Architecture

### 4.1 File tree

```
cli/src/wizard/
├── mod.rs                    entry: run_api_key_rotate_wizard, run_node_rotate_token_wizard
│                             types: RotatePrefill, RotationAckPayload, WizardOutcome
├── server.rs                 axum server (existing) + new FlowKind variants + per-flow handle_complete dispatch
└── assets/                   embedded via rust-embed, served from 127.0.0.1
    ├── wizard.html           one new <section> for confirm-rotate, one for display-once
    ├── wizard.js             top-level FLOW dispatch; rotation flow ~280 lines
    ├── wizard.css            new rules for .wizard-secret-row + .wizard-warn-banner
    └── (logo + fonts unchanged)

cli/src/commands/
├── api_key.rs                Rotate arm: gate + handoff to wizard. Scripted path unchanged.
│                             find_key_by_name now refuses on ambiguous names.
└── node.rs                   RotateToken arm: same pattern.
```

### 4.2 Visual parity with v2

Same brand wordmark (NyxID in DM Serif Display), same purple accent (`#8b5cf6` / `#7c3aed`), same overlay system (✓ success / ✗ cancel / ⚠ disconnect). The DisplayOnce panel uses the same `.wizard-detail-list` shape for the metadata row and the same tiny-button styling for Reveal / Copy / Download.

### 4.3 Security

| Surface                               | Implementation |
|---------------------------------------|----------------|
| Same-origin                           | Unchanged from v2 — everything served from `http://127.0.0.1:<ephemeral>`. |
| CSP / CSRF / Origin pin / Host check  | Unchanged from v2. |
| Body whitelist                        | Existing `body_fields: &[]` enforcement at server.rs:526 rejects any non-empty body for rotation routes. |
| **Typed completion payload** (NEW)    | Rotation flows post `{ acknowledged: true, resource_id }` to `/api/proxy/complete`. Parsed into `RotationAckPayload` with `#[serde(deny_unknown_fields)]`. Server returns 400 if any other field is present. The struct's `Debug` impl can only print fields it holds — a future `tracing::debug!("outcome: {:?}", outcome)` is safe. |
| **Field-allowlist printer** (NEW)     | `print_rotation_summary` reads ONLY the resolved `display_name` (CLI-side, pre-wizard) and `ack.resource_id` (validated UUID-ish). Never `to_string(&payload)`, never `{:?}`. Belt-and-suspenders with the typed payload. |
| **Heartbeat bump** (NEW)              | `HEARTBEAT_DEAD_AFTER_ROTATION = 60s` gives users time to alt-tab into a password manager without the watchdog firing. |
| Bearer token scope                    | Unchanged from v2 — lives only in CLI process memory. Browser never sees it. |
| Access-token refresh                  | Unchanged from v2. |
| In-flight mutation guard              | Unchanged from v2 — `cancel-unload` refuses shutdown while a mutating proxy request is open. |

**Threat model:**

In scope:
- Terminal transcript leakage (stdout piping, `script` recording, `tmux` scrollback, LLM context).
- Preservation of existing scripted / CI invocations.

Out of scope (explicitly):
- Hostile browser extensions with DOM access. They can read the secret on Reveal click.
- Compromised browser or OS user account.
- Process memory reads (the rotation response transits through CLI memory briefly while being proxied to the browser).
- The user's own clipboard history / password manager autofill.

### 4.4 Proxy allowlist diff from v2

Two new `FlowKind` variants, four new routes total:

```
FlowKind::ApiKeyRotate:
  GET  /api/v1/api-keys/:key_id           body_fields: &[]
  POST /api/v1/api-keys/:key_id/rotate    body_fields: &[]

FlowKind::NodeRotateToken:
  GET  /api/v1/nodes/:node_id             body_fields: &[]
  POST /api/v1/nodes/:node_id/rotate-token body_fields: &[]
```

Local lifecycle routes (`/api/proxy/heartbeat`, `/cancel`, `/complete`, etc.) are unchanged — same handlers, same CSRF / Origin pinning. `handle_complete` now dispatches body parsing on `state.flow`.

---

## 5. Command Surface

- **Primary entries:** `nyxid api-key rotate <id_or_name>` and `nyxid node rotate-token <id_or_name>`.
- **No new flags.** Everything that worked before still works, byte-identical.
- **`api-key rotate` now refuses ambiguous names.** Previously `find_key_by_name` silently picked the first match; if multiple keys share a name, the rotation command now fails with `"Name 'X' matches N keys. Pass the ID instead."` This is a behavior tightening — the old behavior could rotate the wrong key.

---

## 6. Non-Interactive Contract

`nyxid api-key rotate` and `nyxid node rotate-token` in scripted mode behave **exactly** as they did before the wizard landed. These forms all bypass the wizard:

```
nyxid api-key rotate KEY_ID --output json
nyxid api-key rotate KEY_ID | cat                    # piped — not a TTY
NYXID_NO_WIZARD=1 nyxid api-key rotate KEY_ID
ssh remote 'nyxid api-key rotate KEY_ID'             # SSH — no local browser
```

Same for `nyxid node rotate-token`.

---

## 7. Verification

Manual tests (run each against prod `https://nyx-api.chrono-ai.fun` after `nyxid login`):

1. **api-key rotate happy path.** `nyxid api-key rotate <known-key>` → confirm panel shows the right name + id → click Rotate → secret panel shows masked dots → Reveal → Copy → ack → terminal prints `✓ API key '...' rotated. New value was shown in the browser.` and exits 0. `script -q /tmp/log nyxid api-key rotate ... ; grep -E 'nyxid_(ag_|sk_)' /tmp/log` returns nothing.
2. **node rotate-token happy path.** Same shape, two secret rows. Download bundles both + the `nyxid node rekey ...` template.
3. **Mask on blur.** Click Reveal, alt-tab away, alt-tab back — the row is masked again, Reveal button reads "Reveal".
4. **Download.** Click Download — `.txt` file downloads with header + secret(s) + (for node) rekey command. Open it, content matches the displayed value.
5. **Cancel from confirm panel.** Click Cancel before Rotate — terminal prints rotation cancel message, exit 1, server is NOT touched.
6. **Cancel from DisplayOnce.** Close the browser tab after secret is shown but before clicking ack — terminal prints "rotation already happened" cancel message, exit 1. Re-running `nyxid api-key list` confirms the key has a new prefix.
7. **JSON output bypasses wizard.** `nyxid api-key rotate KEY --output json` prints the raw secret to stdout (legacy behavior preserved).
8. **`NYXID_NO_WIZARD=1` bypasses.** Same.
9. **Ambiguous name refuses.** Create two keys with the same name, run `nyxid api-key rotate <name>` — exits with the `"matches N keys"` error message, no rotation happens.
10. **Disconnect detection.** Open wizard, Ctrl-C the CLI. Within ~6 s the browser shows the amber ⚠ Wizard disconnected overlay (same v2 behavior).
11. **Long alt-tab on rotation.** Rotate, see the secret panel, alt-tab away for 30s. Come back, secret is still rendered (just masked). Click Reveal again, value is intact, Copy works. (The 60s heartbeat window covers this.)
12. **Body smuggling rejected.** Manually fire `curl -X POST -H "x-wizard-csrf: <csrf>" -H "Origin: http://127.0.0.1:<port>" http://127.0.0.1:<port>/api/proxy/complete -d '{"acknowledged":true,"resource_id":"...","full_key":"hax"}'` — server returns 400, CLI does not exit.

Automated tests:

- `cargo build -p nyxid-cli` confirms types line up.
- Test harness extraction (v2 §10.5) is **deferred to a follow-up PR**. v3.0 ships without expanded automated coverage; the v2 ad-hoc pty validation script still covers ai-key.

---

## 8. Out of Scope

v3.0 deliberately excludes:

1. **api-key create + scope picker.** The DisplayOnce panel reuses cleanly; what's missing is a Step 2 form for label + platform + scope multi-select. Designing that is its own UX effort; deferred to v3.1.
2. **node register-token.** Same shape as rotate-token but Step 2 needs a form for node name. v3.1.
3. **channel-bot register.** Webhook URL + bot token shape. v3.1.
4. **mfa setup.** Needs QR rendering (v2 §10.3 work). v3.2.
5. **Pty test harness extraction (v2 §10.5).** v3.0 ships without expanded automated coverage. Follow-up PR.
6. **Browser-level test of mask-on-blur / heartbeat-during-DisplayOnce.** Requires Playwright (or equivalent) — pty harness can't simulate `document.hidden` or browser timer throttling. Follow-up PR.
7. **Server-side enforcement of "ack before shutdown."** Explicitly downgraded to client-driven best-effort — see §3.5. Restoring the v2-style guarantee would require a `WizardPhase` server state machine; punted unless a real user pain point emerges.

---

## 9. Decision Record

**Decision:** Reuse v2's `/api/proxy/complete` endpoint and heartbeat for rotation flows. Do not introduce a new `WizardPhase` server state machine, do not split `/complete` into `/secret-shown` + `/displayonce-acknowledged`. Add three guards (typed payload, allowlist printer, heartbeat bump) to keep the leak surface narrow. Document the weaker interaction contract honestly.

**Alternatives considered and rejected:**

- *Two-endpoint design with `WizardPhase::AwaitingAck`.* Would server-enforce that the CLI doesn't shut down until the user clicks ack; would protect against JS bugs that POST `/complete` early; would let the watchdog skip the killswitch entirely during DisplayOnce. The cost was a new state machine + two endpoints + per-phase watchdog logic. Rejected because failures 1 (heartbeat killswitch) and 2 (JS bug shuts down early) both leave the secret rendered in the DOM and recoverable by the user — UX papercuts, not security failures. The third failure (secret leaks through completion body) is fixed by the typed payload at near-zero cost. Codex flagged this trade explicitly; we accepted it.
- *60s focus-based auto-remask.* Annoying when present, too long when away. Replaced with immediate remask on `blur` and `visibilitychange → hidden`.
- *`data:` URL for the .txt download.* Leaks into browser history / crash logs / referrer. Replaced with `Blob` + `URL.createObjectURL()` + `revokeObjectURL()`.

**Consequences:**

- The leak surface is closed for terminal/LLM context (the original v3 motivation).
- The CLI's "rotation succeeded" terminal output never contains the secret.
- Cancel / disconnect paths admit honestly that rotation is server-atomic and may have already happened.
- Future rotation-shaped flows (api-key create, node register-token) can reuse the DisplayOnce panel with no further server changes.
- A future bug in wizard.js that POSTs `/complete` early would shut the CLI down while the secret is still on screen — the user would still have it (Reveal/Copy/Download all work locally) but would see a confusing terminal exit. We accept this as a known shape.

---

## 10. Known Debt / Follow-up PRs

### 10.1 Pty test harness extraction (v2 §10.5)

Still unextracted. v3 ships without expanded automated coverage. Should land before v3.1 to catch regressions in the rotation flows + the ai-key flow simultaneously.

### 10.2 Browser-level test (Playwright)

Mask-on-blur, visibility-driven remask, and rotation-flow heartbeat behavior need a real browser to test. Standalone PR.

### 10.3 api-key create + scope picker (v3.1)

The next DisplayOnce flow. Step 2 needs a multi-select against the user's services + nodes with allow-all toggles. Treat the picker as its own design exercise.

### 10.4 node register-token + channel-bot register (v3.1)

Same DisplayOnce panel; per-flow Step 2 forms.

### 10.5 mfa setup (v3.2)

Needs the QR rendering work flagged in v2 §10.3.

### 10.6 Disambiguate node names too

`api-key rotate` now refuses ambiguous names (`find_key_by_name` returns Err on multiple matches). `node` lookups (`resolve_node_id`) still pick the first match silently. Should mirror the api-key tightening in a follow-up since the same wrong-resource risk applies.

### 10.7 Server-side ack enforcement

If real users hit the "tab died before ack" path often, revisit the `WizardPhase::AwaitingAck` design from §9. Would restore the explicit ack-before-close guarantee at the cost of a server state machine. Don't build until there's evidence it's needed.
