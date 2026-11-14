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
audit metadata, machine job metadata, and the billing wallet summary, credit
grants received, and bounded usage rollups. Empty sections remain present so
clients can rely on the registry-defined section set.

The registry-defined sections and their allowlisted fields are:

| Section | Fields included |
| --- | --- |
| `account` | Account ID, email, display name, avatar URL, email-verification state, and account timestamps |
| `services` | Service IDs, slugs, type, active/deleted state, and lifecycle timestamps; other owner-scoped service metadata uses the common safe metadata fields |
| `security` | Session IDs, creation/expiry/last-used timestamps, and user-agent metadata |
| `assistant` | Owner-visible assistant message role/content/timestamps, conversation IDs, tool/action metadata, and retained file metadata |
| `channels` | Owner-visible channel message IDs, conversation IDs, direction, text, and timestamps |
| `account_activity` | Action receipt metadata, audit event type/status/timestamps, invite activity, and other owner-scoped activity metadata |
| `machines` | Machine job IDs, status, and lifecycle timestamps |
| `billing` | Wallet balance/reservation/status summary, grants received, and usage meter service/metric/model/quantity/status/timestamps |

Rows from additional registry entries are placed in the section named by that
entry and use the common safe metadata allowlist (`_id`, owner IDs,
service/conversation IDs, names/slugs, status/type/role/action, content
type/filename/size, and lifecycle timestamps). The retained
`downstream_services` section is restricted to opaque owner IDs and service
operational metadata.

Passwords, authentication/session hashes, API keys and provider credentials,
tokens, signatures, ciphertext, wrapped keys, nonces, OAuth state/code values,
push tokens, webhook secrets, and transient coordination, lease, deduplication,
and erasure bookkeeping are excluded. Uploaded and generated files are
represented by metadata only (name/type/size and lifecycle timestamps where
available); raw payloads are never included. While a file is retained, its
filename, MIME type, byte size, creation time, and expiry are included in the
assistant section.

Each section is capped at 1,000 rows and queries have a three-second server
limit. The complete JSON is capped at 32 MiB; the server removes oldest rows
from capped sections until it fits and lists those sections in
`truncated_sections`. A person may request three exports per hour, and each
replica builds at most two exports concurrently. Every successful export emits
the chained metadata-only `user.data_exported` audit event with section counts,
truncation names, format/version, and byte size.
