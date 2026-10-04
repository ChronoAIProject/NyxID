# Channel thread follow: gateway contract change

Status: proposed handoff to the CMA Agent Event Gateway owning team. NyxID T1
ships direct relay first. No gateway implementation or deployment is included.
Existing gateway transports retain legacy behavior and report
`follow_readiness: "unavailable"` until this contract is negotiated and tested.

## Negotiation

Expose an additive channel capability response, tied to the channel's current
configuration version, with `thread_contract_versions: [1]` and per-surface
`thread_facts`, `bound_thread_replies`, `unmentioned_thread_events` booleans.
NyxID requests `thread_contract_version: 1` when updating an opted-in channel;
the gateway echoes the accepted version and capabilities in its response. A
missing version/boolean means unsupported, never implicit acceptance. Advertise
only verified implementations for that channel's platform and credentials.

All three booleans are required before NyxID marks follow ready. Optional
`thread_history` support is independent; metadata fallback is valid. Do not
change OAuth scopes, grant permissions or move a working bot implicitly.
Configuration updates retain existing expected-version conflict handling.
NyxID requests existing group policy `all` before enabling follow and filters
events itself; successful policy update alone does not prove platform visibility.

## Event context

Retain `event_context.conversation_id` (the existing `conv_*` registry alias),
`activity.event_id`, `activity.actor`, `activity.conversation` and `event_ref`.
Add `event_context.thread` for a verified message on a supported thread surface:

```json
{
  "version": 1,
  "kind": "native",
  "chat_id": "C123",
  "parent_chat_id": null,
  "message_id": "1700000001.000001",
  "root_id": "1700000000.000001",
  "native_thread_id": "1700000000.000001",
  "parent_message_id": "1700000000.000001",
  "sender_kind": "human",
  "address": "mention"
}
```

`kind` is `native`, `topic`, `reply_chain` or `email`. `chat_id` is the actual
incoming platform conversation; `parent_chat_id` is its containing channel when
the native thread is itself a channel (Discord). `root_id` is stable across
senders/transports. Null means unresolved, never “most recent thread”. A
reply-chain immediate parent must not be claimed as the root without evidence.
`native_thread_id` is a non-secret alias; Discord interaction tokens are excluded.

`sender_kind` is `human`, `bot` or `unknown`; `address` is `mention`,
`reply_to_bot`, `not_addressed` or `unknown`. A mention must identify this bot,
not any bot or `@all`. Do not infer it merely from gateway admission policy.
Native private email direct-address evidence will be negotiated with PR E;
unknown new kinds/evidence must fail closed in existing readers.

Facts must match the authenticated bot/channel and exact source message, chat
and actor. Preserve the originating NyxID inbound message UUID as `event_id`
through retries; a new idempotency key must not create a new source identity.
If facts cannot be resolved, pass unknown/missing fields and retain legacy
behavior. Never substitute sender partitions, interaction secrets or a neighboring
chat. No fetched history, raw tokens or new body retention in this envelope.

## Bound replies and compatibility

The sealed `event_ref` must bind bot, source message, chat, canonical root,
native destination/anchor, reply authority and expiry. Extend provider response
creation with an optional `reply_target: {"version": 1, "event_ref": "..."}`;
the gateway validates it against the current admitted event before honoring
streamed output. `/events/{event_ref}/replies` must enforce the same binding.
The owning team may adapt endpoint placement, but both paths must share these
semantics and publish the agreed response schema before NyxID enables PR D.

Reply into Slack's root `thread_ts`, Telegram's exact topic/reply anchor,
Discord's validated thread channel or message reference, and Lark/Feishu's native
in-thread reply. All split text/media components and safe notices keep that
target. A later event cannot retarget an earlier response. Email, when supported,
remains tied to its original recipient and send barrier, never the newest sender.

Expired, inaccessible, revoked or conflicting targets produce stable safe codes
(`thread_target_expired`, `thread_target_unavailable`, `thread_target_mismatch`).
No fallback to the parent channel, direct message, another thread or proactive
send. Do not expose native credentials or provider error prose in replies/logs.
Preserve existing single-send/dedup semantics on ambiguous send outcomes.

Keep `conversation_and_sender`, binding auth, `conv_*` PUT/DELETE and
`readEventContext` unchanged. Each event independently chooses its canonical
thread; multiple aliases can share a root and one alias can visit several roots.
Registry deletion does not delete NyxID's shared thread history. Legacy clients
and channels without negotiated v1 continue their existing behavior.

## Acceptance fixtures

Share fixtures for a root mention, later unmentioned human reply, two senders
in one thread, one sender in parallel threads, reply to bot output, bot echo,
unresolved ancestry, expiring targets and policy-update races. Assert identical
canonical facts and outgoing targets across direct relay and gateway; verify
immediate/deferred/error/media replies never escape the root. Include retries
with changed idempotency keys and channel migration without duplicate work.

No message bodies, tokens, sealed refs or provider error text in audit/live
events. Record the deployed gateway version and verified platform capabilities
with the fixtures. X public threads are explicitly excluded from T1.
