---
name: setup-restaurant-queue
description: Guide a signed-in business owner through creating a queue specialist, connecting an authoritative queue source and SMS sender, and testing staff-triggered return-within-X-minutes notifications. Use for setup and repair, not general SMS sending.
---

# Set up a restaurant queue specialist

## Outcome and execution context

The owner asks NyxBot for queue status and explicitly requests an SMS for a
selected entry. A private queue specialist handles the bounded queue operations.
This setup skill belongs to authenticated owner NyxBot, not the public guide or
customer-facing bot. Staff-triggered alerts are the working v1 behavior; confirm
that choice before provisioning a different automated behavior.

This is an unpublished authoring draft. Use tools only when advertised with
their actual schemas. A general SMS API is not the required bounded queue-alert
operation. Missing queue/notification/provisioning support means blocked setup.

## Participate in a shared setup session

When several use cases are selected, continue the existing owned setup and its
coordinator conversation. Read the verified setup projection, current focus,
shared answers and outstanding requirements before asking questions. Reuse
compatible selected connections and resources; do not create a separate setup
or reconnect a provider merely because this skill is being read.

Guide one actionable step at a time. Use supported interactive cards/forms and
bounded setup tools when available. Conversational edits must use the same
validated updates as card actions. Respect setup/step revisions, owner reviews
and idempotency; report completion only after persisted tool results.

A focus change does not deselect other use cases. Preserve their progress and
show blockers separately. Shared changes require affected checks to be rerun.
If durable setup tools or interactive actions are unavailable, explain that gap
and collect only useful guidance; never invent a saved checklist or ready state.

## Collect and inspect

1. Reuse business name, location, timezone and staff contact. Ask for the default
   return window X and how entries enter the queue: staff entry or an existing
   queue system. Do not assume Notion is the queue backend.
2. Identify the authoritative queue, its stable entry IDs/statuses, and how each
   entry collects a verified SMS number and queue-notification consent. The
   Telegram sender ID or a caller ID alone is insufficient for this binding.
3. Inspect `nyxid__list_subagents`, `nyxid__read_subagent` and existing setup state
   before creating anything. Reuse only a compatible owner-selected specialist.
4. Discover connections with `nyx__list_connected_services` and
   `nyx__discover_services`; inspect operations using `nyx__search_tools`.
   Require current queue reads, record/version checks, a bounded notify-entry
   action, durable duplicate prevention and provider delivery tracking.

## Guide connections

Call `nyx__connect_service` with a discovered ID and no `credential` to obtain
the hosted connection URL for missing queue/SMS services. The owner enters
secrets only there. Use bounded `nyx__wait_for_connection` or end the turn for the
existing completion watch, then recheck actual authorization. Do not ask the
owner to say "done" as proof, retry expired links indefinitely or switch to a
platform credential without explicit reviewed choice.

Select the exact queue/location and authorized SMS sender/messaging service.
Verify recipient-country support, status callback verification, opt-out handling
and budgets through the implemented provider flow. A connected voice number
does not prove SMS support. Never ask the model to choose arbitrary recipients.

## Create and restrict the specialist

Use the platform provisioner's reserved agent when one exists. Otherwise create
an empty private specialist through `nyxid__spawn_subagent`, with `services: []`,
`account_read: false`, no machines/logins and no initial task. Record the immutable
ID and inspect before retrying an ambiguous creation. Do not change an unrelated
agent or expose this staff specialist to public channel traffic.

Review the exact queue source, sender, allowed operations, X/deadline semantics
and budgets. Bind authority through a supported reviewed setup action that
atomically applies service grants and operation/input restrictions. Existing
`nyxid__grant_subagent`, `nyxid__set_agent_operations` and
`nyxid__set_guest_access` are building blocks, not proof of an atomic onboarding
transaction. An absent scope permits whole-service access; do not leave a broad
grant as a substitute when the bounded provisioning action is missing.

The specialist needs queue status and notify-entry access, not an unrestricted
SMS send tool. The backend resolves the entry's recipient, sender and template;
the model supplies entry ID and X only. Dynamic consent, cancelled entries and
record ownership require server checks beyond static operation scopes.
NyxBot may delegate operational work through `nyxid__message_subagent` after setup is ready; no
permission to start operational sends follows merely from creating the agent.

Require the definition's default queue operating skill. Use supported setup
orchestration to prepare an owner-private copy from its verified exact Ornn pin,
under the effective permission mode, with source provenance. This copy flow is a prerequisite, not
an existing tool to invent. Pin only the private procedure, preserving unrelated
skills. Do not attach this setup skill or grant onboarding/account tools to the
worker. An operating procedure cannot override the server's SMS policy/template.

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

Use owner-authorized test recipients and two distinct entries. Test current
queue status, selection of one entry, correct X/local deadline, and no message
to the other entry. Prove duplicate intent handling, opt-out/cancellation,
wrong-owner rejection and delivery failure. Do not send test messages silently.

Provider acceptance means submitted, not delivered. Reconcile unknown sends
before retry; do not mark arrived/no-show from delivery alone. An explicit staff
send can proceed without an extra generic confirmation where existing policy
permits, but existing approvals must still be honored.

Report specialist ID, queue/sender references, contact enrollment method, tests,
activation state and blocked steps. If the bounded notify-entry service is
missing, explain that connection setup alone cannot safely enable the flow.
