# CRM records and saved views

Status: in development behind the default-off `assistant:crm` feature flag. Nothing here changes existing routes, collections or behavior. The design source is the [CRM and dynamic artifacts proposal](https://gist.github.com/ctkm-aelf/f0928ec823b8cb408df38bb24c4bcd34).

The CRM has three separate contracts. Object metadata defines meaning and valid writes. Queries define authorized retrieval and aggregation. Saved views define presentation. A stage move changes a record. A dashboard edit changes a view. Neither regenerates the other.

## Ownership and access

Every document carries `owner_id`, the polymorphic owner (a person or an org user), like `UserService.user_id`. Requests may pass `owner_id`; absent means the acting person. Access uses `org_service::resolve_owner_access` with three levels:

- Read (queries, records, activities, views): `can_read()`.
- Work (record create/patch/move/archive/restore, links, notes, view create/update, deleting a view you created): `Direct`, `AsOrgAdmin`, or `AsOrgMember` whose role `can_proxy()` (Member). Viewers stay read-only.
- Schema (field add/patch, deleting another person's view): `can_write()` (`Direct` or `AsOrgAdmin`).

An inaccessible owner is not-found-shaped. Routes that address a document by id take the owner from the document; a mismatching `?owner_id=` is 404.

All routes are under `/api/v1/assistant/crm`, inside the existing human-only router, and every handler calls `require_first_party_human` and the `assistant:crm` flag check (resolved for the acting person). The flag off returns 404. Agent access is a separate slice (see the end of this document).

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

Archiving rules keep every object writable: archiving a field forces `required = false`; the pipeline stage field cannot be archived; an option cannot be archived when it would leave a required select with no live option. Archived options in `closed_stages` are allowed (records keep the value; moves to it are refused).

Field writes require `expected_schema_revision`. The update filters on `{ _id, schema_revision: expected }`, applies `$push` or a positional `$set` plus `$inc`, and a mismatch returns `CrmRevisionConflict` with the current object.

Seeded objects are inserted with `$setOnInsert` only (never modifying an existing row) the first time an owner's `/objects` is read, by the server regardless of the reader's level:

- `company`: name (text, required), domain (text), industry (select), owner (user).
- `contact`: name (text, required), email (text), company (record → company, one), title (text).
- `opportunity`: name (text, required), company (record → company, one), amount (currency USD), stage (select new, qualified, proposal, won, lost; required), expected_close (date), owner (user). Pipeline `stage`, closed `won` and `lost`.
- `task`: title (text, required), status (select open, done; required), due (date), assignee (user), related (record → opportunity, many). Pipeline `status`, closed `done`.

Custom fields can be added to any object. General custom-object authoring is later work.

## Records

`crm_records`:

```text
CrmRecord {
  _id: uuid, owner_id, object_id, object_key,
  revision: i64,                 // starts at 1, +1 per committed change
  values: { field key: stored value },   // scalar fields only
  refs: { field key: [target record id] }, // mirror of crm_links, written in the same transaction
  overrides: [field key],        // human-protected fields (see below)
  created_by: CrmActor, updated_by: CrmActor,
  archived_at: datetime | null, created_at, updated_at
}
CrmActor { kind: "human" | "agent", user_id, agent_id?, api_key_id? }   // server-bound, never from payload
```

Stored values (canonical BSON types, enforced by the validator): text and select are strings, number is a Double, currency is an Int64 amount in minor units (`125000` USD is 1,250.00), boolean, date is a `YYYY-MM-DD` string that must be a real calendar date, datetime is a BSON date, user is a person id string. API values use the same shapes, with datetime as RFC3339 and currency as `{ "amount_minor": 125000, "currency": "USD" }` on read and either that or a bare integer on write. `amount_minor` must be an integer with absolute value at most 2^53 - 1, and a supplied `currency` must equal the field's currency (error path `values.<field>.currency`).

Validation is one server function used by every write path. Unknown field keys are an error, never dropped. Archived fields reject new values. Omitted fields are preserved on patch. `null` clears a value unless the field is required. Validation errors list every failing field path.

### Relationships

`crm_links`: `{ _id, owner_id, field_key, source_object, source_record_id, target_object, target_record_id, single: bool, created_at }`.

Record fields are written through the record write body: a single id string for `one`, an array of ids for `many`, `null` to clear. The service checks that every target exists, is not archived, has the declared object type and the same owner. Failures return one generic validation message that does not reveal whether a hidden record exists. A partial unique index on `{source_record_id, field_key}` where `single: true` enforces one current target. A unique index on `{source_record_id, field_key, target_record_id}` prevents duplicates. Index `{target_record_id, field_key}` serves reverse navigation. Link changes commit in the same transaction as the record revision.

Reads return `links: { field key: [{ id, object_key, label, archived }] }` where `label` is the target's first required text field. Archiving a record keeps its links; archived targets are marked `archived: true`, labelled "(archived)", and excluded from pickers. `refs` is the query path for record filters and grouping (indexed per seeded record field); `crm_links` remains the authority for uniqueness and reverse navigation. Replacing a `one` link deletes the old link before inserting the new one inside the transaction.

### Writes, conflicts and replay

Every POST, PATCH, PUT and DELETE under `/crm` takes an optional `Idempotency-Key` header (1 to 128 visible ASCII characters). The receipt lives in `crm_mutation_receipts` with `_id = sha256(owner_id + "\0" + acting person id + "\0" + key)`, a digest of method, route template, path ids, owner, acting person and canonical body, and the committed response. It is written in the same transaction as the effect and expires after 7 days; retrying past that window may execute again. The receipt lookup runs after the access check (a removed member cannot replay) and before revision checks. Same digest returns the stored response. A different digest fails with `CrmIdempotencyMismatch`. When two concurrent requests race, the loser's receipt insert hits the duplicate key: the transaction aborts, the receipt is re-read outside it, and the stored response (or 13402) is returned; the effect never runs twice (pattern: `services/approval_service.rs` receipt handling).

Patches carry `expected_revision`. A mismatch fails with `CrmRevisionConflict` and the current record in the error body's `current` field.

Human overrides: a human patch may pass `protect: [field key]` and `unprotect: [field key]`. An agent write (later slice) to a protected field leaves the value unchanged and reports it under `skipped_protected`. Human writes are never blocked by overrides.

Stage moves are `POST /records/{id}/move` with `{ expected_revision, to }`. They validate the option exists and is not archived, apply the same revision rules, and append a `stage_changed` activity in the same transaction.

## Activities

`crm_activities`: `{ _id, owner_id, record_id, object_key, kind: "created" | "updated" | "stage_changed" | "note", actor: CrmActor, summary: { changed_fields?, from?, to? }, body?: text (notes only, 1..=10000 chars), created_at }`. These are user-visible business events, separate from the security audit log, which records metadata-only `crm_*` audit events through the existing audit service.

## Queries

One query service serves tables, boards, exports, aggregates, saved views and agents. Saved filters are stored as query inputs and validated by the same service.

```text
CrmTableQuery {
  object, select?: [field key],
  filters: [{ field, op, value? }],          // max 20
  sort?: { field, direction: "asc" | "desc" },   // scalar kinds only (text, number, currency, boolean, date, datetime, select); tie-break by _id
  page_size: 1..=200 (default 50), cursor?: opaque string,
  include_archived: bool (default false)
}
CrmAggregateQuery {
  object, filters,
  group_by?: field key (select, boolean, user, or record "one"),
  measure: { op: "count" } | { op: "sum" | "avg" | "min" | "max", field },  // number or currency
  order: "option_order" | "value_desc" | "label_asc"   // option_order only for select
}
```

Operators by kind: text `eq neq in contains is_set is_unset` (contains is case-insensitive, max 100 chars, regex-escaped); number, currency, date, datetime `eq neq gt gte lt lte is_set is_unset`; boolean `eq is_set is_unset`; select and user `eq neq in nin is_set is_unset`; record `eq in is_set is_unset` (target ids, evaluated on `refs`). `in`/`nin` take at most 100 values. Any other combination is a validation error with the field path.

Table result: `{ object: CrmObject, rows: [{ id, revision, values, links, overrides, archived }], next_cursor, total_count }`. `total_count` counts the whole filtered population, not the page.

Aggregate result: `{ points: [{ key, label, value, archived }], unit: { format: "count" | "number" | "currency", currency? }, total, truncated }`. Group keys for absent values use key `null`, label "None". A group whose option or target record is archived keeps its key with `archived: true`. `total` always covers the whole filtered population. Past 50 groups the response returns the first 50 by `order`, then one `{ key: "__other__", label: "Other" }` point holding the remainder, and `truncated: true`, so points always sum to `total` for count and sum. Currency `sum`, `min`, `max` return integer minor units in the field currency; currency `avg` is rounded half-even to minor units. Filters and access apply before grouping, so hidden or archived source records never appear in totals.

Board data is a table query filtered by one stage option per lane plus one aggregate `count` grouped by the stage field for authoritative lane totals. Records in an archived stage option appear in a read-only "Archived stages" lane so lane totals add up to the object total.

Bounds: 20 filters, 200 rows per page, 50 aggregate groups, 10 seconds `max_time` per query.

## Saved views

`crm_saved_views`: `{ _id, owner_id, title, revision, envelope, created_by, created_at, updated_at }`. `envelope` is the NyxID-owned wrapper:

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

Element ids and query ids match `^[A-Za-z][A-Za-z0-9_-]{0,63}$`. `ObjectTable.columns` and `RecordForm.fields` must be live field keys of the bound object.

Validation runs on save and on read, server-side, and is the authority. It checks the envelope versions, at most 60 elements, depth at most 8, at most 12 queries, element types and props against the catalog (unknown props rejected), children references (exist, no cycles, reachable from `root`), every query against current object metadata, every `queryId` exists, and the query kind and shape match the component. Errors are a list of `{ path, message }` an agent can repair, e.g. `spec.elements.chart.props.queryId`. Saving an invalid replacement fails and keeps the last valid revision. A stored view that no longer validates (for example a field was archived) still loads with `valid: false` and its errors, so the page shows what is broken instead of a blank screen.

`PUT /views/{id}` requires `expected_revision`.

## Routes

All under `/api/v1/assistant/crm`. List and read routes accept `?owner_id=`. Write bodies accept `owner_id`.

- `GET /objects`, `GET /objects/{key}`. Both responses include `access: "read" | "work" | "schema"` for the acting person so the UI hides controls it cannot use; the server still enforces every level.
- `POST /objects/{key}/fields` adds a field. `PATCH /objects/{key}/fields/{field_key}` edits label, required, archived and options (additive, reorder, relabel, archive). Both take `expected_schema_revision` and bump `schema_revision`.
- `POST /query` (table), `POST /aggregate`.
- `POST /records` `{ object, values, protect? }`. `GET /records/{id}`. `PATCH /records/{id}` `{ expected_revision, values, protect?, unprotect? }`. `POST /records/{id}/archive` and `/restore` `{ expected_revision }`. `POST /records/{id}/move`.
- `GET /records/{id}/activities?cursor=`, `POST /records/{id}/notes` `{ body }`.
- `GET /records/{id}/related` returns reverse links grouped by `(object_key, field_key)`, access-filtered, max 50 per group with counts.
- `GET /views`, `POST /views`, `GET /views/{id}` (envelope plus `valid` and `errors`), `PUT /views/{id}`, `DELETE /views/{id}`, `POST /views/validate`.

## Error codes

Reserved block 13400-13409:

- 13400 `CrmRevisionConflict` (HTTP 409, `crm_revision_conflict`), body includes `current`.
- 13401 `CrmValidationFailed` (HTTP 422, `crm_validation_failed`), body includes `errors: [{ path, message }]`.
- 13402 `CrmIdempotencyMismatch` (HTTP 409, `crm_idempotency_mismatch`).
- 13403 `CrmViewInvalid` (HTTP 422, `crm_view_invalid`), body includes `errors`.

## Live updates

`assistant_live` gains identifier-only `crm_record` and `crm_saved_view` events (`{ type, owner_id, id }`), projecting `owner_id` for the two collections. Personal owners receive them directly. Org owners fan out through the existing resolved org path (`publish_resolved`) to active members with read access, coalesced per `(owner_id, collection)` per second. The browser invalidates CRM queries for that owner. Opening a page always refetches and CRM pages poll every 30 seconds while visible, so a missed event costs freshness, not correctness.

## Lifecycle

Person-owned CRM rows are registered in the personal erasure inventory (which admin user deletion uses). Org-owned rows are purged in `org_service::delete_org_user`. `user` field values and `created_by.user_id` that name an erased person remain as dangling ids and render as "Deleted user". Deleting a conversation or agent never deletes CRM data. Deleting a view never deletes records.

## AI view generation (slice 2)

`POST /views/generate { owner_id?, object, prompt, draft? }` (Work level, same flag) asks a dedicated AI service to draft an envelope. The server builds the prompt from the object metadata, the closed `nyxid-crm-v1` catalog and the operator matrix (never record data), calls the bounded one-shot inference path (`services/assistant_oneshot_inference.rs`) with its own caller label, runs the validator, feeds `{ path, message }` errors back for at most one repair round, and returns `{ envelope, valid, errors }` without saving. The human saves through `POST /views`. The model route is an admin-pinned CRM service when configured, else the platform utility route, billed to the acting person as platform usage. Opening, filtering or refreshing a view never calls a model.

## Agent access (slice 3)

Specialists and NyxBot get native `nyxid__crm_*` tools over the same service methods with a server-bound `CrmActor { kind: "agent" }`, a `crm` agent grant, the closed risk classifier (reads free, record writes are changes, archive is destructive, schema changes are sensitive), the owner's access mode for confirmation, and the NyxAgent tool-call id as the idempotency key. Guests are refused. Tool names and arguments mirror the REST routes so a later NyxID-hosted catalog service (in-process managed operations, never an HTTP self-loop) can expose the same operations to external clients.
