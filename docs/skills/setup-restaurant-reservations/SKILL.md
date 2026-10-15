---
name: setup-restaurant-reservations
description: Guide a signed-in business owner through creating a reservation specialist, connecting Telegram and inbound telephone, and logging customer requests in a selected Notion destination. Use for setup and repair, not booking confirmation or direct customer intake.
---

# Set up a restaurant reservation specialist

## Outcome and execution context

Customers submit requests through Telegram or phone. The specialist collects
details and records pending requests in one selected Notion destination. Staff
confirms bookings separately; a Notion log is not a capacity engine.

Run in the owner's authenticated NyxBot context. The public guide hands off;
it cannot inspect personal connections or create specialists using this skill.
This is an unpublished authoring draft. Actual tools, supported intake adapters
and the public installation flow remain prerequisites; never invent endpoints.

## Participate in a shared setup session

Continue the owned setup and coordinator conversation. Read verified progress,
focus, shared answers and missing requirements before asking questions. Reuse
compatible selected resources without creating another setup or connection.

Guide one actionable step at a time. Use supported interactive cards/forms and
bounded setup tools when available. Conversational edits must use the same
validated updates as card actions. Respect setup/step revisions, owner reviews
and idempotency; report completion only after persisted tool results.

A focus change does not deselect other use cases. Preserve their progress and
show blockers separately. Shared changes require affected checks to be rerun.
If durable setup tools or interactive actions are unavailable, explain that gap
and collect only useful guidance; never invent a saved checklist or ready state.

## Collect and inspect

1. Reuse business name, timezone and staff contact; ask maximum party size and
   confirm that requests await staff confirmation. If automatic confirmation is
   wanted, identify the missing authoritative capacity flow before proceeding.
2. Inspect existing setup records, `nyxid__list_subagents`,
   `nyxid__read_subagent` and `nyxid__list_channel_agents`. Reuse only selected
   compatible resources. Shared FAQ/phone resources do not imply shared grants.
3. Discover Notion and a supported telephone/voice service with
   `nyx__list_connected_services` / `nyx__discover_services`. Inspect available
   tools and schemas with `nyx__search_tools`. Require a bounded intake operation
   with reliable request identity/recovery; raw Notion page creation alone does
   not provide the complete reservation flow.

## Guide connections and resource selection

For missing connections, call `nyx__connect_service` using discovered IDs and
omit `credential`; return its hosted URL for owner consent/secret entry. Resume
via bounded `nyx__wait_for_connection` or the existing completion watch, then
verify live permissions. Do not accept user claims or redirect flags as evidence.

Select the exact Notion database/data source and map request ID/title, customer
name, requested date/time, party size, contact/contact kind, source, pending
status, notes and created time. Validate the actual Notion API version and field
types. Use discovered read tools through `nyx__call_tool` only as authorized;
schema creation or test writes need explicit review. Never treat all workspace
pages as the intended destination.

Select or reuse the customer Telegram bot and business telephone number. For
ordinary personal channels `nyxid__channel_bot_setup_link` provides the supported
human flow and may auto-link. Platform BotFather children must instead use the
bounded installation flow and exact reviewed specialist. Do not use ordinary
`nyxid__connect_channel_bot` / `nyxid__link_channel_bot` to bypass its markers.
Missing supported phone/installation setup means blocked, not ready.

## Create and configure the specialist

Use a platform-reserved agent through its provisioner when present. Otherwise
create an empty private specialist with `nyxid__spawn_subagent`, `services: []`,
`account_read: false`, no machines/logins and no initial task. Record the returned
immutable ID. Inspect after ambiguous creation before retrying to avoid duplicates.

The role collects date/time in business timezone, party size and chosen contact,
reads details back, obtains submission confirmation and acknowledges only a
verified logged request. It never claims a confirmed table. Customer records are
private; caller ID and similar names do not authorize cross-channel retrieval.

Review exact destination, operations/input limits, channels, costs and policies.
Use a supported setup action that applies grants/scopes atomically and binds them
to the approved installation. `nyxid__grant_subagent`,
`nyxid__set_agent_operations` and `nyxid__set_guest_access` provide existing policy
building blocks, but do not themselves implement that combined setup action.
Do not enable whole-service grants or public access while waiting for restrictions.

Require the definition's default reservation operating skill. Use supported
setup orchestration to prepare an owner-private copy from its verified exact
Ornn pin, under the effective permission mode, with source provenance. This copy flow is a
prerequisite, not an existing tool to invent. Pin only the private procedure,
preserving unrelated skills. This onboarding skill stays on NyxBot. Public guest
turns currently omit skills; operational runtime integration is still required.

## Improve after owner feedback

Distinguish a procedure edit from configuration, missing data or a failed adapter.
For an eligible private skill, delegate the owner's bounded improvement request
to the specialist: it reads its own skill and uses `nyxid__draft_agent_skill`
with the exact attached base and advertised scope declaration; disclose omissions.
Preview tools expose metadata, not another agent's source. Auto/Ask every time
require an owner card; Full access publishes and pins immediately with a receipt.
Honor feature/access gates. Never edit a shared default in place or claim prose
fixed a missing operation. Active setups require enforced suspension before any
publication, including the drafting call in Full access, then restricted tests
and resume. A paused setup alone is not that gate. Record the new exact pin.

## Test, activate and report

With authorized test data, submit one request through Telegram and one through
an actual incoming call. Verify Notion rows, source, pending status, requested
time and contact. Replay a provider event and exercise uncertain/failed writes:
no duplicate rows or false success acknowledgements. A call ending after a write
must not cause resubmission. Verify customer and owner isolation.

Notion query-before-create alone cannot guarantee uniqueness. Require durable
request identity, serialized retries and reconciliation; leave uncertain results
visible rather than blindly replaying. Locally saved but unlogged means pending
logging, not saved in Notion. Post-call follow-up needs a reachable authorized
contact and a separately supported send flow; a Notion status edit does not send.

Activate only through the reviewed installation after both channel tests pass.
Report specialist ID, exact destination, bot/number, test evidence, pending owner
actions and blockers. A connected account or prepared agent is not a live service.
