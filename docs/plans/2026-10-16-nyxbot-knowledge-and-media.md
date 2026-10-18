# NyxBot general resource retrieval and media delivery

Date: 2026-10-16. Updated: 2026-10-17. Status: shared role/skill guidance and the
first general resource-to-image capability are implemented in this worktree.
The first source adapter is Notion. Indexed retrieval, background synchronization
and conversational onboarding remain proposed. See the implemented
[source resource contract](../chat/10-source-resources.md) for exact current scope.

The [independent Astra review](2026-10-16-nyxbot-astra-review.md) supplies code
references, prioritized findings and evaluation gates. Its accepted findings
are incorporated below.

Scope clarification: image retrieval/display is a shared NyxBot capability across
connected sources. A restaurant menu is an acceptance example. Per-business
menu setup and a RAG deployment are not prerequisites for retrieving and
displaying an authorized image. Saved resource bindings are an optional fast
path for repeated requests.

## Findings

The supplied KEK menu screenshot shows nine activities: three tool searches,
four skill reads (one failed) and two Notion block-children reads. It ends with
a menu link. The screenshot has no per-call timings or tool payloads, so it
does not establish which service was slow or why the skill read failed.

The current implementation explains several limitations:

- `backend/specs/catalog/notion.openapi.json` exposes Notion REST operations.
  Search matches page/database titles; page retrieval returns metadata and
  properties. Body content requires block-child reads, with additional calls
  for pagination and relevant nested blocks.
- `agent_skill_service::read` downloads and verifies a pinned Ornn package on
  each read. There is no shared verified-package cache in this path. Repeated
  reads introduce network work, although their contribution to this reported
  latency needs measurement. Reads deliberately recheck current access.
- `mcp_service::tool_media` recognizes actual PNG/JPEG/GIF/WebP response bytes,
  with a 5 MiB limit and matching MIME type/magic bytes. JSON containing an
  image URL never qualifies. `mcp_transport::service_tool_content` retains
  qualifying images through the existing encrypted attachment pipeline.
- `TextBlock` deliberately renders remote Markdown images as links. Changing
  a response to `![menu](url)` does not produce a retained chat image.
- Notion-hosted file URLs expire after about one hour. Stable page/block IDs
  are suitable source identities; signed file URLs are not permanent assets.
- Existing role prompts explain platform operations in detail but provide
  little direction on economical source retrieval and completing media requests.

## Changes included now

Shared guidance in `assistant_nyxagent::base_prompt` applies to both NyxBot and
specialists. It favors a direct path to a known source, reuse of available tool
schemas and current skill guidance, relevant rather than exhaustive document
reads, sufficient evidence before answering, and honest attachment delivery.
NyxBot's role now describes its effective permission mode instead of assuming
Full access. Existing guest, approval and grant restrictions remain authoritative.
Keep this shared guidance provider-neutral: Notion's title-search/block/file
procedures belong in connector/tool guidance and the relevant skills.

Attached-skill metadata now includes the exact root ID in a `skill` field that
can be passed directly to `nyxid__skill_read`. The reader already accepts IDs
and exact names; this makes the stable ID available without another discovery
step. This is not proof that the screenshot's failed read involved an ID error.

These changes update the stable instruction fingerprint. Threads with a stored
fingerprint reset their upstream binding on the next turn when it differs and
receive the normal bounded recap. Legacy sessions follow the existing
reset-or-adopt migration rules, so deployment does not reset all legacy threads.
The instruction changes do not establish a latency gain. The separate shared
resource implementation now retains retrieved images as encrypted attachments;
it does not add a knowledge index or background source synchronization.

The current image tool is `nyx__resource_get(ref, output="attach"|"inspect")`.
Published Notion page and child-block GETs can issue opaque, turn-scoped image
references. Redemption repeats the authorized source read, checks immutable
source identity/revision, refreshes the temporary file URL, validates a bounded
raster download and commits one attachment to the exact originating turn.
Display-only requests skip model image continuation. Equivalent references and
intent upgrades reuse one attachment within a turn. The detailed contract,
limits and source restrictions are in [Source resources](../chat/10-source-resources.md).
The broader `resources.search` / `resources.get` examples below remain design
sketches for the future knowledge interface.

## Recommended retrieval architecture

Keep existing source applications as the editing systems. Use Notion as the
first connector/example, with a general NyxBot resource and media contract.
Give NyxBot a narrow knowledge interface that returns relevant evidence,
citations, freshness and authorized asset
references together. Skills describe business procedures and when to use this
interface; the knowledge store owns changing facts and files.

### Shared runtime with source adapters

Follow the capability-adapter pattern: provider-specific code interprets a
provider's operations/results; NyxBot owns the common resource, attachment and
delivery behavior. Implement the adapter once per provider/response shape,
rather than per business or requested object. A capability is usable through
the agent's current connection and grants; an adapter does not grant access.

The existing `ToolResponse {status, text, media, resources}` and
`mcp_transport::service_tool_content` are a partial foundation: verified raster
bytes already become encrypted chat attachments. The missing shared behavior
is interpreting authorized file references in structured tool results and
materializing them. `OperationResponseContract` currently describes content
types and binary classification; it does not supply that locator/materializer.
Extend this response/adapter boundary while preserving text/approval digest
semantics. Do not infer a downloadable attachment from every URL in a body.

| Component | General responsibility | Provider-specific part |
| --- | --- | --- |
| Source adapter | Return normalized resource descriptors from authorized search/read results; resolve a descriptor to current content or bytes. | Notion image/file blocks and expiring URLs; Drive file IDs/download operations; a provider's direct binary output. |
| Resource service | Issue opaque references, route reads, check live access/freshness, reuse verified content and expose bounded evidence. | Select the registered source adapter and exact permitted operations. |
| Media materializer | Bounded fetching/validation, original assets and derivatives, retention, and per-request attachment creation. | Authentication and URL refresh rules supplied by the source adapter; credentials remain in the broker. |
| NyxBot runtime | Distinguish delivery from inspection, attach resources to the live turn, settle the reply and expose delivery state. | None of the provider's JSON or URL formats. |
| Destination adapter | Render in the app or send through the existing channel delivery path. | Each destination's image/file support and receipt semantics. |

Proposed model-facing primitives are `resources.search` and `resources.get`;
these are illustrative new contracts, not currently available tools.
Existing provider searches/reads can also emit normalized references, so an
extra global search is unnecessary when a result already identifies the file.

`resources.search` takes a query and optional source/type filters within the
caller's available sources. It returns a bounded candidate set, with permitted
titles, provenance, revision/freshness and opaque references. Source selection
uses connected capabilities and known source context; it does not query every
provider on every request. Backend source calls may still require pagination
or several reads. Reducing model-visible tool rounds does not eliminate that
underlying work or establish a latency guarantee.

`resources.get` takes a server-issued reference and an output purpose such as
`read`, `attach` or `inspect`. The reference identifies an authorized source
object; it is not a bearer grant, arbitrary URL, or model-created authority.
Every use checks current caller/agent/audience and source operation access.
Attaching can reuse verified asset bytes without resending them through model
context. Inspection additionally supplies image input or extracted evidence.
Explicit image requests may materialize a selected image in the original
resource-read operation when selection is unambiguous and policy allows it.

### Direct lookup mechanism

Direct lookup means reading an identified object by reference, rather than
searching for it again. The reference can come from a tool result, a user-given
source link resolved by its adapter, a synchronized index result, or a saved
resource binding. It does not require a business-specific menu tool.

For example, a Notion read yields an image block. The trusted Notion adapter
creates an opaque reference backed by the connection, page/block/property
selector and revision metadata. The model receives the image's reference, type
and label. The handle stores no signed CDN URL; materialization resolves a
fresh URL server-side. The ordinary source JSON is preserved and may itself
include provider URLs, so the annotation does not sanitize that existing result.

```text
user: "Show me the image on this Notion page"
  -> resolve the supplied page through the authorized Notion connection
  -> adapter returns its image resource reference(s)
  -> resources.get(selected_reference, output="attach")
  -> live access check + adapter resolves current bytes/URL
  -> shared bounded media materializer obtains verified bytes
  -> runtime creates a retry-idempotent attachment for this request
  -> app renders it; any channel destination uses its existing adapter
```

If a resource is already identified, start at `resources.get`. If it is unknown,
search first and select the supported result. If several candidates would change
the answer, use the request's context or ask the material clarification. A first
unknown request necessarily has a discovery step; direct lookup is the fast
path after identification, not a promise to resolve arbitrary natural language
without searching.

For repeated requests, an optional saved binding maps a contextual alias to a
stable source selector, for example `(collection, outlet, language, purpose)`
to the menu's page/property. This is configuration/cache data used by the same
resource service. Changes refresh or invalidate it; multiple variants remain
explicit. It is not required for an ordinary request to retrieve an image.

Simple declared binary/JSON reference shapes can eventually use trusted catalog
metadata. Provider rules involving expiring URLs, special download operations,
credential boundaries or async completion need a reviewed adapter. A generic
HTTP loader can handle permitted public image URLs using the same bounded
materializer. Private provider files use their source adapter and connection.
Skills can explain when to use these capabilities; basic attachment handling
belongs to the runtime and works without reading a media skill.

Use three retrieval paths:

1. **Known resources.** Read an already identified object through its reference.
   Optional saved bindings speed repeated requests such as menus, opening hours,
   addresses and policies. A menu binding identifies its source page/block,
   applicable outlet/language, revision and asset IDs. An exact lookup does not
   need vector search or a separately authored skill.
2. **Knowledge questions.** Run a single hybrid keyword/semantic retrieval,
   with metadata filtering and reranking where useful, over synchronized source
   content. Return a small evidence set with source IDs/URLs and freshness.
   Preserve section structure and extract searchable text from menu images/PDFs;
   keep the original files for delivery. OCR is evidence with its own confidence,
   not a substitute for the original menu when prices are unclear.
3. **Live actions.** Reservations, orders, stock checks and changes execute
   through the connected service under current authorization and approvals.
   Indexed content must never be treated as proof of live transactional state.

For repeated procedures, define a typed capability with its input schema,
permitted operation bindings, required evidence and output contract. A menu
is a resource resolved through the shared interface; a saved binding can avoid
rediscovering its source on each request. Do not add a separate implementation
or tool for each menu, invoice, presentation or other business object.
Unknown questions can use normal tool discovery. Route by explicit capabilities
and available sources, not tool-name guesses or unconstrained generated plans.
Plans and retrieved instructions never expand execution permissions.

An illustrative backend flow is:

```text
customer asks for the menu
  -> resolve the authorized menu resource for this agent/outlet
  -> obtain its current retained image/PDF assets
  -> attach through NyxID's existing image pipeline
  -> answer with a brief caption and usable fallback link
```

### Media bridge

Use the shared materializer with a source-bound reference backed by an authorized
connection and stable provider object identity. Resolve the file through that
connection, fetch its current bytes, verify type/size, and materialize them in the existing attachment
store. Use public-address validation, DNS pinning, bounded downloads and a
controlled redirect policy; never forward the Notion bearer token to a file
host. Do not expose signed URLs in logs, memory or long-lived skills.

Enforce byte limits while streaming the download, then validate decoded image
dimensions and format within resource limits. The existing `tool_media` 5 MiB
retention filter is not itself a download bound: the ordinary image-response
path can buffer `response.bytes()` before applying that filter. Reuse the
bounded download patterns in `channel_media_service`, with provider-specific
host policy, instead of relying on generic proxy response handling.

Knowledge assets need a collection-owned lifecycle independent of chat history.
Reuse verified asset bytes across authorized requests, but materialize a fresh
conversation/turn-owned attachment for each new request. Reuse that attachment
on retries of the same request. Current channel deduplication is keyed by
owner + attachment ID + audience for the attachment lifetime; sharing one
attachment ID across later menu requests would suppress legitimate sends.

Distinguish delivery-only media from images that the model needs to inspect.
Current tool-image handling can start another model round to view each new
batch. A request to send the configured menu should attach it without a viewing
round; a request to read or compare its prices still needs evidence extraction
or image input. This requires an explicit output intent in the new capability,
with corresponding runtime support, not a prompt telling the model to skip a
continuation. Preserve current screenshot/analysis behavior.

Keep source lookup and media materialization in services; handlers adapt the
verified result to the existing attachment and destination-delivery contracts.
Cover owner, specialist and permitted guest execution, Stop, retention and
outbound channel capability checks. Reuse the current audience-bound delivery
claims, so an image already sent is not sent again.

Original PDF menus require a separate tool-produced file output contract;
the present tool-image path is for verified raster images. Retained raster
previews can support a limited image pilot, but do not fulfill a request for the
original PDF. Reuse existing storage/rendering/adapter pieces without treating
provider output as an owner upload. A successful text
answer should survive a source download or channel-media failure, with an
accurate fallback instead of a claim that an image was sent.

### Source updates and access

Ingest the initial authorized source tree in background work. Use signed Notion
webhooks to enqueue deduplicated incremental refreshes, fetching current
content from stable IDs. Handle edits, moves, deletions and access loss; event
retries must be idempotent. Add a periodic reconciliation for missed events.
Webhook notifications can be delayed or aggregated and are not an immediate
freshness guarantee. Track source revision, last checked/indexed time and errors.

Partition and filter retrieval by owner/organization, agent knowledge binding
and current caller audience. Permission to index a page with an integration
token does not automatically authorize every NyxBot user or guest to read it.
Recheck live authority on retrieval/materialization; invalidate revoked sources
and cached answers. Do not use a globally shared answer or skill-package cache
as an authorization shortcut. Retain source content and assets under explicit
encryption, deletion and retention policies.

Local connection, membership, grant and publication state are checked live.
Upstream Notion access loss or deletion is known only after a provider
observation; webhook/periodic refresh cannot guarantee immediate detection.
Configure a maximum age for a successful source-access check and deny or
revalidate when it is exceeded. Stop serving a source as soon as access loss
or deletion is observed. If an owner chooses published snapshot semantics,
record that explicitly. Define whether unpublication also disables old NyxID
transcript downloads; copies already delivered to an external chat cannot be
recalled by deleting the stored asset.

Publish complete immutable document/resource generations after their required
text and original assets are ready. Fence stale workers so an older refresh
cannot overwrite a newer version or mix new menu text with old images. Filter
access before excerpts, titles, citations or assets reach the model; answer
filtering alone cannot prevent disclosure.

## Conversational setup across many accounts

The intended entry point is an authenticated owner asking NyxBot to use a
connected knowledge source, for example: "Use our Notion workspace to answer
customer questions and send our menu." One maintained setup skill can guide
this conversation across businesses. Tenant-specific source IDs, resource
bindings, audiences and progress belong in durable configuration, rather than
a newly generated skill for each business.

The skill calls supported setup operations; backend services provision and
operate the knowledge integration. A skill alone does not add a connector,
index, webhook consumer, file importer or background execution authority.

Owner setup and customer answering are separate capabilities. Current NyxBot
guest turns have no tools; customer retrieval must run through a specialist
with an explicitly published collection and the required guest-readable
capability. Do not grant guests the owner's broad Notion connection. Enforce
the publication at retrieval and asset materialization, independently of what
the model or source page says. A collection being successfully indexed is not
permission to publish it.

### Owner experience

1. Reuse an authorized source connection or send the existing hosted connect
   link. The owner completes provider consent; the existing watch resumes the
   thread without asking them to return and say "done".
2. Select a source root and target agent. Ask only for unresolved choices,
   especially which material is approved for customer answers. Do not infer
   that all pages available to the connection are public. Exclude future new
   descendant pages unless the owner explicitly includes them in the scope.
3. Produce a bounded preview of the source scope, audience, likely resources
   and ingestion size/cost. Apply the governing permission mode. Customer
   publication requires an explicit owner-selected audience and source subset;
   imported page instructions cannot make that choice.
4. Submit background ingestion and end the turn. Backend workers fetch,
   parse, index and retain assets, reporting durable progress. Source errors
   and ambiguous resources remain visible and do not silently become ready.
5. Run fixed acceptance probes: retrieve a grounded answer with a citation,
   resolve the menu, and prepare its image/file through the attachment path.
   Check readiness for each capability independently. An image failure must
   not be hidden behind successful text indexing.
6. Bind the ready knowledge collection/resources to the intended agent under
   the owner's explicit publication selection, report the result in the
   originating chat and enable continuing synchronization.
   Creating or linking a customer channel remains an explicit owner choice;
   setup must not post test messages to customers.

An owner can later ask "Use this as the new menu", "What failed to sync?" or
"Stop using these pages" through the same management interface. Source updates
do not require rewriting a skill or keeping an assistant turn running.

### Proposed capability surface

These names describe new contracts to implement; they are not current tools.

| Operation | Contract |
| --- | --- |
| `knowledge_setup_preview` | Resolve the live source connection and scope; return candidate resources, limits and any unresolved owner choices. |
| `knowledge_setup_submit` | Validate the preview/configuration revision, target agent and authority; persist or reuse an idempotent setup job and return its ID. |
| `knowledge_setup_status` / `knowledge_setup_cancel` | Inspect or cancel the exact authorized job; progress survives reconnects and process restarts. |
| `knowledge_search` | Retrieve evidence from collections available to this caller, agent and audience, with citations and freshness. |
| `knowledge_get_resource` | Resolve a configured resource such as the menu; return its revision and authorized assets, materializing turn-owned delivery attachments when requested. Distinguish delivery-only from analysis intent. |

Use the existing hosted connection and async-result patterns where their
contracts apply. Initial indexing can exceed the current async service watch's
two-hour lifetime, so long imports need their own durable job lifecycle and
completion notification; they must not rely on extending a live conversation
turn or treating a watch expiry as job completion. A native ingestion worker
needs explicit, bounded background source authority tied to the live connection
and collection; it cannot reuse an idle conversation key to bypass its live-turn
gate. Revocation, cancellation and deletion fence further work.

Define cancellation separately from continuing sync. Stop fences an active
lookup/materialization and any initial setup job bound to that cancellation
intent. An already activated recurring source is durable owner configuration;
disabling/deleting it stops future sync. Persist these distinctions in job and
source state so an unrelated later chat Stop does not silently remove a source.

### Scaling properties

- Operate a shared retrieval service with enforced tenant partitions. Add
  per-customer deployments only when isolation requirements call for them.
- Index each authorized source revision once per configured collection and
  reuse it across that collection's permitted agent conversations. Deduplicate
  incremental parsing/embedding work; do not crawl Notion for every question.
- Queue ingestion with bounded global, tenant and provider/workspace
  concurrency, retry/backoff and quotas. This protects interactive lookups from
  a large new import and respects source API limits.
- Persist setup configuration, revisions and job state. Repeated chat requests
  must resolve existing work rather than create duplicate collections, webhook
  subscriptions or billable imports.
- Maintain synchronization without model turns. Use the model for setup
  choices and answers; deterministic workers handle change detection, indexing,
  asset refresh and reconciliation.
- Publish only the intended collection subset to a customer-facing specialist.
  NyxBot's owner account authority must not become a public retrieval credential.
- Track readiness, retrieval latency, update lag, failures and cost by tenant
  and connector. Capacity and latency claims require load tests on the intended
  deployment; choosing a RAG product does not establish them.

The first implementation should prove the shared media contract with a Notion
adapter and the existing binary-image path. Add other source adapters behind
the same resource/attachment contracts. The separate synchronization release
can start with one source type, one setup skill and one retrieval backend.

## Build order and release gates

Astra's independent review supports the split above and recommends proving the
deterministic path before selecting a RAG platform. Keep the rollout incremental:

| Step | Deliverable | Evidence needed before proceeding |
| --- | --- | --- |
| 0: baseline and role | Concise shared completion rules, exact skill IDs, and metadata-only timing of discovery, source reads, model rounds and delivery. | Repeat the same menu and factual questions with cold/warm p50/p95 and tool-call counts; verify the existing role and permission-mode checks. Only the instruction/skill-ID portion is implemented here. |
| 1: general images end to end | Shared resource descriptor/materializer contract, Notion adapter, existing binary-image integration, fresh retry-idempotent turn attachment, and delivery-only output support. Saved menu bindings are optional. | An owner asks for an image from a supplied source and sees it without knowledge setup; the same runtime handles binary output and a Notion file reference. Repeat requests, retries, Stop/revocation and PDF fallback behave correctly. |
| 2: source setup and updates | Durable source/collection configuration, minimal normalized text/keyword retrieval, setup preview/job/status/cancel, resource validation and webhook/reconciliation workers. | An owner completes setup in chat, the job resumes after restart, a source edit becomes visible, and deletion/access loss prevents further use. |
| 3: customer publication | Owner-selected collection subset bound to a specialist; independently enforced publication revision and audience. | A guest retrieves approved answers/assets but cannot read unpublished pages, run setup or use the owner's Notion connection; channel delivery and revocation work end to end. |
| 4: broader retrieval | Compare existing REST/keyword baseline, hosted Notion MCP for owner questions, and one hybrid retrieval candidate if the corpus needs it. | Pick the least costly option that passes the fixed accuracy, permission, freshness, media and latency tests. Add vectors/reranking only when they improve measured outcomes. |

Steps 2 and 3 make conversational onboarding usable across tenants. A full
semantic retrieval deployment is not a prerequisite for a restaurant's menu,
hours and a small FAQ. Hosted Notion MCP is an owner-retrieval comparison;
its OAuth authority must not be exposed directly as a customer capability.

Use a bounded verified-skill-package cache only as a measured follow-up if skill
downloads are significant. Key it by acting person, exact skill/version/hash
and bounded expiry/size, rechecking current visibility and live agent pins on
every read. Caching artifact bytes must not cache authorization or allow one
person's private package to be disclosed to another.

## Platform choices

Verified against the linked official documentation on 2026-10-16. No candidate
was benchmarked against the KEK workspace.

| Option | Fit | Freshness and limitations |
| --- | --- | --- |
| [Notion hosted MCP](https://developers.notion.com/guides/mcp/mcp-supported-tools) | First comparison for a Notion-only workspace: richer search/fetch operations with little custom indexing work. | Hosted `notion-search` uses AI search when the workspace and connection permit it; otherwise it uses workspace keyword search. Requires its OAuth/PKCE connection, not a switch of the existing REST operation. Benchmark latency and attachment handling separately. |
| [RAGFlow](https://ragflow.io/docs/add_data_source/data_source_configuration) | Candidate for a dedicated retrieval service where NyxBot remains the assistant. Notion is a documented source. | [Refresh intervals and deletion synchronization](https://ragflow.io/docs/add_data_source/add_to_knowledge_base_and_sync) are documented; test the chosen released version because these links describe the current development documentation. Validate PDF/image extraction and source permissions in the pilot. |
| [Onyx](https://docs.onyx.app/admins/connectors/official/notion) | Candidate for broader internal knowledge search and administration. | The Notion connector defaults to a 30-minute refresh; indexing adds further delay. Notion is not among the documented automatic permission-sync connectors, so configure explicit access. Query/API fit with NyxBot needs validation. |
| [Dify](https://docs.dify.ai/en/cloud/use-dify/knowledge/create-knowledge/import-text-data/sync-from-notion) | Useful when a visual workflow and knowledge editor are primary requirements. | The Notion guide describes synchronization but explicitly documents a manual Sync button and excludes images/files from import. Verify automatic refresh behavior in the deployed edition/version; it does not supply the menu-image bridge. |

For this example, start with the general resource/media capability and its Notion
adapter. Use the existing REST connection for that first release. Compare hosted Notion
MCP for owner-only open questions with a small normalized keyword baseline;
pilot a hybrid index when the corpus or query evaluation justifies it. RAGFlow
is a reasonable first dedicated-backend candidate, while Onyx is a stronger
candidate when internal enterprise search/administration drives the choice.
There is no established vendor winner without testing the actual corpus,
permission model, OCR/files, synchronization and query/API latency.
Keep the existing Notion workspace while doing that comparison; changing the
editing platform is not necessary to reduce per-question tool calls.

## Acceptance and measurement

Use the same fixed questions, source revisions and account scope for each route.
Measure cold and warm runs separately. Record metadata-only durations for model
planning, discovery, skill download, source reads, retrieval, first answer and
attachment delivery. Report p50/p95, tool-call counts, factual/source accuracy,
image-delivery success and source-change-to-answer lag separately.

The screenshot alone cannot establish latency baselines. Proposed pilot goals
are at most two agent-visible calls for a known menu, no repeated tool search
or unrelated skill reads, retrieval p95 below one second on the intended corpus,
and a first useful reply within five seconds for a warm known-resource request.
These are targets to validate, not measured performance or delivery guarantees.

Exercise a menu update, deletion, revoked access, expiring file URL, nested
Notion content, an oversized image, a PDF menu, unsupported channel media and
an interrupted turn. Include two legitimate requests for the same asset to the
same audience, retries of one request, an ambiguous send receipt, and both
delivery-only and image-analysis requests. Test cross-tenant and unpublished
source exclusion independently of answer quality. Customer-facing replies should contain the menu or a
clear fallback, never invented prices or false delivery claims.

Additional references:

- [Notion REST search is title search](https://developers.notion.com/reference/post-search).
- [Notion file objects and temporary URLs](https://developers.notion.com/reference/file-object).
- [Notion webhook semantics and signature validation](https://developers.notion.com/reference/webhooks).
- [Notion hosted MCP client authorization](https://developers.notion.com/guides/mcp/build-mcp-client).
- [Onyx connector permission-sync coverage](https://docs.onyx.app/admins/connectors/overview).
