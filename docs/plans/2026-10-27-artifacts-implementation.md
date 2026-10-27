# NyxID generated file delivery, preview, download, and recovery

**Issue:** [#2037](https://github.com/ChronoAIProject/NyxID/issues/2037)  
**Branch:** `feat/chat-file-delivery-2037`  
**Base:** `b1dd6548a feat: assistant UI lift v1 (#2048)`  
**Plan revision:** `v4` (the SHA-256 of this complete file is the Astra implementation gate)  
**Owner:** GPT-6-Astra PM  
**Scope:** web assistant chat, Sandbox async execution results, private assistant attachments

## Root cause and evidence

Issue #2037 confirms a user-visible delivery failure: an async Sandbox execution was reported as producing an MP4, but the originating Web chat contained no attachment, playable media element, or download action. The user was sent to a machine setup flow even though the requested bytes were supposed to come from the cloud execution. The report explicitly says that the MP4 bytes, codec, and expiry were never independently obtained, so the claim that a valid 20-second video existed is not acceptance evidence.

The current code explains the gap:

* `backend/src/services/async_service_operation.rs` uses `RESULT_BYTES = 16 * 1024`, rejects a result when `ToolResponse.media` is present, and stores only bounded encrypted text in `result_encrypted`. The current states are `submitting -> waiting -> ready -> queued -> delivered`; cancellation and expiry clear the encrypted result.
* `backend/src/services/mcp_service.rs` turns bounded downstream bodies into text and deliberately drops media when `response_body_limit` is set. `ToolMedia` validates raster images only. Raising the global limit or changing `ToolResponse.media` would silently change unrelated consumers and exact-result contracts.
* `AssistantAttachment` and `TurnAttachment` already provide envelope-encrypted, owner-scoped persistence. Settlement copies active-turn attachments to the durable assistant reply, and `GET /api/v1/assistant/nyxagent/conversations/{conversation_id}/attachments/{attachment_id}` already performs ownership and retention checks before returning private bytes.
* The existing attachment mapper and `ToolImage` component support durable images and download-only non-images, but the history mapper currently represents every attachment as a `ChatImage`; it has no video preview path and no generated-result filename/source/revision metadata.
* Existing retention distinguishes available, expired, tombstoned, and missing attachments. This work must reuse those distinctions and must not turn an unverified upstream statement into a delivered-file claim.

The exact live Sandbox OpenAPI document was checked at `https://sandbox.chrono-ai.fun/openapi.json` (title `Chrono Sandbox API`, version `0.3.0`). `GET /executions/{operation_id}/result` returns an `ExecuteResponse` whose nullable `output.files` entries contain only `path`, base64 `content`, nullable `error`, and nullable `size`. There is no upstream MIME field. `GET /executions/{operation_id}` reports `result_available`, status/phase, version, and expiry. A `succeeded` status means a known result, but callers must still inspect `ExecuteResponse.success`; a non-zero script exit is a known result and is not an infrastructure failure. Expired result reads return `410` with an `ExpiredExecutionResponse`.

## Product behavior

For a declared cloud Sandbox result, NyxBot must distinguish these facts in the originating thread:

1. the operation was submitted;
2. the Sandbox reported a result;
3. NyxID verified and retained one or more result files;
4. the files were materialized as private chat attachments; and
5. the owner can currently read them.

Only steps 3–5 permit wording such as “the video is attached” or “download it.” A result card shows the sanitized filename, verified type, byte count, retention/expiry state, and the server-local content revision (with the source identity retained server-side). Supported raster images retain their current thumbnail, full preview, and download behavior. Supported MP4 files render a `<video controls>` preview and a download action. A file that is valid and retained but has no supported preview receives a download action and a precise “preview unavailable for this type” message. Expired, missing, unauthorized, cancelled, and delivery-failed states have separate messages and actions.

The owner does not install a machine agent, handle base64, receive a public URL, or grant unrelated execution authority to retrieve a cloud result. A recovery request reuses the same verified result and operation identity; it never submits a new Sandbox execution. A later content modification is a new Sandbox result revision and a distinct attachment. A failed revision cannot relabel the previous bytes.

## In scope and out of scope

In scope are the Web chat path, verified Sandbox `output.files`, private encrypted storage, image regression, supported MP4 preview/download, bounded download fallback for other safe files, durable history, refresh/revisit behavior, delivery recovery, cancellation/expiry cleanup, and focused backend/browser tests with real media fixtures.

An independent asset library, public publishing, cross-channel distribution, professional editing, arbitrary video codecs, PDF/spreadsheet previewers, and machine-agent setup are out of scope. The existing machine path remains available for tasks that actually require a local computer, but it is never the default recovery path for a cloud result.

## Proposed limits and compatibility contract

These limits are deliberately separate from the existing 16 KiB text limit and were personally assessed by Astra against the coordinator findings before implementation.

| Item | Contract |
| --- | --- |
| Existing status/text result | `RESULT_BYTES = 16 KiB`, unchanged |
| Sandbox result JSON body | `SANDBOX_RESULT_RESPONSE_BYTES = 13 MiB` including JSON and base64 overhead |
| Files per result revision | 8 |
| One generated raster image | 5 MiB, using the existing image signature validator |
| One generated video or download-only file | 8 MiB |
| Total decoded artifact bytes | 9 MiB |
| Filename | UTF-8, basename only, normalized to 160 bytes; fallback `generated-file-{index}` |
| Retention | Pending rows expire at the minimum of source status expiry (when supplied) and watch expiry. A materialized attachment stores `expires_at = min(source status expiry, watch expiry, the normal assistant retention deadline computed at materialization)`; the existing retention evaluator and sweep enforce that single effective deadline, so materialization never extends source or watch availability. |

The result JSON limit is separate from the shared attachment memory budget. Every result fetch reserves one `assistant_upload_service::byte_budget::reserve` permit for `SANDBOX_RESULT_RESPONSE_BYTES + TOTAL_DECODED_ARTIFACT_BYTES` (the reservation itself is weighted at twice that payload limit by the existing 192 MiB semaphore). The permit is acquired before the HTTP body read and held through JSON parsing, base64 decode, structural validation, digesting, envelope encryption, and pending-row persistence. One bounded artifact worker handles a result at a time; it does not reserve independent untracked copies for each file. The permit is dropped on cancellation, timeout, rejection, or commit failure, and authenticated attachment egress continues to reserve the exact stored payload size through the same budget.

The largest pending row is proven before implementation: a test serializes an encrypted 8 MiB artifact plus BSON metadata, encryption overhead, and bounded manifest fields and asserts the BSON document is below MongoDB's 16 MiB document limit. The service rejects any serialized pending row at or above that limit. Multi-file results stay one row per file in `async_service_operation_artifacts`; a watch document never contains a multi-file blob.

Optional model fields preserve decoding of legacy rows, but they do not by themselves make mixed workers safe. Add `ASSISTANT_GENERATED_ARTIFACTS_FLAG_KEY = "assistant:generated-artifacts"` to `feature_flag_service` as a default-off personal flag, using the existing `personal_flag_enabled` resolution and readiness behavior. At Web user-turn admission, the server checks the flag and requires `DeliveryBinding.origin == TurnOrigin::User`, `channel == None`, `reply_channel == None`, `group_id == None`, `group_request_id == None`, `voice_request_id == None`, `trigger_run_id == None`, and `platform_reply == WebOnly`; it then writes a server-owned `artifact_mode = generated_artifact_v1` marker on the watch. The result is later consumed only by the server-created completion `Event` turn whose `event_id(row)` equals that watch ID and whose watch `turn_id` is the original user turn. The materializer checks that exact association, so an unrelated scheduled/event, channel, trigger, or voice turn cannot consume it. Channel, scheduled/event, trigger, and voice watchers are explicitly denied artifact admission and retain their existing text/image behavior. Existing watches remain readable through the legacy path; new workers retain the legacy fallback for rows without `artifact_mode`.

The activation sequence is mandatory: (1) leave the flag disabled; (2) deploy the parser, optional attachment fields, recovery route, and new collection/index creation to every API and async worker; (3) drain and remove all old async workers and old writers, verifying no old binary is still claiming watches; (4) verify every relevant replica reports the new artifact-capable build and indexes exist; (5) enable the flag only for the intended Web audience; (6) observe pending/materialized/cleanup metrics before broadening. The flag controls new watch admission only. Once a watch has the server-written `artifact_mode`, upgraded workers may fetch and materialize it even if the flag is later disabled, so rollback can drain those watches; the materializer still enforces the saved provenance and event association. Rollback first disables new admission, then leaves artifact-capable workers running until every artifact watch is terminal, materialized, or source-expired and cleaned. Old binaries must not be restarted while artifact-enabled watches remain pending. Rollback never deletes the collection or rewrites existing attachment bytes. Tests cover flag-off behavior, Web-only admission, rejection of channel/scheduled/trigger/voice origins, exact completion-event binding, old-row fallback, and mixed-version drain/rollback sequencing.

## Interfaces and data flow

### 1. Bounded Sandbox result adapter

Add a private serde adapter for the verified `ExecuteResponse`/`ExecutionOutput`/`RetrievedFile` shape. The async worker reads the declared result operation with the larger result-body limit, checks HTTP status and `success`, and parses only `output.files`. It preserves a bounded textual summary (stdout/stderr/exit code and stable errors) under `RESULT_BYTES`; it never places base64 or binary bytes in NyxAgent input context.

Each file is processed as follows:

* Decode base64 with strict alphabet/padding and a pre-decode length check derived from the per-file and total limits. A malformed or oversized entry is rejected with a stable `artifact_invalid`/`artifact_too_large` reason and does not partially publish the result.
* Treat `path` and upstream `size` as untrusted. Normalize to a basename, remove control characters and path separators, and use the decoded byte length as authoritative. A conflicting upstream `size` is recorded only as bounded diagnostic metadata.
* Infer a candidate type from the normalized extension, then verify bytes. Reuse the existing PNG/JPEG/GIF/WebP signature checks. For MP4, use a bounded ISO-BMFF parser with checked arithmetic and recursion/depth limits: validate `ftyp`, every box size and boundary, a reachable `moov`, `trak`/`mdia`/`minf`/`stbl` structure, a visual `hdlr`, dimensions, and an actual visual sample entry (`avc1`/`avc3`, `hvc1`/`hev1`, `av01`, or another explicitly tested allowlisted entry). Marker strings without a valid visual track never enable preview. The browser codec/container allowlist is derived from real fixtures and browser playback tests, not from the sample-entry string alone. Files without a supported preview remain download-only if their bytes pass the generic bounded file policy.
* Never fetch a URL supplied by the Sandbox, infer a remote URL from a path, or trust an upstream MIME string (none is supplied by the verified contract). Do not accept active HTML/SVG as inline content; unsupported or ambiguous content is served as an attachment with `application/octet-stream`.

### 2. Pending result artifacts

Create a new `async_service_operation_artifacts` model/collection for encrypted pending bytes instead of enlarging the watch document. The async watch receives one optional server-written `artifact_mode` marker (`generated_artifact_v1`) at Web admission; old rows omit it and old workers are drained before any marker is written. Each artifact row is keyed by a UUID and contains:

* `watch_id`, `user_id`, `conversation_id`, originating `turn_id`, Sandbox `operation_id`, and the NyxID `source_result_identity`;
* `artifact_index`, normalized `filename`, verified `content_type`, decoded `size`, `sha256`, preview capability, and source path metadata;
* optional `source_status_version` retained only as bounded diagnostics (never as content identity) and an optional server-assigned `content_revision` label;
* envelope-encrypted bytes plus the encryption context/version map;
* `state` (`pending`, `materializing`, `materialized`, `expired`, `discarded`), optional `assistant_attachment_id`, `created_at`, and `expires_at`.

Add a unique index on `(watch_id, source_result_identity, artifact_index)` and cleanup indexes on `(expires_at, state)` and `assistant_attachment_id`. Never include encrypted bytes, hashes of secret-bearing context, or raw upstream content in logs/audit. The watch keeps only the optional server-written `artifact_mode`, manifest/count, source identity, bounded diagnostic status version, server-local content revision, and stable delivery reason; its existing `result_encrypted` text lifecycle remains unchanged.

Extend `AssistantAttachment` and `TurnAttachment` additively with optional `filename`, `source`/`source_operation_id`, `source_result_identity`, `source_status_version` (diagnostic only), `content_revision`, and `expires_at` metadata. Existing rows deserialize with defaults. `AttachmentResponse` exposes only safe filename/type/size/expiry/preview metadata and a server-local revision label; it does not expose source operation IDs as links. The existing attachment ID and authenticated endpoint remain the only retrieval capability.

### 3. Materialization and history

When `bind_in_session` admits a queued completion event, it verifies the watch's saved Web provenance (`artifact_mode`, `delivery.origin == User`, all platform destinations empty/WebOnly), `w.turn_id` as the original user turn, `event_id(row) == w.id`, owner, conversation, operation ID, source-result identity, and the saved association in one transaction. It does not require the feature flag to remain enabled for an already-admitted watch. It upserts each pending artifact using the unique key, creates the encrypted `AssistantAttachment`, and appends a corresponding `TurnAttachment` to the active turn. The operation row records a stable materialization result and the assistant attachment ID. A retry seeing an existing materialized row returns it without copying bytes or creating a duplicate.

The existing turn settlement copies active-turn attachment metadata into the durable assistant reply. Therefore reload and revisit use the ordinary history route and retention checks. If an active turn has no remaining attachment slots, materialization records `delivery_capacity_exhausted`, leaves the pending artifact available until the pending source/watch expiry, and the recovery action can retry the same result after the turn is eligible; it never silently drops or regenerates bytes. The normal eight-attachment turn bound remains enforced.

Settlement also writes the server-owned async-delivery association on the assistant message in the same durable update. That association is the only authority the recovery route accepts; clients and models cannot manufacture one by naming a watch or result.

On a successfully materialized result, pending encrypted rows may be cleared only after the assistant attachment insert and active-turn update commit. Cancellation, result expiry, conversation deletion, user deletion, and failed materialization cleanup clear pending bytes and mark the row terminal. Successfully materialized assistant attachments follow the existing attachment retention/tombstone sweep. `delivered_in_session` clears the old encrypted text and marks the watch delivered only after the artifact transaction has committed.

### 4. Recovery and versioning

Add the exact owner-only route `POST /api/v1/assistant/nyxagent/conversations/{conversation_id}/messages/{message_id}/artifact-delivery` with an empty body. The client supplies no watch ID, operation ID, turn ID, source URL, or revision. The handler loads the owner conversation/message and requires a server-written async-delivery association on that assistant message (or an equivalent server-owned association row) containing the exact watch ID, operation ID, source identity, owner, conversation, originating turn, and attachment lineage. It rejects foreign messages, missing or ambiguous associations, expired sources, and stale revisions. It invokes only the saved materialization/recovery path; it cannot submit a new Sandbox execution. Duplicate calls are idempotent. Natural-language recovery uses the same server association through a server-provided internal action and cannot accept model-selected IDs. This route is distinct from generic failed-turn retry and does not reuse `retry_context_in_session` for arbitrary recovery.

The Sandbox status `version` is lifecycle/status metadata only; it is never treated as immutable content identity. A successful fetch first requires the exact server-stored operation ID and a terminal/result-available status, then reads the result using only that ID and computes `source_result_identity` as SHA-256 over canonical operation ID, ordered normalized file metadata, and decoded bytes. The worker rechecks status/expiry around the fetch; a `410` is `result_expired`. If the same operation ID produces a different verified identity, the watch fails closed and no bytes are published. The effective expiry is the one documented above: `min(source status expiry when present, watch expiry, normal assistant retention deadline computed at materialization)`; pending cleanup uses the first two, and materialized reads/sweeps use the stored minimum. A new content revision means a new Sandbox execution/operation ID; NyxID does not claim the Sandbox supports mutable revisions. If a UI label is needed, `content_revision` is server-assigned and local. A later supported result creates a new pending-artifact set and new attachment IDs; a failed revision leaves the old set and its metadata intact.

### 5. Attachment HTTP contract

Keep `GET /api/v1/assistant/nyxagent/conversations/{conversation_id}/attachments/{attachment_id}` as the owner route. Before every read, perform the existing owner/feature/retention check and verify the attachment belongs to the requested conversation. Return `Cache-Control: private, no-store`, `X-Content-Type-Options: nosniff`, and the existing sandbox CSP. Sanitize `Content-Disposition` and use `inline` only for verified image types and supported MP4; use `attachment` for all other types. The endpoint remains bounded to the stored byte size and never redirects.

The endpoint follows HTTP range semantics. The implementation may support one authorized bounded single range with checked arithmetic, `206`, `Accept-Ranges: bytes`, and a correct `Content-Range`; multipart, overlapping, or unsafe ranges are rejected according to the parser. If the server chooses not to implement ranges, a syntactically valid but unsupported range is ignored and returns the complete bounded body with `200`; `416` is reserved for an unsatisfiable/malformed range under the implemented policy, never as a generic “range unsupported” response. Every path repeats owner/retention checks and cannot bypass them. Browser playback uses an authenticated fetch/object URL, and tests cover both the selected range policy and authorization on range requests.

## Frontend behavior

Extend the assistant attachment DTO and `ChatImage`-compatible mapper with `filename`, `contentType`, `size`, `expiresAt`, `source`, `resultVersion`, and a preview kind. Preserve current image IDs/endpoints and image-input flags. Do not expose source operation IDs as links or credentials.

Update `ToolImage` (or a narrowly scoped sibling renderer) to:

* fetch the authenticated endpoint through the existing assistant client, create and revoke a local object URL, and show loading/error/expired states;
* render verified MP4 as `<video controls preload="metadata">` with an accessible name, a visible download button, and keyboard-operable controls; exercise a real supported MP4 fixture in a browser and listen for `error`/decode failure;
* retain the current image thumbnail/full-preview/download behavior and ensure all three actions use the same attachment ID;
* on browser `error`/decode failure, hide/disable playback and retain a working download action with accurate copy; unsupported/ambiguous files are download-only and never render an unavailable play button;
* distinguish “delivery pending/failed,” “expired,” “not found,” and “not authorized” in user-facing copy without leaking whether a foreign attachment exists.

The live AG-UI `MEDIA_CONTENT` and `ArtifactBlock` path remains unchanged. Durable Sandbox files use the attachment path, so refresh/history and live rendering share one private source of truth.

## Lifecycle and failure matrix

| Event | Pending watch/artifact | Materialized attachment | User-visible result |
| --- | --- | --- | --- |
| Sandbox text-only success | Existing text path | None | Existing bounded summary |
| Verified files, materialization succeeds | `materialized`, bytes cleared after commit | Available through the stored effective expiry | Result card with metadata and preview/download |
| Verified files, server-created completion event turn interrupted | `pending` or retryable | None or already-created idempotent row | Delivery pending; retry reuses the Web-origin watch/result and exact completion-event association |
| Duplicate receipt/retry | Existing unique rows | Existing attachment IDs | No duplicate artifact or generation |
| Source result expired before materialization | `expired`, bytes cleared | None | Explicit expired result; regeneration requires intent |
| Cancellation before materialization | `discarded`, bytes cleared | None | Cancelled; no late delivery |
| Failed new revision | New rows discarded/failed | Old attachment untouched | Revision failed; old version remains labeled old |
| Effective expiry reached | Pending gone or materialized tombstone | Existing retention state at `min(source, watch, retention)` | Expired attachment; no download |
| Owner/ACL mismatch | No existence detail | No bytes | Not-found-shaped/unauthorized response |
| Malformed/oversized/malicious bytes | Terminal invalid | None | Stable unsupported/invalid result reason; no preview |

Infrastructure failures preserve the last stable reason and lease retryability. A result read that times out or disconnects never marks the operation delivered. Lease fencing prevents stale workers from publishing artifacts after cancellation or a newer result revision.

## Security and privacy

All artifact reads retain the existing authenticated owner/conversation/turn fences and retention evaluator. Group or channel sharing is not added by this issue. No public, signed, or cross-conversation URL is created. Pending rows and assistant attachments use the existing envelope encryption and zeroization conventions; encryption context includes the exact owner, conversation, turn, watch, operation, and source-result identity.

Base64, filenames, MIME candidates, and Sandbox output are untrusted. Bound every allocation before decode, reject traversal/control characters, verify magic/container structure, and serve potentially active formats only as downloads. Do not perform SSRF or follow redirects. Do not place binary data, full filenames, source paths, or credentials in logs, audit, telemetry labels, model context, or error text. Stable reason codes are sufficient for diagnostics.

## Documentation and shared contracts

Update `docs/chat/08-nyxagent-engine.md`, `docs/chat/09-nyxbot-orchestrator.md`, `docs/chat/10-uploads.md`, `docs/chat/03-stream-protocol.md`, `docs/chat/05-frontend-ui.md`, and `docs/API_DISCOVERY.md` to document:

* the verified Sandbox files contract and separate result-body/artifact limits;
* verified-generation versus materialized-delivery wording;
* pending-artifact lifecycle, idempotent recovery, retention, and owner-only reads;
* MP4 preview allowlist, download fallback, and unsupported/expired UI states;
* the additive attachment metadata and unchanged live media semantics.

Before changing shared assistant runtime/model fields, send the coordinator the exact additive fields and affected files. Keep automation's completion semantics independent: it may report that an async operation completed while artifact delivery remains a separate, persisted state.

## Rollout and rollback

Deploy the backend parser, optional attachment fields, pending-artifact collection/indexes, exact owner/message recovery route, and owner association writes before relying on the frontend. Index creation is idempotent and bounded. The concrete gate is `ASSISTANT_GENERATED_ARTIFACTS_FLAG_KEY` resolved by `personal_flag_enabled`; there is no implicit service capability/configuration fallback. Keep it default-off until all API and async worker replicas are upgraded and old workers/writers are drained as specified above. The upgraded worker accepts legacy text/image watches and creates artifact rows only for newly admitted Web-origin Sandbox watches with `artifact_mode`; already-admitted artifact watches remain materializable while the flag is disabled. If malformed responses or delivery failures appear, disable this flag (status/text remains available), stop new artifact admission, and keep upgraded workers draining existing artifact watches. No flag may claim successful delivery without a materialized attachment and a passing authenticated read.

## Acceptance matrix and executable verification

| Acceptance | Evidence required |
| --- | --- |
| Sandbox contract | Fixture and parser test for `success`, non-zero exit, nullable output, file errors, base64 bounds, missing MIME, and `410` expiry |
| Mixed-version admission | Feature-flag tests prove default-off new admission, Web-origin-only provenance, channel/scheduled/trigger/voice rejection, exact completion-event binding, legacy-watch fallback, and drain-before-enable/disable-before-rollback behavior; already-admitted rows materialize while the flag is disabled |
| Shared memory/BSON bounds | Budget test proves one reservation spans body/decode/hash/encryption/persistence and cancellation releases it; BSON test serializes an encrypted 8 MiB row with manifest overhead below 16 MiB and rejects larger rows |
| Real image regression | Backend fixture stores validated PNG/JPEG; browser test proves thumbnail, full preview, and download fetch the same attachment ID and bytes |
| MP4 delivery | Real small MP4 fixture passes structured container/visual-track validation; browser test plays/pauses, observes no decode error, exercises download, and downloaded bytes reopen with the fixture verifier |
| Unsupported download | Valid bounded non-preview file renders a download-only card with accurate metadata and no play control |
| Durable history | API/integration test settles a completion event, reloads history, reads the same attachment, and proves the stored minimum expiry is enforced and distinct from missing |
| Recovery | Inject a server-associated generated-but-undelivered message, call the exact empty-body owner/message route, assert no new Sandbox submission, one materialized attachment, rejection of client-supplied IDs, and idempotent duplicate retry |
| Versioning | Two result revisions produce two IDs/metadata rows; failed second revision leaves first bytes and label unchanged |
| Source identity race | Status/result race tests use one server operation ID, reject `410`, reject a changed verified digest, ignore Sandbox lifecycle `version` as identity, and apply minimum source/watch/retention expiry |
| Authorization/security | Owner, foreign owner, expired, tombstoned, malformed path, active-content, oversized, range, and no-store/nosniff/disposition tests |
| Cleanup | Cancellation, source expiry, conversation deletion, and user purge remove pending encrypted bytes; materialized retention follows existing sweep |
| UI accessibility | Keyboard-accessible play/download, accessible names, loading/error/expired states, object URL cleanup, and no unavailable play control |
| Actual source-to-chat acceptance | Authorized Web journey against a real Sandbox operation obtains bytes, verifies the file, sees the originating chat card, plays/downloads it, refreshes, retries after simulated delivery failure, and records source/result/attachment IDs without secrets |

Run focused backend tests with `CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2` and normal Cargo locking. Run the frontend unit/component suite and typecheck/lint commands documented by the repository. Use a workstream-unique Mongo database/port and real media fixtures; never run `cargo clean`, remove the target lock, or reuse another workstream's test data.

The final gate stays open until the authorized source-to-chat journey and all focused checks have concrete output. If authentication or the live Sandbox account is unavailable, record the exact blocked command/state in the status record and continue all independent tests; do not call the feature complete with a caveat.

## Open dependencies before implementation

1. The user amendment explicitly supersedes the earlier Opus plan and final implementation gates. Astra's `ASTRA_PLAN_APPROVED` record is bound to the exact SHA of this v4 plan; any later plan edit changes that SHA and requires renewed Astra assessment before dependent implementation.
2. The coordinator must be informed of the proposed shared fields and parser/attachment collection before implementation.
3. An authorized live Sandbox/Web account is required for the final source-to-chat acceptance. If it remains unavailable, record the exact blocked state and keep that acceptance item open while completing all independent checks.

