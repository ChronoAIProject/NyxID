# Assistant uploads

Normative contract for human uploads to NyxAgent conversations and owner-posted
group messages. This extends [08](08-nyxagent-engine.md) and
[09](09-nyxbot-orchestrator.md). Tool images remain owner-only; their default
retention is the conversation lifetime.

## Admission and storage

Only a first-party human owner may upload or remove a pending upload. Guest,
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

Turn instructions list only that message's attachment IDs, safe names, verified
types, sizes and page/section counts. File content, names and image text are
untrusted data: they cannot alter instructions, confer grants or approve actions.
Documents are read on demand, not pasted wholesale into prompts.

## Images and upstream compatibility

NyxAgent advertises an additive `input_image` capability with explicit limits.
NyxID sends images only when that capability is present, using an attachment URL
fetched with the same thread Agent Key. A separate key-authenticated content route
accepts only that thread's bound user images; it never widens the owner-only
browser attachment route or permits tool-image retrieval by an agent.

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

Acceptance coverage includes spoofing, oversize, ZIP expansion, encrypted/malformed
PDF, timeout, owner/guest/thread/specialist ACLs, atomic message binding, expiry,
delete/purge, paging, prompt listing, upstream capability fallback, Codex image
forwarding and composer behavior. Inbound channel media is a follow-up: its sender
verification and download lifecycle require a separate ingress review.

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
version, is the compatibility boundary. Four upload requests and two parser
workers may run concurrently per replica. Busy admission asks the owner to retry.

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
| Concurrent work per replica | 4 upload handlers and 2 extraction workers; full upload admission returns HTTP 400 with a retry message |
| Extraction | 8-second worker wall deadline, 6 CPU seconds, 1 GiB address-space limit on Linux |
| Document expansion | 200 PDF pages, 1,000,000 extracted characters, 40 MiB actual DOCX expansion and 1,000 ZIP entries |
| Image decoding | 32 million pixels; 192 MiB decoder allocation budget |
| Retention defaults | Pending: 24 hours; sent images/documents: 30 days; tool images: conversation lifetime. Admin-overridable; see [Retention](#retention). |

An ingress-generated HTTP 413 below these limits indicates that a proxy's body
limit needs updating. Invalid/encrypted documents and extraction deadlines return
HTTP 400 with a fixed actionable message. No filenames or content are logged.
