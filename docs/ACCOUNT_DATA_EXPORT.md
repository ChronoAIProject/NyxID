# Account data export

`POST /api/v1/users/me/export` downloads a JSON document for the authenticated
first-party human account. OAuth-client, delegated, API-key, service-account,
and relay credentials are rejected by the same human-only router and first-party
client check used by account settings.

The response is an attachment named `nyxid-account-export-YYYY-MM-DD.json`.
It has `format: "nyxid.account-export"`, `version: 1`, an RFC3339 `generated_at`,
the opaque `account_id`, limits, `truncated_sections`, human-readable
`excluded` categories, and section objects containing `items`, `total_count`,
and `truncated`. Sections currently cover account profile, services, security
session metadata, assistant messages, channel messages, account activity and
audit metadata, machine job metadata, and the billing wallet summary. Empty
sections remain present so clients can rely on the registry-defined section set.

Passwords, authentication/session hashes, API keys and provider credentials,
tokens, signatures, ciphertext, wrapped keys, nonces, OAuth state/code values,
push tokens, webhook secrets, and transient coordination, lease, deduplication,
and erasure bookkeeping are excluded. Uploaded and generated files are
represented by metadata only (name/type/size and lifecycle timestamps where
available); raw payloads are never included.

Each section is capped at 1,000 rows and queries have a three-second server
limit. The complete JSON is capped at 32 MiB; the server removes oldest rows
from capped sections until it fits and lists those sections in
`truncated_sections`. A person may request three exports per hour, and each
replica builds at most two exports concurrently. Every successful export emits
the chained metadata-only `user.data_exported` audit event with section counts,
truncation names, format/version, and byte size.
