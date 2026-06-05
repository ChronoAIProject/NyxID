# Managing Services and API Keys

## Table of contents

- [Managing Services](#managing-services)
  - [Attaching an OpenAPI spec to a custom endpoint](#attaching-an-openapi-spec-to-a-custom-endpoint)
- [Managing API Keys](#managing-api-keys)
  - [Scope requirements for management writes](#scope-requirements-for-management-writes)
  - [Browser wizard for one-time secrets (v2 + v3.0 + v3.1 + v4)](#browser-wizard-for-one-time-secrets-v2--v30--v31--v4)

## Managing Services

```bash
nyxid catalog list                                             # browse catalog (connectable services)
nyxid catalog list --all                                       # all services (including system/no-auth)
nyxid catalog endpoints <slug>                                 # list API endpoints from OpenAPI spec
nyxid service add <slug>                                       # add from catalog (CLI prompts for credential)
nyxid service add <slug> --oauth                               # add with OAuth (opens browser)
nyxid service add <slug> --device-code                         # add with device code flow
nyxid service add <slug> --via-node <name>                     # add via node (CLI prompts for credential)
nyxid service add --custom                                     # add custom endpoint (CLI prompts for details)
nyxid service list --output json                               # list services (includes IDs)
nyxid service show <id>                                        # show service details
nyxid service update <id> --label "My Custom Name"             # rename service
nyxid service update <id> --openapi-spec-url https://api.example.com/openapi.json  # attach an OpenAPI spec
nyxid service update <id> --openapi-spec-url ""                # clear the OpenAPI spec URL
nyxid service update <id> --default-header 'x-openclaw-scopes=operator.read,operator.write'
nyxid service update <id> --default-header 'x-api-version=v2:overridable'
nyxid service update <id> --default-header 'x-secret-token=abc123:sensitive'   # redact value in audit logs / API responses
nyxid service update <id> --clear-default-headers
nyxid service delete <id> --yes                                # remove service (no prompt)
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

## Managing API Keys

Each AI agent or integration should use its own NyxID API key (agent key). This gives each caller independent audit trail, optional service bindings, and rate limits.

```bash
# CRUD
nyxid api-key create                                       # interactive: opens scope-picker wizard (v3.1)
nyxid api-key create --name "My Key" --scopes "proxy read"
nyxid api-key create --name "coding-agent" --platform claude-code  # any prefill flag is picked up by the wizard
nyxid api-key create --name "relay-agent" --callback-url "https://..."  # for channel bot relay
nyxid api-key create --name "My Key" --output json         # scripted: prints raw key to stdout (legacy shape)
nyxid api-key list --output json
nyxid api-key show <ID_OR_NAME> --output json
nyxid api-key rotate <ID_OR_NAME>                          # interactive: opens browser wizard
nyxid api-key rotate <ID_OR_NAME> --output json            # scripted: prints raw secret to stdout (legacy shape)
nyxid api-key delete <ID_OR_NAME> --yes

# Org-owned agent keys (for sharing one agent identity across the whole org)
nyxid api-key create --name "shared-coding-agent" --org <ID|SLUG|NAME> --platform claude-code
nyxid api-key list --org <ID|SLUG|NAME>               # list all keys owned by this org
nyxid api-key rotate <ID> --output json               # any org admin can rotate
nyxid api-key delete <ID> --yes                       # any org admin can delete

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

# Per-key rate limits
nyxid api-key update <ID> --rate-limit-per-second 10 --rate-limit-burst 30
```

Set `NYXID_ACCESS_TOKEN` in your agent's environment to authenticate:

```bash
export NYXID_ACCESS_TOKEN="nyxid_ag_..."
```

External Google Cloud service-account credentials can also be owned by an org:

```bash
# Admin required. The encrypted UserApiKey is written under the org user_id,
# and --service rebinds active service slugs owned by that org.
nyxid external-key add-gcp-service-account --org <ID|SLUG|NAME> \
  --key-file ~/Downloads/nyxid-cost-reader.json \
  --service google-bigquery
```

Omit `--org` for a personal credential. Members and viewers cannot create org-owned external credentials (`8103 org_role_insufficient`).

### Scope requirements for management writes

Agent keys need `write` or `admin` scope to call management endpoints via REST (create/update/delete/rotate API keys, services, endpoints, bindings, etc.). `proxy read` is sufficient for proxy traffic only -- paths under `/proxy`, `/llm`, `/ssh`, `/channel-events`, `/channel-relay`, and `/delegation` do not require write scope. The `nyxid` CLI uses session auth (not API keys) and is unaffected.

### Browser wizard for one-time secrets (v2 + v3.0 + v3.1 + v4)

Five commands now open a browser-based wizard for interactive use, so the secret (either collected from the user or minted by the backend) lands in the user's browser tab instead of the terminal / agent context:

| Command                           | Version | Wizard role                                                                                            |
|-----------------------------------|:-------:|--------------------------------------------------------------------------------------------------------|
| `nyxid service add [<slug>]`      |   v2    | Collects a paste-key / OAuth / device-code credential; creates the service + key record.               |
| `nyxid api-key rotate <id>`       |   v3.0  | DisplayOnce: backend mints a new `nyxid_ag_…`, rendered masked with click-to-reveal + copy.            |
| `nyxid node rotate-token <id>`    |   v3.0  | DisplayOnce: backend mints a new auth token + signing secret (two rows).                               |
| `nyxid node register-token`       |   v3.1  | DisplayOnce: backend mints a new `nyx_nreg_…` for bootstrapping a fresh node.                          |
| `nyxid api-key create`            |   v3.1  | Scope picker (name + owner + platform + scopes + expiry + service/node multi-select + rate limits) → DisplayOnce on the new `nyxid_ag_…`. |

All five commands automatically pick between two transports depending on environment, added in v4 (PR #438):

- **Mode A — Local wizard** (v2/v3 original): picked when `is_wizard_eligible()` returns `true`, i.e. the CLI can launch a local browser via `open::that()` (macOS `open`, Linux `xdg-open`, Windows `start`). The CLI boots an axum server on `127.0.0.1:<random-port>`, opens the wizard SPA there, and the browser talks back through a narrow allowlist of proxied endpoints. Access tokens never hit the browser; 10-second heartbeat cancels on tab-close. CLI prints `→ Opening http://127.0.0.1:…/wizard …`. This is the path taken **on any machine with a desktop environment**, including non-TTY agent subprocesses on macOS / Windows / Linux-with-DISPLAY — the subprocess not having a TTY doesn't prevent `open` / `xdg-open` / `start` from reaching the user's default browser.

- **Mode B — Remote pairing** (v4 new): picked when `is_wizard_eligible()` returns `false`, which only happens on SSH sessions (`SSH_CONNECTION` / `SSH_TTY` set), Linux boxes without `DISPLAY`/`WAYLAND_DISPLAY` (CI runners, headless containers), or when `NYXID_NO_WIZARD=1` is set. The CLI creates a short-lived server-side pairing record and prints a pair URL + 8-char Crockford code on `FRONTEND_URL/cli/pair`. The user opens the URL on ANY device with a browser (phone, desktop), logs in, enters the code, and completes the same wizard there. The CLI polls for the typed ack. Same visual experience, same DisplayOnce affordances.

The selection is automatic — callers don't need to pick. The only caller-facing knob is `--no-wait`, which forces Mode B regardless of `is_wizard_eligible()` because it's designed for agent wrappers that want a resumable handoff instead of blocking on a live wizard.

Full specs: [`docs/CLI_WIZARD_V2.md`](../../docs/CLI_WIZARD_V2.md) (v2) + [`docs/CLI_WIZARD_V3.md`](../../docs/CLI_WIZARD_V3.md) (v3 / v3.1). v4's pairing transport lives under `/cli-pairings/*` backend endpoints and `/cli/pair` on the frontend.

**Visual consistency.** Both transports share the same shell: same brand lockup (NyxID wordmark in DM Serif Display), same ✓/✗/⚠ overlay system, same purple accent (`#8b5cf6` / `#7c3aed`), same button and field styling. The local path's footer says "Served locally from 127.0.0.1 · Nothing leaves your machine"; the remote path omits that footer because the page is served from the NyxID frontend origin — but secrets still never leave the browser (the CLI receives only non-secret identifiers via the pairing ack).

**Agent handoff with `--no-wait`.** For agents that can't block on the pairing URL streaming out of stdout, every wizard-capable command accepts `--no-wait`: the CLI creates the pairing, prints a JSON payload on stdout with `{pairing_id, code, pair_url, resume_cmd, requires_access_token_on_resume, expires_at}`, and exits 0 immediately. The agent relays `pair_url` to the user and later runs the printed `resume_cmd` (or `nyxid pairing resume <pairing_id>`) to pick up the result. `--no-wait` works regardless of TTY state.

For scripted / agent use, the wizard is **bypassed** (falls through to the pre-wizard stdin / rpassword path) when ANY of these is true:

- `--terminal` (alias `--no-wizard`) is passed — per-invocation override, available on all five wizard commands.
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
