# Google Drive permission proof of concept

The native backend pilot now uses the same evaluator; see
[NATIVE_DRIVE_PERMISSIONS.md](NATIVE_DRIVE_PERMISSIONS.md) for persisted keys and
server execution. This page describes the standalone harness.

Status: experimental, standalone local proxy. The `nyxid-permissions` workspace
crate provides an executable permission boundary and a reusable evaluator. It
does not attach folder grants to existing NyxID keys or change their authority.

## What it demonstrates

An agent can list, read, download/export, create subfolders, and rename files
inside one permitted Google Drive folder tree. Requests outside that boundary
fail before the requested operation is dispatched. Parameters can be restricted
further with exact values, finite value lists, integer ceilings, and string
length limits. REST and MCP invoke the same `Engine::execute` method.

```text
Agent holding POC client key
  -> Local REST / MCP proxy
  -> Operation and parameter evaluator
  -> Before-execution hooks
  -> Drive ancestry verification
  -> Existing NyxID proxy (server-held upstream key, exact connection)
  -> Google Drive
  -> Provider response sanitization and after-response hooks
  -> Caller
```

The two credentials must differ. Only the trusted POC process holds the NyxID
upstream key; the restricted caller receives `NYXID_POC_CLIENT_KEY`. Giving that
caller the upstream key, a Google token, or another unrestricted connection
would give it authority outside this boundary. Running this local process does
not isolate secrets from other processes controlled by the same OS user.

## Run without an account

```sh
python3 scripts/test-drive-permissions-poc.py
```

The script builds and launches the actual binary on an ephemeral loopback port,
generates an ephemeral client key, executes 21 REST/MCP checks, and shuts it down.
It verifies allowed reads and writes, observes the rename, and tests denials.
All mutations affect an in-memory Drive. No Google or NyxID account is accessed.

To explore manually:

```sh
export NYXID_POC_CLIENT_KEY="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
cargo run -p nyxid-permissions --bin nyxid-drive-permissions-poc -- demo
```

In another terminal with the same client-key environment:

```sh
curl -H "Authorization: Bearer $NYXID_POC_CLIENT_KEY" \
  http://127.0.0.1:4318/drive/v3/files

curl -H "Authorization: Bearer $NYXID_POC_CLIENT_KEY" \
  http://127.0.0.1:4318/drive/v3/files/report

curl -H "Authorization: Bearer $NYXID_POC_CLIENT_KEY" \
  http://127.0.0.1:4318/drive/v3/files/salary
```

The first two return 200; the third returns 403. The fixture contains:

```text
Client A (client-a)                 Payroll (payroll)
  Brief.txt (brief)                   Salaries.txt (salary)
  Reports (reports)
    October report.txt (report)
  Payroll shortcut (shortcut-out)  -> always denied
```

The process prints decision events with policy name, transport, status, and a
fixed denial reason. It does not log credentials, request bodies, file names,
resource IDs, or returned content. These local events are not NyxID's durable
audit chain; forwarded operations still pass through NyxID's existing audit.

## Try an existing Google connection

1. Use an existing managed Google Drive or Workspace connection in NyxID. The
   Google credential must be able to read the file and its ancestors. A partial
   Google grant may fail verification; this POC never widens Google's grant.
2. Obtain the catalog service UUID and the exact `UserService` UUID from the
   service details / `GET /api/v1/keys` or `GET /api/v1/user-services`. These are
   different identifiers. The latter is pinned using NyxID's `_nyxid_via`.
3. Create a dedicated NyxID Agent Key with only `proxy` scope and access to that
   connection. Keep it in the trusted proxy process's
   `NYXID_POC_UPSTREAM_KEY` environment. Existing NyxID service permissions,
   approval rules, and billing continue to apply.
4. Copy `permissions/examples/drive-readonly-policy.json` to a local configuration
   file. Replace `resource.root_folder_id` with the actual Google folder ID.
   This first trial permits only listing and reading metadata. The separate
   `drive-policy.json` example enables all supported operations. When editing,
   remove constraints for any operation you remove from `allowed_operations`;
   constraints on absent operations are rejected at startup.
5. Set a different randomly generated `NYXID_POC_CLIENT_KEY`, and start:

```sh
cargo run -p nyxid-permissions --bin nyxid-drive-permissions-poc -- \
  serve --policy /absolute/path/drive-policy.json \
  --nyxid-url https://YOUR-NYXID-BACKEND \
  --service-id CATALOG-SERVICE-UUID \
  --connection-id USER-SERVICE-UUID
```

The POC binds to `127.0.0.1:4318` by default; `--listen` before the subcommand
changes that address, but only loopback is accepted. HTTPS is required for the
NyxID origin, except for local development on loopback. Redirects and environment
HTTP proxies are disabled. No caller headers are forwarded. The configured
upstream key and connection cannot be overridden by request parameters.

Policy configuration is immutable for the lifetime of the process. Restart with
a revised policy to change access, or stop the process to revoke the POC key.
Do not treat editing the JSON file as immediate revocation. NyxID's upstream
key revocation remains independently authoritative for subsequent upstream calls.

## Supported contract

The default policy includes subfolders. Setting `include_descendants: false`
permits access to direct children but denies listing or creating inside their
subfolders. Every request verifies ancestry afresh; no positive permission cache
survives a request. IDs, rather than folder names, establish the boundary.

| Operation | HTTP request | Boundary |
| --- | --- | --- |
| `drive.files.list` | `GET /drive/v3/files` | Default parent is the permitted root. Optional `q` must be exactly `'FOLDER_ID' in parents`; the adapter verifies that folder and adds `trashed = false`. |
| `drive.files.get` | `GET /drive/v3/files/{id}` | Checks current ancestry and returns only `id`, `name`, `mimeType`. |
| `drive.files.download` | Same GET with `alt=media` | Checks ancestry, denies folders and shortcuts, buffers at most 8 MiB. REST only. |
| `drive.files.export` | `GET /drive/v3/files/{id}/export?mimeType=...` | Checks ancestry. Adapter supports PDF/plain text/CSV; example policy narrows that to PDF/plain text. REST only. |
| `drive.folders.create` | `POST /drive/v3/files` | JSON must contain `name`, folder `mimeType`, and exactly one explicit allowed parent. |
| `drive.files.rename` | `PATCH /drive/v3/files/{id}` | Only `name` may change; the boundary folder itself cannot be renamed. |

The list operation lists one folder at a time, not the entire subtree in one
request. To traverse, list each returned subfolder explicitly. Google page
tokens are supported; the folder query is enforced on every page, and every
returned row must name that exact parent. Shortcuts are omitted. Unknown or
incomplete responses fail closed. Metadata responses are projected regardless
of caller `fields`; parent IDs, sharing details, shortcut targets, and direct
download URLs are never exposed.

Unknown operations, extra body/query fields, duplicate keys, ambiguous path
encodings, arbitrary searches, sharing, deletion, moves, copies, content uploads,
Google multipart batches, and Docs/Sheets/Slides operations are denied. A small
tested operation set is intentional. This is not “full Drive within a folder.”

Parameters are checked against the adapter's prepared request, including its
injected defaults. Query values are strings for exact/one-of rules; body values
retain JSON types. Body constraints use JSON Pointers. A rule cannot authorize
an endpoint or field that the adapter does not support.

Example rename:

```sh
curl -X PATCH -H "Authorization: Bearer $NYXID_POC_CLIENT_KEY" \
  -H 'Content-Type: application/json' \
  --data '{"name":"Updated report.txt"}' \
  http://127.0.0.1:4318/drive/v3/files/report
```

In demo mode this updates a fixture. In live mode a permitted request changes
the real Google file. Writes are dispatched at most once per incoming request;
there is no cross-request deduplication. A possible dispatch followed by an
error or timeout reports an uncertain write outcome. Inspect the resource
before retrying, especially for folder creation. A caller retry is a new request.
If the upstream reports success but a response check fails, the error explicitly
says the write was accepted and its response withheld. A post-execution denial
does not undo the write or mean it is safe to retry.

## Runtime check hooks

Hooks add restrictions to the enclosing policy. The POC implements hooks inside
its shared execution engine; these are not Claude Code/Codex client lifecycle
hooks or outbound HTTP webhooks. Both REST and MCP must pass them. Agent-side
hooks can later provide earlier feedback, but only the server gate controls
whether the credential is used.

Two stages are available:

| Stage | Inputs and ordering | What a denial prevents |
| --- | --- | --- |
| `before_execute` | Immutable prepared operation, parameters, resource IDs, and policy; after static checks, before ancestry verification | Provider metadata requests and the requested operation |
| `after_response` | The same context plus the provider-sanitized response; before any bytes reach the caller | Delivery of the response; it cannot reverse an upstream effect |

An allow continues through every remaining check. Hooks cannot change the
prepared request, enlarge the folder boundary, override an operation denial,
inject a credential, or turn off other hooks. The hook interface contains no
transport or raw key. An empty operation selector applies to all operations
already allowed by the policy; explicit selectors must name allowed operations.

`permissions/examples/drive-hooks-policy.json` demonstrates both built-ins:

```json
"hooks": [
  {
    "name": "pause-writes",
    "handler": "pause_switch",
    "stage": "before_execute",
    "operations": ["drive.files.rename", "drive.folders.create"],
    "timeout_ms": 500,
    "config": { "path": "/tmp/nyxid-drive-poc.pause" }
  },
  {
    "name": "confidential-output",
    "handler": "response_markers",
    "stage": "after_response",
    "timeout_ms": 500,
    "config": { "markers": ["CONFIDENTIAL"] }
  }
]
```

This is a field in the policy document. Run the full example against fixtures:

```sh
cargo run -p nyxid-permissions --bin nyxid-drive-permissions-poc -- \
  demo --policy permissions/examples/drive-hooks-policy.json
```

Use the client-key environment described above. The example flag path is for
local demonstration; choose an existing owner-controlled directory for a live
trial. Creating the flag pauses the selected writes without a restart; deleting
it resumes them. Reads continue. A missing or inaccessible parent directory
fails the check. Flag contents are never opened, and a dangling symlink also
counts as a pause marker. A flag change only affects checks that observe it;
it does not cancel a request already past the hook.

`response_markers` withholds a whole response when a configured, case-sensitive
literal UTF-8 byte sequence appears. It scans in bounded chunks and covers marker
matches across chunk boundaries. This demonstrates a response gate; it does not
parse PDF/Office files, decode arbitrary encodings, detect semantic sensitivity,
or provide complete DLP. Use a purpose-built scanner through `PermissionHook`
when that is required. A read blocked here has already reached Google, although
its response has not reached the caller.

The evaluator supports at most eight hooks, run in configured order within each
stage. Each has a timeout of 1–2,000 ms (default 500 ms), also bounded by the total
20-second request deadline. Denial short-circuits later hooks. Unknown handlers,
invalid configuration, and duplicate instance names fail at startup. Runtime
failure, asynchronous timeout, and unwind panic fail closed. Before-execution
denial returns HTTP 403; check failure returns 503. MCP returns an error tool
result. A successful write whose response is withheld returns 502 with explicit
write-accepted wording; the engine never automatically replays it.

For a new check, implement `PermissionHook`, validate its stage/configuration,
register it in a trusted `HookRegistry`, and construct the engine with
`Engine::new_with_hooks`. This can support live entitlement checks, time windows,
or an internal content classifier without changing REST/MCP handling. The two
built-ins are filesystem pause checks and response marker checks only; external
decision services and approval workflows are not implemented.

Handlers are trusted host code. The POC does not load agent-provided scripts,
shell commands, or arbitrary webhook URLs. In-process hooks are not a sandbox:
timeouts require cooperative async execution and cannot preempt blocking code
or reverse a hook's own side effects. Implement checks as read-only, bounded
operations and isolate untrusted extensions in a separate runtime.

## MCP

Configure a client to use `http://127.0.0.1:4318/mcp` with the POC bearer key.
The endpoint implements a stateless JSON HTTP subset of MCP 2025-03-26:
initialize, initialized notification, ping, tools/list, and tools/call.
It exposes one `drive_request` tool:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "drive_request",
    "arguments": {
      "method": "GET",
      "path": "/drive/v3/files/report"
    }
  }
}
```

There is no SSE/session management, OAuth discovery, resource subscription, or
binary tool result. REST handles binary reads. An existing NyxID MCP endpoint
does not gain this policy; a restricted caller must use this POC endpoint and
must not hold the upstream key.

## Architecture and scale limits

`src/lib.rs` owns versioned policy validation, parameter rules, admission,
concurrency, deadlines, hooks, and dispatch. `ProviderAdapter` maps requests to
operations, verifies resources, and sanitizes responses. `Transport` provides
bounded I/O. `drive.rs` is the first provider adapter; `http_transport.rs`
connects it to NyxID. `hooks.rs` owns the trusted check registry and built-ins.
`mock.rs` supplies deterministic fixtures.

Cost depends on ancestry depth, not Drive size. Each supported request has one
resource to verify and a maximum of 32 metadata lookups, plus one execution
request. Lookup results are reused only within that request. The engine admits
32 concurrent requests, rejects excess work with 429, caps request bodies at
64 KiB and responses at 8 MiB, and applies a 20-second total deadline. The
upstream client also has a 5-second connect timeout and 15-second request timeout.
Google API quota and network latency will determine useful production limits;
no production throughput claim is made by the mock tests.

The standalone process serves one policy, connection, and client key. The traits
allow additional providers without changing REST/MCP admission. The
[native pilot](NATIVE_DRIVE_PERMISSIONS.md) now persists separate permission-bound
keys, pins connection authority, checks pause/revocation, and confines execution
to dedicated endpoints. It rejects generic rotation and child-credential login.
Organization policy ceilings, inherited child grants, atomic policy-preserving
rotation, additional providers, and broader operation support remain future work.

Concurrent moves remain a limit: checking ancestry and executing a Google
operation are separate upstream calls. A person can move a file between them.
Response validation catches some drift but cannot undo a mutation or guarantee
atomic folder containment. Stricter isolation needs Google-side ACL restrictions
on the upstream identity. Sharing inheritance and Google document references
are also not a data-loss-prevention boundary; permitted content may already
contain information copied from elsewhere.

## Verification

```sh
cargo fmt -p nyxid-permissions --check
cargo clippy -p nyxid-permissions --all-targets --locked -- -D warnings
cargo test -p nyxid-permissions --locked
python3 scripts/test-drive-permissions-poc.py
```

Tests cover allowed effects, denied effects with no mutation dispatch, fresh
ancestry, disabled descendants, shortcuts, missing/trashed resources, cycles and
depth limits, unsafe fields and alternative endpoints, path/query/JSON ambiguity,
response containment, parameter constraints, REST/MCP parity, credential checks,
size limits, concurrency, deadlines, exact upstream connection selection,
redirect rejection, and uncertain writes without automatic replay. Hook tests
cover live pause/resume, operation selectors, unavailable checks, panic/timeout,
ordering, inability to widen base permissions, withheld REST/MCP results, marker
chunk boundaries, and accepted-write outcomes after a response denial. They require
neither MongoDB nor Google credentials. CI runs the suite and executable smoke
test as a required dependency of `CI Pipeline`.
