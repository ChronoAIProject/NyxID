# CRM records and saved views

Status: in development behind the default-off `assistant:crm` feature flag. Nothing here changes existing routes, collections or behavior. The design source is the [CRM and dynamic artifacts proposal](https://gist.github.com/ctkm-aelf/f0928ec823b8cb408df38bb24c4bcd34).

The CRM has three separate contracts. Object metadata defines meaning and valid writes. Queries define authorized retrieval and aggregation. Saved views define presentation. A stage move changes a record. A dashboard edit changes a view. Neither regenerates the other.

## Ownership and access

Every document carries `owner_id`, the polymorphic owner (a person or an org user), like `UserService.user_id`. Requests may pass `owner_id`; absent means the acting person. Access uses `org_service::resolve_owner_access`. Reads need `can_read()`, writes need `can_write()`. An inaccessible owner is not-found-shaped.

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

Seeded objects are created idempotently (upsert on the unique key) the first time an owner reads `/objects`:

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
  values: { field key: stored value },   // record fields are NOT stored here
  overrides: [field key],        // human-protected fields (see below)
  created_by: CrmActor, updated_by: CrmActor,
  archived_at: datetime | null, created_at, updated_at
}
CrmActor { kind: "human" | "agent", user_id, agent_id?, api_key_id? }   // server-bound, never from payload
```

Stored values: text and select are strings, number is a double, currency is an i64 amount in minor units (`125000` USD is 1,250.00), boolean, date is a `YYYY-MM-DD` string, datetime is a BSON date, user is a person id string. API values use the same shapes, with datetime as RFC3339 and currency as `{ "amount_minor": 125000, "currency": "USD" }` on read and either that or a bare integer on write.

Validation is one server function used by every write path. Unknown field keys are an error, never dropped. Archived fields reject new values. Omitted fields are preserved on patch. `null` clears a value unless the field is required. Validation errors list every failing field path.

### Relationships

`crm_links`: `{ _id, owner_id, field_key, source_object, source_record_id, target_object, target_record_id, single: bool, created_at }`.

Record fields are written through the record write body: a single id string for `one`, an array of ids for `many`, `null` to clear. The service checks that every target exists, is not archived, has the declared object type and the same owner. Failures return one generic validation message that does not reveal whether a hidden record exists. A partial unique index on `{source_record_id, field_key}` where `single: true` enforces one current target. A unique index on `{source_record_id, field_key, target_record_id}` prevents duplicates. Index `{target_record_id, field_key}` serves reverse navigation. Link changes commit in the same transaction as the record revision.

Reads return `links: { field key: [{ id, object_key, label }] }` where `label` is the target's first required text field. Archiving a record keeps its links but excludes archived targets from labels, counts and pickers.

### Writes, conflicts and replay

Every create and patch takes an optional `Idempotency-Key` header (1 to 128 visible ASCII characters). The receipt lives in `crm_mutation_receipts` with `_id = sha256(owner_id + "\0" + key)`, the request digest, and the committed response. It is written in the same transaction as the effect and expires after 24 hours. Replay with the same digest returns the stored response before revision checks run. Reuse with a different digest fails with `CrmIdempotencyMismatch`.

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
  sort?: { field, direction: "asc" | "desc" },   // tie-break by _id
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

Operators by kind: text `eq neq in contains is_set is_unset` (contains is case-insensitive, max 100 chars); number, currency, date, datetime `eq neq gt gte lt lte is_set is_unset`; boolean `eq is_set is_unset`; select and user `eq neq in nin is_set is_unset`; record `eq in is_set is_unset` (target ids). Any other combination is a validation error with the field path.

Table result: `{ object: CrmObject, rows: [{ id, revision, values, links, overrides, archived }], next_cursor, total_count }`. `total_count` counts the whole filtered population, not the page.

Aggregate result: `{ points: [{ key, label, value }], unit: { format: "count" | "number" | "currency", currency? }, total }`. Group keys for absent values use key `null`, label "None". Sums of currency fields keep the field currency. Filters and access apply before grouping, so hidden or archived records never appear in totals.

Board data is a table query filtered by one stage option per lane plus one aggregate `count` grouped by the stage field for authoritative lane totals.

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
- `BarChart`, `PieChart` `{ queryId, title, height?: "small" | "medium" | "large" }` bound to a grouped aggregate.
- `ObjectTable { queryId, columns?: [field key] }` bound to a table query.
- `PipelineBoard { queryId }` bound to a table query on an object with a pipeline.

Components read data only through `queryId`. The renderer executes each named query through the CRM query API and supplies results to the bound component. Props never carry rows, URLs, HTML, scripts or expressions, and components gain no authority from props.

Validation runs on save and on read, server-side, and is the authority. It checks the envelope versions, at most 60 elements, depth at most 8, at most 12 queries, element types and props against the catalog (unknown props rejected), children references (exist, no cycles, reachable from `root`), every query against current object metadata, every `queryId` exists, and the query kind and shape match the component. Errors are a list of `{ path, message }` an agent can repair, e.g. `spec.elements.chart.props.queryId`. Saving an invalid replacement fails and keeps the last valid revision. A stored view that no longer validates (for example a field was archived) still loads with `valid: false` and its errors, so the page shows what is broken instead of a blank screen.

`PUT /views/{id}` requires `expected_revision`.

## Routes

All under `/api/v1/assistant/crm`. List and read routes accept `?owner_id=`. Write bodies accept `owner_id`.

- `GET /objects`, `GET /objects/{key}`.
- `POST /objects/{key}/fields` adds a field. `PATCH /objects/{key}/fields/{field_key}` edits label, required, archived and options (additive, reorder, relabel, archive). Both bump `schema_revision`.
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

`assistant_live` gains identifier-only `crm_record` and `crm_saved_view` events (`{ type, owner_id, id }`). The browser invalidates CRM queries for that owner. Opening a page always refetches, so a missed event costs freshness, not correctness.

## Lifecycle

CRM collections are registered with personal erasure and admin user deletion. Deleting a conversation or agent never deletes CRM data. Deleting a view never deletes records.

## Agent access (next slice, open)

Specialists and NyxBot will operate on the same service methods with a server-bound `CrmActor { kind: "agent" }`. The transport (native `nyxid__crm_*` tools or a NyxID-hosted catalog service) is decided separately and is out of scope for the first slice.
