---
name: github-via-nyxid
description: Operate a user's GitHub account through NyxID's credential-brokering proxy (service slug api-github) — read and write repositories, files, issues, pull requests, commits, branches, Actions, gists and anything else the GitHub REST API exposes, all on the user's behalf and without ever handling a raw token. NyxID injects the user's GitHub credential server-side. Use when an agent needs to read from or act on GitHub for a user who has connected their GitHub account in NyxID.
version: "1.1"
license: MIT
metadata:
  category: plain
  tag:
    - github
    - nyxid
    - credential-broker
    - proxy
    - pull-requests
    - issues
    - rest-api
    - agent-guide
---

# Operate GitHub via NyxID

## Overview

This skill lets an AI agent act on a user's **GitHub** account using the GitHub REST
API — read repos, open issues and pull requests, commit files, trigger Actions, etc. —
**on the user's behalf**.

The one premise: the agent **never holds a GitHub token**. Every call goes through
**NyxID's credential-brokering proxy** for the connected service **`api-github`**. NyxID
stores the user's GitHub credential (connected via OAuth — "the GitHub account the user
bound in NyxID") encrypted at rest, strips any client Authorization header, injects the
GitHub credential server-side, and forwards the request to `https://api.github.com`. You
only ever send your **NyxID** bearer; you never see, request, or log the GitHub token.

> **What "operate GitHub" really means here:** anything the GitHub REST API allows **and**
> that the OAuth scopes granted to the `api-github` connection permit. The ceiling is the
> granted scopes, not the entire API surface (see [Scopes & permissions](#scopes--permissions)).

## 1. Preflight — confirm GitHub is connected

Before the first call, confirm the user has the `api-github` service connected in NyxID. In an
Aevatar turn, call the available `nyxid_service_inventory` with `{}` and select an exact
`user_service_id`; use only the schema emitted for that request. The CLI and raw HTTP remain
operator alternatives:

- CLI: `nyxid service list --output json` → look for an entry with `"slug": "api-github"`.
- Raw HTTP: `GET https://nyx-api.chrono-ai.fun/api/v1/services` with `Authorization: Bearer <NYXID_TOKEN>`.

If `api-github` is **not** present, the user must connect it once (one-click OAuth to their
own GitHub account). Tell them to run:

```bash
nyxid service add api-github --oauth
```

To grant write/admin capabilities up front, request extra scopes at connect time, e.g.:

```bash
nyxid service add api-github --oauth --scope "repo,workflow,read:org,gist"
```

Do **not** ask the user to paste a GitHub token into chat — the OAuth flow handles it.
(If broader, user-controlled scopes are needed, the `api-github-pat` service — a
user-supplied Personal Access Token — is the fallback; same proxy, slug `api-github-pat`.)

## 2. How to call the GitHub API

**Always call through the NyxID proxy for slug `api-github`.** The path you send is
appended to the service base URL `https://api.github.com`, so paths are **relative** and
standard GitHub REST v3 — e.g. `/user`, `/repos/{owner}/{repo}/issues`. Do not put a host
or a GitHub token in the request.

For Aevatar tool calls, keep these three invocation modes separate. They hit the same NyxID proxy,
but do not accept the same fields.

**a) Raw one-off `nyxid_proxy`.** The raw caller owns the route and must send exact
`service_id + slug + path`; `method`, `body`, allowed non-sensitive headers, and response mode are
optional. To read the connected account:

```json
{
  "tool": "nyxid_proxy",
  "arguments": {
    "service_id": "us-gh-7",
    "slug": "api-github",
    "path": "/user",
    "method": "GET"
  }
}
```

**b) Interactive request-local operation.** Call the exact `nyxid_service_operation__*` tool
emitted by the current request. Its dynamic schema requires an enumerated `user_service_id` and
only the operation fields declared by the exact UserService OpenAPI. For an emitted `get_user`
operation with no other parameters:

```json
{
  "tool": "nyxid_service_operation__get_user",
  "arguments": {"user_service_id": "us-gh-7"}
}
```

Do not derive the tool name from `api-github`, reuse it in another request, or add route fields not
present in the emitted schema.

**c) Compiled workflow admitted operation.** The step calls `nyxid_proxy`, copies the exact
selector into step-level `capability.nyxid_operation`, and supplies only admitted runtime values.
For an admission that owns `get-user`, `GET`, UserService `us-gh-7`, slug, and `/user`:

```yaml
- id: read_user
  type: tool_call
  capability:
    nyxid_operation:
      user_service_id: us-gh-7
      operation_id: get-user
  parameters:
    tool: nyxid_proxy
    arguments: '{}'
```

The server-owned proof also owns the contract digest, schemas, and response policy. Runtime
`arguments` may contain only admitted `path_params`, `query`, `headers`, `body`, and
`response_mode`; never put `service_id`, `user_service_id`, `operation_id`, `slug`, `path`,
`method`, or proof fields there.

Outside the Aevatar tool surface, operators may still use the lower-level transports below:

**Raw HTTP** (any runtime that holds a NyxID bearer):

```
{METHOD} https://nyx-api.chrono-ai.fun/api/v1/proxy/s/api-github/{path}
Authorization: Bearer <NYXID_TOKEN>
Content-Type: application/json          # for write bodies
```

**NyxID CLI:**

```bash
nyxid proxy request api-github /user -m GET
nyxid proxy request api-github /repos/OWNER/REPO/issues -m POST \
  -d '{"title":"Bug: ...","body":"..."}'
```

**Recommended GitHub content headers** (optional — NyxID handles auth, you set content):
`Accept: application/vnd.github+json` and `X-GitHub-Api-Version: 2022-11-28`.

## 3. Capability cookbook

Paths are relative to `https://api.github.com`. `{owner}/{repo}` and `{number}` are
placeholders.

### Read

| Goal | Method & path |
|---|---|
| Identify the connected user | `GET /user` |
| List the user's repos | `GET /user/repos?per_page=100&sort=updated` |
| Get a repo | `GET /repos/{owner}/{repo}` |
| List branches | `GET /repos/{owner}/{repo}/branches` |
| Read a file (base64 in `.content`) | `GET /repos/{owner}/{repo}/contents/{path}?ref={branch}` |
| List commits | `GET /repos/{owner}/{repo}/commits?sha={branch}&per_page=30` |
| List issues | `GET /repos/{owner}/{repo}/issues?state=open` |
| List pull requests | `GET /repos/{owner}/{repo}/pulls?state=open` |
| Search repos / issues / code | `GET /search/repositories?q=...` · `GET /search/issues?q=...` · `GET /search/code?q=...` |
| Check rate limit (free) | `GET /rate_limit` |

### Write (needs the matching scope)

| Goal | Method & path | Minimal body |
|---|---|---|
| Open an issue | `POST /repos/{owner}/{repo}/issues` | `{"title":"...","body":"...","labels":[],"assignees":[]}` |
| Comment on issue/PR | `POST /repos/{owner}/{repo}/issues/{number}/comments` | `{"body":"..."}` |
| Create or update a file | `PUT /repos/{owner}/{repo}/contents/{path}` | `{"message":"...","content":"<base64>","branch":"main","sha":"<existing-sha-if-updating>"}` |
| Create a branch (git ref) | `POST /repos/{owner}/{repo}/git/refs` | `{"ref":"refs/heads/<name>","sha":"<base-commit-sha>"}` |
| Open a pull request | `POST /repos/{owner}/{repo}/pulls` | `{"title":"...","head":"<branch>","base":"main","body":"..."}` |
| Merge a pull request | `PUT /repos/{owner}/{repo}/pulls/{number}/merge` | `{"merge_method":"squash"}` |
| Create a gist | `POST /gists` | `{"description":"...","public":false,"files":{"a.txt":{"content":"..."}}}` |
| Trigger a workflow | `POST /repos/{owner}/{repo}/actions/workflows/{id}/dispatches` | `{"ref":"main","inputs":{}}` |
| Star a repo | `PUT /user/starred/{owner}/{repo}` | _(empty)_ |

**Updating a file** is read-then-write: `GET .../contents/{path}` to obtain the current
blob `sha`, then `PUT` with that `sha` and the new base64 `content`. Omitting `sha` on an
existing file returns **409 Conflict**.

**Creating a branch** is two steps: `GET /repos/{owner}/{repo}/git/ref/heads/{base}` for the
base commit `sha`, then `POST /git/refs` with `refs/heads/<new-name>`.

## 4. Pagination, rate limits, conditional requests

- **Pagination:** use `per_page` (max **100**) and `page`; follow the `Link` response header
  `rel="next"` until absent. Search endpoints cap at **1000** results total.
- **Rate limits:** authenticated GitHub allows ~**5000 req/hr**. `GET /rate_limit` is free.
  Watch `X-RateLimit-Remaining`; on **403/429** with a `Retry-After` header, back off for
  that many seconds before retrying. Avoid tight polling loops.
- **Conditional requests:** send `If-None-Match: <etag>` to get cheap **304 Not Modified**
  responses that don't count against the limit.

## 5. Scopes & permissions

The real capability ceiling is the **OAuth scopes** granted when the user connected
`api-github`, not the full API. Typical needs:

| To do this | Needs scope |
|---|---|
| Read/write **private** repos & contents | `repo` |
| Trigger/manage Actions | `workflow` |
| Read org membership/teams | `read:org` |
| Create/manage gists | `gist` |
| Delete a repo | `delete_repo` |

If a call fails for lack of scope, **don't work around it** — tell the user which scope is
missing and how to grant it:

```bash
nyxid service add api-github --oauth --scope "repo,workflow"   # re-consent with more scopes
```

…or fall back to the PAT connection (`nyxid service add api-github-pat`, then call slug
`api-github-pat`) when they need scopes their OAuth app can't grant.

## 6. Error handling

| Status | Meaning | What to do |
|---|---|---|
| **401** | NyxID couldn't inject a valid GitHub credential | User must (re)connect `api-github` (`nyxid service add api-github --oauth`) |
| **403** | Missing scope **or** rate limit | Read the message + `X-RateLimit-Remaining`; if scope, see §5; if rate, back off per `Retry-After` |
| **404** | Resource missing **or** token can't see it | GitHub returns 404 for private resources the scope can't reach — verify the path, then suspect a missing scope |
| **409** | Conflict | Stale `sha` on a file update — re-GET the blob `sha` and retry |
| **422** | Validation failed | Inspect `errors[]` (e.g. duplicate ref, invalid `head`/`base`) and fix the body |
| **429** | Secondary rate limit | Back off per `Retry-After`; serialize bursty writes |

## 7. Guardrails

- **Never** ask for, echo, log, or store a GitHub token, and never try to read the stored
  credential back — NyxID owns it.
- GitHub writes are **at-least-once**, not exactly-once. Before creating issues/PRs/comments,
  **search first** (`GET /search/issues?q=...`) to avoid duplicates.
- **Confirm destructive or high-blast-radius actions** with the user before doing them —
  deleting a repo/branch, force-merging, closing or commenting in bulk — unless explicitly
  authorized for that specific action.
- **Report exactly what GitHub returns** (status + message). Never claim success on a
  non-2xx response.
- Act only within the user's granted scopes; when blocked, name the missing scope and the
  `nyxid service add ... --scope` remedy instead of silently failing.
