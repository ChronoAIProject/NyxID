# Service authorship and history

AI Services records the creator and latest configuration editor of a service instance. Authorized cards and table rows show both summaries; the detail page has a History tab, including platform-managed instances. The Services page's **Deleted service history** entry discovers retained histories after deletion or automatic physical cleanup. UUIDs identify histories across slug changes; creating a new service with an old slug starts a separate history.

Personal owners and current organization admins may read history only within their current service scope. The owner must remain active, and API-key service restrictions apply in addition to owner permissions. Members, viewers, revoked admins, and admins outside the service scope receive no authorship metadata. Ordinary service visibility does not imply history access. The history timeline, archived-history discovery, and management writes retain their human-only route gates. Existing inventory GET routes admit scoped API keys under the current owner/membership ACLs; their optional authorship summaries additionally require the history owner/admin permission. API-key inventory reads never provision or reconcile service rows. Handler-level service/node restrictions remain additional defense in depth. Archived discovery uses the same gates before returning any UUID or display snapshot.

Legacy rows have no invented creator or editor. Their footer says `Creator not recorded` / `Earlier edits not recorded`; the timeline reports when tracking began. History is read-only. Credential values, ciphertext, tokens, private keys, URL values, header values, frame templates, arbitrary free-form configuration and unreviewed OAuth scopes are never journaled.

## Capture and ordering

`services/service_history/mutation.rs` is the write boundary for `user_services`, `user_endpoints`, and `user_api_keys`. It exposes supported writes explicitly, with no dereference to an unwrapped collection. Each write reads actual before and after documents inside its transaction. The journal event, attribution summary and local business mutation commit or abort together. New endpoint/key references also touch a separate backing-record reference fence inside the transaction so concurrent shared-resource edits retry against a snapshot containing the reference. The fence has no business-version or history meaning. Pipeline updates retain aggregation semantics, including `credential_epoch`; history does not replace `state_version` or approval fencing.

A supplied session must be a `service_history::transaction::Transaction`, constructed by the transaction runner after starting the transaction. Plain `ClientSession` values cannot be supplied to journal writes. Only local database effects belong inside this retry boundary. Provider/node delivery and existing routing/approval gates remain with their callers.

Owned bulk writes select stable UUID pages of 128 and commit each page under the same operation group. Caller-owned transactions process bounded batches inside that single transaction. Shared endpoint/key changes fan out to every referencing instance, including disabled instances, by streaming references. There is no new 10,000-document deployment ceiling. MongoDB transaction duration and document-size constraints still apply; failed transactions do not leave partial history.

`service_change_events` is authoritative. `service_history_heads` allocates a transactional, per-instance sequence that survives physical cleanup, orders same-millisecond commits, and is independent of configuration versions. Timeline pagination anchors a group at its first sequence so later commits in the same operation cannot move it between pages. Pages target 1,000 events and at most 50 groups; an individual larger group is returned whole by itself. No logical group is split across pages.

Verified request context keeps the authenticated actor separate from the polymorphic resource owner. Person/API-key/service-account/application/system identities include bounded display-name snapshots, stable IDs and verified app context, without adding email snapshots. Tokio spawned tasks do not inherit request attribution. OAuth/device initiation stores context on `OAuthState`; callbacks restore it only after state/provider/expiry validation. Hosted connect links bind their verified requesting app and link UUID before both first completion and resumed initiation. Assistant receipts bind operation groups across recovery. Native assistant account tools use the verified MCP identity to bind the conversation API key and its owning person before service mutations; separate requests retain separate history groups. Automatic provisioning and placeholder reconciliation explicitly use system scopes and restore the caller's context afterward.

Event UUIDs remain stable across transaction retries within an operation; the group and transactional entity-event ordinal select the same UUID. Cross-request receipt recovery retains the operation group. Already committed, configuration-equal retries produce no extra event. Events describe actual local outcomes, not the success of later remote effects.

## Writer inventory and exclusions

The boundary covers these production writers; reads and index management may use ordinary Mongo collections:

| Writer family | Local changes captured | Attribution / exclusions |
| --- | --- | --- |
| `user_service_service`, `user_endpoint_service`, `user_api_key_service` | Direct creation, endpoint/configuration changes, credential replacement, routing, SSH, Enable/Disable, shared backing changes; startup removal of legacy public org auto-connections | Verified request; real ciphertext replacement is names-only. Saving equal non-secret config is silent. Startup cleanup uses a system actor and removes each service plus its orphan resources in one journaled transaction. |
| `unified_key_service` and `unified_key_service/platform` | Custom/catalog/HTTP/SSH/direct/node provisioning, platform/user rebinding, user lifecycle, tombstone Delete, automatic physical cleanup | Request actor for explicit management; automatic provisioning has a system context. Deleting a backing resource does not imply service Delete. |
| `assistant_action_effects_services` / endpoint helpers and native assistant account tools | Session-aware configuration, credential and routing commits | Existing receipt group for action effects; verified MCP actor for native account tools. Database retries preserve journal identity. |
| `codex_connection_service`, imported-key provisioning | Import, replacement and multi-record provisioning | Existing local transaction; preserves credential epoch and literal provider strings. |
| `user_token_service`, `handlers/user_tokens`, `connect_link_service` | OAuth and device completion, manual provider replacement, OAuth application credentials, hosted completion/resumption | Persisted initiation actor/app/group. Legacy per-provider callback also syncs its unified keys. |
| `user_credentials_service`, `channel_credentials`, `provider_service` | Credential application changes, resets/removal, propagation and dynamic unreferenced endpoint/key cleanup | Actual impacted instances are journaled even if an earlier reference check found none. |
| `catalog_identity_service`, `node_service` | Catalog identity propagation, node transfer/removal routing changes | Human admin context where authenticated; startup/background fallback is an explicit maintenance system actor. |
| `db` migrations, `retired_service_service`, `org_service`, `cleanup_cli` | Service-instance migrations, retirement Disable, and physical removal; cleanup includes disabled references | Retirement uses an explicit system actor and repeated sweeps do not duplicate events. No history cascade deletion. Existing records are not assigned a historical creator. |
| OAuth/GCP lazy/background refresh | Operational token material, scopes and expiry refresh | Explicit `routine_refresh` mode suppresses refresh noise while still detecting unrelated configuration. |
| Proxy `touch_last_used` | Usage timestamps only | A narrow typed helper performs one cheap update of `last_used_at` and `updated_at`; it accepts no arbitrary fields. |

Other operational exclusions are creation/update/usage timestamps, `state_version`, credential epoch alone, credential status/error messages, OAuth attempt nonce, and source migration bookkeeping. Reads and proxy traffic do not move the footer. Effective defaults, including platform binding and nested frame defaults, are normalized before comparison while unknown nested configuration is preserved. Unknown persisted fields create a generic `Service updated` entry without exposing their names or values.

Safe value policy is separate from the label registry. Approved booleans, domain enums, priority, reviewed provider scopes, and header/rule name/count/direction summaries may be stored. Node UUIDs are additionally removed at read time unless the current reader can access the current node and its active owner. Other recognized settings show field names only. When safe previews are equal but the underlying setting changed, the UI says values are omitted.

## Audit publication and retention

The durable relay publishes `service_change_recorded` through the existing HMAC chain every 10 seconds. The audit UUID equals the event UUID. Duplicate IDs succeed only when immutable payloads match and the stored row has valid chain fields and a valid HMAC. A legacy unchained or hash-corrupt row never counts as a successful mirror. A crash after append and before marking publication is safe to retry.

Failed publications persist exponential retry delays so a bad oldest event cannot monopolize each sweep. Logs report total pending count, the age of the actual oldest pending event (including delayed retries), and failures. History batches audit-mirror reads and compares immutable event content plus HMAC validity; pending and mismatched mirrors remain distinct. This journal is durable capture, not a separate cryptographic chain; the existing audit chain's unanchored-tail limitation remains.

Events and sequence heads have no TTL and are not cascaded when services/endpoints/credentials/accounts disappear. Their minimal actor/resource snapshots follow existing audit retention. Deleting or deactivating an owner removes product-history access under the live owner gate. Any future legal anonymization workflow must account for immutable mirrored audit retention rather than silently rewriting chained rows.

## API and shared definitions

- `GET /api/v1/keys/{service_id}/history?limit=20&cursor=...&actions=service.updated` returns whole operation groups with embedded action/field labels and safe changes. IDs are service UUIDs, including deleted instances. Unknown historical action codes remain filterable and render as `Service updated`.
- `GET /api/v1/keys/history/archived?limit=20&cursor=...` discovers retained deleted instance UUIDs within current permissions. This multi-segment route preserves arbitrary existing `/keys/{id_or_slug}` slugs.
- `GET /api/v1/options/service-history-action` and `service-history-field` expose static definitions from `services/service_history/definitions.rs`, using existing route authentication. They accept search/offset/limit, and reject owner/principal/account context even if supplied empty. `/options/service-scope` keeps its owner ACL, custom-input and wire contracts. See [OPTIONS_API.md](OPTIONS_API.md).

Mutation capture and timeline rendering do not fetch options. Existing codes should remain in the registry if retired; unknown older/future codes still have a generic fallback. Registry labels never authorize persistence of a value. Responses are private/no-store, and frontend caches include the authenticated identity. List summaries share the request's membership snapshot with provisioning and listing; membership snapshots are never cached across requests. Summary authorization still checks active owners, admin roles, effective service scopes, and API-key scope restrictions.

Service detail editors retain open drafts when a background read fails with a network or server error, and offer a read-only retry. Rejected reads discard cached details so a subsequent network failure cannot restore previously accessible data. Switching the service route or signed-in identity ends the previous editor and confirmation lifetime. The admin edit-form contracts, including sparse updates, reviewed payloads, and custom scope entry, remain described in [ADMIN_FORM_SAFETY.md](ADMIN_FORM_SAFETY.md).

## MongoDB deployment prerequisite

NyxID now checks `hello` for a replica set or mongos with logical sessions **before** indexes/migrations. Standalone MongoDB fails startup with a transaction-prerequisite error. There is no lossy capture mode.

New local Compose volumes use MongoDB 8, authenticated root initialization, a persistent internal keyfile in `mongodb_config`, and the single-member `nyxid-rs` replica set. `mongodb-init` waits for authenticated replica-mode startup, initializes only an unconfigured replica set, refuses another configured set name, and waits for primary election. Production backend startup depends on successful initialization. Local host tools use `directConnection=true` because the replica advertises the Docker-internal `mongodb:27017` hostname; containers use that hostname normally.

For an existing standalone Compose volume, coordinate a maintenance window before changing a running database:

1. Stop application writers and scheduled jobs. Take and verify a database backup and record the existing root credentials, volume names and current image version. Do not delete or recreate the data volume.
2. Review the replica-set name/advertised hostname and `DATABASE_URL`. External replica sets/mongos must keep their own topology; do not run the bundled initializer against them.
3. Start only MongoDB and `mongodb-init` with the new Compose configuration and the **existing** root credentials. The entrypoint creates the separate internal-auth keyfile if absent and preserves an existing keyfile. The initializer does not replace a configured replica set or modify application documents.
4. Verify authenticated `hello` reports `setName: nyxid-rs` and a writable primary. Verify a disposable transaction's commit/abort and read existing application records. Then start the backend and review migration/startup logs before reopening traffic.
5. Retain the backup and keyfile volume. Rollback requires another coordinated stop: restore the previous app/database configuration or verified backup as appropriate; never run a standalone process and replica-set process against the same data files.

No existing running database is automatically migrated by this feature. Docker wiring has been validated with Compose configuration checks; the bootstrap algorithm has also been exercised against real authenticated MongoDB 8 with preexisting standalone data, repeat initialization, retained data and commit/abort. Container execution requires an available Docker daemon.

## Validation

Set `NYXID_TEST_DATABASE_URL` to a disposable MongoDB 8 replica set before running backend tests. The CI configuration uses `minSnapshotHistoryWindowInSeconds=0` on its dedicated test instance to release dropped test collections promptly. Use the repository's pinned Node version for frontend commands.

```sh
cargo nextest run -p nyxid --profile ci
cargo nextest run -p nyxid-cli --profile ci
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
npm --prefix frontend test
npm --prefix frontend run test:coverage
npm --prefix frontend run lint
npm --prefix frontend run build
```

The `service_history` backend tests cover transaction rollback, concurrent edits and new references, shared-resource fan-out, safe projections, bulk grouping, pagination, audit retries, and live authorization. Frontend regressions cover history access/filtering and the service-detail draft and identity boundaries. Changes to wizard manifest inputs also require `npm --prefix frontend run build:wizard`; the CLI test suite verifies the resulting source hash.
