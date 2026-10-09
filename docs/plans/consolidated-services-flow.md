# NyxID Services: consolidated flow

Revised 27 September 2026 · One collapsed card per service, with its connections inside.
Execution and data contracts reviewed with Fable on 25 September; card layout
revised below to reflect the user’s subsequent direction.
Grounded in main `1b031c77`. This describes the target experience and required
server guarantees. The grouped collapsed-card UI is now implemented in the local
React frontend and uses production service data. Backend fallback, complete caller
attribution and the remaining target behavior below are not claimed implemented.
It supersedes the earlier UI proposal and its terminology.

## The experience in one minute

**One collapsed card per service by default. Its duplicate/configured connections
live inside that card. Expand to compare them; click into one to see everything.**

1. See **OpenAI · 3 connections** once in the service grid. Personal, work and
   sandbox configurations do not occupy three separate top-level cards.
2. Click **3 connections** or Expand. The same card grows to reveal three named
   connection cards within its border, each with owner, address, credential state,
   latest caller and an Open action.
3. Click the service title or **Open service** for its complete grouped view.
   Click **Work account → Open** for that concrete connection’s Route, Activity
   and Details. Its history and identity remain distinct from the others.
4. When an actual canonical address exists, its routing summary is labelled with
   that address. Change its source in this card’s route section: Automatic keeps
   personal → organization → platform; a specific-source choice is explicit.
5. Pools retain their own cards. Only a real pool’s Priority strategy enables
   dragging its member services. A group of same-service connections is not a pool.

The page stays collapsed on initial load. Expansion is local to each service and
is restored when returning from details. All connection data remains inside the
parent card or that service’s full page; there is no data panel beneath the grid.

## Display reference

**[Open the live frontend](http://127.0.0.1:4317/keys?view=routing)**.
This is the real React application, reading service/catalog metadata from
`https://nyx-api.chrono-ai.fun`. If the local session has expired, use the
[production sign-in flow](http://127.0.0.1:4317/__routing-preview/login).

The implementation groups actual service records into collapsed cards. Expanding
a card contains the original connection cards and metadata within the parent;
clicking a connection opens its existing full detail/history page. Search matches
nested connections without losing siblings. The normal `/keys` grid uses the same
component. It does not invent routing decisions or last-caller evidence absent
from the backend.

The earlier `services-card-reference.html` attachment is a historical design
reference, not the review surface. Use the live frontend link above. The updated
application passed 98 focused tests, TypeScript checking, lint and the production
build. The local server is serving the changed application modules. Signed-in
visual verification remains unavailable because no browser is connected.

The pattern is **a grouped disclosure card**: a collapsed service summary, nested
individual connection cards, and a full detail page. The two states below are
views of the same card:

```text
COLLAPSED                         EXPANDED
┌────────────────────────────┐    ┌─────────────────────────────────────┐
│ OpenAI                     │    │ OpenAI                              │
│ 3 connections              │    │ 3 connections                       │
│                            │    │                                     │
│ llm-openai · Automatic     │    │ llm-openai · Automatic              │
│ Would use Work account     │    │ Would use Work account              │
│ Checked 20s · for you      │    │ Checked 20s · for you               │
│                            │    │                                     │
│ Latest: Codex dev · 2m     │    │ ▾ Hide connections                  │
│                            │    │ ┌─────────────────────────────────┐ │
│ ▸ 3 connections     Open ↗ │    │ │ Personal production · You       │ │
└────────────────────────────┘    │ │ Needs reconnect          Open ↗ │ │
                                  │ └─────────────────────────────────┘ │
                                  │ ┌─────────────────────────────────┐ │
                                  │ │ Work account · Acme             │ │
                                  │ │ Selected for llm-openai  Open ↗ │ │
                                  │ └─────────────────────────────────┘ │
                                  │ ┌─────────────────────────────────┐ │
                                  │ │ Sandbox · You                   │ │
                                  │ │ Exact calls only         Open ↗ │ │
                                  │ └─────────────────────────────────┘ │
                                  │                     Open service ↗ │
                                  └─────────────────────────────────────┘
```

The diagram illustrates the full target design. The live implementation includes
actual connection slugs, saved state, metadata and detail links; evaluated routing
and per-service last-caller summaries still require the backend contracts below.

## 1. A small vocabulary

| Term | What the user means |
| --- | --- |
| **Service** | A configured service with an identity, address and full details. Existing UserService records keep this name. |
| **Address** | The slug/API address a caller requests. It may resolve automatically, target an exact service, or select a pool member. |
| **Connection** | A particular configured instance inside a service card: for example Personal production, Work account or Sandbox. It retains its existing UserService ID and full details. This UI label does not create or merge OAuth grants. |
| **Source** | Whose credential supplies a request: Personal, a named organization, or NyxID platform. This is separate from the service's owner. |
| **Pool** | An existing collection of member services with one callable address and a selection strategy. |
| **Origin** | How this service was created. |
| **Variant** | A separate service explicitly created from another service. Its origin records that relationship. |

A catalog service is a template and catalog identity; some catalog addresses also
have execution behavior. Grouping cards by a catalog does not create a new callable
address, fallback policy, pool, shared history or readiness status.

## 2. Services home: collapsed service groups

Cards are the primary and default presentation. One top-level card represents one
service identity and contains its visible configured connections. The initial
view is collapsed; the count makes the contents discoverable. Use **Connections**
in the UI rather than **Duplicates**: these may be intentional accounts, endpoints
or variants, not redundant records to delete.

Search, Owner, Needs attention, Kind (Services/Pools) and Add act on this card
collection. Search includes nested connection names and addresses. A connection
match keeps the parent visible and marks that there is a match inside; it does not
create a detached result row. Apply permissions before grouping, counts and search.

### Grouping and identity

- Group catalog-backed configurations by their explicit shared catalog service
  identity. Do not group every OpenAI product together solely by provider name.
- Custom services stay separate unless they have an explicit common service/group
  identity. A copied name, slug suffix, matching URL or shared credential is not
  enough. Explicit variant provenance remains a relation even when a variant no
  longer belongs to the same service identity.
- Personal and permitted org connections for the same service appear inside the
  same card; each nested card labels its own owner. A parent group has no single
  credential owner. Disabled connections remain nested with their state; the
  count means configured visible connections, not working connections.
- Pools remain separate objects/cards and can contain members from several
  service groups. Grouping connections never creates membership or enables drag.
- Preserve immutable IDs, addresses, approvals, scopes and histories for every
  nested connection. No merging, deletion, automatic source preference or
  inheritance happens merely because the cards are grouped.

### Three levels of disclosure

| Level | Content | Interaction |
| --- | --- | --- |
| **Collapsed service card** | Name/icon, description, visible connection count; an explicitly labelled canonical-address summary when one exists; a scoped latest request | Click the count/chevron to expand, or Open service |
| **Expanded service card** | Same header; named individual connection cards with owner, exact address, credential/route state and latest caller; canonical source controls scoped to that address | Compare or expand individual connection details inside the card; open a connection |
| **Full service / connection** | Complete Route, Activity and Details; full configuration, requests, changes, origin and related services | Service title opens the group; a nested Open action opens that exact connection |

The service card expands vertically or takes more columns where space allows.
Its border encloses all nested cards and controls. Neighbors reflow in stable
reading order. Mobile uses the available width with a single column of nested
connections. There is no full-row detail area outside the parent border and no
separate inspector below the grid.

Expansion is independent: users can compare several expanded service cards.
Keep expansion for the current browsing session and restore it on return from
full details; a fresh visit starts collapsed. This is presentation state, never a
routing preference. Show a bounded first set for unusually large groups, with
**Show all N connections** expanding inside that card or opening its full service
view. Never silently omit connections or make a hidden subset look like the total.

### Resolution and last use belong to named targets

A group is a browsing container. When a real canonical address exists, display
it explicitly, for example **llm-openai · Automatic**. Any **Would use Work account**
label belongs to that address and includes caller, operation and evidence age.
An alternate configured connection is not automatically a fallback candidate.
Distinguish **Selected for llm-openai**, **Preferred personal source**, **Exact calls
only**, **Disabled**, and pending verification using server facts.

Without a real canonical address, the group shows its connection count and
**Open a connection**; it does not acquire an invented Automatic mode or API URL.
Its latest visible request may summarize the authorized group feed only when it
names the actual connection. A canonical latest-request summary labels that
address. Each nested connection shows its own latest request. Missing evidence
says Not recorded; a group never receives a blanket Ready status.

**Change source** edits only the named canonical address. Each connection keeps
an Open/Copy exact address action. Platform offerings appear only when real and
permitted; catalog branding never creates a fake platform connection in the count.
Auto-provisioned platform instances retain their mutation restrictions. Disabled
duplicates remain distinct by ID; deleted services move to authorized archives.

## 3. Click into the service: Route / Activity / Details

The parent title and **Open service** open the complete service group. Route shows
its actual canonical behavior, when present, and all its connections. Activity
can aggregate authorized requests/changes only with the target connection named
on every event. It does not create a merged history or fabricate group authorship.
Details includes catalog metadata and links to each concrete configuration.

A nested connection’s title/Open action opens its own full page, scoped by its
immutable service ID: Route, Activity and Details belong only to that connection.
Keep a breadcrumb **Services → OpenAI → Work account**. The group and concrete
views visibly state their scope. **View all activity** retains the scope of the
card it was clicked from. Returning restores filters, scroll and expanded cards.
All permitted data is available by clicking in; the collapsed state loses no
configuration or history.

### Route: what will a call do?

Show the requested address first. Then one short resolution summary, source
selection, and a **Why this source** disclosure.

```text
OpenAI                                   llm-openai
Route    Activity    Details

Selection       Automatic                         Change
For             You · POST /v1/responses
Would use       Acme · Work OpenAI
Checked         20 seconds ago
Paid by         Acme · org wallet; provider charges to Acme’s key

Why this source
  Personal      Needs reconnect
  Acme          Selected · credential and route checks passed

Last completed  Codex dev · Personal · 2 minutes ago
```

The platform row is absent in this example because no actual permitted platform
source exists. An existing broken personal service stays visible with its repair
reason; it is excluded from the usable set. A configured platform binding that
later loses availability remains a repairable record in that service's details,
but is not offered as a usable fallback or selectable source.

**Do not use a generic “Ready via [source]” badge.** Distinguish these statements:

| Label | Evidence it requires |
| --- | --- |
| **Saved · Not verified** | Configuration exists; upstream validity has not been established. |
| **Would use Acme · checked 20s ago** | Server evaluation for this caller, operation, policy version and current route/credential state. It predicts selection, not upstream success. |
| **Verified 2m ago** | A supported credential/operation check with version-bound evidence and a stated scope. |
| **Last completed 2m ago · Codex dev** | An actual completed execution record; not credential preparation or response headers alone. |
| **Needs reconnect / Node offline / Approval required** | A specific actionable state. |
| **Unavailable** | Evaluation found no permitted usable source; the action explains the repair needed. |

“Working” cannot mean a permanent guarantee about a third party. The server must
validate authority, configuration and credential preparation at request time,
perform supported refresh, check transport and payment requirements, and exclude
known unusable sources. It must show verification evidence separately. Unknown
upstream health stays unknown; no generic active flag becomes proof of validity.
A read-only check never dispatches a business operation, advances a pool counter,
refreshes a credential, reserves money or grants approval. If such work is needed,
it reports **Refresh required**, **Verification needed**, or another pending gate.
A dedicated supported Verify action is separate and discloses any metered probe.

The target's **working-only rule** is explicit: Automatic and pool selection do
not select a credential merely because it can be decrypted. They require
provider/operation-appropriate validity evidence: a completed OAuth authorization,
successful refresh, supported validation, or a qualifying recorded success bound
to the same credential, endpoint and configuration version. No arbitrary past 200
proves every operation will work. Legitimate no-auth services use route/operation
checks appropriate to them.

**Missing evidence is not failure.** Execution performs a supported safe check
when possible. If a higher-priority candidate still cannot be assessed, return
**Verification required**; do not replace its identity with an org/platform
account merely because evidence is absent. Known unusable candidates can be
skipped under the saved policy. A pending gate blocks a definite preview.
Round robin/Weighted select only within the working set explicitly approved for
that pool; their editor discloses excluded/unverified members before activation.
For Priority, an unassessed higher member requires verification before proceeding
past it, unless the user explicitly excludes it from the pool.

Evidence includes its age and provider-defined validity. Credential/configuration
changes, known revocation and expiry require revalidation. An adapter may require
a freshness check, but elapsed time alone never marks a credential broken or
silently changes the selected identity. Show **Re-check required** and perform a
supported safe check; if unavailable, stop with an actionable state. Verification
still cannot guarantee that the third party accepts the next business operation.
An adapter may impose an age-based re-check only when it supplies a safe check.
For unsupported providers, version-bound evidence does not expire by age alone;
actual token expiry, configuration changes and classified failures still matter.

For providers without a safe check, offer **Use exact service** with a generated
exact URL/agent configuration, or an explicit user-requested first real operation.
This authenticates and executes once through that chosen service, labelled
unverified, and can produce qualifying evidence. It is never a hidden business
probe or an automatic exception to working-only selection. Exact requests still
enforce authorization, known credential failures, route, approval and billing.
Setup must show this path rather than leaving a newly added key stuck at Saved.
While unverified, Copy request and generated agent configuration must use the
server-provided exact target/selector, not the unresolved Automatic slug URL.

Default context is **You**. An inspectable agent/application context is available
only when the backend can evaluate its real scope and bindings and the viewer is
authorized to inspect it. Otherwise show recorded caller facts in Activity.
Changing this context does not impersonate the caller, execute a request or save
a preference. Advanced operation checks belong on the full page; a generic page
must not silently assume that permission for one operation grants all operations.
An unresolved higher-priority candidate prevents a definite selection preview:
show the pending check instead of claiming a lower-priority source will be used.

### Choosing the source

**Change** opens a focused editor with two choices:

- **Automatic**: personal → named eligible organization(s) → actual platform
  offering. Show only real authorized candidates. Users may exclude optional org
  or platform fallback; the remaining source order is fixed and visible.
- **Use a specific source**: select a permitted concrete personal/org service or
  the actual platform offering. This is strict: if it fails, return its error.
  Selecting Platform works even while a personal service is usable.

Platform joins Automatic only when an actual offering is available, its use is
permitted, and the payer already has explicit billing authorization covering this
route. Otherwise the editor requires opt-in with payer/pricing shown. Catalog
visibility or an existing wallet does not supply that consent. Migration never
enables new platform charges implicitly. Excluding a source removes it without
reordering the remaining tiers.

The editor shows **Applies to: your calls to this address**, the actual payer and
any changed pricing before Save. A caller's explicit exact target or agent
binding remains stronger than a saved preference. Org-owned settings require
org write permission; their editor names the affected org scope. Do not silently
save a personal choice as an org-wide setting. A per-call exact selection is
supported; Copy request uses the existing UUID/exact-call form or explicitly
includes `_nyxid_via` in the slug URL.
Hard constraints intersect: an exact request and an agent binding that conflict
produce an error, rather than one silently overriding the other.

Within a source tier, a user chooses a preferred concrete service when several
accounts are possible. Preserve an existing exact same-catalog match as the
migration default; select a sole compatible account when unambiguous. Do not pick
an arbitrary database record or move between unchosen accounts. An unresolved
account choice returns **Choose a source** instead of silently charging platform.
For multiple orgs, show the configured org order (initially primary org, then a
stable saved order); edit that order with explicit move controls. Dragging service
cards never changes source order. Intentional member failover belongs to a pool.

### What the API must enforce

Connect/Reconnect returns to the same service and starts supported safe validation.
The card progresses from Saved/Checking to Verified or a specific repair state.
A metered verification needs its own disclosed action. A repaired personal source
is preferred on the next Automatic call. Adding it does not change pool membership.
When no source qualifies, the Route panel shows Connect, Reconnect or Check as
appropriate; execution remains unavailable until the server requirements pass.

Canonical Automatic is a versioned, explicitly enabled address contract, not a
new interpretation applied to every catalog slug during a UI rollout. A viewer
gets its entry when it is enabled and has at least one visible personal/org
candidate or an included, authorized platform offering. A saved policy or prior
activity keeps its entry visible when candidates disappear, as Unavailable.
Discoverable catalog entries with no such configured route stay in the catalog.
New calls to an enabled canonical contract with no usable candidate return its
structured error; they never fall through to a hidden legacy platform source.
Legacy personal connections are explicit migration candidates, not a second
resolver consulted after the new one fails.

Existing exact/custom/pool address behavior is preserved until an explicit
migration. The migration preview names the affected addresses, agents, source
policy, evidence gaps and payer; it cannot enable Automatic while required
verification is unresolved. New setup likewise offers Automatic only after its
candidates and fallback choices are reviewed. Unsupported verification keeps a
usable exact address/configuration available; it does not silently move calls to
another account. Newly enabled tracking does not make existing traffic eligible
for new charges or force all old keys into a first-day fallback.

Inventory legacy implicit platform traffic separately, including callers with no
service row. Use final credential class plus legacy route markers to identify it;
a missing concrete service ID alone does not prove platform use. Moving these
callers onto the new contract requires explicit platform billing authorization.
Retiring implicit access without it is a deliberate, announced migration cutoff,
not covered by the preservation promise for existing exact/custom/pool contracts.

For an enabled canonical Automatic address, every call performs these steps:

1. Resolve the address contract and authenticated caller. Apply caller-wide scope,
   consent, operation and exact approval/binding restrictions.
2. Evaluate the selected personal candidate, then permitted organization
   candidates, then an actual platform offering. Each must independently pass
   account/operation compatibility, credential, transport and billing checks.
3. Skip only classified candidate failures that the saved policy permits, such
   as a missing credential, terminal refresh rejection, or no permitted online
   node route. Keep a safe reason. Missing/stale evidence requires checking or an
   explicit choice, not fallback. Caller-wide denials, unresolved explicit account
   choices, database/KMS failures and integrity errors are terminal.
4. Revalidate the selected identity and effective authority before dispatch.
   If an exact approval is needed, obtain it for that identity. An existing
   approval or agent binding for another identity never transfers to the fallback.
5. Dispatch once. Record the decision, target, verified caller and outcome. If no
   usable source exists, return a structured error before any provider call.

No legacy fallthrough may select a source excluded by the terminal decision.
An upstream timeout, error or interrupted stream after dispatch is that request's
outcome. It does not trigger replay under another account. Subsequent independent
calls evaluate again; a repaired personal source regains precedence in Automatic.
Do not globally invalidate credentials from an arbitrary resource 403, 429 or
provider outage. Candidate evidence is bound to identity, version and scope.

Canonical catalog addresses are explicit server-reported contracts. Exact UUID/
`_nyxid_via` selection, custom addresses and pool addresses retain their semantics.
Custom/pool collisions with catalog slugs require explicit migration; the UI must
not relabel existing traffic Automatic. A matching canonical personal record can
be the preferred candidate, but keeps its UUID, full card, exact-call link and
own history. There is no newly invented API address for a visual provider group.

## 4. Pools: ordering lives here

A pool has a self-contained card beside the service cards. Its expanded card
contains its strategy and member services; the full pool view’s Route tab
contains the same controls with complete member details. Current ownership rules
remain: all members must belong to the pool's owner. Provider similarity alone does not prove operation,
protocol, account or approval compatibility.

- **Round robin**: selects among eligible members per request.
- **Weighted**: selects among eligible members with the configured weights.
- **Priority**: selects the first eligible, usable member in saved order.

All three strategies use the same working set; their choice within it differs.
Preserve existing strategy/weights while adding the shared eligibility checks.

Only Priority displays drag handles on member services inside that pool’s
expanded card or full view. The card itself never becomes draggable. The user
moves member services, reviews the new order and saves. Keyboard Move up/down
controls do the same thing. Changes name the pool, actor and version and appear in its change history. Unsaved edits
do not affect traffic; concurrent edits cannot silently overwrite one another.

```text
Production AI · owned by You
Strategy: Priority

1  OpenAI primary   You   Needs reconnect · skipped
2  OpenAI backup    You   Credential and route checks passed
3  OpenAI spare     You   Disabled · skipped

One eligible member for the checked operation.
```

The saved order includes broken/disabled members so users can repair them. The
effective eligible set excludes them. Preview never consumes the round-robin
counter. No pool leaves its member set for a global org/platform fallback. A pool
member may already have an explicitly configured platform credential binding;
that remains the member's binding, subject to its own authorization and payer.
Priority is backend work: “first active row” is insufficient.

## 5. Activity: who used it, and what happened?

**Activity** has Requests and Changes filters, with Requests selected initially.
They share context and visual structure but retain separate records and clocks.

Scope is named at the top: **Through this address** for a canonical address/pool,
**Across visible connections** for the service group, or **Handled by this
connection** for a concrete service. Grouped events retain their concrete target
labels and current disclosure checks; the group is not an execution identity. Canonical Changes records
that routing policy's edits; concrete service Changes records its own edits.
Selecting a source opens that concrete connection’s full view with a back breadcrumb. Do not merge a canonical address's policy, a candidate's history and
other same-catalog services into one unnamed timeline.

A request entry shows **time · verified caller · operation · outcome**. Expanding
it shows:

- Requested address and mode at that time.
- Actual selected service and source, credential override if applicable, and node
  or pool member used. Selected-but-not-dispatched is distinct from executed.
- Safe skip reasons, actual payer, start/end time and request ID.
- The evidence captured for that request, not today's routing preview.

Example: **Codex dev · Agent key → production-ai → OpenAI backup · Personal →
Completed**. Opening the pool or the member reaches the same execution record.
One request is counted once, regardless of routing or billing event count.

Caller labels come from verified auth: **Alice · Session**, **Codex dev · Agent
key**, or **Release dashboard · Application · on behalf of Alice** when those
identities were authenticated. A user-assigned key name does not prove a particular
executable was running. User-Agent, service creator, provisioning app and key
owner are never substitutes for caller attribution. Generic keys show their
recorded name/identity with **Application not identified** when appropriate.

**Latest request**, **Last completed request** and **Last change** are separate.
A failed latest request must not erase an earlier completed one. Outcomes include
in progress, completed, failed, denied, disconnected and unknown. HTTP completion,
stream termination and WebSocket closure need protocol-specific rules; a 200
header or successful upgrade alone is not completed usage. “Completed” describes
the recorded transport/operation outcome, not proof of a provider's business
result. If a protocol cannot establish completion, keep unknown explicitly.

No data reads **Not recorded**, with **Recorded since [date]** when coverage is
partial. Neither a shared credential's `last_used_at` nor missing retained events
justifies “Never used.” Projections expose an observation window and update lag.

Visibility is enforced by the server before pagination, counts or summaries:

| Viewer | Request activity | Change history |
| --- | --- | --- |
| Personal service owner | Authorized activity for that service | Existing owner access |
| Scoped org admin | Activity within permitted org resources | Existing write + resource scope |
| Org member | Their own authorized requests, labelled **Your latest request** | Restricted under current history rules |
| Org viewer | No execution activity through this feature | Restricted under current history rules |

An org member's summary must be computed for that member; filtering a global
latest row is insufficient. Visible relationships, actors, targets and historical
snapshots need current disclosure checks too. Do not leak hidden caller names,
slugs or counts through provider headings, search, exports or related items.

## 6. Details: metadata, origin and variants

Preserve existing endpoint, protocol, auth method, safe headers, identity
propagation, node routing, catalog docs/capabilities/limitations, ownership,
credential binding, pricing and scoped-agent information. Show a compact summary
first, with technical sections expandable. Secrets retain existing protected
flows and never enter activity/history payloads.

**Origin** states who created the service, when and through which recorded
channel/application. Older data says **Origin not recorded**.

**Related** uses specific labels:

| Relationship | What it means |
| --- | --- |
| Based on OpenAI | Shared catalog template/configuration origin. |
| Created from Production OpenAI | A recorded derivation from that specific service ID. |
| Used by Production AI | Actual pool membership. |
| Shares credential with Work OpenAI | Shared credential reference, subject to disclosure rights. |
| Bound to Codex dev | Explicit agent service/credential binding. |
| Created through Release dashboard | Provisioning provenance, not last caller. |

**Create variant** is the explicit future action for a spin-off. It opens a form
prefilled from permitted configuration, shows what is copied, requires a new name/
address and explicit credential choice, and creates a new immutable service ID
with `derived_from_service_id`. It does not inherit approvals, grants or agent
scope, and edits to the parent do not propagate. Record the creation and origin
in authorized histories; opening a parent/child preserves a breadcrumb to the
originating service and the card overview’s state.

Credential replacement, rename and Enable/Disable are changes to the same service.
Similar slugs, shared keys, timestamps and `rotation_predecessor_id` are not
service lineage. Delete archives that ID's history; recreating the slug creates
a new identity and history. Authorized archived views retain safe snapshots.
Shared-credential edits already fan out through the existing journal, so there
is no extra “include related history” toggle.

## 7. Backend delivery: the point at which this is solved

The target flow requires backend work. A new layout alone cannot deliver it.

| Capability | Present in checked main | Required addition |
| --- | --- | --- |
| Rich service cards/details, ownership and lifecycle | Yes | Collapsed service groups, nested connection cards and complete scoped views |
| Explicit platform credential binding | Yes; availability is metadata | Preserve this; add strict source preference to the canonical resolver |
| Service authorship and change journal | Yes, with restricted readers | Reuse it; add routing-policy/pool-order/variant events where not recorded |
| Working personal → org → platform fallback | No common resolver; an unusable personal match can return early | Shared resolver, versioned policy, consistent preview/discovery/execution and typed terminal decision |
| General upstream verification | No; Codex has a specific flow | Truthful check states; supported evidence bound to credential/configuration version |
| Pool Priority | No; RoundRobin and Weighted only | Authorized operation-compatible eligibility, Priority strategy and versioned order edits |
| Who last used this exact service | Partial audit fields, no complete correlated view | Complete execution attribution and authorized summaries/feed |
| Service spin-off lineage | No general derivation edge | Explicit variant creation and immutable origin relationship |

Deliver in this order:

1. **Contracts and resolver.** Record address identity, policy ownership/version,
   request context, saved org order, candidate eligibility, typed skip/stop reasons,
   approval and billing gates. Evaluation is read-only; execution revalidates and prepares.
   All proxy paths, approvals, MCP/discovery and UI consume compatible decisions.
2. **Execution activity.** Mint a request ID at ingress. Thread it through
   selection, audit and metering. Record requested target/slug snapshot, concrete
   service/catalog/pool IDs, source, actor user, API key, authenticated application/
   delegation/child credential IDs when present, credential class, node, payer,
   dispatch state and terminal outcome. Separate protocol completion from headers.
   Pre-dispatch errors retain an execution record with no executed target.
3. **Reliable read models.** Build indexed, authorized recent-request queries and
   rebuildable latest/latest-completed projections, including per-actor org views.
   No N+1 audit scans. Persist terminal evidence with bounded retry/reconciliation;
   a dropped fire-and-forget update must leave an explicit gap, not a false success.
4. **Pool and provenance writes.** Add real Priority semantics, compatibility
   checks, concurrency-safe order saves and explicit variants/origin. Reuse safe
   history writers and their authorization rules.
5. **The consolidated UI.** Bind collapsed/expanded service groups, nested
   connection cards and their full scoped views to those contracts. Metadata/history improvements may ship earlier with accurate
   no-data states; Automatic/source preview and Priority controls ship only with
   their execution guarantees.

The execution contract can extend existing audit events and projections; this
proposal does not require a second competing audit log. New audit rows must use
the existing tamper-evident append path. Changes stay in the service journal.
Platform absence, allowed fallback, final payer and caller visibility come from
the server. `platform_key_available`, row ownership or a last-used timestamp is
never enough for the frontend to reconstruct these decisions.

## 8. Acceptance scenarios

| Scenario | Required result |
| --- | --- |
| First visit with three OpenAI configurations | One collapsed OpenAI card says 3 connections; no three duplicate top-level cards. |
| Expand one service card | Its own border encloses all visible connections, their resolution/last caller and metadata; no detached data area. |
| Open Work account inside OpenAI | Exact connection identity, configuration and history; breadcrumb returns to the expanded group. |
| Different service IDs happen to share a provider name | Separate groups unless an explicit common service identity exists. |
| Hidden org connection shares this service | It contributes no count, search result, caller or history to an unauthorized viewer. |
| Keep several cards expanded | Each retains its own identity and controls; presentation changes do not change routing. |
| Click into a card or View all activity | Open that service’s full view; returning restores filters, scroll and expanded cards. |
| Expand a pool and choose Priority | Its member ordering stays inside the pool card/full view, with no global card drag. |
| Personal source works; org and platform also exist | Automatic uses personal. |
| A personal key is merely saved and org has current valid evidence | Check personal safely when supported; otherwise Verification required. Missing evidence alone cannot switch the caller to org. Offer an explicit exact/source choice. |
| Every configured credential is unverified and cannot be safely checked | Verification required for Automatic/Priority; setup provides an exact-call configuration and explicit first-request path. No hidden bootstrap or account substitution. |
| Existing keys lack the new activity history on rollout | Existing address contracts continue; Automatic migration requires a reviewed policy and qualifying evidence. No first-day outage or paid fallback. |
| Previously valid evidence needs a freshness check | Re-check or return a pending gate; age alone never marks failure or switches identity. |
| Personal refresh is terminally rejected; permitted org source works | A new Automatic request uses org and records why personal was skipped. |
| Personal and org unusable; actual permitted platform source works | Automatic uses platform only if allowed; payer/pricing are explicit. |
| No source qualifies | Structured error before provider dispatch; UI shows an actionable unavailable state. |
| No platform offering exists | No platform row, selector option or phantom fallback slot. |
| User explicitly selects platform while personal works | Platform is used; strict failure does not silently switch to personal. |
| Multiple unchosen accounts in one tier | Ask for a preferred account; no arbitrary database order or paid escape. |
| Agent binding or exact approval names a failing target | Honor its constraint; no cross-identity fallback. |
| A custom/pool slug collides with a catalog slug | Preserve current semantics pending explicit migration. |
| User enables Priority and reorders a pool | Only that pool's saved member order changes; broken members are skipped by real eligibility. |
| Preview a pool during concurrent traffic | No counter mutation and no guaranteed-next-member claim. |
| Upstream write timed out after dispatch | Record failure/unknown delivery as appropriate; do not replay under another identity. |
| Stream returned 200 then disconnected | Latest request shows disconnected; last completed does not advance. |
| Same credential is used by two services | Attribute use to the concrete executed service, not both. |
| One member is called directly and through a pool | Preserve requested entry point; deduplicate each execution by request ID. |
| Org member opens service activity | Their own latest request; no other actors or restricted change history leaks. |
| Rename, rotate key, delete and recreate slug | Rename/rotation keep history; recreation has a new immutable identity. |
| Create a variant | Explicit origin edge; no inferred lineage or inherited execution authorization. |
| Only legacy credential last-used data exists | Not recorded; no invented actor or success. |

## 9. Adversarial review decisions

Fable challenged the first draft against the actual resolver, pool and audit code.
The 25 September review settled the address identity, source resolution, activity,
privacy and origin contracts. The user’s 27 September direction supersedes the
review’s list-first/side-inspector presentation: use self-contained service cards,
one collapsed group per service with duplicate/configured connections inside,
individual expansion and a scoped full service/connection view on click.
Route / Activity / Details remain the full view’s structure. This grouping/card
revision was not separately reviewed by Fable; the user’s latest direction supersedes the earlier flat-list grouping
recommendation. It retains the reviewed execution and disclosure constraints.

The review also established that last-caller needs correlated concrete execution
records and that org use rights do not grant org history rights. The final contract
adds per-viewer summaries and protocol completion rules. Priority must check usable
members, not merely active rows. Platform fallback is never promised from catalog
availability alone. The full target includes automatic resolution; a metadata-only
interim release is explicitly insufficient to call the original routing problem
solved.

Fable's closing review found a first-call/migration trap in strict verification.
The final decision keeps working-only Automatic while treating missing evidence
as a pending gate, never as permission to switch accounts. Supported providers
validate during setup; unsupported ones get explicit exact-call setup and a
visible first-request path. Existing address contracts remain until a reviewed
migration. OAuth authorization counts as validity evidence. Freshness rechecks
never silently change identity. This preserves the user's requirement without
adding an invisible unverified exception to Automatic.

The review also fixed platform billing opt-in, canonical entry/no-row behavior,
activity scope, and the org payer example. Working notes and the alternatives
challenged in review remain in `services-consolidated-fable-review.md`.

**Final Fable disposition: review closed; G1 and G2 resolved.** Its three
nonblocking follow-ups are incorporated: inventory and disclose the legacy
platform consent cutoff, copy exact targets while unverified, and require a safe
adapter check before age alone can trigger mandatory revalidation.
