# Use Supabase as an AI Agent Database

Connect a Supabase project's PostgREST Data API to NyxID, then let an AI agent
read and write project tables without receiving the Supabase API key.

This integration uses HTTPS. A PostgreSQL connection string such as
`postgresql://postgres:...` is not accepted by the NyxID HTTP proxy.

## Prerequisites

- NyxID CLI installed and authenticated.
- A Supabase project with the Data API enabled.
- The project URL (`https://<project-ref>.supabase.co`) or full Data API URL
  ending in `/rest/v1`.
- A Supabase API key from **Project Settings -> API Keys**.

Choose the key deliberately:

- A new `sb_secret_...` key is suitable for server-side agent access, but it
  uses the `service_role` Postgres role and bypasses all Row Level Security.
- A publishable or legacy `anon` key runs with the anonymous Postgres role and
  only reaches rows allowed to `anon` by your RLS policies. This connector does
  not acquire, refresh, or forward an end-user Supabase JWT.
- Legacy `service_role` JWT keys still work, but Supabase recommends migrating
  new server-side integrations to `sb_secret_...` keys.

Use a dedicated project or narrowly exposed tables when an agent does not need
full database access.

## 1. Add the Supabase service

```bash
export SUPABASE_API_KEY='sb_secret_...'

nyxid service add api-supabase \
  --label "Personal Supabase" \
  --endpoint-url "https://<project-ref>.supabase.co" \
  --credential-env SUPABASE_API_KEY
```

NyxID stores the key encrypted and injects it as the `apikey` request header.
It normalizes the project URL to `https://<project-ref>.supabase.co/rest/v1`.
It is not sent as an `Authorization: Bearer` token because new Supabase
publishable and secret keys are not JWTs.

The command returns the created service slug. It is normally `api-supabase`,
but may be suffixed when the account already has a service with that slug. Use
the returned slug in subsequent commands.

## 2. Read rows

For a table named `todos`:

```bash
nyxid proxy request api-supabase \
  'todos?select=id,title,done&order=id.desc&limit=10'
```

The query string is standard PostgREST syntax and is forwarded unchanged.

## 3. Insert a row

```bash
nyxid proxy request api-supabase todos \
  --method POST \
  --header 'Content-Type: application/json' \
  --header 'Prefer: return=representation' \
  --data '{"title":"Review NyxID audit log","done":false}'
```

NyxID forwards PostgREST's `Prefer`, `Accept-Profile`, `Content-Profile`, and
`Range-Unit` request headers. This also supports selecting a non-`public`
schema:

```bash
nyxid proxy request api-supabase reports \
  --header 'Accept-Profile: analytics'
```

## 4. Scope an agent to the service

Create a dedicated agent key and allow only the Supabase service:

```bash
nyxid api-key create \
  --name supabase-agent \
  --platform codex \
  --scopes proxy

nyxid service list  # copy the Supabase service UUID
nyxid api-key list  # copy the supabase-agent key UUID

nyxid api-key update <KEY_ID> \
  --allowed-services <SUPABASE_SERVICE_ID> \
  --allow-all-services false
```

This prevents the agent key from reaching unrelated NyxID services. It does
not reduce the database privileges carried by a Supabase secret key; Supabase
roles, RLS policies, and exposed schemas remain the downstream authorization
boundary.

## MCP behavior

Supabase table schemas and PostgREST filter keys are project-specific. NyxID
therefore exposes this connection through its generic proxy MCP tool instead
of shipping unsafe static update/delete tools. The agent supplies a method and
a path such as `todos?select=id,title&done=eq.false`.

To expose typed per-table tools, publish a project-specific OpenAPI document
and attach it with `nyxid service update <SERVICE_ID> --openapi-spec-url <URL>`.

## Current scope

The catalog entry targets the Supabase Data API only. PostgreSQL connection
strings, SQL sessions, Storage, Edge Functions, Realtime, and Supabase Auth
session refresh are not part of this connector.
