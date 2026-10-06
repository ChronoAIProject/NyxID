# NyxID: service connections and pool ordering

The current product recommendation is [Consolidated Services](consolidated-services-flow.md)
(revised 27 September 2026). This document records the earlier preview design; its view
names and readiness wording are superseded by that recommendation.

**Revised 17 September 2026.** Proposal; the local frontend previews production
metadata. Health-based selection and Priority pool routing still require backend
implementation. This revision replaces the earlier global card-sorting and
placeholder-platform design.

Local preview: <http://127.0.0.1:4317/keys?view=routing>

## 1. Keep the service cards

Open **External Services** and see the individual cards, including service name,
endpoint, proxy slug, credential state, owner and node routing. Expand **Details**
for description, permissions, dates and the existing full detail page.

A compact **Individual cards / By service** switch changes presentation. By
service groups cards with the same catalog identity. It does not turn those
connections into a pool. Neither view has drag handles.

Each service gets one short routing status once the server can supply it:

| Status | Meaning |
| --- | --- |
| Ready via You | A verified personal connection is selected. |
| Ready via Acme | A verified, permitted Acme connection is selected. |
| Ready via NyxID | A real, verified platform connection is selected. |
| Not verified | The required checks have not completed. |
| Unavailable | No permitted working connection remains. |

Credential status and routing readiness are separate. An active saved key does
not establish that the provider accepts it. A usage timestamp is not a successful
health check. Ready is a current, operation-scoped result with a verification
time; it cannot guarantee every future request will succeed.

## 2. Show only actual connections

Open **Connections** from a card. Show the actual connections, owner, and one
short status per row. Known failures stay visible here so the user can repair
them, but never enter the usable routing order. Unverified connections wait for
a check and do not count as working.

If there is no platform execution service, show no platform row, option, fallback
slot, or platform label in the order. The catalog and shared OAuth application
credentials are not evidence that a platform execution service exists.

For example, with a working personal connection, a working organization
connection, and no platform service:

```text
OpenAI connections
Connection choice    [Automatic ▾]

1  Personal account       You         Selected
2  Team account           Acme        Available
```

If the personal connection fails verification, Acme becomes first. The personal
card remains accessible with **Reconnect needed**. If nothing can be verified,
show an empty usable order with **Not verified** or **Unavailable**, according
to whether checks are pending or conclusively failed.

## 3. Automatic selection and explicit choice

For an eligible canonical service slug, Automatic selects the first working,
authorized connection in this order:

**User → organization → platform.** Absent sources are omitted. If none works,
return an actionable error and do not call the provider.

**Connection choice** also offers actual verified connections as explicit
choices. Selecting a real platform connection is permitted even when a personal
connection works. This is an exact choice: if that connection is unavailable,
return its error. Do not silently switch back to the user's account. Missing
platform services never produce an option; existing but unverified/unavailable
connections show a disabled option and their reason.

The proposed per-request override remains `X-NyxID-Connection-Source: platform`
(or `auto`) on the same canonical slug URL. Request choice takes precedence over
a saved choice, which takes precedence over the Automatic default. The header
and saved preferences are proposed additions, not currently deployed behavior.
Exact service IDs, custom slugs, pool slugs, agent credential bindings, narrower
scopes and approvals retain their contracts; they cannot be silently overridden.
Multiple equally eligible accounts require an explicit choice or an established
server policy rather than an arbitrary database order.

## 4. Drag only members of a real pool

Open **Service Pools**. Each pool shows its real name, proxy slug, current
selection strategy, description and member cards. Keep the same service
information and detail links found in External Services.

The user decides whether they want ordered selection:

| Selection | Behavior |
| --- | --- |
| Round robin | The current round-robin strategy. |
| Weighted | The current weighted strategy, retaining member weights. |
| Priority order | Proposed: use the first working member in the preferred order. |

Choosing **Priority order** reveals drag handles on the members of that pool.
Dragging changes only that pool's preferred member order. It cannot reorder the
whole page, add another service to a pool, move a member between pools, or change
the user/organization/platform source hierarchy. Keyboard arrows and move buttons
provide equivalent controls.

A preferred member order is not proof of health. Before dispatch, the server
checks members in that order, skips known unusable ones and resolves pending
checks. Disabled or excluded members cannot execute. If none works, the pool
returns an actionable error. The member cards retain their details and individual
states throughout.

## 5. Server checks define “working”

The UI and API must use the same server decision, bound to the actual caller,
requested operation and selected connection. At minimum it must verify:

- Service/member enabled state, caller access and scope, organization permissions,
  exact credential bindings and operation compatibility.
- Credential existence and validity, supported OAuth refresh, route/node
  availability and provider authentication evidence from a safe supported check.
- Applicable approval and funding requirements without bypassing either through
  fallback. The selected route supplies its actual payer and pricing.

Pending or failed checks must not be represented as Ready. Providers without a
safe validation method need an explicit unverified state; never send arbitrary
production calls merely to manufacture a green badge. Verification freshness
must be bounded. Execution revalidates before dispatch and records the selected
service, source and reasons for skipped candidates in request history.

Fallback is a decision made before the provider effect. Do not replay writes or
streams with another credential after a request has been sent. This avoids
changing identity, billing ownership or approval authority mid-request.

## What is available in the local preview

The local preview reads actual connection and personal-pool metadata, preserves
cards/details, omits absent platform choices, and lets users arrange real pool
members after opting into Priority. View and pool preferences are saved only in
this browser, scoped by account and pool. Production writes and execution are
blocked by the local gateway.

There is currently no live per-connection readiness endpoint. The preview labels
saved connections **Not verified** and disables explicit connection choices;
it does not invent successful checks. Pool Priority is also a local proposal:
the production resolver currently selects enabled, active members using
round-robin or weighted selection, without a provider-health gate at that step.
