# Per-operation pricing (Tools and X channels)

Status: draft, in implementation since 2026-10-09; semantics in section 2 are the working decisions.
Owner: implementation worker; reviewed by the session lead.
Related: #1819 / PR #1827 (X channel lane), `docs/plans/2026-10-08-tools-implementation-plan.md` (Tools Phase 1), `docs/PLATFORM_KEYS_AND_INFERENCE.md` (lanes), `docs/BILLING_EXACT_ACCOUNTING.md`.

## 1. Goal

Today a service charges one rate per credential lane (`ServiceBilling.byok_pricing` / `platform_key_pricing`), plus optional additive components on other metrics. Every operation on a service costs the same. Upstream APIs do not: X charges $0.010 per received DM event, $0.015 per DM send, $0.010 per user read, and different amounts for posts. Tools (published API operations) need prices per operation so NyxID can charge what each call costs.

Add **operation prices**: within a lane, an admin can price individual operations. A request that matches a priced operation is charged that operation's rate instead of the lane's base rate. Everything else keeps today's behavior.

## 2. Semantics (fixed decisions)

- **Scope**: operation prices live inside a lane: `LanePricing.operations: Vec<OperationPrice>`. Byok and platform-key lanes are priced independently, exactly like `components`.
- **Unit**: operation prices are per request. They are allowed only when the lane's primary metric is `requests`. Validation rejects them otherwise.
- **Replacement, not addition**: a matched, synced operation price replaces the lane's primary request rate for that request. Additive `components` on other metrics are unchanged.
- **Fallbacks**: an unmatched operation, an operation without a price, or an operation price that is not yet `synced` uses the lane's primary rate. An explicit `"0"` operation price makes that operation free. An unsynced lane primary keeps today's rule (whole lane falls back to legacy or free) and ignores operation prices.
- **Credentials-only restriction** (`platform_charge_nyxid_credentials_only`) still applies after selection, unchanged.
- **Allowances**: operation rows remain metric `requests` on the same service, so existing `requests` allowances still apply. Funding precedence (allowance units, grants, wallet) is unchanged.
- **No new error codes.** Validation failures use the existing validation error.

## 3. Operation keys

An `OperationPrice.operation` is a stable string key:

1. **HTTP operations**: the `ServiceEndpoint.name` of an active operation on the service (for overlays, the overlay `operationId`, e.g. `get_me`, `create_tweet`, `send_dm`).
2. **Declared channel operations**: operations a channel adapter bills that have no catalog endpoint. They are declared in code by the channel billing layer for its catalog service, with a human label.

Validation accepts a key only if it is an active endpoint name on that service or a declared channel operation for that service. Keys are unique per lane.

### X channel mapping (`api-twitter`)

| Channel billing op (today's request id prefix) | X API call | Operation key |
|---|---|---|
| `x-account-verify` | `GET /2/users/me` | `get_me` (same X endpoint as the catalog op) |
| `x-post-reply` | `POST /2/tweets` | `create_tweet` (same X endpoint as the catalog op) |
| `x-dm-send` | `POST /2/dm_conversations/{id}/messages` | `channel_dm_send` (declared; differs from catalog `send_dm`) |
| `x-dm-received` | webhook event | `channel_dm_received` (declared) |
| `x-chat-received` | webhook event | `channel_chat_received` (declared) |
| `x-post-received` | webhook event | `channel_post_received` (declared) |

Keep the mapping in `channel_billing_service.rs` next to the existing X constants, and add a test that `get_me` and `create_tweet` exist in `backend/specs/catalog/twitter.openapi.json` with the same method and path the X adapter calls.

## 4. Data model and Lago

- `OperationPrice { operation, credits_per_unit, lago_metric_code, sync_status, sync_error }`, serde-defaulted so existing documents deserialize. Normalize prices with the existing price normalizer (12 fractional digits, cap).
- Lago: **one billable metric and one standard charge per priced operation**, code `platform_svc_{slug}_{byok|pk}_op_{key}` (key lower-cased, characters outside `[a-z0-9_]` replaced by `_`, total length within Lago's code limit; fail validation rather than truncate into a collision). Reuse the component sync path (`sync_lane_component` pattern), the plan charge round-trip with ids, durable cleanup markers (`component_cleanup_metric_codes` or an equivalent operation list), `retry_pending_service_prices`, and `retire_rate` on removal. Removing an operation price retires its rate row; history keeps pricing.
- Rate cache: one row per operation code, written by sync exactly like components. Reservations use `fresh_rate` on the operation code.
- Do **not** use Lago charge filters. The model-filter path is unused at reservation time today, and per-metric codes reuse the proven component machinery and keep drift comparison per code.
- Usage meter rows: add optional `operation: Option<String>` (stored only when an operation price was selected) for display. `lago_metric_code` is the operation code. Lago events need no new properties.

## 5. Request path

- `BillingRouteContext` gains an optional operation key input. `BillingRouteContext::new` stays the single selection point: after lane selection, if the lane primary is synced and the operation has a synced price, set `platform_lago_metric_code` to the operation code (metric stays `requests`).
- **No extra database reads on unrelated paths**: resolve the operation only when the selected lane has at least one operation price. Tools already load active endpoints in `tool_publication_service::gate`; change it (or add a sibling) to return the winning endpoint name so the proxy reuses it. For non-tool catalog services whose lane has operation prices, load that service's active endpoints once and match with the same most-specific rule. Extract the matcher from `require_published_operation` into one shared function; do not duplicate it.
- MCP tool calls already know the endpoint; pass its name directly.
- X channels: `ChannelBilling` passes the operation key from the mapping above for each billed event.
- Add a test with the existing MongoDB command-monitoring helper proving a proxy request to a service without operation prices issues no endpoint query.

## 6. API, admin UI, user UI, CLI

- `LanePricingView` (catalog, keys, MCP, tools) gains `operations: [{ operation, label, credits_per_unit, sync_status }]`. Admin `ServiceBilling` responses carry the full `OperationPrice` list. Register new schemas in `api_docs.rs` and extend the OpenAPI contract test.
- Admin update: `PUT /services/{id}` accepts `byok_pricing.operations` / `platform_key_pricing.operations` with the same presence rules as `components` (omitted preserves, `null`/`[]` clears). Extend `BillingUpdate` presence tracking accordingly.
- Admin form (`platform-service-fields.tsx`): under each charged lane with metric `requests`, an "Operation prices" table listing the service's active endpoints and declared channel operations, each "Base price" or a price input with sync status. Use `useAppForm`, follow DESIGN.md.
- `/tools` card: show "From {min} to {max} credits per request" when operation prices exist, else today's line. Tool detail lists each operation's price.
- Usage views (`GET /billing/usage`, admin usage) show the operation label when present (additive field).
- CLI: if `cli/src/commands` has lane price authoring or display, mirror operation prices there; otherwise no CLI change.

## 7. Rollout and docs

- Upgrade all replicas before authoring operation prices: old replicas ignore the field and would charge the base rate. State this in `docs/PLATFORM_KEYS_AND_INFERENCE.md`, `docs/TOOLS.md` and the CLAUDE.md lane paragraph (one sentence).
- `docs/CHANNEL_BOT_RELAY.md`: replace "one configured request rate applies to each..." with the X operation table and how to price it.
- No code seeds prices. After deploy, an admin prices X operations at X's published rates; look them up at https://docs.x.com/x-api/getting-started/pricing at that time.

## 8. Tests (required)

- Route context: operation price selected when synced; pending/failed falls back to base; zero price free; unmatched op uses base; non-`requests` lane rejects operation prices; credentials-only restriction still applies.
- Validation: unknown key, duplicate key, bad price, metric mismatch, code-length overflow.
- Sync: create/update/remove operation charges, plan charge round-trip with ids, cleanup retry, rate retirement and history pricing.
- Proxy: tool operation priced end to end (reservation, settlement, meter row code and operation); no endpoint query without operation prices.
- MCP tool call priced by endpoint name.
- X channels: each of the six channel operations settles at its own price; unpriced ones at the base rate; grant funding still first.
- Allowance covers an operation-priced request.
- Usage response includes the operation label.
- Frontend: operation price editor (add, set base, clear, sync status) and tools price range rendering.

## 9. Verification

Run on a MongoDB 8.0 replica set (MongoDB 7.0 breaks billing settlement tests): `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, the touched backend test modules, `npm run lint`, focused vitest, `npm run build`, `git diff --check`. If the CLI wizard source closure changes, rebuild the wizard bundle per CONTRIBUTING.md.
