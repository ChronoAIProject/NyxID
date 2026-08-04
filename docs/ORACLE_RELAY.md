# Oracle Relay — Call ChatGPT Pro (and other browser LLMs) through NyxID

The oracle relay turns a logged-in ChatGPT Pro browser tab into shared
capacity that any NyxID user or agent can call. A **pool** is one such
capacity unit; its owner runs the NyxID oracle userscript in one or more
ChatGPT tabs (the **workers**). Consumers submit prompts through the NyxID
API and poll for answers — they never touch the browser, the ChatGPT
account, or any credential.

NyxID itself stays a **neutral async task relay**: nothing in the backend
is specific to ChatGPT. All browser/LLM-specific behavior (prompt
injection, completion detection, answer extraction) lives in the
userscript. The pool's `chatgpt_project_url` and `default_model_label` are
opaque hints relayed verbatim to workers.

```
consumer (any NyxID user / nyxid_ag_ agent key)
   │  POST /api/v1/oracle/pools/{slug}/tasks   → task_id
   │  GET  /api/v1/oracle/tasks/{task_id}      (poll, seconds-scale)
   ▼
NyxID backend — MongoDB-backed FIFO queue (no in-memory state, any
   ▲              instance serves any request)
   │  GET  /api/v1/oracle/worker/task?worker=tab_1   (30s poll, Bearer nyx_owk_…)
   │  POST /api/v1/oracle/worker/{ack,result,pin-conv-url}
   ▼
ChatGPT Pro tab + NyxID oracle userscript
```

Why route a browser LLM through NyxID instead of an API key: ChatGPT Pro
(o-series deep reasoning, long thinking) has no comparable API tier. The
relay lets a pipeline, a cloud worker, or a teammate consume that capacity
with NyxID's auth, per-agent rate limiting, audit attribution, and quotas
for free.

---

## Concepts

| Term | Meaning |
|---|---|
| **Pool** | A capacity unit owned by a user or org (`OraclePool`). Holds the worker token, visibility, quotas, and optional project/model hints. |
| **Worker** | A ChatGPT tab running the userscript, identified by a per-tab label (`tab_1`, `tab_2`, …). Authenticates with the pool worker token. |
| **Task** | One prompt → one answer (`OracleTask`). Async: submit returns a `task_id`; the answer arrives later. |
| **Session** | A multi-turn conversation (`OracleSession`), addressed by `conversation_id` (`conv_…`). |
| **Worker token** | `nyx_owk_<64 hex>`. Minted at pool creation, rotatable, SHA-256-hashed at rest, shown once. Sent as `Authorization: Bearer`. |

### Visibility

A pool's `visibility` controls who may submit:

- `private` (default) — only the owner (or, for an org-owned pool, an org admin).
- `org` — any member of the owning org. Only valid for org-owned pools.
- `platform` — any authenticated user on the instance. This is the
  "anyone can call NyxID to use Pro" setting.

Management (update settings, rotate token) is always restricted to the
owner or an org admin, regardless of visibility.

---

## Quickstart (pool owner)

You have ChatGPT Pro and want to share it.

1. **Create a pool** and capture the one-time worker token:

   ```bash
   nyxid oracle pool create chatgpt-pro \
     --name "ChatGPT Pro" \
     --visibility platform \
     --model chatgpt-5.5-pro
   # → prints a worker token: nyx_owk_…
   ```

   Optional: pin workers to a ChatGPT Project (carries system instructions
   / attached files) with `--project-url https://chatgpt.com/g/g-p-…/project`.

2. **Connect a ChatGPT browser** — two options, same pool:

   **Option A — CDP worker (recommended, lower friction).** Drives your
   real logged-in Chrome over the DevTools protocol as a background daemon;
   no extension, no tab to babysit. See
   `integrations/oracle/cdp-worker/README.md`:

   ```bash
   cd integrations/oracle/cdp-worker && npm install
   ./start-chrome.sh                       # launches Chrome on a debug port; log into ChatGPT once
   NYXID_BASE_URL=https://auth.nyxid.dev \
   NYXID_WORKER_TOKEN=nyx_owk_… \
   node worker.mjs
   ```

   **Option B — userscript (zero local process).** Install
   `integrations/oracle/nyxid_oracle.user.js` in Tampermonkey, open
   chatgpt.com (logged into Pro), click ⚙ in the NyxID Oracle panel, set
   the NyxID base URL + worker token + label (`tab_1`, or open with
   `?nyx=1`), and click **Start**. Open more tabs with `?nyx=2`, `?nyx=3`,
   … (up to `max_workers`) for capacity.

   Both speak the same worker API; pick whichever fits. The CDP worker
   drives your real Chrome session (lowest setup friction, low detection);
   the userscript needs nothing installed locally.

4. **Verify** the tab is seen:

   ```bash
   nyxid oracle status chatgpt-pro
   ```

### Rotating the token

```bash
nyxid oracle pool rotate-token chatgpt-pro
```

Invalidates the old token immediately; re-paste the new one into every
tab's settings.

---

## Quickstart (consumer)

You have a NyxID account or an agent API key and want to ask Pro a question.

```bash
# One-shot, wait for the answer (answer prints to stdout):
nyxid oracle ask chatgpt-pro "Prove that the BEDC closure of item 8 is well-defined."

# From a file, with a PDF attached:
nyxid oracle ask chatgpt-pro --file prompt.txt --pdf paper.pdf

# Fire-and-forget, fetch later:
TASK=$(nyxid oracle ask chatgpt-pro "…" --no-wait --output json | jq -r .task_id)
nyxid oracle result "$TASK"

# Multi-turn:
nyxid oracle ask chatgpt-pro "First question" --new-conversation
# note the conv_… id from the output, then:
nyxid oracle ask chatgpt-pro "Follow-up" --conversation conv_abc123…

# Attach an EXISTING conversation by URL (a worker tab must have access):
nyxid oracle attach chatgpt-pro https://chatgpt.com/c/<uuid>
# scrapes the whole transcript into a conv_… session, then:
nyxid oracle session conv_abc123…                     # read the imported history
nyxid oracle ask chatgpt-pro "Keep going" --conversation conv_abc123…  # write back into it
```

### Attaching an existing conversation

`oracle attach` is the bidirectional bridge: instead of NyxID originating
the chat, you point it at a conversation you already have in the browser.
A worker tab navigates to the URL, scrapes every user/assistant turn, and
NyxID imports them as a normal session (`origin: "imported"`). From then
on the conversation is first-class — read it with `oracle session`,
continue it with `oracle ask --conversation`. Each scraped
(user, assistant) pair becomes a completed turn, so the transcript and
continue flows work unchanged. The worker must be in a tab that can open
the URL; if the pool pins a ChatGPT Project, attaching conversations
inside that project works best.

Agents authenticate with a scoped key instead of a session:

```bash
NYXID_ACCESS_TOKEN=nyxid_ag_… nyxid oracle ask chatgpt-pro "…"
```

Because `oracle ask` prints only the answer to stdout (status goes to
stderr), it composes in pipelines:

```bash
nyxid oracle ask chatgpt-pro --file q.md | tee answer.md
```

---

## HTTP API

### Consumer endpoints (JWT or `nyxid_ag_` API key)

All under `/api/v1/oracle`. Submits accept a base64 PDF, so this router
has a 16 MiB body cap.

| Method · Path | Purpose |
|---|---|
| `POST /pools` | Create a pool. Returns the pool + one-time `worker_token`. |
| `GET /pools` | List visible pools (platform + owned + your orgs'). |
| `GET /pools/{id_or_slug}` | Pool detail (`can_manage` reflects the caller). |
| `PATCH /pools/{id_or_slug}` | Update settings (owner / org admin only). |
| `POST /pools/{id_or_slug}/rotate-token` | New worker token, shown once. |
| `POST /pools/{id_or_slug}/tasks` | Submit a task. Returns `task_id` + `queue_position`. |
| `POST /pools/{id_or_slug}/attach` | Attach an existing conversation by `{chatgpt_url, tag?}`. Returns `conversation_id` + `task_id` (a `scrape` task). |
| `GET /pools/{id_or_slug}/status` | Queue depth + active workers. |
| `GET /tasks/{task_id}` | Poll a task. Terminal `status` carries `response`. |
| `POST /tasks/{task_id}/cancel` | Cancel a queued/in-flight task. |
| `GET /sessions[?pool=&limit=]` | Your conversations. |
| `GET /sessions/{conversation_id}` | Transcript (turns with prompts + answers). |
| `POST /sessions/{conversation_id}/close` | Block further turns. |

**Submit body** (`POST …/tasks`):

```json
{
  "prompt": "…",                 // required
  "model": "chatgpt-5.5-pro",    // optional; defaults to the pool's
  "tag": "bedc-deep",            // optional
  "conversation_id": "",         // omit = single-shot; "" = open session; id = continue
  "pdf_base64": "…",             // optional; worker uploads on turn 1
  "pdf_name": "paper.pdf",       // required if pdf_base64 set
  "client_ref": "retry-key-1"    // optional submitter-scoped idempotency key
}
```

**Task poll** (`GET /tasks/{id}`): `status` is one of `queued`,
`dispatched`, `completed`, `failed`, `cancelled`. While `queued`,
`queue_position` is 1-based. `completed` carries `response`; `failed`
carries `failure_reason` (`extraction_failure` / `empty_response`).

### Worker endpoints (pool worker token)

Under `/api/v1/oracle/worker`, authenticated by `Authorization: Bearer
nyx_owk_…` **inside each handler** — these mount outside the JWT
middleware (like `/api/v1/node-agent`). Results can carry multi-MB
answers: 16 MiB body cap. The wire format mirrors the local oracle servers
the relay replaces, so porting their userscript is a thin diff.

| Method · Path | Body / Query | Response |
|---|---|---|
| `GET /task` | `?worker=tab_1&script_version=&page_url=` | `{status:"idle", required_project_url?}` or `{status:"task", task_id, prompt, conversation_id?, conversation_url?, is_followup, model?, tag?, pdf_base64?, pdf_name?, required_project_url?, assigned_worker, submitted_at}` |
| `POST /ack` | `{task_id, worker, phase?, phase_detail?, script_version?, page_url?}` | `{status:"ok"}` or `{status:"cancelled"}` |
| `POST /result` | `{task_id, worker, response, chatgpt_url?, model?, script_version?}` | `{status:"saved"\|"saved_failed"\|"ignored"}` |
| `POST /pin-conv-url` | `{task_id, worker, chatgpt_url}` | `{status:"pinned"}` |
| `POST /transcript` | `{task_id, worker, turns:[{role,text}], chatgpt_url?}` | `{status:"imported"\|"ignored", imported_pairs}` |

A `task` poll carries `kind` (`"prompt"`, `"scrape"`, or `"extract"`): on
`"scrape"` the worker navigates to `conversation_url`, extracts the full
transcript, and POSTs `/transcript` instead of injecting a prompt; on
`"extract"` it navigates to an arbitrary `target_url` and POSTs the page's
readable main text back as the `/result` (see the SSRF note under Security).

`ack` doubles as the cancellation back-channel: a heartbeat for a task
that's been cancelled or reclaimed returns `{status:"cancelled"}`, telling
the worker to abandon it and re-poll.

---

## Queue semantics

- **FIFO per pool**, backed by MongoDB (`find_one_and_update` with a
  `created_at` sort) — no in-memory queue, so any backend instance serves
  any poll and there's no sticky routing.
- **Lease + heartbeat**: a claimed task gets a lease of
  `task_timeout_secs` (default 4 h — browser deep-reasoning is slow).
  `ack` heartbeats refresh it. A lease that expires (dead tab) is requeued
  **to the front** on the next claim (preserved `created_at`), the Mongo
  analogue of the local servers' `appendleft`.
- **Idempotent re-claim**: a worker polling while it already holds a task
  gets the same task back — this is what lets a tab survive ChatGPT's
  mid-task full-page reload.
- **Quotas**: `max_queue_length` caps queued tasks per pool (`429`
  `oracle_queue_full`); `per_user_max_inflight` caps queued+dispatched per
  submitter (`429` `oracle_quota_exceeded`); `max_workers` caps concurrent
  dispatch.
- **Idempotency**: a submit carrying a `client_ref` already used by the
  same submitter returns the original task instead of enqueuing a
  duplicate.
- **Extraction-failure detection**: an empty or `ERROR:`-prefixed worker
  result marks the task `failed`, mirroring the local oracle servers.
- **Retention**: terminal tasks (prompt + response bodies) are TTL-expired
  after `ORACLE_TASK_RETENTION_DAYS` (default 30). Queued/dispatched tasks
  are never auto-expired.

---

## Security & privacy

- Worker tokens are 32-byte random values; only SHA-256 hashes are stored;
  the raw token is shown once at create/rotate. Deactivating a pool
  (`--active false`) detaches all workers immediately.
- Worker endpoints are reachable by anyone holding the token, so the token
  is the pool's trust boundary — treat it like a node auth token.
- Consumer access is gated by visibility ACL + per-API-key rate limiting +
  `allowed_service_ids`-style scoping on agent keys.
- **Prompt and response bodies live only on the task document** (and are
  TTL-expired). Audit events and tracing are **metadata-only** — task id,
  pool id, sizes, outcomes — never the prompt or the answer, matching the
  WS-frame-injection logging discipline.
- The browser side runs under the operator's own logged-in session with
  the default User-Agent; the userscript does not spoof or evade. Routing a
  browser-automation bridge through a shared service changes the *consumer*
  transport only — be mindful of the upstream provider's terms when
  widening `visibility` to `platform`.
- **`extract` (read any web page) is an SSRF-shaped primitive — opt-in per
  pool, off by default.** Because the worker fetches `target_url` inside the
  operator's real browser (on its private network, with its cookies), an
  unrestricted `extract` on a `platform` pool would let any authenticated
  submitter read internal dashboards, cloud-metadata
  (`169.254.169.254`), and other private-network services and get the text
  back. Three layers contain this: (1) the pool's `allow_extract` flag must be
  explicitly enabled by the owner (default `false`, gated with
  `oracle_extract_disabled` / **11010**); (2) the server-side
  `validate_extract_url` rejects non-`http(s)`, credentialed URLs, and
  loopback/private/link-local/ULA/CGNAT/metadata hosts (literal IPs and an
  internal-name denylist); (3) the worker re-resolves the host at navigation
  time and refuses any non-public address, closing the DNS-rebinding gap the
  server can't see. Only enable `allow_extract` on pools whose submitters you
  trust with that blast radius.

---

## Error codes

Oracle errors occupy the **11000–11099** block (see
`backend/src/errors/mod.rs`):

| Code | Variant | HTTP |
|---|---|---|
| 11000 | `oracle_pool_not_found` | 404 |
| 11001 | `oracle_pool_slug_taken` | 409 |
| 11002 | `oracle_pool_inactive` | 503 |
| 11003 | `oracle_worker_token_invalid` | 401 |
| 11004 | `oracle_queue_full` | 429 |
| 11005 | `oracle_quota_exceeded` | 429 |
| 11006 | `oracle_task_not_found` | 404 |
| 11007 | `oracle_session_not_found` | 404 |
| 11008 | `oracle_session_closed` | 409 |
| 11009 | `oracle_payload_too_large` | 413 |
| 11010 | `oracle_extract_disabled` | 403 |

---

## Relationship to the local oracle servers

The relay generalizes the local `bedc_oracle_server.py` / `oracle_server.py`
bridges (Python HTTP server on loopback + Tampermonkey userscript) into a
hosted, multi-tenant, authenticated service. The userscript at
`integrations/oracle/nyxid_oracle.user.js` is a direct fork of the
bedc-deep bridge: the DOM-automation core is verbatim; only the config +
networking layer was retargeted from `http://localhost:8767` (no auth) to
the NyxID worker API over HTTPS with a Bearer worker token. Existing local
pipelines can migrate by pointing their consumer at `/api/v1/oracle`
instead of the local server — the submit/poll shapes are nearly identical.
