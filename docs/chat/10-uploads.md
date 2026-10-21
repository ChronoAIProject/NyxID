# Assistant uploads

Normative contract for human uploads to NyxAgent conversations, owner-posted
group messages and verified owner Lark/Feishu media. This extends [08](08-nyxagent-engine.md) and
[09](09-nyxbot-orchestrator.md). Tool images are owner-readable and may also be fetched by the bound thread key
during their producing live turn, including that same guest turn; see
[Tool images](08-nyxagent-engine.md#tool-images). Their default retention is the
conversation lifetime. User-upload selection and access rules are unchanged.

## Admission and storage

Only a first-party human owner may use the browser routes to upload or remove a pending upload. Guest,
OAuth-client, delegated and API-key callers cannot upload. Conversation uploads
belong to that conversation; group uploads belong to the owner's group thread.
Draft chats acquire a durable conversation before their first upload.

Accept PNG, JPEG, GIF, WebP, PDF, DOCX, UTF-8 plain text, Markdown, CSV and JSON.
Validate raster structure/signatures and document structure; MIME declarations
and extensions never establish trust. Reject binary text, malformed/encrypted
documents and unsupported formats with fixed, actionable errors. Normalize safe
display filenames; never log names, bytes or extracted text. Limits are 20 MiB
per file, ten attachments per message, and 30 upload attempts per owner per minute.
Limit concurrent extraction independently of upload admission.

Store `origin: user_upload` (legacy absent origin means `tool`) and encrypted
payloads in `assistant_attachments`. Files exceeding MongoDB's document limit
use bounded encrypted chunk records in the same collection. Extracted text and
page/section offsets are separately envelope-encrypted. Metadata contains only
scope, verified type, safe display name, size, offsets/counts and lifecycle IDs.
An accepted message atomically claims its pending uploads; an ID cannot be
reused by another message, conversation, owner or group. Apply the runtime
retention policy below to existing and new files. Conversation/group deletion
and owner purge remove all payload chunks, extracted text and expiry markers.
Removing an unsent attachment deletes it.

## Inbound channel media

The per-person `assistant:channel-media` flag defaults off. When enabled, verified
owners can send images, files and rich `post` messages to a linked Lark/Feishu
NyxBot or specialist through NyxID's relay or the Agent Event Gateway, including
admitted followed threads. Telegram owner files use the same flag and shared
byte budget; their preparation before turn admission and per-file fallback are
described in [Telegram forwarding](../TELEGRAM_NEW.md#telegram-forwarding-and-secure-pairing).
Other adapters and Lark audio/video retain their existing behavior. Guests never download media or
create uploads; their admitted turn receives the fixed note
`attachments from guests are not shared with the assistant`. Media-bearing messages cannot answer
approval cards or text decisions and do not enter duplicate-question coalescing.

Admission records source selectors, not bytes. Gateway source IDs are accepted
only from `nyxid_relay` Lark/Feishu activities and must match NyxID's own inbound
row, owner, bot, route, chat, sender and thread/follow binding. Provider handles
from the gateway are never dereferenced. A missing source produces a fixed
unavailable-media note; a foreign source is refused. Each fetch rechecks live
owner, org-admin, specialist and follow authority before resolving the bot
credential. Revocation means no download.

Ingestion is the first step of the consuming turn's run, after the conversation
and user message exist. A busy conversation queues the media event IDs and sent
time; no upload exists while queued. The run sniffs and validates each file,
then stores it encrypted through the ordinary user-upload service with a unique
source selector and a content digest. It binds uploads in one transaction that
writes the active-turn conversation fence, rejects Stop, and records first use
for retention. The immutable `media_bound` receipt holds the bound IDs and
per-item outcomes. A retry reads that receipt first, so it cannot bind twice,
reset retention dates or change a committed result. Bound images use the same
upstream capability negotiation and explicit fallback as browser images.

Stop, failure, timeout and orphan recovery call `media_finalize` outside the
cancellable run. It clears `media_pending`, records terminal per-item outcomes
and deletes known unbound uploads and chunks for that turn. Bound uploads retain
the ordinary retention policy, including `images_delete_after_turn` on stopped
or failed turns. An uncertain upload commit that lands after finalization stays
unbound, can never be adopted by a later turn, and is purged by pending retention.

| Limit | Channel behavior |
| --- | --- |
| Items | At most 10 ingested per event; extra items produce an omitted-count note |
| Payload | At most `min(CHANNEL_MEDIA_MAX_BYTES, 20 MiB)` per item; ordinary upload type and rate limits apply |
| Collection | At most 90 seconds to collect media; unfinished items receive timeout outcomes and a fixed note, successful items are still bound, and the turn continues. Stop cancels the turn |
| Event retention / native dedup | 24 hours for text events; 7 days for media events. Native replays after that horizon are outside the dedup guarantee |
| Queue lifetime | After 6 days, the maintenance sweep converts queued media to a durable owner transcript notice before removing it |

Media admission and native dedup share one atomic event insert keyed by bot,
platform, native chat and native message. A duplicate with a different relay
UUID records an alias to the winning event and starts no fetch, turn or reply.
Health reuses the winner's result. An accepted media event still without a turn
or queue binding after five minutes is marked `interrupted`; the next owner
turn in that chat receives its fixed interruption note once.

Failures, omitted items and post text-cap notices appear in the current agent
request and persisted message, including when every item fails. Expired queued
media produces `a photo sent at HH:MM UTC expired before it could be processed;
please resend` in the owner's app transcript. The original text is quoted
history, never re-executed as a new request. Revoked queued authority is also
removed with a durable notice. Guest and trigger turns never consume queued
owner media.

### Shared byte budget

One process-wide weighted semaphore provides 192 MiB across owners, browser
uploads, channel downloads and managed attachment egress. Each item reserves
`2 × payload_limit` before reading or decrypting and holds that reservation
through extraction/encryption or its outbound response. Readers enforce the
payload limit, not the doubled reservation. Reservations are per item, never
for a whole batch, and cancellation releases them.

Browser requests reserve the full 20 MiB payload capacity before their single
body read; four concurrent files reserve 160 MiB. Channel GETs begin at the
smaller of a positive declared size, 4 MiB and the configured payload maximum;
unknown, zero or invalid sizes begin at 4 MiB, capped by that maximum. If the
body exceeds that limit, the reader drops its partial buffer, releases the
reservation, rechecks live authority and retries once with the full channel
maximum (up to 20 MiB). It acquires a fresh reservation before retrying. Egress
reserves twice the exact stored size. Failed attempts still count against upload
admission; no partially held reservation waits for a larger one.

### Channel rollout and rollback

Upgrade **all NyxID replicas** and drain pre-release workers before enabling
`assistant:channel-media`. Gateway delivery also requires CMA at least
`a570be21` and `CMAEG_MEDIA_ENABLED=true`. CMA's default provider header budget is
30 seconds (`CMAEG_PROVIDER_REQUEST_TIMEOUT_MS`); admission performs no downloads,
and ingestion begins only in the run after admission. For inbound downloads,
the channel bot app needs any one of `im:message`, `im:message:readonly`, or
`im:message.history:readonly`, configured in the Lark/Feishu developer console.
The official [Obtain resource files in messages](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message-resource/get)
API (`/reference/im-v1/message-resource/get`) lists these alternatives. These
bot-app permissions are separate from the service connection's permissions
for writing files to Base.

For rollback, disable both `assistant:channel-media` and
`assistant:attachment-writes`. The flags stop new admission/calls; normal turn
processing and the media maintenance sweep continue draining accepted work.
Keep the new binaries and sweep running. A first-party human platform admin may
convert queued media early with:

```http
POST /api/v1/admin/assistant/channel-media/drain
```

The response is `{"converted": N}`. Repeat until `converted` is zero, then check
the completion gates below on the application database. Each call is bounded
and writes the durable expiry notice before removing a queued event; it neither
downloads files nor executes the queued request.

```javascript
db.assistant_conversations.countDocuments({
  pending_events: {$elemMatch: {"media_event_ids.0": {$exists: true}}}
}) // must be 0
db.assistant_messages.countDocuments({media_pending: {$ne: null}}) // must be 0
db.lark_base_writes.countDocuments({
  state: {$in: ["uploading", "dispatching"]}
}) // must be 0
```

Without the admin conversion, queue conversion becomes eligible after six days.
Orphan recovery uses the existing 75-second stale-heartbeat threshold; stale
managed writes become eligible after their 180-second deadline. Bounded sweeps
must finish; no fixed waiting period proves completion.

After all three gates are zero, a human platform admin must delete the two
attachment operation rows, `bitable_media_upload` and
`bitable_record_create_with_attachments`, for **each** catalog slug `api-lark`,
`api-lark-bot`, `api-feishu`, and `api-feishu-bot`. List each catalog's rows with:

```bash
nyxid catalog endpoint list <catalog-slug> --output json
```

Match the two operation names and retain their endpoint UUIDs. The listing's
`service_id` is the catalog service UUID, not a UserService/connection UUID.
Delete each matching row through the authenticated admin API:

```http
DELETE /api/v1/services/{service_id}/endpoints/{endpoint_id}
```

This physically deletes the operation row. The catalog CLI's `endpoint disable`
command only pauses publication and is not this deletion step. Repeat the list
for all four slugs and verify both names are absent. Keep the drained new
replicas running until deletion is complete, then cut over to the old release
without restarting a new replica or running new-release spec synchronization:
startup additively upserts these compiled overlay rows and would recreate them.

Before the old binary starts, also remove every scope using `JsonShape::Number`,
including compiled key copies. See
[operation-scope compatibility](../AGENT_OPERATION_SCOPES.md#compatibility)
and the managed Base-write rollout in [API discovery](../API_DISCOVERY.md).

## Retention

Opt-in machine command excerpts use the same encrypted attachment lifecycle
with `origin: machine_preview`. They expire after the shorter of 30 days and
the current sent-document retention. The tool-image “keep with conversation”
option never extends command excerpts. Group cards expose metadata only.

Platform admins configure **Admin → Upload retention** (`/admin/upload-retention`).
This is a MongoDB override in `platform_settings`, not an environment variable.
`GET /api/v1/admin/settings/upload-retention` returns defaults, effective policy,
revision and the responding replica's last refresh. `PUT` replaces all five
controls; `DELETE` restores defaults. All three require the existing platform
admin guard (operators cannot use them). Every change commits with an audit-chain
entry containing the actor, old/new values, reset flag and revision; never file
names or content. An audit failure aborts the change.

| Control | Default | Allowed values / clock |
| --- | --- | --- |
| `pending_hours` | 24 hours | Integer 1–8760 hours (365 days), from upload creation |
| `image_days` | 30 days | Integer 1–365 days, from first message binding |
| `images_delete_after_turn` | false | Also expire sent user images once their first referencing turn settles |
| `document_days` | 30 days | Integer 1–365 days, from first message binding; original bytes and extracted text expire together |
| `tool_image_days` | null | null keeps tool images with the conversation; otherwise integer 1–365 days from creation |

Replicas refresh the policy snapshot every five seconds without restarting.
Attachment downloads, agent image fetches, document paging, machine transfers and
transcript metadata check the **current MongoDB policy**, rather than that cache.
A shortened policy therefore denies already-expired reads immediately, even before
refresh or physical deletion. Failed policy reads fail closed. Increasing a limit
can preserve files still present, but cannot restore deleted bytes.

The first referencing turn includes owner and specialist group turns and image
capability fallback. Automatic continuations remain the same turn. Successful,
failed and stopped turns all settle. Settlement records the first-use marker and,
when enabled, deletes those user images in the same transaction. Enabling the
option later also expires images whose first turn already settled; abandoned
turns are considered settled once their live lease ends. Documents and tool
images do not follow this option.

Cleanup runs every minute under a renewable cluster lease. Each pass examines at
most 100 roots and 100 chunk records, saving cursors for the next pass. Deletion
transactions fence both the current lease and policy revision, remove encrypted
chunks and extracted text with the root, and safely collect legacy orphan chunks.
Reads enforce expiry independently of sweep latency. Expired bound files retain
only a scoped metadata marker so the transcript displays **“Attachment expired
per retention policy. Upload it again to continue.”** Downloads return HTTP 410 /
`AssistantAttachmentExpired` (12101); `nyx__attachment_read` returns an MCP
`isError` result with that code and guidance. Pending expiry leaves no transcript marker. Markers disappear with the conversation,
group or owner; no filename or payload is retained in them.

**NyxID's copy only:** an image the agent already saw may remain in NyxAgent's
session until it expires, and model providers process it. This policy does not
remove upstream copies or files the owner/agent already downloaded elsewhere.

## Documents and untrusted content

### Context recovery

Assistant upload metadata is scoped to its conversation and can be listed with
`nyx__attachment_list`. Document reads are paged; a page with `next_offset`
sets `complete: false` and includes a fixed continuation hint. Uploads remain
untrusted data and every read is re-authorized against the active assistant
turn.

Use maintained, pure-Rust `lopdf` for PDF parsing/text and `zip` plus `quick-xml`
for DOCX. They avoid external converters, shell commands and Office execution;
DOCX extraction needs only bounded ZIP entries and WordprocessingML paragraphs.
UTF-8 text, CSV and JSON receive bounded structural validation. Pin compatible
versions in Cargo.lock. `lopdf` 0.38 is a compatible release in this workspace:
newer releases require `rand_core` 0.10 stable, which conflicts with the existing
`russh` prerelease pin. The parser runs only behind the worker limits; upgrade
it with that dependency when the pins converge. PDF pages and DOCX paragraphs
provide section offsets.

Extraction runs off the async executor with `spawn_blocking`, a wall-clock
limit and bounded concurrency. A separate worker process provides a killable
boundary for parsers, with CPU/address-space limits where supported, no inherited
secrets and bounded stdin/stdout. Limit PDF pages to 200, extracted text to one
million characters, DOCX expansion to 40 MiB and entry count to 1,000; read actual
expanded bytes rather than trusting ZIP size declarations. XML external entities
are never resolved. Reject encrypted PDFs and encrypted ZIP entries.

`nyx__attachment_read {attachment_id, offset, limit}` returns at most 6,000 text
characters, section/page information and `next_offset`, keeping the full tool
result below NyxAgent's 10,000-character ceiling. It requires the active thread's
actual assistant credential, and a message-bound upload in that conversation or
its current owner-controlled group. A specialist's unrelated thread has no access.

The per-turn input context lists only that message's attachment IDs, safe names,
verified types, sizes and page/section counts, including on resumed sessions and
attachment-only messages. It uses the session marker contract in
[08](08-nyxagent-engine.md#session-instructions-and-per-turn-context), never stored
user text. Expiry and unsupported-image notices travel in that same context,
once per request, including continuations and recovery. File content, names and image text are
untrusted data: they cannot alter instructions, confer grants or approve actions.
Documents are read on demand, not pasted wholesale into prompts.

When a reset recap omits or truncates owner requirements, an owner chat key may
use `nyx__conversation_history {before_seq, offset, limit}` to page complete owner
messages for that conversation. The tool returns a bounded, doubly escaped MCP
envelope with `complete` and a continuation cursor. Messages are returned newest
first. `before_seq` is an exclusive sequence bound; `offset` is a character
offset into the newest message in that window. Use both cursor fields from the
response to continue a split message before moving to older ones. Guest turns
and stale or other-conversation keys are refused.

The metadata list is restricted to message-bound user uploads in this conversation,
including policy-expired rows. Its opaque keyset cursor follows `(created_at, _id)`
newest first; pages return at most 50 rows and fewer when serialization requires it.
Both list and document pages budget the final, doubly escaped MCP envelope below
9,000 bytes. Offsets and recorded coverage reflect only the final returned text.
Existing group-member read authority still permits the group uploads included in
that thread's per-turn context; the list and recap manifest do not enumerate groups.

Review cards show upload coverage as observations in the drafting conversation,
which may belong to a different agent from the skill's attachment destination.
Atomic interval appends retain
the latest 512 returned ranges; their union never fills unread gaps. After interval
eviction, percentages are lower bounds. Since-reset coverage uses only ranges
recorded after the conversation's latest context reset. Review observations show the newest 50
message-bound uploads in this conversation and count any earlier uploads. New
extractions persist their character count; only legacy rows need decryption to
compute it and back-fill the count once. Unreadable legacy text leaves that upload’s
coverage unavailable without hiding the draft. A complete scope declaration is the author's
claim, not a server guarantee; reduced scope needs separate owner acceptance.

## Images and upstream compatibility

NyxAgent advertises an additive `input_image` capability with explicit limits.
NyxID sends images only when that capability is present, using an attachment URL
fetched with the same thread Agent Key. A separate key-authenticated content route
accepts that thread's bound user images and tool images produced in its current
live, non-stopped turn. Owner, conversation, turn, credential and retention fences
apply to tool images, including guest turns. The browser attachment route retains
its existing human access policy.

NyxAgent restricts remote images to its configured NyxID origin and the attachment
content path, forbids redirects and credentials in URLs, uses the request's own
key, and validates size and raster type before handing image input to Codex.
Sessions and responses use the same validation. No arbitrary remote image fetches.
An absent/older capability gives an explicit user and agent notice: the image was
received but this agent version cannot view it. Offer
`nyx__machine_save_attachment`; never silently omit an image or claim it was seen.
Attachment saves support 20 MiB on updated machine nodes. Older nodes retain
5 MiB: update them before saving larger files. Tool-image reads and clipboard
transfers retain their existing 5 MiB limits.

## User experience and validation

The composer supports attach, drop and paste with previews, progress, individual
errors and removal before send. Pending/failed uploads prevent sending until
resolved or removed. Text is optional when files are attached. Conversation
switches must not send another draft's attachments. Transcripts show safe metadata
and owner-authenticated downloads/previews with `nosniff`, sandbox CSP and private
caching. Follow DESIGN.md; unsupported engines show no misleading attach control.
The picker does not filter by browser MIME or suffix: extensionless text must be
selectable too. Sniffing and case-insensitive extensions on the server classify
the file; browser MIME is ignored. Plain text accepts `.txt`, `.text` and no
extension, and Markdown accepts `.md` and `.markdown`.
Unsupported types return an actionable server error shown beside that file in
the composer; sending stays blocked until the rejected file is removed.

Each attachment gets a composer handle (`@image-1`, `@file-1`, `@pasted-text-1`)
shown on its chip and offered when the writer types `@`. Before sending, each
`@handle` becomes a plain description of its file (`image "chart.png"`), so
servers that route `@name` to agents never see a handle; removing a chip removes
its references. Pasting more than six lines, or more than 1,500 characters, of
text attaches it as `pasted-text-N.md` (when it looks like Markdown) or `.txt` and
places its handle at the caret. While a reply runs, uploads are closed, so a long
paste stays ordinary text. Attachments do not survive leaving the composer, so a
saved draft never keeps a handle: pasted text returns inline (bounded like the
textbox) and other files become their description. File names never add an
`@mention`. Images show above the text of the message they belong to.

Acceptance coverage includes spoofing, oversize, ZIP expansion, encrypted/malformed
PDF, timeout, owner/guest/thread/specialist ACLs, atomic message binding, expiry,
delete/purge, paging, prompt listing, upstream capability fallback, Codex image
forwarding and composer behavior. Inbound channel coverage also exercises live
sender/route authority, native dedup, queued media, Stop/finalization and the
shared byte budget described above.

## HTTP and rollout

- `POST /assistant/nyxagent/drafts` creates the owner’s empty thread before upload.
- `POST /assistant/nyxagent/{conversations|groups}/{id}/attachments` takes raw bytes
  and a percent-encoded `X-Attachment-Name`; the declared Content-Type is ignored.
- `DELETE` the corresponding attachment URL removes only an unbound upload;
  owner `GET` serves bytes with `nosniff` and sandbox CSP.
- `GET /assistant-attachments/{uuid}/content` accepts only the bound thread Agent
  Key and only user-upload images. Delegated account-read credentials are denied.

Paths above are relative to `/api/v1`. Deploy NyxID and the separate NyxAgent
image PR independently: missing capability metadata is a supported fallback.
Protocol version 1 advertises ten images, 8 MiB each and 16 MiB total; larger
NyxID uploads remain downloadable and machine-transferable but are explicitly
marked unviewable by that agent. This protocol version, not a guessed package
version, is the compatibility boundary. The shared byte budget admits four full
browser uploads concurrently when otherwise idle; two parser workers may run
concurrently per replica. Budget waits are cancellable.

### Retention rollout

Upgrade and drain **all old replicas before changing retention**. Old binaries
can recreate the fixed TTL index and use the old read rules. Startup idempotently
removes only the `assistant_attachments.expires_at` TTL index; the upload-rate
window TTL stays in place. New rows use `created_at` and `bound_at`, never a fixed
expiration. For older bound roots without `bound_at`, derive it from the old
`expires_at` minus 30 days (creation time is the final fallback). No payload
rewrite or full collection scan is required during startup. Already-deleted
legacy files cannot be recovered.

### Ingress and operator limits

Reverse proxies, load balancers and ingress controllers MUST allow request bodies
of at least **21 MiB (22,020,096 bytes)** on both upload routes:
`POST /api/v1/assistant/nyxagent/conversations/{id}/attachments` and
`POST /api/v1/assistant/nyxagent/groups/{id}/attachments`. For nginx, configure
`client_max_body_size 21m;` in the existing location serving these routes; retain
its existing upstream and authentication configuration. Check every hop, including
any CDN. This ingress allowance does not change NyxID's 20 MiB file limit.

Admission and extraction limits are fixed application limits, not environment
variables. Retention is the runtime admin policy above:

| Limit | Value / behavior |
| --- | --- |
| Raw upload body | 1 byte through 20 MiB (20,971,520 bytes), inclusive; larger bodies return HTTP 400 with `Attachment exceeds 20 MiB.` |
| Attachments per message | 10 distinct files |
| Owner rate | 30 attempts per UTC minute across replicas; excess returns HTTP 429 |
| Concurrent work per replica | 192 MiB shared byte budget (2 × payload capacity per item); four full browser uploads fit when idle; 2 extraction workers |
| Extraction | 8-second worker wall deadline, 6 CPU seconds, 1 GiB address-space limit on Linux |
| Document expansion | 200 PDF pages, 1,000,000 extracted characters, 40 MiB actual DOCX expansion and 1,000 ZIP entries |
| Image decoding | 32 million pixels; 192 MiB decoder allocation budget |
| Retention defaults | Pending: 24 hours; sent images/documents: 30 days; tool images: conversation lifetime. Admin-overridable; see [Retention](#retention). |

An ingress-generated HTTP 413 below these limits indicates that a proxy's body
limit needs updating. Invalid/encrypted documents and extraction deadlines return
HTTP 400 with a fixed actionable message. No filenames or content are logged.
