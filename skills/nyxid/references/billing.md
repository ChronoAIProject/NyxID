# Billing, managed execution and credit grants

All `nyxid admin` and `nyxid billing` commands require a human account session.
An Agent Key profile or credential is refused locally before any request. Use a
separate profile authorized with `nyxid login --profile billing-admin`; retain the
agent profile for its normal execution scope. Billing reads are self-scoped;
admin reads require admin/operator authority and billing writes require admin.
The server separately authorizes credit grant managers.

Every leaf accepts `--profile`, `--base-url` (or `NYXID_URL`) and `--output json`.
JSON results go to stdout; human tables and diagnostics go to stderr. Exit codes
are 0 for success/ready, 2 for not ready/pending/broken, 1 for errors, and 3/4 for
existing login/refresh failures. HTTP errors in JSON mode include `error:
http_error`, `status`, `path` and the JSON or text `body` on stderr.

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
explicit UUID. Service show joins effective prices, class charging reasons,
raw billing lanes and the managed contract; BSON tariff dates render as RFC3339.
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

A billing file is either a bare billing object or `{"billing": {...}}`.
Omitted fields retain their current values; explicit null clears optional
fields. The CLI strips server-owned sync and cleanup fields and warns. A dry
run prints the merged request body and field diff without a PUT. Write requests
are never automatically replayed. Resync runs once and prints row status;
its `synced` flag means something changed, and exit 2 reports remaining unsynced
rows. Inspect state before deciding to retry.

## Author a Telnyx managed offering

Use verified facts, prices in **credits per token**, and a fresh tariff evidence
timestamp. Obtain a probe response under separately authorized vendor spending.
The dedicated template contains one text-chat operation and keeps platform
access disabled with a restricted, empty allowlist. It contains no billing or
credential material.

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
output/cache-read components, enables platform billing only for NyxID credentials,
and clears operation overrides, BYOK, legacy and resale billing. Model identifiers use the template's
`model_id` type (which permits `/` and `:`); integer output bounds render as JSON
numbers. Other string variables retain their restricted charset.

Submit the upstream credential using `nyxid service update telnyx-platform
--catalog-admin --credential-env TELNYX_PLATFORM_KEY`, with its value supplied
securely through the environment. Never put it in argv or a manifest.
`service verify --managed` checks billing/Lago availability, active service,
configured credential, dedicated restricted settings, fresh tariff evidence,
three synced prices with exactly matching fresh nonretired cache rates, and
the effective platform charging reason. It can pass while access is disabled.
It does not prove replica versions, payer funding, tariff provenance or upstream
behavior. Verify those independently before enabling access.

## Fund a person

```sh
nyxid billing wallet --owner-id "$OWNER_UUID" --output json
nyxid billing wallet --provision --owner-id "$OWNER_UUID"
nyxid billing topup --amount-credits 5 --open
nyxid admin credits grant issue --user "$PILOT_UUID_OR_EMAIL" --amount 5 --service telnyx-platform --reason "Attended Telnyx pilot"
nyxid admin credits grant list --user "$PILOT_UUID_OR_EMAIL" --output json
nyxid admin credits grant revoke "$GRANT_UUID"
```

Wallet `--owner-id` is sent on both GET and provisioning; org access is checked
by the server. Grants accept 1..=1,000,000 whole credits. Repeat `--service` for
multiple catalog services or use `--all-services`. Optional `--expires-at` is an
RFC3339 instant. A nonblank `--reason` and confirmation (or `--yes`) are required.
Issue creates a new batch on every successful invocation; it is not idempotent.
Save the returned `batch_id` and check every recipient's `activation_state`.
Pending activation grants are unspendable until their issuance is journaled.
A grant does not provision a wallet/subscription or activate a person's billing
rollout. Allowances and recurring schedules are outside this CLI surface.

## Execute and correlate

These HTTP POST and MCP tool calls can spend real credits and vendor funds.
Authorize spending and enable only the intended restricted owners first.

```sh
nyxid proxy request telnyx-platform ai/openai/chat/completions --method POST \
  --header 'Content-Type: application/json' --data @chat-request.json --output json --envelope
nyxid mcp tools --service telnyx-platform --output json
nyxid mcp call telnyx-platform__chat_completions --args @chat-request.json --envelope --output json
nyxid billing executions list --since "$T0" --output json
nyxid billing executions show "$EXECUTION_UUID" --wait --timeout-secs 120 --output json
nyxid admin usage --from "$T0" --to "$T1" --service telnyx-platform --user "$PILOT_UUID" --output json
nyxid admin usage --period 24h --page 1 --per-page 25
```

Proxy stdout stays the provider body unless `--envelope --output json` is given.
An execution header prints `Execution: <uuid>` on stderr. The envelope includes
HTTP status, execution ID, allowlisted NyxID headers and a JSON-or-text body.
A managed pending result exits 2 with `error: managed_execution_pending` and
`execution_id`; inspect it and do not automatically repeat the paid request.

MCP tools/call uses native Streamable HTTP, negotiates a session and parses JSON
or SSE. It reinitializes once only for explicit HTTP 404/session-expired -32002.
Human sessions expose service tools lazily: `tools --service` supplements
`tools/list` with `nyx__search_tools`, and service calls use `nyx__call_tool` to
activate and execute the named tool. Native meta-tools are called directly.
Tool `isError` exits 1, or 2 with its execution ID when managed usage is pending.
Successful MCP results currently omit execution IDs: correlate the self-scoped
latest-50 execution list with a tight `--since` timestamp. That list has no
pagination, owner override, usage breakdown or settlement amounts. Show waits
for `settled`/`released`, with exit 2 on a nonterminal timeout.

Admin usage accepts a period or an RFC3339 from/to window, limited to 31 days.
It shows service/credential-class token quantities and exact gross, wallet,
grant and allowance costs. Compare an attended window with provider usage;
it cannot provide per-execution settlement evidence. Public tariff list rates
do not prove vendor invoice parity.
