# Typed channel activity

NyxID records received activity separately from callback acceptance. X channels can observe unencrypted DMs, encrypted chat notifications, mentions, replies and their own posts. A received encrypted message can be counted without decrypting it. This does not make its text available to NyxID or the agent.

## Event selection

Open a connected X channel bot and use **Events to receive**. The existing **Save event subscriptions** action reconciles the selected events on the shared callback:

`https://<nyxid-host>/api/v1/webhooks/channel/x/platform`

| Selection | `x_events` value | Provider event | Activity kind | Agent behavior |
| --- | --- | --- | --- | --- |
| Direct messages | `dm` | `dm.received` | `dm` | Existing private message and reply behavior |
| Encrypted chat notifications | `chat` | `chat.received` | `encrypted_chat` | Metadata only; no text, media or reply authority |
| Mentions | `mentions` | `post.mention.create` | `mention` | Existing public message and bound reply behavior |
| Replies to my posts | `replies` | `post.reply.create` | `reply` | Existing public message and bound reply behavior |
| My posts | `posts` | `post.create` | `post` | Metadata only; no automatic replies |

Existing bots continue to default to `dm`. Selection is explicit and performs remote subscription reconciliation; the displayed activity count does not poll X. Private events use the connected account's OAuth credentials. Mentions/replies retain the existing requirement for public reply permission (`tweet.write`). Selecting chat or own-post notifications does not require public reply permission. The provider can still refuse event access; setup failures remain visible on the bot and can be retried with Verify Bot.

`post.create` means posts authored by the connected account, including that account's replies, quote posts and reposts. It does not subscribe to all posts on X. NyxID discards own-post text on this notification path and grants no reply authority, so posting a reply cannot create an automatic response loop through this route.

CLI example:

```sh
nyxid channel-bot update <BOT_ID> --x-events dm,chat,mentions,replies,posts
```

## Receiver declaration and owner consent

Legacy callbacks retain the same body fields, field omission behavior, signing, response handling and reply semantics. NyxID does not add an `activity: null` field. Notification-only activity is recorded but is not sent to a legacy receiver.

An agent that implements the extended contract declares its supported kinds using its assigned agent API key:

```http
PUT /api/v1/channel-relay/conversations/<ROUTE_ID>/activity-capability
Authorization: Bearer <ASSIGNED_AGENT_API_KEY>
Content-Type: application/json

{"version":1,"kinds":["dm","encrypted_chat","mention","reply","post"]}
```

The endpoint returns HTTP 204. Human sessions, other agents, other owners, unsupported versions, unknown kinds and duplicate kinds are rejected. Declare only the kinds the receiver handles. A successful ordinary callback and Verify Bot do not declare support.

The owner then opens the conversation route and selects **Enable typed callbacks**, or calls:

```http
PUT /api/v1/channel-conversations/<ROUTE_ID>/activity-callback
Authorization: Bearer <OWNER_SESSION_TOKEN>
Content-Type: application/json

{"enabled":true}
```

`GET` on the same URL returns `{declared, enabled, version, kinds}`. Enabling requires a first-party human session with write access to that route. The receiver declaration is bound to the assigned key, exact callback URL and key state revision. Reassignment clears it; endpoint/key changes invalidate it. An identical repeated declaration preserves consent, including when the submitted kind order changes. A changed declaration requires fresh owner consent. Disabling retains the declaration for re-enabling and remains available when the key is no longer usable.

Only supported kinds get the extended callback. Readable kinds that the receiver has not opted into retain their legacy callback. Unsupported notification kinds are recorded with `callback_status = "not_enabled"`. Previously observed notifications are never replayed when consent is enabled.

## Signed callback contract

Version 1 adds this object to the existing callback envelope for opted-in kinds:

```json
{
  "activity": {
    "version": 1,
    "kind": "encrypted_chat",
    "provider_event_type": "chat.received",
    "platform_event_id": "e4f4d3fc-8bbf-4928-92eb-e5058d6bb6f6",
    "content_availability": "encrypted",
    "reply_supported": false,
    "occurred_at": "2026-07-23T20:33:03.370+00:00"
  },
  "content": {"type": "unknown"}
}
```

This is a fragment. The callback also contains the existing NyxID `message_id`, correlation ID, platform, agent, route, actual provider conversation, sender and receipt timestamp. `activity.platform_event_id` identifies the provider resource; `message_id` remains the callback idempotency key. Provider occurrence time is optional. `content.type` still describes text/media and is never repurposed as the event kind.

The activity object is inside the exact body signed by the existing RS256 callback JWT and transitional HMAC. The agent verifies those signatures as before, dispatches by `activity.kind`, and acknowledges HTTP 202. HTTP 200 remains accepted under the existing legacy rule; its response body is ignored. NyxID sends one callback for each unique admitted resource, with no additional “processing started” callback, queue or replay.

Encrypted and own-post notifications omit `reply_token`, `X-NyxID-User-Token`, plaintext, attachments, raw provider data, thread/reply references and all crypto material. The X parser drops conversation tokens, encrypted bodies and signatures before normalization. The agent must handle them as notifications, without claiming to have read the content. Reply and attachment endpoints look only in ordinary message storage, so an agent API key cannot turn a notification ID into reply or download authority. X's bound reply implementation also rejects notification records directly.

## Activity history and counts

Owner-authorized metadata endpoints:

- `GET /api/v1/channel-bots/<BOT_ID>/activities`
- `GET /api/v1/channel-conversations/<ROUTE_ID>/activities`

Both accept `kind`, `page` and `per_page` (maximum 100; page maximum 10,000). They return `activities`, the filtered `total`, unfiltered per-route `count`/`last_activity`, `retention_days`, `page` and `per_page`. `kind=message` selects earlier rows without typed metadata. Such rows report unknown content availability, no provider event type and no asserted reply capability.

Counts cover received resources in the last 30 days and exclude outbound messages. A mention and reply describing the same post still produce one row, at most one callback and one charge; the first admitted event supplies its classification. The routes returned alongside a filtered page always describe all received kinds. **Configured routes** is a routing configuration count, not a message count.

The latest activity comes directly from durable admission metadata, sorted by NyxID receipt time, with a stable ID tie-breaker. Provider occurrence time is separate, so a delayed event is visible when it arrives. Duplicate delivery does not update receipt time. The summaries do not depend on callback success and do not update legacy `last_message_at`. Ordinary message-history endpoints and their counts keep their existing behavior.

The UI refreshes these database reads every 15 seconds while the page is visible and on focus. It distinguishes no observations, unavailable data, recorded activity without enabled notifications, pending callbacks, accepted callbacks and failed callbacks. Callback acceptance does not prove that an agent processed or completed anything.

## Storage, admission and billing

Readable activity adds optional `activity` metadata to the existing `channel_messages` row. Notification-only activity uses `channel_activity_notifications`, isolated from old message readers and reply paths. Both contain metadata only. Activity queries project only the fields they need, filter each collection by owner/resource/time before unioning, and have a five-second database timeout. Notification metadata has a 30-day TTL and participates in account/org deletion cleanup.

X uses the existing atomic admission transaction and owner/bot-scoped coordination claims. Encrypted message UUIDs use `x-chat-inbound`; existing numeric message identities retain `x-inbound`. The callback cannot run before metadata and deduplication are committed. The existing signature, bot tag, event selection, account target and live OAuth checks remain mandatory. Unrouted or invalid ingress does not invent a route or increase its count.

Every accepted X activity uses the connected service's configured request price and the existing funding/suspension machinery. Encrypted notifications use the distinct billing identity `x-chat-received:<bot>:<event>`. Public activity uses the existing `x-post-received` identity; DMs retain `x-dm-received`. Redelivery does not reserve fresh funds or charge again. Observation with agent notifications disabled is still provider activity and is billed under the same policy.

Adapters opt into this UI and callback mechanism through `activity_descriptors()` and `activity_metadata()`. Platforms with no activity descriptors retain their current presentation and legacy callbacks.

## Deployment and provider verification

Upgrade every backend replica before selecting the new `chat` or `posts` values: older X bot readers do not understand those enum values. Keep new selections disabled during a rolling upgrade. Before rollback, change selections back to supported values and disable typed callbacks. Isolated notification storage protects old message counts and reply lookups throughout that sequence.

After deployment, select `chat`, confirm subscription reconciliation succeeds, and send a new encrypted message to the connected account. Verify a new activity ID in the owner API/UI, then enable a receiver that has declared support and verify acceptance of a subsequent signed callback. Existing notifications are not replayed. Test a normal DM and a public mention/reply as well.

Local signed fixtures and mock receivers validate NyxID's implementation. A successful public webhook HTTP 200 only acknowledges the request; it does not establish that X sent the intended event or that a subscribed agent received it. The original incident's screenshots identify encrypted X Chat alongside `dm.received` subscriptions, but production ingress still requires the post-deployment check above.

Provider references: [X Activity API](https://docs.x.com/x-api/activity/introduction), [event payloads](https://docs.x.com/x-api/activity/event-payloads). Related: [Channel Bot Relay](CHANNEL_BOT_RELAY.md), [HTTP Event Gateway / ADR-013](CHANNEL_EVENT_GATEWAY.md).
