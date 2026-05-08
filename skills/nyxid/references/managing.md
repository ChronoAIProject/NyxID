# Managing Services and API Keys

## Table of contents

- [Capturing user intent: edit vs. create](#capturing-user-intent-edit-vs-create)
- [Managing Services](#managing-services)
  - [Attaching an OpenAPI spec to a custom endpoint](#attaching-an-openapi-spec-to-a-custom-endpoint)
- [Managing User Endpoints (`nyxid endpoint`)](#managing-user-endpoints-nyxid-endpoint)
- [Managing External Credentials (`nyxid external-key`)](#managing-external-credentials-nyxid-external-key)
- [Managing API Keys](#managing-api-keys)
  - [Scope requirements for management writes](#scope-requirements-for-management-writes)
  - [Browser wizard for one-time secrets (v2 + v3.0 + v3.1 + v4)](#browser-wizard-for-one-time-secrets-v2--v30--v31--v4)

## Capturing user intent: edit vs. create

Most NyxID requests look like "manage X" — and most of the time, X already exists. Before reaching for `create` / `add` / `register`, list what's there and try to match the user's reference. **The single most common skill failure mode is creating a new record when the user wanted to edit an existing one** (e.g. minting a fresh `nyxid_ag_…` agent key when the user said "change my agent key's scopes").

### Quick decision tree

1. **List first.** Run the matching read command — `nyxid api-key list --output json`, `nyxid service list --output json`, `nyxid node list --output json`, `nyxid channel-bot list --output json`, etc. — before doing anything that mutates state.
2. **Match by what the user said.** If they referenced a name, label, slug, or "the OpenAI one / my coding agent / the Telegram bot", look for a matching entry in the list output. If you find exactly one, that's the target. If you find more than one, ask which.
3. **Classify the verb.** Map the user's wording to a command family:
   - "Change / edit / update / rename / fix / add a service to / remove a service from / re-scope / restrict / re-route / move / re-bind" → **update / bind** (no new record)
   - "Rotate / replace / refresh / it leaked / regenerate the secret" → **rotate / rotate-secret / rotate-credential / rotate-token**
   - "Connect / add / set up / register / a new / another / a second / I don't have one yet" → **create / add / register**
   - "Disable / pause / take offline / turn off" → **`update --is-active false`** (service-account / api-key) or **`disable`** (approval) — not delete
   - "Remove / delete / get rid of / I'm not using this anymore" → **delete --yes**
4. **Ask once if intent is genuinely ambiguous.** "I see one existing agent key called `coding-agent`. Do you want to (a) update its scopes, (b) rotate its secret, or (c) create a second key?" One short clarifying question beats minting an unwanted record.

### "API key" disambiguation — `api-key` vs `external-key`

NyxID uses the phrase "API key" for two different things. Confusing them is the second-most-common failure mode (right after "kept creating instead of editing").

| User says… | They probably mean… | Use |
|---|---|---|
| "my OpenAI API key" / "my GitHub token" / "my Anthropic key" / "the key I got from \<provider\>" | The third-party credential NyxID stores and injects on outbound requests | `nyxid external-key …` or `nyxid service rotate-credential …` |
| "my NyxID API key" / "my agent key" / "my Claude Code key" / "the key my agent uses" / `nyxid_ag_…` | NyxID's own bearer token an agent presents on the way IN | `nyxid api-key …` |
| "rotate my OpenAI key" | Provider-side credential rotation | `nyxid service rotate-credential <SERVICE_ID> --credential-env NEW_TOKEN` |
| "rotate my agent key" | NyxID-side identity rotation (the wizard mints a new `nyxid_ag_…`) | `nyxid api-key rotate <ID_OR_NAME>` |

When the noun is ambiguous and the user hasn't said which they mean, **list both and ask**:

```bash
nyxid api-key list --output json        # NyxID agent keys (nyxid_ag_…)
nyxid external-key list --output json   # downstream credentials (OpenAI / Lark / etc.)
```

Then ask "Which one — your NyxID agent key (used to call NyxID) or the downstream provider credential (the OpenAI key NyxID stores)?"

### Edits agents commonly mistake for "create"

| User intent | Wrong command (don't do this) | Right command |
|---|---|---|
| "Add `proxy` scope to my agent key" | `nyxid api-key create --scopes "read write proxy"` | `nyxid api-key update <ID> --scopes "read write proxy"` |
| "Restrict my agent to only the OpenAI service" | `nyxid api-key create --allowed-services <ID>` | `nyxid api-key bind <ID> --service llm-openai` then `nyxid api-key update <ID> --allow-all-services false` |
| "Set a callback URL on my agent" | `nyxid api-key create --callback-url "https://…"` | `nyxid api-key update <ID> --callback-url "https://…"` |
| "Change the OpenAI key I gave NyxID" | `nyxid service add llm-openai` (creates a duplicate) | `nyxid service rotate-credential <SERVICE_ID> --credential-env NEW_KEY` |
| "Switch which OpenAI account my service uses" | `nyxid service add llm-openai` | `nyxid service rotate-credential …` (different value) or replace via `nyxid external-key rotate <KEY_ID>` if shared |
| "Move the routing for my llm-openai service to my new node" | `nyxid service add llm-openai --via-node …` | `nyxid service route <ID> --node <NEW_NODE>` |
| "Change which node my SSH service uses" | `nyxid service add-ssh …` | `nyxid service update <ID> --node-id <NEW_NODE_ID>` (or `--no-node` for direct) |
| "Add a redirect URI to my OAuth client" | `nyxid developer-app create …` | `nyxid developer-app update <ID> --redirect-uri "…"` (repeat to set multiple — replaces the list) |
| "Disable a service-account" | `nyxid service-account create …` | `nyxid service-account update <ID> --is-active false` |
| "Rename my org" | `nyxid org create …` | `nyxid org update <ORG_ID> --display-name "…"` |
| "Re-route a channel-bot conversation to a different agent" | `nyxid channel-bot route create …` (creates a second route) | `nyxid channel-bot route update <ROUTE_ID> --agent-key-id <NEW_KEY_ID>` |

## Managing Services

```bash
nyxid catalog list --output json                               # browse catalog (connectable services)
nyxid catalog list --all --output json                         # all services (including system/no-auth)
nyxid catalog endpoints <slug> --output json                   # list API endpoints from OpenAPI spec

# `service add` is wizard-capable — DEFAULT to the bare form. The CLI auto-picks
# local browser vs remote pairing. Adding `--output json` here without
# `--no-wait` skips the wizard entirely.
nyxid service add <slug>                                       # add from catalog (wizard prompts for credential)
nyxid service add <slug> --oauth                               # OAuth flow (wizard opens upstream consent page)
nyxid service add <slug> --device-code                         # device code flow (wizard guides through code entry)
nyxid service add <slug> --via-node <name>                     # add via node (wizard prompts for credential)
nyxid service add --custom                                     # custom endpoint (wizard prompts for URL/auth/details)
# Headless agent variant: append --no-wait --output json and resume with
#   nyxid pairing resume <pairing_id>
nyxid service add <slug> --oauth --no-wait --output json

nyxid service list --output json                               # list services (includes IDs)
nyxid service show <id> --output json                          # show service details
nyxid service update <id> --label "My Custom Name"             # rename service
nyxid service update <id> --openapi-spec-url https://api.example.com/openapi.json  # attach an OpenAPI spec
nyxid service update <id> --openapi-spec-url ""                # clear the OpenAPI spec URL
nyxid service update <id> --default-header 'x-openclaw-scopes=operator.read,operator.write'
nyxid service update <id> --default-header 'x-api-version=v2:overridable'
nyxid service update <id> --default-header 'x-secret-token=abc123:sensitive'   # redact value in audit logs / API responses
nyxid service update <id> --clear-default-headers
nyxid service delete <id> --yes                                # remove service (no prompt)
nyxid service rotate-credential <id> --credential-env NEW_KEY  # rotate stored credential value (NOT wizard-driven; use this for "change my OpenAI key")
nyxid service route <id> --node <NODE_NAME>                    # change node routing without re-creating the service (also --direct)
nyxid service convert-ssh <slug> --to-node-key                 # SSH services: switch auth mode (also --to-cert / --to-proxy-only)
```

> Default request header precedence is `catalog defaults -> UserService defaults -> caller`. The default is non-overridable unless `:overridable` is set on the value.

> Node commands accept names (e.g., `--via-node test-server`) in addition to UUIDs.
> For org-owned node operations, the two-machine VM playbook, and transfer cleanup behavior, see [`nodes.md`](nodes.md#two-machine-org-node-setup).
> For remote credential provisioning, use `nyxid node-credential push/list/cancel` on the admin laptop and `nyxid node credentials pending/accept/decline` on the VM. See [`nodes.md`](nodes.md#remote-credential-provisioning).

### Attaching an OpenAPI spec to a custom endpoint

Custom endpoints default to a single generic proxy tool. If the target service publishes an OpenAPI spec, attach the spec URL so AI agents (MCP, `/api/v1/endpoints/{id}/openapi-endpoints`) surface one tool per operation instead. Catalog-backed services inherit the catalog entry's spec URL automatically -- pass an empty string (`--openapi-spec-url ""`) on create if you want to opt out.

```bash
# Custom endpoint with OpenAPI discovery
nyxid service add --custom --label "My API" \
  --endpoint-url https://api.example.com/v1 \
  --openapi-spec-url https://api.example.com/openapi.json \
  --credential-env MY_API_TOKEN

# Pick a custom slug instead of letting NyxID derive one from --label
nyxid service add --custom --slug home-assistant --label "Home Assistant" \
  --endpoint-url https://ha.local:8123/api \
  --credential-env HA_TOKEN

# `--slug` also works on catalog-backed keys for running multiple instances
nyxid service add llm-openai --slug llm-openai-prod --credential-env OPENAI_PROD_KEY
nyxid service add llm-openai --slug llm-openai-staging --credential-env OPENAI_STAGING_KEY

# `--slug` also works with OAuth and device-code flows
nyxid service add api-lark --oauth --slug lark-team-engineering

# Catalog-backed key that suppresses the catalog's default spec URL
nyxid service add llm-openai --openapi-spec-url ""

# Attach or update the spec URL after the fact
nyxid service update <id> --openapi-spec-url https://api.example.com/openapi.json
```

URLs must be `http(s)://` and cannot contain `user:pass@` userinfo. The backend fetches them through a hardened path (DNS pinning, 5 MB size cap, no redirects, per-user cache scoping) and falls back to the generic proxy tool if the spec can't be fetched or parsed, so a broken spec URL never takes the service offline. SSH services ignore this field.

## Managing User Endpoints (`nyxid endpoint`)

`nyxid service add` auto-provisions a `UserEndpoint` (the target URL the proxy hits) alongside the `UserService` routing record and the `UserApiKey` credential. Most users never touch endpoints directly — but when a downstream URL changes (region migration, rebrand, custom DNS) you can edit the endpoint in place rather than recreating the service.

```bash
nyxid endpoint list --output json                  # list user-managed endpoints
nyxid endpoint update <ENDPOINT_ID> --url https://new.example.com/v1
nyxid endpoint delete <ENDPOINT_ID> --yes          # only safe when no UserService still points to it
```

Get the `<ENDPOINT_ID>` from the `endpoint_id` field on `nyxid service list --output json`.

## Managing External Credentials (`nyxid external-key`)

`UserApiKey` records (the external credentials NyxID injects on outbound requests) can be inspected and rotated independently of the service that owns them. This is useful when one credential backs multiple services, or when a rotation is happening on the provider side and you need to push the new value into NyxID without recreating the service binding.

```bash
nyxid external-key list --output json                          # list all external credentials
nyxid external-key rotate <KEY_ID> --credential-env NEW_TOKEN  # rotate without re-creating the binding
nyxid external-key delete <KEY_ID> --yes                       # remove unused credential
```

`<KEY_ID>` is the `api_key_id` field on `nyxid service list --output json`. `--credential-env` reads the new value from an environment variable; omit the flag to be prompted securely.

> Don't confuse `nyxid external-key` (third-party credentials NyxID stores on the user's behalf) with `nyxid api-key` (NyxID's own `nyxid_ag_…` keys agents use to call NyxID). External keys are the credentials NyxID injects into proxied requests; API keys are the credentials NyxID itself accepts on inbound requests.

## Managing API Keys

Each AI agent or integration should use its own NyxID API key (agent key). This gives each caller independent audit trail, optional service bindings, and rate limits.

```bash
# Create — DEFAULT to the bare wizard form. The CLI auto-opens the local
# scope-picker wizard (v3.1) on a GUI machine, falls through to a remote
# pairing URL on a headless agent. Prefill flags seed the form; the user
# can still change anything inside the wizard.
nyxid api-key create                                       # full wizard
nyxid api-key create --name "coding-agent" --platform claude-code         # wizard with prefill
nyxid api-key create --name "relay-agent" --callback-url "https://..."    # wizard with prefill (channel bot relay)

# Headless agent that can't block on the wizard? Get a machine-readable
# pairing handoff and resume later:
nyxid api-key create --name "coding-agent" --platform claude-code --no-wait --output json
# → { pairing_id, pair_url, resume_cmd, requires_access_token_on_resume, expires_at }
nyxid pairing resume <pairing_id>

# Read commands — always use --output json to parse the response
nyxid api-key list --output json
nyxid api-key show <ID_OR_NAME> --output json

# Rotate — same wizard semantics (interactive by default, --no-wait for handoff)
nyxid api-key rotate <ID_OR_NAME>
nyxid api-key rotate <ID_OR_NAME> --no-wait --output json

nyxid api-key delete <ID_OR_NAME> --yes

# Scripted-only escape hatch — bypasses the wizard and prints the raw key
# / new secret on stdout. Use only when every required arg is supplied as
# a flag AND the caller explicitly wants stdout (CI, Dockerfile, scripts).
# Don't pick this on user-facing flows: it dumps secrets where they may be
# captured by tool transcripts.
nyxid api-key create --name "ci-bot" --scopes "proxy" --output json
nyxid api-key rotate <ID_OR_NAME> --output json

# Org-owned agent keys (for sharing one agent identity across the whole org)
nyxid api-key create --name "shared-coding-agent" --org <ID|SLUG|NAME> --platform claude-code   # wizard with org pre-selected
nyxid api-key list --org <ID|SLUG|NAME> --output json     # list all keys owned by this org
nyxid api-key rotate <ID>                                 # any org admin can rotate (wizard)
nyxid api-key delete <ID> --yes                           # any org admin can delete

# Consumers authenticate as the org: the agent's NYXID_ACCESS_TOKEN is the
# org's key, proxy calls see org-shared services directly without needing
# membership resolution, and audit logs attribute requests to the key
# (not the admin who created it).

# Service bindings (credential auto-resolved from service)
nyxid api-key bind <ID_OR_NAME> --service <SERVICE_SLUG>
nyxid api-key bind <ID_OR_NAME> --service <SLUG> --credential <LABEL>  # explicit override

# By default, agents can access all services with default credentials.
# Bindings override which credential is used for specific services.
# To restrict an agent to ONLY access bound services:
nyxid api-key update <ID> --allow-all-services false

# Callback URL for channel bot relay
nyxid api-key update <ID> --callback-url "https://my-agent.example.com/webhook"
nyxid api-key update <ID> --callback-url ""    # clear

# Per-key rate limits — the CLI does NOT expose flags for these. Configure
# them on the key's detail page in the web UI, or via raw HTTP:
#   PUT /api/v1/api-keys/{id} with body
#     {"rate_limit_per_second": 10, "rate_limit_burst": 30}
# The browser-wizard `nyxid api-key create` form also includes a rate-limits
# section that posts the same fields on creation.
```

Set `NYXID_ACCESS_TOKEN` in your agent's environment to authenticate:

```bash
export NYXID_ACCESS_TOKEN="nyxid_ag_..."
```

### Scope requirements for management writes

Agent keys need `write` or `admin` scope to call management endpoints via REST (create/update/delete/rotate API keys, services, endpoints, bindings, etc.). `proxy read` is sufficient for proxy traffic only -- paths under `/proxy`, `/llm`, `/ssh`, `/channel-events`, `/channel-relay`, and `/delegation` do not require write scope. The `nyxid` CLI uses session auth (not API keys) and is unaffected.

### Browser wizard for one-time secrets (v2 + v3.0 + v3.1 + v4)

Ten commands open a browser-based wizard for interactive use, so the secret (either collected from the user or minted by the backend) lands in the user's browser tab instead of the terminal / agent context:

| Command                                | Version | Wizard role                                                                                            |
|----------------------------------------|:-------:|--------------------------------------------------------------------------------------------------------|
| `nyxid service add [<slug>]`           |   v2    | Collects a paste-key / OAuth / device-code credential; creates the service + key record.               |
| `nyxid api-key rotate <id>`            |   v3.0  | DisplayOnce: backend mints a new `nyxid_ag_…`, rendered masked with click-to-reveal + copy.            |
| `nyxid node rotate-token <id>`         |   v3.0  | DisplayOnce: backend mints a new auth token + signing secret (two rows).                               |
| `nyxid node register-token`            |   v3.1  | DisplayOnce: backend mints a new `nyx_nreg_…` for bootstrapping a fresh node.                          |
| `nyxid api-key create`                 |   v3.1  | Scope picker (name + owner + platform + scopes + expiry + service/node multi-select + rate limits) → DisplayOnce on the new `nyxid_ag_…`. |
| `nyxid mfa setup`                      |   v3.1  | TOTP enrollment: QR-code render, verify TOTP, recovery codes shown once.                               |
| `nyxid service-account create`         |   v3.2  | Owner picker + scopes + role IDs → DisplayOnce on the new `client_secret`.                             |
| `nyxid service-account rotate-secret`  |   v3.2  | DisplayOnce on the new `client_secret`; old secret + tokens revoked atomically.                        |
| `nyxid developer-app create`           |   v3.2  | Owner picker + redirect URIs + scopes + broker capability; DisplayOnce on the secret for confidential clients (public clients skip the secret panel). |
| `nyxid developer-app rotate-secret`    |   v3.2  | DisplayOnce on the new `client_secret`; old secret revoked.                                            |

All ten commands automatically pick between two transports depending on environment, added in v4 (PR #438):

- **Mode A — Local wizard** (v2/v3 original): picked when `is_wizard_eligible()` returns `true`, i.e. the CLI can launch a local browser via `open::that()` (macOS `open`, Linux `xdg-open`, Windows `start`). The CLI boots an axum server on `127.0.0.1:<random-port>`, opens the wizard SPA there, and the browser talks back through a narrow allowlist of proxied endpoints. Access tokens never hit the browser; 10-second heartbeat cancels on tab-close. CLI prints `→ Opening http://127.0.0.1:…/wizard …`. This is the path taken **on any machine with a desktop environment**, including non-TTY agent subprocesses on macOS / Windows / Linux-with-DISPLAY — the subprocess not having a TTY doesn't prevent `open` / `xdg-open` / `start` from reaching the user's default browser.

- **Mode B — Remote pairing** (v4 new): picked when `is_wizard_eligible()` returns `false`, which only happens on SSH sessions (`SSH_CONNECTION` / `SSH_TTY` set), Linux boxes without `DISPLAY`/`WAYLAND_DISPLAY` (CI runners, headless containers), or when `NYXID_NO_WIZARD=1` is set. The CLI creates a short-lived server-side pairing record and prints a pair URL + 8-char Crockford code on `FRONTEND_URL/cli/pair`. The user opens the URL on ANY device with a browser (phone, desktop), logs in, enters the code, and completes the same wizard there. The CLI polls for the typed ack. Same visual experience, same DisplayOnce affordances.

The selection is automatic — callers don't need to pick. The only caller-facing knob is `--no-wait`, which forces Mode B regardless of `is_wizard_eligible()` because it's designed for agent wrappers that want a resumable handoff instead of blocking on a live wizard.

Full specs: [`docs/CLI_WIZARD_V2.md`](../../docs/CLI_WIZARD_V2.md) (v2) + [`docs/CLI_WIZARD_V3.md`](../../docs/CLI_WIZARD_V3.md) (v3 / v3.1). v4's pairing transport lives under `/cli-pairings/*` backend endpoints and `/cli/pair` on the frontend.

**Visual consistency.** Both transports share the same shell: same brand lockup (NyxID wordmark in DM Serif Display), same ✓/✗/⚠ overlay system, same purple accent (`#8b5cf6` / `#7c3aed`), same button and field styling. The local path's footer says "Served locally from 127.0.0.1 · Nothing leaves your machine"; the remote path omits that footer because the page is served from the NyxID frontend origin — but secrets still never leave the browser (the CLI receives only non-secret identifiers via the pairing ack).

**Agent handoff with `--no-wait`.** For agents that can't block on the pairing URL streaming out of stdout, every wizard-capable command accepts `--no-wait`: the CLI creates the pairing, prints a JSON payload on stdout with `{pairing_id, code, pair_url, resume_cmd, requires_access_token_on_resume, expires_at}`, and exits 0 immediately. The agent relays `pair_url` to the user and later runs the printed `resume_cmd` (or `nyxid pairing resume <pairing_id>`) to pick up the result. `--no-wait` works regardless of TTY state.

For scripted / agent use, the wizard is **bypassed** (falls through to the pre-wizard stdin / rpassword path) when ANY of these is true:

- `--terminal` (alias `--no-wizard`) is passed — per-invocation override, available on all ten wizard commands.
- `NYXID_NO_WIZARD=1` is set in the environment.
- `--output json` is passed AND `--no-wait` is NOT — agents that want machine-readable output stay scripted, unless they explicitly opt into the pairing transport via `--no-wait`.
- stdin is a TTY AND stdout is piped / redirected — the user is scripting output but has an interactive shell for prompts.

Note: having no TTY at all (agent subprocess, SSH without X11, CI container) does NOT bypass — the command routes through remote pairing instead, since a scripted stdin prompt would just hang. Set `NYXID_NO_WIZARD=1` explicitly if a caller wants the scripted path on a headless box.

When the wizard is bypassed the commands print the raw secret to stdout in the same shape as the pre-wizard CLI. Agents calling these commands programmatically have three clean options:

- `--output json --credential-env VAR` or other scripted flags → fully non-interactive, no browser or pairing involved.
- `--no-wait --output json` → machine-readable pairing URL + resume command; agent relays the URL to the user.
- `--terminal` with all args supplied → pre-wizard scripted prompts skipped because every prompt has a flag value.

Behavior change to be aware of: `nyxid api-key rotate <name>` now **refuses ambiguous names** — if multiple keys share the same name, the command exits with `Name 'X' matches N keys. Pass the ID instead.` Previously it silently rotated the first match (which could rotate the wrong key). Always prefer ID over name for scripted rotation.

Rotation is **server-atomic** in both modes: the old key is deactivated and a new key is created with a new ID, preserving name + scopes + bindings. Anything that hard-codes the old ID (CI configs, dashboards, prior bindings registered out-of-band) will need updating to the new ID. Existing `AgentServiceBinding` records are cloned to the new key automatically.
