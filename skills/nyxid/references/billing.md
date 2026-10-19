# Billing, managed execution and credit grants

The billing commands in this reference (`admin billing`, `admin usage`, `admin
credits` and `billing`) require a human account session. The selected credential
is checked locally before any request; explicit or environment token overrides
take precedence over the saved profile. API keys are refused. Use a separate
profile authorized with `nyxid login --profile billing-admin`; retain the agent
profile for its normal execution scope. Billing reads are self-scoped; admin
billing reads require admin/operator authority and admin billing writes require
admin. The server separately authorizes credit grant managers.

Every leaf accepts `--profile`, `--base-url` (or `NYXID_URL`) and `--output
json`. JSON results go to stdout; human tables and diagnostics go to stderr.
Exit codes are 0 for success/ready, 2 for not ready/pending/broken, 1 for
errors, and 3/4 for existing login/refresh failures. HTTP errors in JSON mode
include `error: http_error`, `status`, `path` and the JSON or text `body` on
stderr.

Billing commands, catalog apply, proxy requests and MCP tools/calls check saved
sessions before work and refresh expired tokens. Reads, including MCP discovery
and proxy GET/HEAD, retain 401 refresh and replay. Billing configuration and
grant writes, other proxy methods and MCP execution calls do not replay on 401.
Catalog apply retains refresh and replay; wallet provisioning and checkout
exceptions are described under [Fund a person](#fund-a-person). Explicit or
environment credentials never switch to a saved session for refresh.

## Inspect billing

```sh
nyxid admin billing config show --output json
nyxid admin billing overview
nyxid admin billing rates --service telnyx-platform
nyxid admin billing rates --include-retired --output json
nyxid admin billing service show telnyx-platform
nyxid admin billing ledger verify --all
nyxid admin billing ledger verify --from-seq 1 --to-seq 1000 --limit 100
```

Service references accept catalog slugs or UUIDs. Ambiguous slugs require an
explicit UUID. Service show joins effective prices, class charging reasons, raw
billing lanes and the managed contract; BSON tariff dates render as RFC3339.
Prices and cached rates remain exact decimal strings. Rates can be filtered to
the service's configured metric codes; retired rows are hidden by default.
Config shows secret presence only, never deployment secret values. Ledger
`--all` follows bounded verification cursors and exits 2 for a broken chain or
an invalid anchor. An unanchored recent tail remains within the server's
anchoring interval; use the audit verifier for the anchor's chain linkage.

## Change defaults or prices

```sh
nyxid admin billing config set --default-overdraft-cap-credits 5
nyxid admin billing config set --default-overdraft-cap-credits default
nyxid admin billing service update telnyx-platform --file billing.json --dry-run --output json
nyxid admin billing service update telnyx-platform --file billing.json
nyxid admin billing service resync telnyx-platform
nyxid admin billing service verify telnyx-platform --managed --output json
```

The overdraft default accepts whole credits in 0..=1,000,000,000,000 and applies
to newly provisioned wallets. `default` clears the global override to the
environment value. Existing wallet caps are unaffected.

A billing file is either a bare billing object or `{"billing": {...}}`. Omitted
fields retain their current values; explicit null clears optional fields. The
CLI strips server-owned sync and cleanup fields and warns. A dry run prints the
merged billing preview and field diff without a PUT. Billing configuration
writes are never automatically replayed. Resync runs once and prints row status;
its `synced` flag means something changed, and exit 2 reports remaining unsynced
rows. Inspect state before deciding to retry.

Service updates send only the validated authored patch. The dry-run preview
follows the server's merge rules: lane patches preserve omitted fallback/resale
fields and omitted component/operation arrays. A billing block without either
lane key keeps the legacy full-block defaults for omitted platform/resale fields
(false, null or `tokens` as appropriate). Inspect the preview before applying
legacy-only changes; contract and credential-only settings retain omission
semantics independently.

## Author a Telnyx managed offering

Use verified facts, prices in **credits per token**, and a fresh tariff evidence
timestamp. Obtain a probe response under separately authorized vendor spending.
The dedicated template contains one text-chat operation and keeps platform
access disabled with a restricted, empty allowlist. It contains no billing or
credential material. The internal service uses the server defaults:
`requires_user_credential: false`, `identity_propagation_mode: none` and
`inject_delegation_token: false`. The template leaves these fields unauthored
and explicitly sets `forward_access_token: false`.

```sh
nyxid admin catalog add --template telnyx-platform --var model="$MODEL" --var max_output_tokens="$MAX_OUTPUT_TOKENS" --save telnyx-platform.catalog.json --dry-run
nyxid admin catalog apply --file telnyx-platform.catalog.json
nyxid admin billing service set-managed telnyx-platform \
  --model "$MODEL" --max-input-tokens "$MAX_INPUT_TOKENS" \
  --max-output-tokens "$MAX_OUTPUT_TOKENS" --max-request-bytes "$MAX_REQUEST_BYTES" \
  --max-response-bytes "$MAX_RESPONSE_BYTES" --tariff-version "$TARIFF_VERSION" \
  --tariff-source "$TARIFF_SOURCE" --tariff-verified-at "$TARIFF_VERIFIED_AT" \
  --input-price "$INPUT_PRICE" --output-price "$OUTPUT_PRICE" \
  --cache-read-price "$CACHE_READ_PRICE" --probe-response probe-response.json --dry-run
```

Remove `--dry-run` from set-managed only after reviewing its body, diff and
conservative full reservation ceiling. It configures an input-token primary and
output/cache-read components, enables platform billing only for NyxID
credentials, and clears operation overrides, BYOK, legacy and resale billing.
Model identifiers use the template's `model_id` type (which permits `/` and
`:`); integer output bounds render as JSON numbers. Other string variables
retain their restricted charset.

Applying a manifest that disables platform access or empties a live service's
owner allowlist requires an interactive confirmation. For non-interactive use,
run `nyxid admin catalog apply --allow-access-change --file <manifest>` after
reviewing the change. `catalog add --yes` does not authorize this access change;
`--dry-run` only warns. Review the Telnyx template's disabled access defaults
before reapplying it to a pilot.

If catalog creation succeeds but a follow-up update fails, apply reports
`created`, the service ID and the failed fields, then exits 1. Rerun apply to
reconcile the existing service. An authored `requires_user_credential` is an
assertion checked against the server response; a mismatch is an error because
the field cannot be set directly.

Submit the upstream credential using `nyxid service update telnyx-platform
--catalog-admin --credential-env TELNYX_PLATFORM_KEY`, with its value supplied
securely through the environment. Never put it in argv or a manifest.

`nyxid admin billing service verify <service> --managed` checks billing/Lago
availability, active service, configured credential, dedicated
restricted settings, fresh tariff evidence, one active POST chat endpoint with
pinned model/output schema, an exact proxy allowlist, the required platform-only
billing flags, three synced prices with exactly matching fresh nonretired cache
rates, and the effective platform charging reason. It can pass while access is
disabled. It does not prove replica versions, payer funding, tariff provenance
or upstream behavior. Verify those independently before enabling access.

## Fund a person

```sh
nyxid billing wallet --owner-id "$OWNER_UUID" --output json
nyxid billing wallet --provision --owner-id "$OWNER_UUID"
nyxid billing topup --amount-credits 5 --open
nyxid admin credits grant issue --user "$PILOT_UUID_OR_EMAIL" --amount 5 --service telnyx-platform --reason "Attended Telnyx pilot"
nyxid admin credits grant list --user "$PILOT_UUID_OR_EMAIL" --output json
nyxid admin credits grant revoke "$GRANT_UUID"
```

Wallet `--owner-id` always uses POST `/billing/wallet` with the owner in its
JSON body, even without `--provision`. This provisions the selected wallet if
missing, as the personal GET does. The server checks org access. Provisioning
and top-up checkout POSTs retain saved-session 401 refresh and one replay:
provisioning is idempotent, and checkout reuses the same idempotency key in both
`topup` and `verify-topup-flow`.

Grants accept 1..=1,000,000 whole credits. Repeat `--service` for multiple
catalog services or use `--all-services`. Optional `--expires-at` is an RFC3339
instant. A nonblank `--reason` and confirmation (or `--yes`) are required. Issue
creates a new batch on every successful invocation; it is not idempotent. Save
the returned `batch_id` and check every recipient's `activation_state`. Pending
activation grants are unspendable until their issuance is journaled. A grant
does not provision a wallet/subscription or activate a person's billing rollout.
Allowances and recurring schedules are outside this CLI surface.

Grant lists expose `total` and accept `--page` and `--per-page` (1 through 500).

## Execute and correlate

These HTTP POST and MCP tool calls can spend real credits and vendor funds.
Authorize spending and enable only the intended restricted owners first.

```sh
nyxid proxy request telnyx-platform ai/openai/chat/completions --method POST \
  --header 'Content-Type: application/json' --data @chat-request.json --output json --envelope
nyxid mcp tools --service telnyx-platform --output json
nyxid mcp call telnyx-platform__chat_completions --args @chat-request.json --envelope --output json > mcp.json
nyxid billing executions list --since "$(jq -r .started_at mcp.json)" --output json
nyxid billing executions show "$EXECUTION_UUID" --wait --timeout-secs 120 --output json
nyxid admin usage --from "$(jq -r .started_at mcp.json)" --to "$(jq -r .finished_at mcp.json)" --service telnyx-platform --user "$PILOT_UUID" --output json
nyxid admin usage --period 24h --page 1 --per-page 25
```

Proxy stdout stays the provider body unless `--envelope --output json` is given.
An execution header prints `Execution: <uuid>` on stderr. The envelope includes
HTTP status, execution ID and a JSON-or-text body. Its `headers` object includes
only `x-nyxid-execution-id`, `x-nyxid-agent-id` and `x-request-id` when present.
A managed pending result exits 2 with `error: managed_execution_pending` and
`execution_id`; inspect it and do not automatically repeat the paid request.

The native MCP client uses Streamable HTTP, negotiates a session and parses JSON
or SSE. `mcp tools --service` enumerates the service's complete visible
operation list from GET `/mcp/config` and reads every page of `tools/list`. For
lazily exposed tools absent from that list, it queries `nyx__search_tools` with
each exact fully qualified name and accepts only the matching tool's
server-built input schema. Any missing or invalid schema fails loudly. Service
calls use `nyx__call_tool` to activate and execute the named tool; native
meta-tools are called directly. The client reinitializes once on HTTP 404 or
JSON-RPC session-expired -32002 only when a session was sent. A 404 during
initialization is an `http_error`.

MCP calls wait indefinitely by default, matching `proxy request` (the transport
still has a 10-second connection timeout). Set `--timeout-secs` to bound the
entire handshake and result stream. A timeout before `tools/call` is dispatched
exits 1 and reports that nothing was executed. After dispatch, a timeout exits 2
with `managed_outcome_uncertain`, the tool and timestamp bounds. Check execution
state before retrying: the provider may have completed and charged the call.

Tool `isError` exits 1, or 2 with its execution ID when managed usage is
pending. Successful MCP envelopes provide `started_at` and `finished_at` RFC3339
bounds. An execution ID is emitted only when known (for example, a pending
result).

Correlate the self-scoped latest-50 execution list using the saved envelope's
`started_at` for `--since`, then select only executions of the called service
created at or before `finished_at`. The list has no pagination, owner override,
usage breakdown or settlement amounts. Plain `executions show` exits 0 even for
nonterminal state. With `--wait`, it waits for `settled`/`released` and exits 2
if the timeout expires while the execution is nonterminal.

Admin usage accepts a period or an RFC3339 from/to window, limited to 31 days.
It shows service/credential-class token quantities and exact gross, wallet,
grant and allowance costs. Compare an attended window with provider usage; it
cannot provide per-execution settlement evidence. Public tariff list rates do
not prove vendor invoice parity.
