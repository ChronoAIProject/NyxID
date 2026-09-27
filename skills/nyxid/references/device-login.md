# Device login

Use an existing scoped credential when available. When the user requests new
access, discover the service, start a human-approved login, share the CLI's complete
approval link, and resume after the human approves. Never self-approve.

## Discover, request and resume

Before login, discover public catalog metadata without sending saved credentials:

```sh
nyxid catalog list --public --all --base-url https://nyx-api.chrono-ai.fun --output json
nyxid catalog show api-google-gmail --public --base-url https://nyx-api.chrono-ai.fun --output json
nyxid catalog endpoints api-google-gmail --public --base-url https://nyx-api.chrono-ai.fun --output json
```

Use actual catalog slugs and `scope_catalog[].scope` values. This public menu does
not prove a human has a usable connection or the agent is authorized to call it.

Request restricted access, including the preferences in the CLI command:

```sh
nyxid login --agent-key --no-wait --output json --profile mail-agent \
  --base-url https://nyx-api.chrono-ai.fun \
  --key-source new --key-name "Mail assistant" \
  --scopes read,proxy \
  --service-permission 'api-google-gmail::https://www.googleapis.com/auth/gmail.readonly' \
  --expiry-days 30 --platform codex
```

Share **`verification_uri_complete`** from the JSON output directly with the human.
The CLI encodes all hints; no manual URL construction is needed. It also returns
`user_code`, bare `verification_uri`, local `request_id`, expiry and poll interval.
The private polling secret stays local. `--no-wait` opens no browser and does not
poll. Adapt the service/permissions in this example to the requested task.

Optionally append `&show_details=true` to open the requester details by default.
This is a browser presentation option, not a CLI scope flag or authorization.
It preserves all permission hints and does not reveal credentials or skip consent.

- `--scopes` (alias `--permissions`) requests NyxID permissions.
- `--service` requests catalog slugs; `--service-permission` requests qualified
  `catalog-slug::provider-scope` values. These options accept repeated or CSV values.
- `--key-source existing|new`, `--key-name`, `--expiry-days` and `--platform` prefill
  editable key preferences. They do not create or approve a key.
- `--device` permits the human to choose full or restricted access. Use it instead
  of `--agent-key` only if either result is acceptable. `--login-type agent` is a
  suggestion; `--agent-key` enforces restricted delivery. Never combine both flows.
- Preferences cannot accompany `--password`, `--callback`, `--code` or `login resume`.
  They default the suggested login type to `agent`; they cannot limit a `full` grant.

After explicit human approval, resume using the local handle, not `user_code`:

```sh
nyxid login resume <request_id> --once --output json
nyxid whoami --profile mail-agent --output json
nyxid mcp discover --profile mail-agent --output json
```

`--once` checks once and returns `login_pending` if approval is still outstanding;
respect the request's poll interval before trying again. Without it, resume waits.
The request retains its destination profile. Inspect the delivered `auth_kind` and
grant. A denied/expired request needs a new request and human approval.

`mcp discover` fetches the live operation catalog using the granted credential. It
returns services and operations filtered by the key's service/node scope, with input
schemas, recommended skills and diagnostics. Follow that response to choose calls;
execution still checks live permissions and availability. Do not treat public
catalog entries as authorized calls or switch to full credentials after a refusal.
`mcp config` generates client configuration and is a different command.

## Signed-out approvers

Public preview grants nothing. The signed-out human stays on the request page and
chooses **Only for this request** (default) or **Keep me signed in** before password,
MFA or configured social verification. The first creates a request-bound proof,
accepted only by the dedicated approval endpoints, with no general browser session.
The second also signs the browser in. Both require explicit final approval and
leave the requester pending until then. Browser persistence is independent of the
full-account or restricted credential granted to the requester.

Browsers and QR payloads use `/login/device?user_code=XXXX-XXXX` plus the documented
hints. Use the server-issued URL unchanged to preserve installed scanner support.
Identity changes clear prior choices; closed or expired proofs cannot be reused.
An Agent Key or third-party OAuth credential cannot approve a new login. Identity
verification cannot create missing connection or organization rights. Choose an
eligible grant, obtain access, or deny; never silently substitute full account access.

## Connection choices and protocol

For new keys, choosing multiple accounts selects multiple existing service
connections and grants access through each. It does not change the NyxID sign-in
identity. Labels/slugs do not prove distinct upstream mailboxes. A read-and-send
Gmail connection keeps sending access even when the URL asks only for reading;
review extras and choose a narrower connection when appropriate. Existing-key
review shows effective bindings and does not edit the parent key.

Effective options include per-key credential overrides and owner-bound platform
services. The durable current/future platform grant is always extra access;
unreported provider scopes cannot be described as exact. Approval submits consent
snapshots for explicit and currently implied connections. Filters never downscope
provider credentials. URL hints cannot supply owners, key IDs, resource UUIDs,
allow-all grants, credentials or approval authority.

The canonical [device login protocol](https://github.com/ChronoAIProject/NyxID/blob/main/docs/DEVICE_LOGIN_PROTOCOL.md)
contains exact parameters/limits, direct API URL examples, identity return rules
and source pointers. This implementation requires an updated CLI and frontend.
Eight-character v2 issuance separately requires the staged server gate in
[ADR-015](https://github.com/ChronoAIProject/NyxID/blob/main/docs/ADR-015-auth-device-login.md).
Normal `/login` remains account-only. This flow is separate from `/devices/code/*`
hardware provisioning.
