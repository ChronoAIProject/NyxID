---
name: setup-restaurant-faq
description: Guide a signed-in business owner through creating or reusing a FAQ specialist, publishing business answers, and connecting customer Telegram and inbound telephone channels. Use for setup and repair, not customer FAQ execution.
---

# Set up a restaurant FAQ specialist

## Outcome and execution context

Prepare one owner-controlled FAQ specialist that answers from reviewed business
facts in Telegram and on inbound telephone calls. This skill runs in the owner's
authenticated NyxBot context. The external public guide hands off to that context;
it receives no personal inventory or management authority from this skill.

This is an unpublished authoring draft. Existing connection/team tools can be
used when advertised. Public installation provisioning, publication, telephone
routing and activation must use implemented supported flows; do not invent tools
or simulate success. Do not attach this setup skill to the customer specialist.

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

1. Ask which business to configure and whether this is a new setup or a repair.
   Reuse existing business name, timezone, staff contact and setup references.
2. Collect owner-approved FAQ facts: hours, location, menu/contact links and
   policies. Separate private setup notes from the content to publish. Unknown
   answers must say so and offer staff contact; do not promise live transfer.
3. Inspect existing specialists with `nyxid__list_subagents` and
   `nyxid__read_subagent`. Reuse only the owner-selected compatible specialist.
   Inspect channels with `nyxid__list_channel_agents`. Do not move a live channel
   or overwrite another specialist's purpose merely because its name matches.
4. Use `nyx__list_connected_services` and `nyx__discover_services` to find a
   supported telephone/voice connection. Discover actual operations and schemas
   with `nyx__search_tools`. Catalog presence or browser voice is not proof of
   inbound telephone support. FAQ knowledge itself needs no private data service.

## Guide connections

For a missing provider connection, call `nyx__connect_service` using the discovered
service ID and omit `credential`. Share the exact returned hosted URL. Credentials
belong in that page, never chat. On `pending_connection`, either use bounded
`nyx__wait_for_connection` or explain the next step and end the turn for the
existing connection-completion watch. Recheck live availability after resuming;
a user saying "done" or a callback success string is not evidence of access.

Select an authorized business number or supported forwarding route. Reuse the
number chosen for reservations if compatible. Do not claim a provider account
connection creates a number, configures voice, or verifies call routing.

For an ordinary personal channel, `nyxid__channel_bot_setup_link` provides the
supported human setup page and may auto-link a channel. For a platform BotFather
child, use only its bounded reviewed installation flow; never use ordinary
connect/link tools to bypass its markers or auto-link before approval. If no
compatible setup action is exposed, explain the missing step and remain blocked.

## Create and configure the specialist

After the owner chooses the role, create or reuse a private FAQ specialist.
Where the platform setup already reserves an agent ID, use its provisioner; do
not spawn a second agent. Otherwise `nyxid__spawn_subagent` can create an empty
specialist with `services: []`, `account_read: false`, no machines/logins and no
initial task. Record its returned immutable ID; after an uncertain response,
inspect existing resources before any retry.

Set its role to answering only the reviewed FAQ publication. No Notion, queue,
SMS or private account grants are needed. Publish facts and associate the exact
specialist through the approved installation flow. If that flow is not callable,
report "specialist prepared; publication/activation blocked" accurately.

Require the definition's default FAQ operating skill. Use supported setup
orchestration to prepare an owner-private copy from its verified exact Ornn pin,
under the effective permission mode, with source provenance. This copy flow is a prerequisite, not
an existing tool to invent. Pin only the private procedure on the specialist,
preserving unrelated skills. Keep this setup skill on NyxBot. Existing guest
turns omit skills, so attachment alone cannot make a public FAQ runtime work.

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

Obtain owner authorization for real test calls/messages. Test an independent
Telegram customer and an inbound call: correct facts, unknown-question fallback,
same publication revision and no private account information. Preserve the
existing independent-customer delivery test and explicit activation review.
Do not infer call success from a web voice session or API acknowledgement.

Finish with the specialist ID, connected resources, publication revision,
per-channel test evidence, blocked steps and next human action. Mark active only
from authoritative activation state. Telegram-only readiness is partial when
the requested outcome includes telephone. Declined/expired connection or review
does not trigger automatic new requests; explain how the owner can resume.
