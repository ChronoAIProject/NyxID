# Native Drive permission keys (pilot)

NyxID can issue a key whose authority is limited to a Google Drive folder,
selected operations, parameter constraints, and response hooks. The backend
persists the binding and runs the same provider adapter and evaluator used by
the [standalone POC](DRIVE_PERMISSIONS_POC.md). Google credentials remain in
NyxID. This is an experimental backend API; there is no permission editor UI or
CLI command yet.

The native host also supports declarative API/parameter rules across Google
REST services, including all six Workspace products. See
[Google API permission keys](GOOGLE_API_PERMISSIONS.md) for the key format,
coverage, examples and limits. This page describes the Drive folder adapter;
its ancestry boundary is distinct from generic API input rules.

```text
Owner session -> create immutable policy + restricted API key in one transaction

Agent's permission key -> dedicated REST or MCP endpoint
  -> authenticate key, load live policy, admit bounded request
  -> operation + parameter rules, before-execution hooks
  -> verify folder ancestry using the pinned Google connection
  -> revalidate binding, ordinary proxy approvals + billing
  -> Google Drive
  -> sanitize response, response hooks, live pause/revocation check
  -> return bounded response
```

The key's purpose is `permission_bound`. Ordinary REST proxy, ordinary MCP,
LLM, SSH, management, and token-delivery routes refuse this purpose. MCP's
separate authentication implementation also rejects it. A missing policy
never grants unrestricted access. The normal proxy executor additionally
requires a private in-process permission ingress for these keys.

## Supported boundary

The pilot accepts one personal, active, server-held OAuth2 connection to the
`api-google-drive` or `api-google-workspace` catalog service. Its effective
base URL must be `https://www.googleapis.com`, using bearer authentication.
Node routing, platform credentials, credential overrides, custom default
headers, token exchange, and identity forwarding are unsupported. Restrictions
apply to the resolved destination as well as the catalog identity.

Issuance binds the resolved credential ID and epoch, execution authority, and
destination routing configuration. Every provider request resolves that exact
connection and compares its authority before forwarding. Changing the account,
credential material, destination, or relevant configuration requires reissuing
the permission key. Ordinary OAuth refresh preserves the credential epoch.
The binding is checked again after an approval wait. Existing catalog operation
rules, approvals, rate limits, and billing still apply to provider requests,
including the metadata requests needed for ancestry verification.

Supported operations are list one folder, get sanitized metadata, download,
export, create folders, and rename files. See the POC operation table for exact
paths and parameter restrictions. Sharing, deletion, moving, copying, uploads,
batch APIs, and Docs/Sheets/Slides editing remain denied. Unknown operations
and ambiguous query/body/path encodings fail closed. Shortcuts cannot extend
the boundary.

## API usage

Create a key with a first-party human session or first-party access token.
Third-party OAuth clients and API keys cannot manage these policies.

`POST /api/v1/permission-keys` accepts:

```json
{
  "name": "Project documents",
  "user_service_id": "YOUR-PERSONAL-GOOGLE-USER-SERVICE-UUID",
  "expires_at": "2026-10-06T12:00:00Z",
  "policy": {
    "version": 1,
    "name": "Project folder",
    "provider": "google_drive",
    "resource": {
      "type": "google_drive_folder",
      "root_folder_id": "YOUR-GOOGLE-FOLDER-ID",
      "include_descendants": true
    },
    "allowed_operations": ["drive.files.list", "drive.files.get"],
    "constraints": {},
    "hooks": []
  }
}
```

Choose an expiry in the future, at most 90 days away. The response includes
`key` once, `expires_at`, and `binding` containing the key ID, policy, connection
ID, pause state, and revision. Store the returned key in the caller's secret
store. Do not give that caller an unrestricted NyxID key or the Google token.

Use the returned key as `Authorization: Bearer …` or `X-API-Key`:

```sh
curl -H "Authorization: Bearer $NYXID_PERMISSION_KEY" \
  "$NYXID_URL/api/v1/permission-execution/rest/drive/v3/files/FILE_ID"
```

For listing, pass a URL-encoded `q` of exactly `'FOLDER_ID' in parents`.
The adapter adds its containment and trash filters and checks each returned row.

The stateless MCP endpoint is `POST /api/v1/permission-execution/mcp`. It supports
`initialize`, `ping`, `tools/list`, and `tools/call` with the `drive_request`
tool. Binary download and export responses use REST. A tool call looks like:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "drive_request",
    "arguments": {"method": "GET", "path": "/drive/v3/files/FILE_ID"}
  }
}
```

Owner management endpoints:

| Method and path | Behavior |
| --- | --- |
| `GET /api/v1/permission-keys/{id}` | Read policy and current revision; no raw key returned |
| `PATCH /api/v1/permission-keys/{id}` | Set `paused` with an expected `revision` |
| `DELETE /api/v1/permission-keys/{id}` | Revoke the API key through the existing key lifecycle |

Pause with `{"revision":1,"paused":true}`. The response increments the revision.
A stale revision returns 409. Resume using the new revision; requests that
started under an earlier revision stay invalid even after resuming. Revocation
through the ordinary API-key management endpoint also takes effect here.
Policy authority is immutable. Issue a replacement and revoke the old key to
change it; generic key update and rotation refuse these keys. Child Agent Key
login and credential overrides are unsupported.

## Hooks and limits

Native policies may configure up to six `response_markers` hooks. NyxID reserves
two additional hooks for live authorization checks before execution and after
the sanitized response. The transport also checks live authorization before
each provider call. Users cannot configure filesystem paths, executable code,
or outbound webhook URLs. The standalone `pause_switch` hook is unavailable
in native policies; use the database-backed pause endpoint.

Limits: 64 KiB request body, 8 MiB response, 32 active permission evaluations per
server process, a 10-second admission/body-read deadline, a 20-second evaluator
deadline, 32 ancestry reads, and bounded
hook timeouts. There is no policy cache shared across requests. Existing NyxID
rate limits apply as well. The concurrency cap is per replica, not a distributed
tenant quota. Hooks execute trusted host code; they are not a script sandbox.

Response marker checks match literal UTF-8 bytes. They do not constitute full
DLP for encoded or binary documents. A response hook can withhold output after
a write, but cannot undo the write. Errors distinguish an uncertain write from
an accepted write whose response was withheld. Neither case is automatically
retried. Pausing or revoking cannot recall an effect already dispatched.

Google can move a file between the ancestry check and the requested operation.
This upstream race also exists in the standalone POC. The pilot does not claim
an atomic Google-side folder ACL. Strong isolation requires Google-side access
controls as well. A live Google account trial is separate from fixture tests.

## Validation and rollout

Backend tests use a transaction-capable MongoDB 8 replica set configured through
`NYXID_TEST_DATABASE_URL`:

```sh
cargo test -p nyxid permission_keys
cargo test -p nyxid-permissions
python3 scripts/test-drive-permissions-poc.py
```

Tests cover persisted key/policy bindings, BSON dates, owner checks, expiry,
pause/resume revision fencing, revocation, credential epoch drift, alternate
HTTP routes, ordinary MCP rejection, and shared evaluator hooks. The standalone
suite covers the provider adapter's operation, parameter, ancestry, and response
rules without Google credentials. Production transport integration also needs
an end-to-end trial with an isolated Google folder before production use.

Upgrade all backend replicas before issuing this new key purpose: older
binaries cannot deserialize `permission_bound`. The Docker build includes the
shared permissions crate, and permissions changes trigger backend CI. Owner
deletion removes persisted policies; normal revocation retains policy metadata.
