# Account data export

`POST /api/v1/users/me/export` returns a JSON attachment for the authenticated
first-party human account. The human-only route and
`require_first_party_human` check reject OAuth-client, delegated, API-key,
service-account and relay credentials. The response is named
`nyxid-account-export-YYYY-MM-DD.json` and uses
`format: "nyxid.account-export"`, `version: 1`, an RFC3339 UTC
`generated_at`, the opaque `account_id`, `limits`, `truncated_sections`,
`excluded`, and nested section/subsection objects.

Each `sections.<section>.<subsection>` value is exactly one source collection:

```json
{
  "items": [],
  "total_count": 42,
  "truncated": false
}
```

Rows are newest first. A subsection reads at most 10,000 rows and reports the
unbounded matching `total_count`; if more rows exist, `truncated` is true and
its fully qualified name (`section.subsection`) appears in
`truncated_sections`. The complete serialized document is capped at 32 MiB;
when that cap is reached the oldest rows are removed incrementally from the
largest subsections until the document fits. Queries have a three-second
MongoDB `maxTimeMS` bound. Each replica permits two concurrent exports and a
person may start three exports per hour; a busy or rate-limited request returns
`429` with `Retry-After`.

The stable section set is `account`, `services`, `security`, `assistant`,
`channels`, `account_activity`, `machines`, and `billing`. Empty sections are
present so clients can discover the schema. The account section also contains
`profile` and `org_memberships` subsections. The profile contains only the
visible account email, display name, avatar URL, email-verification state, MFA
state, social-provider name, profile preferences, and created/updated/last-login
timestamps. Memberships contain organization/member IDs, role, and lifecycle
timestamps.

The remaining subsections are explicit registry mappings. They expose only
user-visible metadata or content appropriate to that collection: service
connections and service metadata; session lifecycle metadata; assistant
conversation/message content and attachment metadata; channel message content
and routing metadata; action, notification, machine-job and other account
activity metadata; audit event type/time metadata for events authored by the
person; and billing wallet summary, received grants and usage rollups. Every
source collection has a named field allowlist, owner predicate, subsection name
and newest-first sort in `account_export_service.rs`; there is no generic
collection fallback. Uploaded and generated files are metadata only (name,
MIME type, byte size, creation and expiry timestamps).

The export omits password and authentication hashes, session/token hashes,
OAuth codes and states, raw access or refresh tokens, API-key prefixes and
hints, push and webhook tokens, private keys, KMS/JWK identifiers, wrapped
keys, DEKs, nonces, signatures, HMACs, ciphertext, binary payloads, encrypted
file contents, transient lease/coordination/deduplication records, erasure
bookkeeping, and retained financial or integrity content. MCP session IDs are
credentials and are excluded. Decimal128 values are emitted as decimal strings;
BSON documents and arrays are converted recursively, dates are RFC3339 strings,
and BSON binary values are omitted. The successful response emits a chained
metadata-only `user.data_exported` audit event containing format/version,
subsection counts, truncation names and serialized byte size; it never records
export content.
