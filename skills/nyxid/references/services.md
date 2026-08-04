# Services: discover, add, and connect credentials

## Table of contents

- [Discover Services](#discover-services)
- [Slug rules for `service add`](#slug-rules-for-service-add)
- [Requesting additional OAuth scopes](#requesting-additional-oauth-scopes)
- [Scopes with node-routed services (`--via-node`)](#scopes-with-node-routed-services---via-node)
- [Helping Users Add Services and Credentials](#helping-users-add-services-and-credentials)
  - [The new default: CLI → browser wizard](#the-new-default-cli--browser-wizard)
  - [Wizard transport selection: two predicates, two transports](#wizard-transport-selection-two-predicates-two-transports)
  - [When no local browser is available: remote pairing (wizard v4)](#when-no-local-browser-is-available-remote-pairing-wizard-v4)
  - [When the CLI falls back to terminal (rpassword) mode](#when-the-cli-falls-back-to-terminal-rpassword-mode)
  - [Step 1: Check the catalog](#step-1-check-the-catalog)
  - [Step 2: Choose the right auth flow](#step-2-choose-the-right-auth-flow)
  - [Step 3: Common provider portals](#step-3-common-provider-portals)
  - [Tips for non-technical users](#tips-for-non-technical-users)

## Discover Services

Before using a downstream service, list what the user has configured:

```bash
nyxid service list --output json
```

The response includes:

- `slug`: service identifier for proxy URLs
- `status`: whether the service is active
- `endpoint_url`: where requests are routed
- `node_id`: whether routing through a node agent
- `credential_source`: `{ "type": "personal" }` for the user's own credentials, or `{ "type": "org", "org_id": ..., "org_name": ..., "role": ..., "allowed": ... }` for credentials inherited from an organization the user belongs to. Org-inherited services with `allowed: false` are visible to viewers but cannot be proxied -- do not attempt to call them.
- Discovery projection fields: `name`, `description`, `service_category`, `connected`, `requires_connection`, `has_node_binding`, `proxy_url`, `proxy_url_slug`, `docs_url`, `openapi_url`, `asyncapi_url`, `streaming_supported`, `websocket_supported`, and `source` (`catalog` or `custom`). Use these fields when an agent needs the same read-only proxy metadata exposed by `GET /api/v1/proxy/services`.

If the target service is missing, help the user add it:

```bash
nyxid catalog list --output json                          # browse available services
nyxid catalog list --all --output json                    # include system services without auth
nyxid catalog show <slug> --output json                   # full metadata: links, capabilities, auth notes
nyxid catalog endpoints <slug>                            # list API endpoints from OpenAPI spec
nyxid service add <slug> --oauth                          # OAuth flow (opens browser -- easiest)
nyxid service add <slug> --device-code                    # device code flow (enter code on website)
nyxid service add <slug>                                  # API key (CLI prompts securely)
nyxid service add --custom                                # add custom endpoint (CLI prompts for details)
```

> For API key services, just run `nyxid service add <slug>` without flags. The CLI securely prompts for the key (input hidden). Never ask the user to paste secrets into chat or set environment variables manually.
> For automation/scripting only: `--credential-env <VAR>` reads from an environment variable.
> For multi-field credentials such as AWS access-key JSON: `--credential-file <PATH>` reads the credential bytes from a file. Pass `-` to read from stdin.

## Slug rules for `service add`

- Lowercase letters, digits, and hyphens only. 1-80 characters. No leading, trailing, or consecutive hyphens.
- Slugs are unique **within your own namespace** only -- two users can have services with the same slug without conflict.
- When `--slug` is omitted, NyxID derives the slug from `--label` (or the catalog slug) and appends `-2`, `-3`, ... if you already own one with that name. No random suffixes.
- When `--slug <NAME>` is set and you already own a service with that slug, NyxID returns a **409 Conflict**. The CLI will not silently rename user-supplied slugs -- pick a different slug and retry.

## Requesting additional OAuth scopes

Some OAuth providers (Lark, Google, GitHub, Atlassian, ...) expose many scopes but NyxID's catalog only configures a sensible default set. When a user needs a capability that isn't covered -- for example Lark's contact/attendance APIs -- add extra scopes on top of the defaults with `--scope`:

```bash
# Single scope
nyxid service add api-lark --oauth --scope contact:contact.base:readonly

# Multiple scopes (repeat the flag or comma-separate)
nyxid service add api-lark --oauth \
  --scope contact:contact.base:readonly \
  --scope contact:department.base:readonly

nyxid service add api-lark --oauth \
  --scope "contact:contact.base:readonly,contact:department.base:readonly"

# Works the same way for device-code services
nyxid service add llm-openai --device-code --scope "custom-scope-1,custom-scope-2"
```

The extra scopes are merged (deduped) on top of the provider's `default_scopes` and forwarded in the authorization URL (or device code request). The upstream provider decides whether to grant them -- if the user's app/client doesn't have a scope enabled on the provider side, the authorization flow will still fail there.

**Supported flows:**
- `--oauth` (all OAuth2 providers) -- scopes are appended to the authorization URL.
- `--device-code` (RFC 8628 providers like GitHub, Google, most standard device-code providers) -- scopes are sent in the device code request.
- `--custom` -- `--scope` is accepted for symmetry but has no effect (custom endpoints use direct credentials, not OAuth). The CLI prints a warning.
- OpenAI-format device-code providers (e.g. the seeded `openai-codex` entry) do **not** accept additional scopes -- scopes are baked into the upstream client registration. The backend returns a validation error if you pass `--scope` to one of these, and the "AI Services" UI hides the scope input for them.

In the "AI Services" UI, the OAuth step and the standard device-code step include an optional "Additional scopes" input that accepts the same comma- or space-separated format.

## Scopes with node-routed services (`--via-node`)

Node-routed OAuth flows run on the node agent (so user credentials never leave the node machine). The two-step pattern is:

```bash
# Step 1: On any machine -- create the placeholder record on NyxID. The CLI
# prints the exact next-step command with your scopes pre-filled.
nyxid service add api-lark --oauth --via-node my-node \
  --scope contact:contact.base:readonly,contact:department.base:readonly
# -> "Next step: run this on the node that owns the credential:"
# -> "  nyxid node credentials setup --service api-lark --scope \"contact:contact.base:readonly,contact:department.base:readonly\""

# Step 2: On the node machine -- run the OAuth flow locally with the extras
# merged on top of the catalog's default scopes.
nyxid node credentials setup --service api-lark \
  --scope contact:contact.base:readonly,contact:department.base:readonly
```

`nyxid node credentials add-oauth` also accepts the same `--scope` flag (additive, repeatable) for manual setups. It still accepts the legacy `--scopes` flag (which **replaces** the default scope list entirely) for backward compatibility; prefer `--scope` unless you specifically need override semantics.

## Helping Users Add Services and Credentials

Most users do not know where to find API keys or what authentication method to use. Follow this workflow:

### The new default: CLI → browser wizard

Running `nyxid service add <slug>` with no scripted flags on an interactive TTY now launches a **local browser wizard** (PR #396, wizard v2). This is the recommended flow for non-technical users -- they get the same visual experience as the frontend's Add Key dialog without leaving the terminal to open the web app. End-to-end:

1. User runs `nyxid service add llm-openai` (or with no slug, to pick from the catalog in-wizard).
2. Terminal prints:
   ```
   → Opening http://127.0.0.1:<port>/?csrf=... … (Ctrl-C to cancel)
     Waiting for browser …
   ```
   The CLI boots a local axum server bound to `127.0.0.1:0` (random port, localhost-only), mints a per-session CSRF token, and hands the URL to `open::that` to launch the user's default browser.
3. Browser opens to the wizard SPA -- served entirely from the CLI binary (no remote scripts, strict CSP: `default-src 'none'; script-src 'self'; style-src 'self'`). The page inherits the prefill: catalog card pre-selected, label/endpoint/via-node pre-filled from the flags the user typed.
4. User completes the form: picks a catalog service or `--custom`-style custom endpoint, enters credentials (API key / OAuth / device-code all supported in-browser), clicks **"Done — return to terminal"**.
5. CLI rings the terminal bell (`\x07` + OSC-9 attention cue for iTerm2/WezTerm/Kitty) and prints a summary:
   ```
   ✓ Service 'OpenAI' created.
     Slug:      llm-openai
     Proxy URL: https://nyx-api.chrono-ai.fun/api/v1/proxy/s/llm-openai/
     Next:
       curl https://nyx-api.chrono-ai.fun/api/v1/proxy/s/llm-openai/<api-path> -H "Authorization: Bearer $NYX_KEY"
   ```
6. User closes the browser tab (or the tab says "You can close this tab and return to your terminal").

**Safety posture:** everything is local -- the browser never talks to the NyxID backend directly. The wizard's narrow allowlist of endpoints is proxied through the local server with the user's bearer token attached server-side, so the access token never hits the browser. A 10-second heartbeat watchdog cancels the CLI if the browser tab is closed without clicking Done; a 30-minute ceiling catches walked-away tabs.

**Prefill args** (safe to combine with the wizard -- they just seed the form):
the positional catalog slug (e.g. `llm-openai`), `--label`, `--via-node`, `--endpoint-url`, `--org <ID|SLUG|NAME>` (resolved to org user UUID before being threaded into the wizard prefill so the owner picker opens with the org pre-selected). Passing `--slug` bypasses the wizard and runs the scripted terminal flow instead (see the fallback triggers below).

### Wizard transport selection: two predicates, two transports

Before diving into the remote-pairing path below, it's worth making the layering explicit because the skill section above collapses two separate decisions. The wizard code (see `cli/src/wizard/mod.rs`) makes them as follows:

1. **`is_browser_flow_eligible()`** — *"should we use the wizard path at all, vs. the scripted stdin-prompt path?"* Returns `true` when stdin is NOT a TTY (headless), or when both stdin and stdout are TTYs (interactive). Returns `false` only for `TTY stdin + piped stdout` (classic "user scripting output") and for `NYXID_NO_WIZARD=1`.

2. **`is_wizard_eligible()`** — *"inside the wizard path, can we launch a browser on THIS machine?"* Returns `false` on SSH sessions (`SSH_CONNECTION` / `SSH_TTY` set), on Linux without `DISPLAY`/`WAYLAND_DISPLAY`, and with `NYXID_NO_WIZARD=1`.

Inside each wizard runner the two predicates stack as:

```rust
if is_wizard_eligible() {
    // Mode A: launch local axum wizard, `open::that(url)` the browser
} else {
    // Mode B: remote pairing — print code + pair URL, poll for ack
}
```

`open::that()` is the standard Rust wrapper for `open` (macOS), `xdg-open` (Linux), and `start` (Windows) — the same mechanism Lark CLI uses. It means a non-TTY caller that still has a local GUI (macOS agent subprocess, GNOME terminal tab, Windows shell) lands on the **local wizard**, not remote pairing. Remote pairing is reserved for the cases where no local browser can open at all.

Concrete examples of how the layering resolves:

| Environment                                                        | `is_browser_flow_eligible` | `is_wizard_eligible` | Transport                              |
|--------------------------------------------------------------------|:--------------------------:|:--------------------:|----------------------------------------|
| macOS agent subprocess (no TTY)                                     | true                       | true                 | **Local wizard** via `open` (macOS)    |
| Linux GUI agent subprocess with `DISPLAY`                           | true                       | true                 | **Local wizard** via `xdg-open`        |
| Windows subprocess, no TTY                                          | true                       | true                 | **Local wizard** via `start`           |
| SSH session (no X forwarding)                                       | true                       | false (SSH_CONNECTION) | **Remote pairing** (code + URL)       |
| Linux CI container / Docker, no `DISPLAY`                           | true                       | false                | **Remote pairing**                     |
| Interactive TTY on any OS                                           | true                       | true                 | **Local wizard**                       |
| Interactive TTY with piped/redirected stdout (`> file`)            | false                      | —                    | Scripted stdin prompts                 |
| `NYXID_NO_WIZARD=1`                                                 | false                      | —                    | Scripted stdin prompts                 |

Guidance for integrators:

- **Users on a GUI machine** (laptop, desktop) always get the local wizard, whether they invoked the CLI from an interactive terminal or from a launcher / IDE that captured stdio.
- **Agents on the user's local machine** (Claude Code / Zed / Codex bash tools, VS Code terminal in an editor window) also get the local wizard — `open`/`xdg-open` opens the user's default browser.
- **Truly remote or headless environments** (SSH from a phone, CI runners, Dockerfile builds, cloud sandboxes) get remote pairing so the user can complete the flow on a separate device.
- To force a specific transport, set `NYXID_NO_WIZARD=1` for the scripted path, or pass `--no-wait` to always use remote pairing.

### When no local browser is available: remote pairing (wizard v4)

Introduced in PR #438 / wizard v4. When the CLI can't launch a local browser (SSH without X11, Docker container, no `DISPLAY` on Linux) — i.e. `is_wizard_eligible()` returns `false` — the wizard is NOT disabled. It transparently switches to a **remote pairing transport**: the CLI prints a short pairing code + a URL on `FRONTEND_URL/cli/pair`, the user opens that URL on any device with a browser (phone, laptop, desktop), logs in, enters the code, and completes the exact same wizard there. The CLI polls and picks up the typed ack. Secrets NEVER transit the CLI — only non-secret identifiers (`service_id`, `slug`, `label`).

End-to-end for an agent:

```bash
# The agent runs the wizard-capable command inside its bash tool. No
# TTY, no local browser — the CLI detects this and prints:
$ nyxid service add llm-openai
  Pair with NyxID to finish:
    1. Open:   https://auth.nyxid.dev/cli/pair?code=ABCD-1234
    2. Enter:  ABCD-1234
  Waiting for browser ... (Ctrl-C to cancel)
```

The agent relays the URL to the user; the user completes the wizard on their own device; the CLI exits with the usual summary. Same works for `api-key create`/`rotate`, `node register-token`/`rotate-token`.

**For agents that can't block on stdout** (streaming tool frameworks), pass `--no-wait` to get a resumable handoff instead:

```bash
# One-shot: print machine-readable pairing info and exit.
nyxid api-key create --name coding-agent --platform claude-code \
  --no-wait --output json
# → { "pairing_id": "pair_…", "code": "ABCD-1234",
#     "pair_url": "https://auth.nyxid.dev/cli/pair?code=ABCD-1234",
#     "resume_cmd": "nyxid pairing resume pair_…",
#     "requires_access_token_on_resume": false,
#     "expires_at": "…" }

# Later, after the user has completed the pair page:
nyxid pairing resume pair_…
```

`--no-wait` also works for agents that DO have a TTY but want an explicit "hand-off and return" semantic.

**Safety posture for remote pairing:** codes are 8 Crockford chars (32^8 ≈ 2^40) stored only as HMAC-SHA256 with a server-side key that never touches MongoDB, user-bound at create time (user A's code cannot be claimed as user B), per-IP rate-limited to 5 claim attempts per 60s on real TCP peer, and expire after 15 min. The backend validates the ack references an actual active UserService before transitioning the pairing to `Completed`, so a malicious / buggy browser page cannot trick the CLI into thinking a placeholder is a connected service.

### When the CLI falls back to terminal (rpassword) mode

Terminal (scripted stdin-prompt) mode is selected when **any** of the following is true:

- `--terminal` (alias `--no-wizard`) is passed on the command line -- **per-invocation override**, useful for a one-off scripted call on a GUI machine.
- `NYXID_NO_WIZARD=1` is set in the environment -- **sticky** across all invocations. Right choice for CI runners, Dockerfiles, and systemd units that want the pre-wizard behavior.
- A **scripted flag** is present (tells the CLI the caller already decided the flow): `--oauth`, `--device-code`, `--credential-env`, `--credential`, `--credential-file`, `--custom`, `--slug`, `--auth-method`, `--auth-key-name`, `--scope`, `--openapi-spec-url`, or `--output json` (unless combined with `--no-wait`, which always uses remote pairing). Note: `--org` is **not** in this list -- it is a wizard prefill, so `nyxid service add --org chronoai` opens the wizard with the org pre-selected as owner.
- stdin is a TTY **and** stdout is piped / redirected -- the user is clearly scripting output (`nyxid api-key create > key.txt`, `| jq`), so the CLI respects that and uses the stdin-prompt path.

Note: fully-headless environments (agents, SSH without display, CI containers) NO LONGER fall through to stdin-prompt mode. They route through remote pairing (Mode B) or the local wizard (Mode A via `open`/`xdg-open`/`start`) depending on whether a local browser can actually launch — see the transport-selection table above. Set `NYXID_NO_WIZARD=1` to opt out if a caller genuinely wants the stdin-prompt behavior on a headless box (rare — usually means all args are supplied via flags).

Examples:

```bash
# One-off terminal prompt on a GUI machine
nyxid service add llm-openai --terminal

# Sticky opt-out (put in .bashrc, Dockerfile, or systemd Environment=)
NYXID_NO_WIZARD=1 nyxid service add llm-openai

# Scripted flow (auto-falls-through, no flag needed)
nyxid service add llm-openai --credential-env OPENAI_KEY --output json

# Agent: get a pairing URL + resume command for the user
nyxid service add llm-openai --no-wait --output json
```

**Guidance for AI agents using this skill:** prefer scripted flags (`--credential-env`, `--output json`) when the agent already has the credential in hand — this stays fully non-interactive and never touches a browser. When the agent doesn't have the credential (e.g. the user needs to log into an OAuth provider), pass `--no-wait --output json` to print a machine-readable pairing URL the agent can surface to the user, then `nyxid pairing resume <id>` once the user confirms they've completed the browser flow. Agents should NOT rely on `--terminal` without supplying all required args — the scripted path will hang on the first stdin prompt.

### Step 1: Check the catalog

```bash
nyxid catalog show <slug> --output json
```

The response includes `auth_type` which tells you what the service needs. Use this to guide the user.

### Step 2: Choose the right auth flow

- **OAuth** (`--oauth`): Best for non-technical users. Opens a browser -- the user signs in with their existing account. No API key needed. Use this for Google, GitHub, Twitter, and any service that supports OAuth.
  ```bash
  nyxid service add api-github --oauth
  ```

- **Device code** (`--device-code`): Good when the user can't open a browser from the terminal. Shows a short code to enter on the provider's website. Works well for services like OpenAI Codex.
  ```bash
  nyxid service add llm-openai --device-code
  ```

- **API key**: The user needs to get an API key from the provider's website. Guide them:
  1. Check the common portals table below for the provider's developer portal URL
  2. If the provider is not listed, search the web for "<provider name> API key" to find the right page, then tell the user exactly where to go
  3. Walk them through creating a key on the provider's site
  4. Tell them to run the command **without** `--credential-env` -- the CLI will securely prompt for the key (input is hidden, never shown on screen):
     ```bash
     nyxid service add llm-openai
     # CLI prompts: "Enter API key/credential:" (input hidden)
     ```
  Never ask the user to paste secrets into chat. The CLI's secure prompt keeps the key out of shell history and conversation logs.

### Step 3: Common provider portals

When users need API keys, direct them to the right place:

| Service | Where to get the key | Env var example |
|---------|---------------------|-----------------|
| OpenAI | https://platform.openai.com/api-keys | `OPENAI_KEY` |
| Anthropic | https://console.anthropic.com/settings/keys | `ANTHROPIC_KEY` |
| GitHub | https://github.com/settings/tokens | `GITHUB_TOKEN` |
| Groq | https://console.groq.com/keys | `GROQ_KEY` |

For services not listed here, check `nyxid catalog show <slug> --output json` for the provider's documentation URL.

### Cloud-billing services: AWS Cost Explorer + Google Cloud APIs

Two catalog entries cover the cloud cost and Google API paths used for
`/daily`-style P&L work (NyxID#716 / NyxID#778):

| Slug | What it proxies | Credential shape |
|------|-----------------|------------------|
| `aws-cost-explorer` | AWS Cost Explorer JSON-RPC API (`ce.us-east-1.amazonaws.com`). Use `X-Amz-Target` to pick the operation (e.g. `AWSInsightsServiceV20210101.GetCostAndUsage`). | JSON: `{"access_key_id","secret_access_key","region","service"}` |
| `api-google-cloud` | Generic Google Cloud proxy. Pick the concrete Google API host at add time with `--endpoint-url`: common hosts include `https://cloudbilling.googleapis.com`, `https://bigquery.googleapis.com`, and `https://compute.googleapis.com`. | **Service account (recommended for Cloud APIs):** JSON key via `nyxid external-key add-gcp-service-account` — unattended, never re-auths. **User OAuth:** browser sign-in, default scope `https://www.googleapis.com/auth/cloud-platform.read-only` — but breaks every ~16h for Cloud scopes (`invalid_rapt`). |

AWS SigV4 takes its credential as a JSON blob on the existing
`credential` field -- pass it in via `--credential-file`:

```bash
# AWS — write the access-key JSON to a temp file (or pipe via stdin: -)
cat > /tmp/aws-cred.json <<'EOF'
{"access_key_id":"AKIA...","secret_access_key":"...","region":"us-east-1","service":"ce"}
EOF
nyxid service add aws-cost-explorer --credential-file /tmp/aws-cred.json
rm /tmp/aws-cred.json
```

Google Cloud has **two** credential paths. The catalog base URL is only a
placeholder, so every useful add must supply the concrete Google API host.

> **Use a service account for BigQuery / Cloud Billing.** User OAuth
> (`--oauth`) **cannot run unattended for Google Cloud Platform scopes**:
> Google enforces a ~16-hour session-length reauthentication on Cloud
> scopes and rejects the refresh with `invalid_grant` /
> `error_subtype=invalid_rapt` once it lapses — the service flips to
> `failed` and only an interactive human re-login (which NyxID can't do)
> revives it. This is scope-specific, not account-wide: non-Cloud Google
> APIs (YouTube, Drive, Gmail) refresh fine via OAuth. For BigQuery, Cloud
> Billing, Compute, etc., register a **service account** instead — its
> tokens mint from a key with no reauth, forever:

```bash
# Create a service account in the GCP project holding the data, grant it
# least-privilege roles (see "Required IAM" below), download a JSON key,
# then register it and re-point the services onto it in one step:
nyxid external-key add-gcp-service-account \
  --key-file ~/Downloads/nyxid-cost-reader.json \
  --service google-bigquery \
  --service google-cloud-billing

# Org-owned Cloud APIs: admin creates/binds the service-account credential
# under the org owner, and members proxy through the resulting org service.
nyxid external-key add-gcp-service-account --org acme-corp \
  --key-file ~/Downloads/nyxid-cost-reader.json \
  --service google-bigquery

# Optional: narrow the minted scope (default cloud-platform). IAM roles on
# the service account are the real authorization boundary -- the scope only
# has to be broad enough to reach the API.
nyxid external-key add-gcp-service-account \
  --key-file ~/Downloads/nyxid-cost-reader.json \
  --scopes https://www.googleapis.com/auth/bigquery.readonly \
  --service google-bigquery
```

NyxID validates the key by minting a token once at registration, stores
the key encrypted, and re-mints the hourly access token lazily at proxy
time. The key's `token_uri` is pinned to Google's endpoint server-side
(SSRF guard). The `--service` slugs must already exist (add them with the
`--oauth` commands below, or any add that creates the UserService); the
flag re-points them onto the service account and sets `auth_method=bearer`.
With `--org <ID|SLUG|NAME>`, the caller must be an org admin, the credential
is stored with `user_id = <org_user_id>`, and `--service` resolves active
service slugs under that same org owner. Omit `--org` for the personal path.

The user-OAuth flow below still applies to **non-Cloud** Google APIs, or
to operators who accept the ~16h re-login cadence.

**Operator setup (must be done once before users can `--oauth`):**

The seeded `google-cloud` provider ships with `credential_mode=user` and
no platform-level client_id / client_secret. Until those are configured,
every `--oauth` add has to supply BYO OAuth client credentials. There
are two ways to satisfy this:

1. **Platform-default (recommended for multi-user deployments).** An
   admin registers a Google OAuth web client in the Google Cloud Console,
   then PATCHes the provider with the resulting client_id / secret and
   sets `credential_mode` to `admin` (or `both` to keep BYO as fallback):

   ```bash
   curl -X PATCH "$BASE_URL/api/v1/providers/google-cloud" \
     -H "Authorization: Bearer $ADMIN_TOKEN" \
     -H "Content-Type: application/json" \
     -d '{
       "client_id": "<GOOGLE_OAUTH_CLIENT_ID>",
       "client_secret": "<GOOGLE_OAUTH_CLIENT_SECRET>",
       "credential_mode": "both"
     }'
   ```

   The admin UI exposes the same field. After this, users can
   `nyxid service add api-google-cloud --oauth ...` with no extra flags.

2. **BYO per user.** Each user registers their own Google OAuth web
   client and passes it on the add command:

   ```bash
   nyxid service add api-google-cloud --oauth \
     --endpoint-url https://cloudbilling.googleapis.com \
     --slug google-cloud-billing \
     --oauth-client-id "$GOOGLE_OAUTH_CLIENT_ID" \
     --oauth-client-secret-env GOOGLE_OAUTH_CLIENT_SECRET
   ```

In **both** cases the Google OAuth client must list this as an
authorized redirect URI:

```
<BASE_URL>/api/v1/providers/callback
```

The provider seed also requests `access_type=offline` + `prompt=consent`
on every authorize call so Google returns a refresh token; without these
Google only issues a 1-hour access token and proxy calls start failing
the moment it expires.

```bash
# Generic shape (platform-default credentials configured)
nyxid service add api-google-cloud --oauth --endpoint-url <google-api-host>

# Cloud Billing metadata: billing accounts, projects, SKUs.
# This API does not expose historical spend totals.
nyxid service add api-google-cloud --oauth \
  --endpoint-url https://cloudbilling.googleapis.com \
  --slug google-cloud-billing

# BigQuery billing export queries for spend-by-project / day.
nyxid service add api-google-cloud --oauth \
  --endpoint-url https://bigquery.googleapis.com \
  --slug google-bigquery

# Other Google APIs use the same catalog entry with a different host.
nyxid service add api-google-cloud --oauth \
  --endpoint-url https://compute.googleapis.com \
  --slug google-compute

# Add extra scopes when the Google API operation needs more than read-only.
nyxid service add api-google-cloud --oauth \
  --endpoint-url https://compute.googleapis.com \
  --scope https://www.googleapis.com/auth/cloud-platform
```

**Required IAM (enforced by the cloud, not NyxID):**

- AWS: the IAM user / role must be in the AWS **Organization management
  (payer) account** — linked-account credentials return `AccessDenied`.
  Minimum policy: `ce:GetCostAndUsage`, `ce:GetCostAndUsageWithResources`.
- Google Cloud: the principal (the signed-in user for OAuth, or the
  **service account** for the SA path) needs IAM for the target project /
  billing account. Typical read-only billing setup is `roles/billing.viewer`
  (org or billing-account scope) plus, for BigQuery billing export queries,
  `roles/bigquery.dataViewer` on the export dataset and `roles/bigquery.jobUser`
  on the host project. For the service-account path these roles are granted
  to the SA's email; the minted scope and IAM both have to allow the
  operation.

**Operational requirements before this works end-to-end:**

- AWS cost allocation tags activated for the dimensions you want to
  group by (e.g. `project`, `namespace`) — `GroupBy: TAG:project` returns
  empty data otherwise.
- Google Cloud billing-export to BigQuery must already be enabled and
  the dataset name known before spend-by-project SQL works. Use
  `nyxid catalog show api-google-cloud --output json` for the upstream
  documentation link.

**Caching:** AWS Cost Explorer requests using `aws_sigv4` can use the
in-process response cache (default disabled; set
`CLOUD_RESPONSE_CACHE_TTL_SECS` to enable). Identical proxy bodies in a
short window replay from memory, which helps avoid repeated Cost
Explorer charges for polling-style dashboards.

### Tips for non-technical users

- **Prefer `--oauth` or `--device-code`** over API keys whenever available -- the user just signs in.
- **Explain what an API key is**: "It's like a password that lets NyxID call the service on your behalf. You create it once and NyxID stores it securely."
- **Environment variables are temporary**: `export VAR="value"` only lasts for the current terminal session. The credential is stored in NyxID after `service add`, so the env var is only needed once.
- If the user is confused, break it into smaller steps. For example: "First, let's check what services are available" then `nyxid catalog list`.
