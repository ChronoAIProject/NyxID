# Complete IM replies

Related gateway work: [CMA #1029](https://github.com/ChronoAIProject/cma/issues/1029).

CMAEG sends one complete rendered logical reply to NyxID. NyxID plans the
destination's messages before sending, sends them sequentially, and returns
HTTP 200 only after every part has a valid platform acceptance receipt and its
delivery metadata has been recorded. Acceptance does not establish recipient
delivery or reading.

## HTTP contract

The existing request and success response remain compatible:

```http
POST /api/v1/channel-relay/reply
Authorization: Bearer <reply-token-or-channel-key>
Content-Type: application/json

{"message_id":"<original inbound ID>","reply":{"text":"<complete rendered reply>","metadata":null}}
```

```json
{"message_id":"<last NyxID outbound part ID>","platform_message_id":"<last platform part ID>"}
```

The success receipt covers **all** parts generated for this request. It never
means only that the final part was attempted. The returned IDs identify the
last part, preserving existing edit/receipt semantics. Native thread replies
use `/api/v1/channel-relay/thread-reply` and the existing bound-target contract;
all their parts retain the original target, with no parent-chat fallback.

The reply token is consumed once for the logical request. Splitting does not
consume it again or require an additional channel key. Bot ownership and key
revocation are still checked during delivery; native thread replies retain
their live admission checks. A preflight failure after authorization does not
restore a consumed reply token.

## Bounds and splitting

Text-only request bodies accept at most **2,097,152 bytes (2 MiB)** of serialized
JSON, including escaping, metadata and whitespace. The wire extractor enforces
this before consuming a reply token, including when Content-Length is absent.
Attachment requests retain their existing configured media budget. Reply route
limits have a 2 MiB minimum even when a smaller media budget is configured.

The shipped nginx template allows 32 MiB on the two reply endpoints, covering
the 2 MiB text contract and the default base64 media envelope; the backend
enforces the applicable limit. An external ingress must also allow at least
2 MiB. A custom media setting above the nginx envelope needs a matching ingress
configuration.

NyxID preflights a maximum of **64 parts**. The request-byte limit is not a
promise that every body can fit within that part count. Exceeding the count
fails before the first platform send. No excerpt or app link substitutes for
the full reply. A plan containing a whitespace-only part is also rejected
before dispatch, rather than risking a platform refusal after a sent prefix.

| IM platform | Text budget per part |
| --- | --- |
| Lark / Feishu | 30 KiB serialized `{"text":"..."}` content, including JSON escaping |
| Telegram / managed Telegram | 4,096 UTF-16 units |
| Discord | 2,000 UTF-16 units |
| Slack | 40,000 Unicode scalars |
| WhatsApp | 4,096 Unicode scalars |
| X DMs | 10,000 Unicode scalars |

Text that fits remains one message. Overflow prefers paragraphs, newlines and
whitespace, preserving every byte and keeping backslash escape pairs together.
Oversized tokens may require a split between Unicode scalars; arbitrary long
URLs cannot always remain clickable. The splitter does not rewrite or balance
Markdown code fences. It preserves text, not identical rendering across IMs.

The complete-text path applies to ordinary and native thread replies. Rich
cards, interactive controls and edits retain their existing adapter contracts;
ordinary mixed text/media or interactive replies that would require text
splitting are rejected before sending. Public X posts and email retain their
separate transport contracts. Proactive `/send` automation remains outside this
reply-endpoint contract.

NyxBot's normal, delayed, followed-thread and image-failure answer formatting
no longer applies the former 8,000-character excerpt. Failure/image notices
append to the complete answer. Upstream output/API limits still apply.

## Evidence and failures

`channel_reply_deliveries` stores metadata only: a UUID identifying the logical
reply, bot/owner/inbound IDs, expected part count, completion, and each part's
status, accepted platform IDs, outbound row ID and numeric failure code. It
contains no text, media, captions, credentials or provider error prose. Records
expire after 30 days; indexes support inbound and outbound receipt correlation.

Each part starts `not_attempted`. Before provider dispatch it becomes
`uncertain`, so cancellation or process loss cannot leave a false accepted
receipt. A validated response and persisted outbound row make it `accepted`.
An authority failure before dispatch is `failed`; adapter errors after dispatch
are conservatively uncertain unless the existing adapter record provides more
specific evidence. Any error stops the remaining parts and prevents HTTP 200.
A recording failure also prevents success. Known earlier receipts are retained.

Rate-limit errors after dispatch are reported as unconfirmed delivery (HTTP
502), rather than HTTP 429, so callers cannot mistake an accepted prefix for a
safe whole-reply retry. The delivery record retains the original numeric error
code. Admission rate limits before delivery keep their existing behavior.

There is no body queue, timed reassembly, automatic retry or pending-success
response. CMAEG reports failures/timeouts as unknown and must not replay a
possibly accepted prefix. Separate intentional replies and acknowledgements
are separate logical requests even when they reference the same inbound ID.

## Deadline and rollout

CMAEG currently has a 5-second exchange timeout and an 8-second tool deadline.
NyxID waits synchronously for all sends. Three platform calls taking two seconds
each already exceed the exchange timeout, even before authorization and
database work. A 64-part allowance therefore cannot imply a five-second latency
guarantee. Existing provider timeouts remain in force; this change adds no
background completion protocol.

Keep `CMAEG_NYXID_COMPLETE_REPLIES=false` until all NyxID replicas and ingress
limits are updated, reconstruction/token/failure tests pass, and the teams
validate their supported reply sizes against live platform latency or coordinate
a larger exchange/tool deadline. The gateway timeout must cover the complete
operation, and the tool deadline must exceed that timeout. A timeout remains
uncertain even if NyxID subsequently records complete acceptance.

The regression `complete_reply_waits_for_all_acceptance_even_beyond_cma_five_second_deadline`
models this boundary explicitly: NyxID does not return an early success to fit
the gateway deadline. Production platform latency must be checked before
enabling the gateway flag.

## Verification

With a local MongoDB replica set available through `NYXID_TEST_DATABASE_URL`:

```sh
cargo test -p nyxid --bin nyxid-server -- channel_text:: channel_reply_plan:: channel_reply_delivery:: handlers::channel_relay::tests services::channel_thread_service::operation_tests services::channel_thread_service::delivery::tests gateway_complete_thread_reply late_delivery_tests handlers::nyxbot::images::tests --test-threads=2
```

These checks cover reconstruction, exact chat/topic targets, one-token use,
64-part preflight, the serialized request bound, partial/uncertain acceptance,
and honest completion beyond the gateway deadline. IM HTTP endpoints are mocked;
passing these tests does not establish deployed ingress limits or live latency.
