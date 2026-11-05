# CRM records and saved views

CRM is behind the default-off `cms:poc-v1` feature flag. The design source is the [CRM and dynamic artifacts proposal](https://gist.github.com/ctkm-aelf/f0928ec823b8cb408df38bb24c4bcd34).

The CRM has three separate contracts. Object metadata defines meaning and valid writes. Queries define authorized retrieval and aggregation. Saved views define presentation. A stage move changes a record. A dashboard edit changes a view. Neither regenerates the other.

## Ownership and access

Every document carries `owner_id`, the polymorphic owner (a person or an org user), like `UserService.user_id`. Requests may pass `owner_id`; absent means the acting person. Access uses `org_service::resolve_owner_access` with three levels:

- Read (queries, records, activities, views): `can_read()`.
- Work (record create/patch/move/archive/restore, links, notes, view create/update, deleting a view you created): `Direct`, `AsOrgAdmin`, or `AsOrgMember` whose role `can_proxy()` (Member). Viewers stay read-only.
- Schema (field add/patch, deleting another person's view): `can_write()` (`Direct` or `AsOrgAdmin`).

An inaccessible owner is not-found-shaped. Routes that address a document by id take the owner from the document; a mismatching `?owner_id=` is 404.

All routes are under `/api/v1/assistant/crm`, inside the existing human-only router, and every handler calls `require_first_party_human` and the `cms:poc-v1` flag check (resolved for the acting person). The flag off returns 404. Organization workspaces resolve this flag for the acting person through personal flag resolution, including current organization-membership overrides; the resource owner does not choose the flag. Native assistant access uses the separate conversation-key boundary described below.

## Object metadata

`crm_objects`: one document per `(owner_id, key)`, unique.

```text
CrmObject {
  _id: uuid, owner_id, key, label, plural_label,
  schema_revision: i64,          // +1 on every field change
  fields: [CrmField],
  pipeline: { stage_field, closed_stages: [option value] } | null,
  default_columns: [field key],
  seeded: bool, created_at, updated_at
}
CrmField {
  key,                           // ^[a-z][a-z0-9_]{0,47}$, immutable
  label, required: bool, archived: bool,
  kind: tagged by "type":
    text      { max_length: 1..=10000, default 1000 }
    number    { format: "plain" | "percent" }   // percent stores a fraction, 0.25 = 25%
    currency  { currency: ISO 4217 code with 2 minor digits (USD, EUR, GBP, SGD, AUD, CAD, HKD, CNY) }
    boolean   {}
    date      {}                 // calendar date, "YYYY-MM-DD"
    datetime  {}                 // instant, RFC3339 in the API, BSON date in MongoDB
    select    { options: [{ value, label, color?, archived }] }   // value immutable, ^[a-z0-9_]{1,48}$
    record    { target: object key, cardinality: "one" | "many" }
    user      {}                 // a person id with access to the owner
}
```

Key, type and record target are frozen after creation. Labels, option labels, option order, `required` and `archived` are editable. Archiving a field or option keeps stored values readable and displays them as archived. Adding an option or field is additive. Removing is archiving.

Archiving rules keep every object writable: archiving a field forces `required = false`; the pipeline stage field cannot be archived or made optional; an option cannot be archived when it would leave a required select with no live option. Archived options in `closed_stages` are allowed (records keep the value; moves to it are refused).

Objects contain at most 100 fields, including archived definitions. Select fields contain at most 100 options; pipeline stage fields contain at most 50 options, including archived options. Labels are at most 200 characters. Required text must contain non-whitespace content; required boolean `false` is valid.

Field writes require `expected_schema_revision`. The update filters on `{ _id, schema_revision: expected }`, applies `$push` or a positional `$set` plus `$inc`, and a mismatch returns `CrmRevisionConflict` with the current object.

Seeded objects are inserted with `$setOnInsert` only (never modifying an existing row) the first time an owner's `/objects` is read, by the server regardless of the reader's level:

- `company`: name (text, required), domain (text), industry (select), owner (user).
- `contact`: name (text, required), email (text), company (record → company, one), title (text).
- `opportunity`: name (text, required), company (record → company, one), amount (currency USD), stage (select new, qualified, proposal, won, lost; required), expected_close (date), owner (user). Pipeline `stage`, closed `won` and `lost`.
- `task`: title (text, required), status (select open, done; required), due (date), assignee (user), related (record → opportunity, many). Pipeline `status`, closed `done`.

Custom fields can be added to any object. Schema-authorized people create custom objects through `POST /api/v1/assistant/crm/objects` or **New object** in the schema editor. The body contains immutable `key`, `label`, `plural_label`, 1–100 initial typed `fields`, and an optional `pipeline`. Initial fields use the same definition validator as additive fields, have unique keys, and must be active. Record targets must belong to the same owner (self-references are supported). The pipeline names a required select field, at most 50 stages, and unique live closed-stage values. Seeded keys are reserved. Each owner can contain at most 50 objects, including the four seeded objects; the owner fence enforces the cap inside the creation transaction. Object deletion and archiving are outside this version.

Text editors choose solely from metadata: text with `max_length <= 254` uses a single-line input; longer text uses a textarea regardless of its key. New-owner insert-only seeds use name/title 200, domain 253 and email 254; existing owner metadata is never rewritten, so legacy 1000-character text remains multiline. Custom objects follow the same threshold. Table headers match `DESIGN.md` (Tables) and `frontend/src/components/ui/table.tsx`: the shared `type-table-header` role (text-12/600), uppercase, tertiary text, and 1.5px letter spacing. Relationship pickers retain the chosen record label when their search closes.

## Records

`crm_records`:

```text
CrmRecord {
  _id: uuid, owner_id, object_id, object_key,
  revision: i64,                 // starts at 1, +1 per committed change
  values: { field key: stored value },   // scalar fields only
  refs: { field key: [target record id] }, // mirror of crm_links, written in the same transaction
  overrides: [field key],        // human-protected fields (see below)
  field_sources: { field key: BSON datetime }, // last accepted observation order
  created_by: CrmActor, updated_by: CrmActor,
  archived_at: datetime | null, created_at, updated_at
}
CrmActor { kind: "human" | "agent", user_id, agent_id?, api_key_id?, source?: { conversation_id, turn_id, run_id?, ordered_at } }   // server-bound, never from payload
```

Stored values (canonical BSON types, enforced by the validator): text and select are strings, number is a Double, currency is an Int64 amount in minor units (`125000` USD is 1,250.00), boolean, date is a `YYYY-MM-DD` string that must be a real calendar date, datetime is a BSON date, user is a person id string. API values use the same shapes, with datetime as RFC3339 and currency as `{ "amount_minor": 125000, "currency": "USD" }` on read and either that or a bare integer on write. `amount_minor` must be an integer with absolute value at most 2^53 - 1, and a supplied `currency` must equal the field's currency (error path `values.<field>.currency`).

Validation is one server function used by every write path. Unknown field keys are an error, never dropped. Archived fields reject new values. Omitted fields are preserved on patch. `null` clears a value unless the field is required. Validation errors list every failing field path.

### Relationships

`crm_links`: `{ _id, owner_id, field_key, source_object, source_record_id, target_object, target_record_id, single: bool, created_at }`.

Record fields are written through the record write body: a single id string for `one`, an array of ids for `many`, `null` to clear. The service checks that every target exists, is not archived, has the declared object type and the same owner. Failures return one generic validation message that does not reveal whether a hidden record exists. A partial unique index on `{source_record_id, field_key}` where `single: true` enforces one current target. A unique index on `{source_record_id, field_key, target_record_id}` prevents duplicates. Index `{target_record_id, field_key}` serves reverse navigation. Link changes commit in the same transaction as the record revision.

Reads return `links: { field key: [{ id, object_key, label, archived }] }` where `label` is the target's first required text field. Archiving a record keeps its links; archived targets are marked `archived: true`, labelled "(archived)", and excluded from pickers. `refs` is the query path for record filters and grouping (indexed per seeded record field); `crm_links` remains the authority for uniqueness and reverse navigation. Replacing a `one` link deletes the old link before inserting the new one inside the transaction.

### Writes, conflicts and replay

Ordinary effectful POST, PATCH, PUT and DELETE operations under `/crm` take an optional `Idempotency-Key` header (1 to 128 visible ASCII characters). The receipt lives in `crm_mutation_receipts` with `_id = sha256(owner_id + "\0" + acting person id + "\0" + key)`, a digest of method, route template, path ids, owner, acting person and canonical body, and the committed response. It is written in the same transaction as the effect and expires after 7 days; retrying past that window may execute again. The receipt lookup runs after the access check (a removed member cannot replay) and before revision checks. Same digest returns the stored response. A different digest fails with `CrmIdempotencyMismatch`. When two concurrent requests race, the loser's receipt insert hits the duplicate key: the transaction aborts, the receipt is re-read outside it, and the stored response (or 13402) is returned; the effect never runs twice (pattern: `services/approval_service.rs` receipt handling).

Patches carry `expected_revision`. A mismatch fails with `CrmRevisionConflict` and the current record in the error body's `current` field.

The owner fence serializes record, schema, link, import and view mutations. Metadata and target validation run in the same transaction as the effect, so an archived field/target cannot race through a stale validation snapshot.

Human overrides: a human patch may pass `protect: [field key]` and `unprotect: [field key]`. An agent write to a protected field leaves the value unchanged and reports it under `skipped_protected`. Human writes are never blocked by overrides.

Agent patches carry server-derived observation order. A scheduled run uses its admitted `scheduled_at`; an ordinary turn uses its admitted start time. An older observation cannot replace a newer accepted field result even if it rereads the current record revision; `skipped_stale` identifies refused fields. Accepting the same value at a newer observation time advances its source fence. Existing matches require the revision the agent inspected.

Stage moves are `POST /records/{id}/move` with `{ expected_revision, to }`. They validate the option exists and is not archived, apply the same revision rules, and append a `stage_changed` activity in the same transaction. An ordinary PATCH that changes the pipeline stage uses the same from/to history path, including detail edits, imports and native tools. Each logical patch produces one activity: `stage_changed` with `from`, `to` and every changed field key when the stage changes, otherwise `updated`. A stage-and-amount edit therefore retains both field changes in one history row.

## Activities

`crm_activities`: `{ _id, owner_id, record_id, object_key, kind: "created" | "updated" | "stage_changed" | "note", actor: CrmActor, summary: { changed_fields?, from?, to? }, body?: text (notes only, 1..=10000 chars), created_at }`. These are user-visible business events, separate from the security audit log, which records metadata-only `crm_*` audit events through the existing audit service.

Activities use 50-row cursor pages. The timeline keeps live first-page refresh and offers older-page retry. Actor/source DTO dates are RFC3339, and record details expose field observation times. Notes retain draft and retry identity on failure; pending writes disable editing.

## Queries

One query service serves tables, boards, exports, aggregates, saved views and agents. Saved filters are stored as query inputs and validated by the same service.

```text
CrmTableQuery {
  object, select?: [field key],
  filters: [{ field, op, value? }],          // max 20
  sort?: { field, direction: "asc" | "desc" },   // scalar kinds only (text, number, currency, boolean, date, datetime, select); tie-break by _id
  page_size: 1..=200 (default 50), cursor?: opaque keyset cursor bound to the query shape, direction, last sort value/null state, and _id,
  include_archived: bool (default false)
}
CrmAggregateQuery {
  object, filters,
  group_by?: field key (select, boolean, user, record "one", date, datetime),
  date_bucket?: { unit: "day" | "week" | "month", timezone: IANA timezone },
  measure: { op: "count" } | { op: "sum" | "avg" | "min" | "max", field },  // number or currency
  order: "option_order" | "value_desc" | "label_asc"   // option_order only for select
}
```

Operators by kind: text `eq neq in contains is_set is_unset` (contains is case-insensitive, max 100 chars, regex-escaped); number, currency, date, datetime `eq neq gt gte lt lte is_set is_unset`; boolean `eq is_set is_unset`; select and user `eq neq in nin is_set is_unset`; record `eq in is_set is_unset` (target ids, evaluated on `refs`). `in`/`nin` take at most 100 values. Any other combination is a validation error with the field path.

Table result: `{ object: CrmObject, rows: [{ id, revision, values, links, overrides, archived }], next_cursor, total_count }`. `total_count` counts the whole filtered population, not the page.

Aggregate result: `{ points: [{ key, label, value, archived }], unit: { format: "count" | "number" | "percent" | "currency", currency? }, total, truncated }`. Group keys are `string | boolean | null`; absent values use key `null`, label "None". Aggregate values and `total` are nullable when no measured value exists. A group whose option or target record is archived keeps its key with `archived: true`. `total` and points come from one grouped snapshot and cover the whole filtered population. Past 50 groups the bounded response returns the first 50 by `order`, then one `{ key: "__other__", label: "Other" }` point holding the remainder, and `truncated: true`, so available points sum to `total` for count and sum. Only the final point of a `truncated: true` result is the synthetic remainder; a real group may also have the literal key `__other__` or `__none__`. Renderers use separate typed presentation identities and retain archived markers on every real group. A sum with no measured values remains null for groups, Other and total; zero is returned only when a measured zero exists. Nonfinite results and unsafe monetary totals are rejected explicitly. Currency `sum`, `min`, `max` return checked integer minor units in the field currency; currency `avg` is rounded half-even to minor units. Filters and access apply before grouping, so hidden or archived source records never appear in totals.

Date/datetime grouping requires an explicit bucket and timezone. Calendar dates require `UTC` and retain the written date. Datetimes use the declared IANA timezone; weeks are ISO weeks starting Monday. Missing dates form a missing-date group; absent calendar buckets are omitted. These reports group current record values and do not reconstruct historical pipeline snapshots.

User group labels use the same owner-scoped eligibility as the people API: an active person who owns the workspace or has a current membership. Removed, inactive and unrelated people render as “Unavailable member”.

Board data is a table query filtered by one stage option per lane plus one aggregate `count` grouped by the stage field for authoritative lane totals. Records in an archived stage option appear in a read-only "Archived stages" lane so lane totals add up to the object total. Boards consistently exclude archived records; saved boards with `include_archived:true` are rejected. Stage fields are required and capped at 50 definitions so the 50-group bound cannot turn real lanes into Other. Every lane and record picker exposes bounded pagination. Count loading/failure is explicit; a failed count is never displayed as zero.

Bounds: 20 filters, 200 rows per page, 50 aggregate groups, 10 seconds `max_time` per query.

## Saved views

`crm_saved_views`: `{ _id, owner_id, title, revision, envelope, created_by, deleted_at?, created_at, updated_at }`. Delete is a soft delete: deleted rows are excluded from reads and validation, retained for idempotent replay, and TTL-purged after seven days. `envelope` is the NyxID-owned wrapper:

```json
{
  "formatVersion": 1,
  "renderer": "json-render",
  "rendererSpecVersion": "nyxid-crm-v1",
  "queries": {
    "openDeals": { "kind": "table", "query": { "object": "opportunity", "filters": [], "page_size": 50 } },
    "pipeline": { "kind": "aggregate", "query": { "object": "opportunity", "filters": [], "group_by": "stage", "measure": { "op": "sum", "field": "amount" }, "order": "option_order" } }
  },
  "spec": {
    "root": "main",
    "elements": {
      "main": { "type": "Stack", "props": { "gap": "md" }, "children": ["title", "chart", "deals"] },
      "title": { "type": "Text", "props": { "content": "Sales workspace", "variant": "title" }, "children": [] },
      "chart": { "type": "BarChart", "props": { "queryId": "pipeline", "title": "Open pipeline by stage" }, "children": [] },
      "deals": { "type": "ObjectTable", "props": { "queryId": "openDeals", "columns": ["name", "company", "amount", "stage"] }, "children": [] }
    }
  }
}
```

The component catalog is closed for `nyxid-crm-v1`:

- `Stack { gap: "sm" | "md" | "lg", direction?: "vertical" | "horizontal" }` with children.
- `Grid { columns: 1..=4 }` with children.
- `Text { content: 1..=2000 chars, variant: "title" | "heading" | "body" | "muted" }`.
- `Metric { queryId, title }` bound to an aggregate query without `group_by`.
- `RecordForm { object, fields?: [field key], title, submitLabel? }` creates a record of `object` through `POST /records` under the viewer's own authority (Work level). It renders the same field editors as the record sheet. `fields` must be live field keys; required fields are always included.
- `BarChart`, `PieChart` `{ queryId, title, height?: "small" | "medium" | "large" }` bound to a grouped aggregate.
- `ObjectTable { queryId, columns?: [field key] }` bound to a table query.
- `PipelineBoard { queryId }` bound to a table query on an object with a pipeline.

Components read data only through `queryId`. The renderer executes each named query through the CRM query API and supplies results to the bound component. Props never carry rows, URLs, HTML, scripts or expressions, and components gain no authority from props.

Element ids and query ids match `^[A-Za-z][A-Za-z0-9_-]{0,63}$`. `ObjectTable.columns` and `RecordForm.fields` must be live field keys of the bound object. Explicit table columns must be included in a narrow query projection. Pipeline queries include the label and stage fields they display. Pie charts require count/sum; negative totals or slices produce an unsupported-data state. Monetary display uses exact decimal/BigInt conversion, including safe-integer boundary cents; chart geometry alone may use approximate scaled numbers.

Envelope bytes are capped at 64 KiB, individual props at 8 KiB. Oversized structure is rejected before metadata I/O; validation loads each referenced object once. View lists use 20-row UUID keyset pages and share metadata across each page. Create/update validate behind the owner fence.

Validation runs on save and on read, server-side, and is the authority. It checks the envelope versions, at most 60 elements and 60 child edges, depth at most 8, at most 12 queries, element types and props against the catalog (unknown props rejected), and requires the element graph to be a tree: only `Stack`/`Grid` have children, child ids are unique, every non-root element has exactly one parent, and every element is reachable from `root`. It also checks every query against current object metadata, every `queryId` exists, and the query kind and shape match the component. Errors are a list of `{ path, message }` an agent can repair, e.g. `spec.elements.chart.props.queryId`. Saving an invalid replacement fails and keeps the last valid revision. A stored view that no longer validates (for example a field was archived) still loads with `valid: false` and its errors, so the page shows what is broken instead of a blank screen.

`PUT /views/{id}` requires `expected_revision`. Stable links retain owner/view/record selection through reload and Back/Forward using TanStack Router. Opening a table/board record in a saved workspace opens the shared detail sheet. Record and view editors keep identity-based keys and a dirty revision baseline; live refetch cannot reset a draft. Authorization/not-found refusals mask and evict the affected cached data, while transient failures retain drafts with a refresh warning.

## Routes

All under `/api/v1/assistant/crm`. List and read routes accept `?owner_id=`. Write bodies accept `owner_id`.

- `GET /objects`, `GET /objects/{key}`. Both responses include `access: "read" | "work" | "schema"` for the acting person so the UI hides controls it cannot use; the server still enforces every level.
- `POST /objects/{key}/fields` adds a field. `PATCH /objects/{key}/fields/{field_key}` edits label, required, archived and options (additive, reorder, relabel, archive). Both take `expected_schema_revision` and bump `schema_revision`.
- `POST /query` (table), `POST /aggregate`.
- `GET /people?cursor=`, `GET /people/{id}`, `POST /people/lookup` `{ ids }` (at most 200 IDs). Visible user labels are batched; per-cell lookups are avoided.
- `POST /import/preview`, `POST /import/commit`, `POST /export` (shared table query, up to 100 rows per download).
- `POST /views/generate` (required Idempotency-Key; separate paid-generation namespace).
- `POST /records` `{ object, values, protect? }`. `GET /records/{id}`. `PATCH /records/{id}` `{ expected_revision, values, protect?, unprotect? }`. `POST /records/{id}/archive` and `/restore` `{ expected_revision }`. `POST /records/{id}/move`.
- `GET /records/{id}/activities?cursor=`, `POST /records/{id}/notes` `{ body }`.
- `GET /records/{id}/related` returns reverse links grouped by `(object_key, field_key)`, access-filtered, max 50 per group with counts.
- `GET /views?cursor=` returns `{ views: [...], next_cursor }`; `POST /views`, `GET /views/{id}` (envelope plus `valid` and `errors`), `PUT /views/{id}`, `DELETE /views/{id}`, `POST /views/validate`.

## Error codes

Reserved block 13400-13409:

- 13400 `CrmRevisionConflict` (HTTP 409, `crm_revision_conflict`), body includes `current`.
- 13401 `CrmValidationFailed` (HTTP 422, `crm_validation_failed`), body includes `errors: [{ path, message }]`.
- 13402 `CrmIdempotencyMismatch` (HTTP 409, `crm_idempotency_mismatch`).
- 13403 `CrmViewInvalid` (HTTP 422, `crm_view_invalid`), body includes `errors`.
- 13404 `CrmGenerationUnavailable` (HTTP 503, `crm_generation_unavailable`).

`current` and `errors` are top-level recovery fields in REST and native tool errors. Native conflict links are redacted against live object grants before serialization.

## Live updates

`assistant_live` watches records, object metadata, activities and saved views and emits `{ type: "crm", owner_id }`. Its Mongo projection carries identifiers only, never values, note bodies, envelopes or source payloads. Changes coalesce by owner through a dedicated bounded queue and 250 ms worker, separate from organization-group delivery. Personal owners receive direct delivery after a person lookup; org delivery considers subscribed users in bounded membership batches. The SSE boundary rechecks live owner access and the acting person's CRM flag before delivery.

The client invalidates CRM queries for that owner. Stream ready/resync also invalidates CRM caches after reconnect. Visible pages retain the 30-second polling fallback. Unrelated-owner events do not refresh the workspace; model inference is never part of reads or refresh.

## Lifecycle

Person-owned CRM rows are registered in the personal erasure inventory (which admin user deletion uses). Org-owned rows are purged in `org_service::delete_org_user`. `user` field values and `created_by.user_id` that name an erased person remain as dangling ids and render as “Unavailable member”. Generation claims/results have a seven-day TTL and belong to both personal and org erasure inventories. Deleting a conversation or agent never deletes CRM data. Deleting a view never deletes records.

## Import and export

Preview/commit accept `{ owner_id?, object, mapping: { source_column: field_key }, rows, match_field, policy, snapshot? }`. Bounds are 1–100 rows, at most 100 mapped columns and 512,000 encoded bytes. The UI accepts RFC 4180-style CSV, a JSON array of row objects, or the downloaded `{ records: [...] }` export envelope. JSON mapping uses the union of columns across the bounded batch. Missing JSON keys preserve existing values on updates; explicit null or blank clears an optional field. Creates still require all required fields.

Matching uses an active text identity field, trims surrounding whitespace and ignores case for an exact match. It never guesses names or relationships. Ambiguous, archived and duplicate batch identities fail. Policies are `create_only`, `update_or_create` and `skip_existing`. Preview returns inspectable per-row action, exact matched ID and revision, plus a snapshot binding schema revision and canonical input digest. Commit recomputes that snapshot under the owner fence and refuses schema, identity, revision or create/update/skip drift. The whole commit is atomic and its receipt is transactional. A fresh preview is necessary after a human edit. Native upsert requires an explicit inspected revision when a match already exists.

Exports use the displayed table filters (or the displayed board query) and typed raw values, with schema, total_count and next_cursor. Each download contains at most 100 records and can be imported directly. Currency decimal strings are parsed exactly to integer minor units, accept one optional minus sign and reject excess precision or unsafe amounts. Percentage numbers are fractions.

## AI view generation

`POST /views/generate { owner_id?, object, prompt, draft? }` requires Work access and Idempotency-Key. The server sends object metadata, the closed catalog and operator matrix to the bounded one-shot inference path, without record data. It validates the response and permits one repair round. The result is `{ envelope, valid, errors }`; saving is an explicit separate action. Generated catalog instructions include current-record dated reports and explicit timezone semantics.

The route is the admin-configured CRM platform route, otherwise the configured platform utility route. An absent/unusable selected service or model fails closed; CRM never substitutes another model, scans unrelated services, or falls back to personal BYOK. Platform usage bills the acting person. Admin-only `GET/PUT /api/v1/admin/settings/crm-inference` reuses utility route validation and accepts a complete `{ service_slug, model }` pair or null to clear.

Paid generation uses a deliberately separate retry namespace in `crm_generation_jobs`, independent of ordinary `crm_mutation_receipts`. The same owner/actor/key binds the canonical generation request for seven days. A claim commits before network work; no transaction spans inference. Concurrent identical requests dispatch at most once. Completed results replay before new-attempt metadata/size preflight, while current owner/work authority is still checked. Predispatch failures become retryable under the same identity. Running or potentially dispatched/uncertain requests never blindly dispatch again. The UI offers unchanged-request retry and an explicit new attempt that may incur another charge; editing object/prompt/JSON starts a clearly labelled new attempt. Opening/filtering/refreshing a view never calls a model.

## Native assistant access

NyxBot and specialists use native `nyxid__crm_*` tools through direct and universal MCP dispatch, sharing REST services. Tool discovery publishes operation-specific closed schemas and bounds. Reads need no mutation identity. Each write requires an explicit 1–128 visible-ASCII `mutation_key`; reuse it with unchanged arguments after response loss, even when the JSON-RPC correlation ID changes. Distinct logical writes use distinct keys, including different scheduled runs and conflict rebases. Receipt identity binds owner, acting person, conversation and this logical key; JSON-RPC IDs confer no retry identity. Confirmation IDs are excluded from the committed payload digest. An exact committed retry returns its stored result after current authority checks, without consuming the single-use confirmation again or repeating an effect. Native deletion can authorize the owner-scoped tombstone for this replay; a new delete key on a deleted view still fails.

Specialist grants are typed entries `{ owner_id, object, capabilities: ["read", "write", "schema"] }`, at most 16 unique objects. Write/schema require Read; the owner must match the specialist's owner. Humans configure these grants in agent details. Read grants govern schema/query/record/history/views, Write governs record effects and saving/deleting views over their referenced objects, and Schema governs field changes and object creation. A Schema grant with Read may name an unreserved, valid object key before that object exists; the grant authorizes creation of that exact key, with no wildcard access. For example, Read/Write/Schema on `research_list` permits creation and subsequent reads/writes of `research_list`, and confers nothing on `different_list`. Read/Write without Schema on a missing key is refused. The grant editor exposes this as **Grant new object**. Related-object fields in the initial schema also require Read on each target. Related-object targets need Read for writes, filters and grouping; view forms also require Write. A specialist text-only view is refused because it references no explicitly granted object. Deleting another person's view additionally requires the human-equivalent schema role. NyxBot uses the acting person's live owner authority. Guests, shared chats and collaboration contexts are refused.

The additive `crm` field on the specialist grants request is optional. Omitting it preserves the CRM grants read inside the update transaction; an explicit empty array revokes them. Nonempty CRM grant authoring must remain disabled until every API replica and grant writer has been upgraded and old writers have been drained, because older binaries replace the whole grants document without retaining unknown fields. Once a nonempty CRM grant has been written, rollback to those binaries is unsupported until the grant has been explicitly removed.

Actors, owner/role, exact key/session, live turn, confirmation and run provenance are server-derived. Operation grants and reference targets are evaluated before and after confirmation. Mutation admission is scoped only to writes; reader initialization remains available to org viewers. Transactions fence agent grants, owner membership, person/key/credential and exact conversation/turn/session, including server-time expiry/stop checks, and fence again before receipt insertion. Confirmation cards identify the bounded operation, owner/object/record/view, changed field keys or schema properties/options and a stable CRM link. Security audits retain metadata only, without field/note payloads. Business activity remains separate.

Scheduled specialists reuse existing schedule/trigger turn execution. Their field observation order is the admitted run time, preventing an older run from overwriting fresher enrichment merely by rereading the revision.

## Conversation artifacts

`crm_save_view` and `crm_update_view` produce a Created View artifact. An upsert that actually creates a record produces a Created Record artifact; ordinary record updates do not. `crm_create_object` is Sensitive and produces a Created Object artifact under the same live Schema authority as the human route. `crm_present` is Read with the closed arguments `{ kind: "view" | "record", id, owner_id? }`: it checks live owner access and the same object/reference grants as the corresponding view or record read, returns a bounded metadata-and-scalar summary to the model, and produces a Presented artifact. Save a view for a reusable dataset presentation; present an existing view or record to show it again.

Successful effects return success even if optional artifact recording fails after commit; the warning contains only conversation/turn metadata. Upsert responses carry an explicit `created` boolean stored in the same transaction receipt, so matching a revision-one record never invents a creation artifact. `crm_present` returns bounded summaries: records include their metadata-selected title, object label, revision and at most five scalar API values (strings bounded to 200 characters); views include title, referenced object keys and component types without envelopes.

`TurnCrmArtifact` is a serde-tagged enum with flat wire fields `kind`, `id`, `owner_id`, a sanitized title of at most 200 characters, and `role` (`created` or `presented`). Record/Object variants require `object_key`; View omits it (old nulls remain readable). The frontend mirrors the discriminated union. Nothing has shipped, and no wire migration is needed. There are no record values, query results, or view envelopes in chat history. `ActiveTurn.crm_artifacts` and `AssistantMessage.crm_artifacts` default empty for old rows. One atomic MongoDB update pipeline retains at most eight distinct `(kind, id)` references without a client retry loop; a repeated identity updates its metadata and keeps `created` sticky over `presented`. New identities evict the oldest inserted reference when full. Settlement preserves the references for successful, failed, stopped, and reclaimed turns alongside attachments. Owner Retry inherits the bounded references before completed-tool digest checks, so a refused duplicate cannot remove an already-created artifact.

CRM references are independent of generated Sandbox files (`generated_artifacts`); a reply can display both, preserving the generated-file block order and layout.

History, settled-turn SSE, and ordinary web-group transcripts carry additive artifact references. The CMS flag is evaluated for the viewing person. With it off, CRM fields are omitted from responses, the renderer and grant editor are not loaded, no CRM requests or live events are issued, and routes return 404. Guests, shared executions, and collaboration executions have no CRM tools and cannot produce artifacts. Group viewers open resources with their own session and current owner ACL; a reference confers no access.

Inline cards load live data: compact saved views have bounded scrolling; records show current key fields; objects show the current active record count. Each has an Open link. Loading, empty, connection errors, forbidden, and deleted states are explicit; denied or removed resources show “This artifact is no longer available” without cached data. Chat cards refetch on mount, normal window focus and CRM live invalidation, with no interval polling, including their nested queries. A 401 follows the normal authentication reset and shows session expiry rather than deletion. The renderer loads through a separate dynamic import; a flag-off chat does not import CRM/json-render UI.

Enabled users see a real **Artifacts** sidebar entry opening a grouped catalog of **Custom datasets** and **Saved views**. `GET /artifacts/count` returns `{ count, views, objects }` for live saved views and custom (non-seeded) objects in the person's workspace. The sidebar count uses CRM live invalidation without polling. Both groups reopen the existing CRM surfaces with owner and object/view selection. Records reopen from conversation cards. This catalog reuses existing objects and views, with no independent retention inventory. Flag-off users retain the existing workspace-preview placeholder behavior.

Telegram, Lark, and other channel replies append ordinary encoded NyxID links through `AssistantPage::CrmView`, `CrmRecord`, or `CrmObject`. A link already present in the reply is not appended again. No values or view snapshots are sent through this artifact mechanism. Current flag resolution also gates delivery of stored references.

Channel links are appended once through the shared `AssistantPage` URL builder. Streaming/native provider responses and late relay replies use the late-delivery renderer; managed personal delivery uses its saved-message renderer; asynchronous chat updates, duplicate recipients and trigger results use the settled hook's already loaded artifact references. Empty reference lists skip feature resolution and add no CRM database reads. Platform adapters receive ordinary text links and do not load CRM. `nyxid__post_to_chat` is an explicit text/image action, with no artifact payload or settled reply; links explicitly supplied in its text pass through unchanged. Personal and authorized org crews publish references into web group transcripts under each viewer's access. Their transcript/handoff path has no external platform destination; an explicit post-to-chat action uses its supplied text, while channel/followed-thread turns use the renderers above. Org-channel public transcripts deliberately keep no CRM references or private tool cards; external replies still use the saved-message channel renderer with the acting person’s flag and current resource ACL on open. Shared, guest and collaboration lanes cannot produce CRM artifacts.

Deploy all API replicas and drain old grant writers before enabling `cms:poc-v1` or authoring CRM grants. Persisted reference fields are additive and defaulted; older readers ignore them. Older grant writers remain unsafe after nonempty grants have been authored, as described above. No old flag alias or production data migration exists.

## Verification

The completion record and exact command/log paths are maintained in [crm-implementation-completion.md](plans/crm-implementation-completion.md). The G2 merge pass uses local formatting, backend type checking (including test code), frontend lint/type checking and Vitest only. Backend runtime, nextest stack behavior, production chunks, browser workflows and the live model loop are not reverified by G2; older Mongo and mocked-browser results are historical evidence only. The external G2 report separates those results and lists the tests awaiting CI execution.
