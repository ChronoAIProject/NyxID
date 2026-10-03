# Assistant uploads

Normative contract for human uploads to NyxAgent conversations and owner-posted
group messages. This extends [08](08-nyxagent-engine.md) and
[09](09-nyxbot-orchestrator.md); existing owner-only tool images keep their behavior.

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
reused by another message, conversation, owner or group. Pending uploads expire
after 24 hours; bound user uploads expire after 30 days. Enforce expiry in reads
as well as TTL cleanup. Conversation/group deletion and owner purge remove all
payload chunks and extracted text. Removing an unsent attachment deletes it.

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

### Ingress and operator limits

Reverse proxies, load balancers and ingress controllers MUST allow request bodies
of at least **21 MiB (22,020,096 bytes)** on both upload routes:
`POST /api/v1/assistant/nyxagent/conversations/{id}/attachments` and
`POST /api/v1/assistant/nyxagent/groups/{id}/attachments`. For nginx, configure
`client_max_body_size 21m;` in the existing location serving these routes; retain
its existing upstream and authentication configuration. Check every hop, including
any CDN. This ingress allowance does not change NyxID's 20 MiB file limit.

These limits are fixed application limits, not environment variables:

| Limit | Value / behavior |
| --- | --- |
| Raw upload body | 1 byte through 20 MiB (20,971,520 bytes), inclusive; larger bodies return HTTP 400 with `Attachment exceeds 20 MiB.` |
| Attachments per message | 10 distinct files |
| Owner rate | 30 attempts per UTC minute across replicas; excess returns HTTP 429 |
| Concurrent work per replica | 4 upload handlers and 2 extraction workers; full upload admission returns HTTP 400 with a retry message |
| Extraction | 8-second worker wall deadline, 6 CPU seconds, 1 GiB address-space limit on Linux |
| Document expansion | 200 PDF pages, 1,000,000 extracted characters, 40 MiB actual DOCX expansion and 1,000 ZIP entries |
| Image decoding | 32 million pixels; 192 MiB decoder allocation budget |
| Retention | Pending uploads: 24 hours; message-bound uploads: 30 days |

An ingress-generated HTTP 413 below these limits indicates that a proxy's body
limit needs updating. Invalid/encrypted documents and extraction deadlines return
HTTP 400 with a fixed actionable message. No filenames or content are logged.
