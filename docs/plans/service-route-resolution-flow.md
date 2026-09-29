# Service routing, billing and dependency presentation

Status: proposed flow. The current refresh implements connection metadata and
insight panels; it does not implement ordered cross-owner fallback.

## One service, one explained route

Keep the collapsed service card and its expandable connection table. A group of
connections is not automatically an executable route. Show a service call slug
only after the backend has an explicit route configuration for it; preserve
individual connection identity and direct addressing.

Use three distinct terms:

- **Connection order** selects the connection to execute. Default to personal,
  organization, then platform when available. A saved custom order can put
  platform first. Personal A and Personal B remain distinct connections.
- **Billing flow** explains the payer and rates after caller context and final
  credential selection. Moving to another connection may change the payer.
- **Derived from** describes catalog/copy lineage. It has no routing or billing
  meaning by itself.

## Expanded card

The card header contains a caller selector: **For: You / managed agent key**.
Verified application identity appears with a recorded request; an arbitrary app
name is not a credential or execution context.

Directly below, show the expected result in one compact line:

`Next: ChronoAI OpenAI · Bills: ChronoAI · NyxID rate: [effective unit price]`

The table stays inside this card and makes the effective order visible:

| Order | Connection / source | Route state | Credential | NyxID payer | Rate |
| --- | --- | --- | --- | --- | --- |
| 1 | NyxID OpenAI | Skipped · reason | NyxID key | Acting person | Applicable platform-key rate |
| 2 | ChronoAI OpenAI | Selected now | Organization key | ChronoAI | Applicable own-key rate |
| 3 | Personal A | Fallback | Your key | You | Applicable own-key rate |
| 4 | Personal B | Fallback | Your key | You | Applicable own-key rate |

This is an illustrative layout, not a claim about live availability or prices.
Render only real, authorized sources; an absent platform connection has no row
or empty placeholder. If an existing visible connection becomes unusable, retain
it with a concrete skip reason. Do not expose inaccessible connection metadata.

Source-priority controls are explicit. Drag handles reorder connections within
their pool; card-grid ordering does not configure routing. Pinning a connection
is a separate mode with no fallback, and does not masquerade as an agent
credential override. A credential override can change the credential class and
payer without changing the selected connection.

Billing and dependency information expands in the existing single row panel:

`Caller → selected connection → effective credential → payer → rates`

Show ownership, source catalog, direct predecessor when recorded, and change
history under **Details / Derived from**. A common catalog association does not
prove that one connection was copied from another. Keep private configuration
editor-only; authorized readers retain permitted history.

## Execution contract

Persist the chosen source policy and pool member priorities as execution
configuration, independently of saved list filters. Add an ordered strategy and
caller-aware cross-owner eligibility; the existing pools only select enabled,
active same-owner members using round-robin or weights.

The real execution path and read-only explanation must use the same candidate
eligibility rules. For the authenticated caller, check live service scopes,
organization access, platform grants, enabled state, credential availability,
applicable overrides, node availability and request capabilities. Use credential
validation evidence with timestamps; missing or stale evidence is unverified,
not proof of a working connection. Execution still validates/materializes the
credential and rechecks authorization before provider effects.

Select the first eligible candidate in the saved order. If none is usable,
return a structured no-usable-connection error and safe reasons. Do not silently
use an unauthorized platform key. Exact connection calls retain exact semantics.

After final credential selection, use the existing billing owner and pricing
resolvers. NyxID master keys charge the acting person's account; organization
credentials normally charge the service owner organization; personal credentials
charge the person. Shared OAuth applications and agent credential overrides
require the actual credential-class resolver, not a source badge heuristic.

Funding is a separate order inside the resolved account: matching allowance
units, grant credits, then wallet credits. A platform → organization → personal
connection order is never an implicit permission to cascade charges through
unrelated wallets. Show unit rates before a request and actual coverage/debit
after settlement. Own provider charges can be separate from NyxID charges.

Automatic failover must be bounded and replay-safe. Skip known unusable
candidates before dispatch; do not replay an ambiguous request after provider
effects or streaming have begun. Any real attempted upstream work can incur
cost; record attempts independently rather than promising only successful calls
are charged. Any permitted automatic switch to a different payer must be part
of the saved policy and visible in the table.

## Preview versus recorded result

The explanation API reports **Expected route now**, ordered candidates and skip
reasons for the selected caller. Reading it does not call providers, refresh
credentials, reserve credits or charge anything. Reuse execution policy logic
without stateful execution effects. The preview can change before a call.

The request record is authoritative for what happened. Capture the route/policy
version, candidate attempts and reasons, selected exact connection, verified
caller/app identity, final credential class, billing owner, rate basis and
settlement reference. Show the latest three requests inside the card; full
history remains on the service page. Old events without these fields remain
explicitly incomplete.

The current `/api/v1/service-insights` endpoint in draft PR #1685 explains exact
connections and recorded callers. It must be extended to explain a configured
route once the shared ordered resolver exists. Deploying that endpoint alone
does not create cross-owner fallback.
