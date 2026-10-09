# Channel thread follow: gateway contract 1

Implemented for review in NyxID `feat/gateway-thread-follow` and
[CMA PR #980](https://github.com/ChronoAIProject/cma/pull/980)
(`feat/nyxid-gateway-thread-follow`). CMA's `docs/CMAEG_Protocol.md` is the normative
wire authority. Deployment requires the CMA owner's merge and rollout; this
change does not deploy the gateway. The original handoff proposal is superseded
by the concrete additive contract below.

## Negotiation and admission

A channel management response advertises `thread_contract_versions: [1]` only
for a verified, single Lark/Feishu relay source with a pinned bot identity.
NyxID discovers this from the existing bounded sweep (at most every ten minutes),
then updates the channel with expected-version concurrency control:

```json
{"reply":{"thread_contract":{"version":1,"follow_chat_ids":["oc_chat"]}}}
```

NyxID requires the accepted version in `definition.reply.thread_contract` before
persisting readiness. Version 1 guarantees verified thread facts, selected-chat
admission and explicitly bound replies. No advertisement, refusal or missing
acknowledgement leaves legacy behavior in control. Other platforms do not
advertise this contract. `nyxbot:thread-follow` remains the existing rollout flag.

For known follow-enabled chats or active followed children, NyxID projects up to
256 chat IDs into the policy. CMA admits ordinary messages in those chats while
preserving sender/chat ACLs and explicit group deny. Initial mentions arrive
through ordinary admission; NyxID records the parent and widens that chat after
activation. The sweep and chat updates reconcile the policy. If the bounded
parent snapshot overflows, NyxID processes only positively selected chats; it
never guesses that omitted explicitly disabled chats permit follow.

This projection is a cheap event gate. Private chats, unsupported gateways,
missing metadata, dormant follow and explicitly non-follow chats add no follow
DB reads. Live follow eligibility, sender access, org authority, stop/expiry and
pre-model checks still use the existing relay services. Newly created unknown
chats use the enabled default; explicit per-chat opt-outs are projected.

## Verified event metadata

The additive field is `event_context.activity.thread`:

```json
{
  "version":1,"kind":"native","chat_id":"oc_chat",
  "message_id":"om_message","root_id":"om_root",
  "native_thread_id":"omt_thread","parent_message_id":"om_parent",
  "sender_kind":"human","address":"not_addressed","mentions_others":true
}
```

CMA derives it from signed raw Lark/Feishu events, not normalized caller hints.
Root, alias and parent are optional; an immediate parent is never invented as a
root. The sealed Activity digest covers these facts. NyxID also correlates the
source event UUID, bot, owner, route, route key, sender, platform chat and message
to original persisted ingress before retaining normalized facts. No mentioned
user identities, bodies or opaque refs enter thread metadata.

A real bot mention addresses the bot. Other-user-only mentions stay quiet in an
active followed child but remain context metadata. `@_all` only suppresses
`mentions_others`; it does not activate a follow. NyxID correlates reply parents
to its authenticated outbound receipts for admitted events; selected follow
chats admit those replies even when CMA cannot prove the parent's author.
Unselected chats retain legacy gateway admission. A selected chat with missing
metadata cannot infer addressing from its widened admission.
Events carrying the new metadata also cannot infer addressing while a lost
management acknowledgement leaves NyxID's local readiness unset.

## Bound immediate and delayed replies

NyxID emits `response.created.response.thread_reply: true` only when the shared
follow handler selected this path. CMA uses it for that stream's automatic
answer, acknowledgements and notices. Event replies and durable reply-target
messages use `OutboundMessage.thread_reply: true`. Omission/false preserves
legacy routing even when the channel supports the contract.

The sealed event and durable reply target bind native-reply availability and the
original NyxID inbound UUID. CMA sends `thread_reply: true` to `/api/v1/channel-relay/thread-reply`.
The distinct endpoint makes old replicas reject before any platform send; it
never falls back to `/reply`. NyxID accepts no caller-supplied root/chat ID: it reconstructs the
original target from persisted facts and the live admitted child binding, then
uses the existing Lark/Feishu native reply endpoint with `reply_in_thread=true`.
All split components and follow notices retain that target. Missing/revoked
bindings fail closed with `thread_target_unavailable`; there is no parent-chat
fallback. Delayed sends retain their original encrypted reference, expiry,
authorization and dispatch barrier, never a newer event reference.

Deploy NyxID to **all** replicas first, then upgrade **all** gateway replicas
before authoring opted-in definitions (old strict gateway readers cannot
deserialize the new policy). Keep Lark/Feishu gateway platform flags off during
this first mixed-version deployment; enable gateway routing/negotiation only
after both rollouts complete. Ordinary channels and clients retain their
serialized behavior. Opaque refs remain encrypted at rest and never logged.

## Deferred surfaces

Lark/Feishu remain text-only on CMAEG. No media, edit or gateway history feature
is added. NyxID reuses existing bounded adapter history with metadata fallback,
without new scopes. Slack, Discord, Telegram topics/chains, WhatsApp and Aurinko
need the follow-ups listed in [the parity table](CHANNEL_EVENT_GATEWAY.md#agent-gateway-group-thread-parity).
X public thread follow remains out of scope.
