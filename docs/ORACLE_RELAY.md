# Oracle Relay: call browser LLMs through NyxID

The oracle relay turns a logged-in ChatGPT Pro browser into shared capacity
that any NyxID user or agent can call. A **pool** is one capacity unit. Its
owner runs the supervised CDP worker or the NyxID userscript in one or more
ChatGPT tabs. These clients are the **workers**. Consumers submit prompts
through the NyxID API and poll for answers. They never touch the browser, the
ChatGPT account, or any credential.

NyxID stays a **neutral async task relay**. Nothing in the backend is specific
to ChatGPT. All browser-specific behavior, including prompt injection,
completion detection, answer extraction, and crash recovery, lives in the
worker clients. The pool's `chatgpt_project_url` and `default_model_label` are
opaque hints relayed verbatim to workers.

```
consumer (any NyxID user / nyxid_ag_ agent key)
   │  POST /api/v1/oracle/pools/{slug}/tasks   → task_id
   │  GET  /api/v1/oracle/tasks/{task_id}      (poll, seconds-scale)
   ▼
NyxID backend — MongoDB-backed FIFO queue (no in-memory state, any
   ▲              instance serves any request)
   │  GET  /api/v1/oracle/worker/task?worker=worker_1
   │  POST /api/v1/oracle/worker/{ack,heartbeat,result,pin-conv-url}
   ▼
CDP worker + dedicated Chrome, or ChatGPT tab + userscript
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
| **Worker** | A CDP daemon or userscript tab, identified by a label unique within its pool. New CDP installations also bind the label to a stable installation ID. |
| **Task** | One prompt → one answer (`OracleTask`). Async: submit returns a `task_id`; the answer arrives later. |
| **Session** | A multi-turn conversation (`OracleSession`), addressed by `conversation_id` (`conv_…`). |
| **Worker token** | `nyx_owk_<64 hex>`. Minted at pool creation, rotatable, SHA-256-hashed at rest, shown once. Sent as `Authorization: Bearer`. |
| **Command** | A manager request delivered through the next capable-worker heartbeat. Commands have delivery leases, bounded redelivery, and a terminal result code. |

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

2. **Join a worker machine.** Log in to the NyxID CLI on that machine, then
   run:

   ```bash
   nyxid oracle worker install --pool chatgpt-pro
   ```

   The command asks for the one-time pool worker token with hidden input. You
   can also pass `--worker-token-file`. It verifies Node 18 or newer, npm, and
   Chrome or Chromium. It then allocates a unique worker label (server-generated,
   or your own with `--label share-account-8`; see below), verifies the
   server-embedded bundle checksum, installs the exact `playwright-core`
   version from the bundle manifest, writes mode `0600` config and token files,
   installs a launchd or systemd user service, starts a dedicated Chrome
   profile, and starts the worker. Use `--profile <name>` for a second
   installation of the same pool on one machine.

   **Choosing labels.** Labels are how you address a worker (`worker show`,
   `drain`, `upgrade --label`). Pass `--label <name>` (letters, digits, `-`,
   `_`; max 64) to keep a naming convention such as `share-account-8`. The
   server still guarantees uniqueness: a label already bound to another
   managed installation is refused (error 11014). A label that only a
   legacy worker uses (no installation binding) is **adopted** — the managed
   install takes it over and the legacy process is rejected on its next poll
   with 11014, so unload that legacy worker after the managed one reports
   online. This is the in-place migration path for existing named workers.
   Renaming an existing install (`install --force --label <new>`) leaves the
   old label row bound to this installation; it shows as offline in
   `worker list` until it ages out of interest.

   The installed service uses KeepAlive on macOS or `Restart=always` on Linux.
   Its token stays in a mode `0600` file. The service environment contains only
   the token file path.

3. **Log every worker in from one machine.** Run this on a machine where you
   can complete the ChatGPT login:

   ```bash
   nyxid oracle login chatgpt-pro
   ```

   The command opens a local dedicated Chrome profile for password, OTP, SSO,
   and Cloudflare checks. After login, the CLI captures and encrypts the
   session locally, uploads the sealed envelope, terminates the capture Chrome,
   deletes the temporary profile, then waits for each capable worker to import
   and verify it. See [Pool-wide ChatGPT login](#pool-wide-chatgpt-login) for
   the security model and device-binding limitation.

4. **Verify** worker presence:

   ```bash
   nyxid oracle worker list chatgpt-pro
   ```

The Tampermonkey userscript remains supported without changes. Install
`integrations/oracle/nyxid_oracle.user.js`, then configure a distinct label and
the same pool token. Userscript workers submit tasks but do not receive manager
commands or login snapshots.

### Rotating the token

```bash
nyxid oracle pool rotate-token chatgpt-pro
```

This invalidates the old token immediately. Replace every installed worker
token file and re-paste the token into every userscript. Worker install and
`oracle login` never rotate the token implicitly. The server stores only its
SHA-256 hash, so an existing raw token must come from an installed token file,
`--worker-token-file`, `NYXID_WORKER_TOKEN_FILE`, `NYXID_WORKER_TOKEN`, or the
hidden prompt.

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
has a 16 MiB body cap. The login-snapshot route overrides that limit with a
bound based on the 512 KiB decoded envelope cap.

| Method · Path | Purpose |
|---|---|
| `POST /pools` | Create a pool. Returns the pool + one-time `worker_token`. |
| `GET /pools` | List visible pools (platform + owned + your orgs'). |
| `GET /pools/{id_or_slug}` | Pool detail (`can_manage` reflects the caller). |
| `PATCH /pools/{id_or_slug}` | Update settings (owner / org admin only). |
| `POST /pools/{id_or_slug}/rotate-token` | New worker token, shown once. |
| `GET /pools/{id_or_slug}/workers` | Manager-only worker presence list. |
| `DELETE /pools/{id_or_slug}/workers/{label}?force=` | Manager-only removal of a worker's presence row and command history; releases session affinity owned by the label. Refuses an online worker or one with a task in flight unless `force=true`. |
| `POST /pools/{id_or_slug}/workers/allocate` | Manager-only worker label allocation. Body `{"label": "..."}` requests a specific label; `null`/empty body generates one. Returns `{label, adopted}`. |
| `GET /pools/{id_or_slug}/workers/{label}` | Manager-only worker detail. |
| `GET, POST /pools/{id_or_slug}/workers/{label}/commands` | Manager-only command history and enqueue. |
| `POST /pools/{id_or_slug}/login-snapshots` | Validate and fan out an opaque encrypted login snapshot. |
| `GET /worker-bundle` | Authenticated embedded worker source, version, SHA-256, and exact `playwright-core` version. |
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
`dispatched`, `completed`, `failed`, or `cancelled`. While queued,
`queue_position` is 1-based. Every response carries `attempts`, `retry_count`,
and `max_retries`. A completed task carries `response`. A failed task carries
`failure_reason`, such as `extraction_failure`, `empty_response`,
`prompt_delivery_uncertain`, or `infrastructure_retry_exhausted`.

### Result artifacts

The CDP worker captures generated images and ChatGPT-hosted download links from
the last assistant turn. Generic links are not followed. The privileged,
cookie-bearing fetch accepts only `chatgpt.com` / `chat.openai.com`
`/backend-api/` paths, `*.oaiusercontent.com`, and page-local ChatGPT `blob:`
URLs; every redirect is checked against the same allowlist. File names come
from the link's `download` attribute, anchor text, or URL and are reduced to a
128-character safe basename. A file already captured as an image is omitted
from the generic file list.

The worker captures at most four images and eight files, with a 6 MiB per-item
limit and a 9 MiB decoded-byte budget shared by all artifacts. Nine decoded MiB
base64-inflate to about 12 MiB, leaving headroom under the 16 MiB worker request
limit. The server independently accepts at most eight images and eight files,
caps each item at 8,000,000 bytes, and enforces the same 9 MiB combined budget.
Malformed or over-limit entries are skipped independently. Empty response text
is successful when at least one valid image or file remains.

`nyxid oracle ask --artifacts <dir>` and
`nyxid oracle result <task-id> --artifacts <dir>` save every returned artifact
with collision-safe suffixes. Table output lists each artifact's name, MIME,
and size even when it is not saved. `--output json` includes artifact bytes as
base64. The legacy `--out` option remains image-only. Old workers omit `files`,
and the deployed userscript remains compatible and unchanged.

Artifact bytes live as BSON Binary on the task document and expire with the
prompt and response after `ORACLE_TASK_RETENTION_DAYS`. File bodies and file
names are private task content: logs and audit events contain counts and sizes
only.

### Worker endpoints (pool worker token)

Under `/api/v1/oracle/worker`, each handler authenticates
`Authorization: Bearer nyx_owk_...`. These routes mount outside the JWT
middleware, like `/api/v1/node-agent`. Results can carry multi-megabyte
answers, so this router has a 16 MiB body cap. Existing request and response
fields remain valid. New fields are additive.

| Method · Path | Body / Query | Response |
|---|---|---|
| `GET /task` | `?worker=worker_1&script_version=&page_url=&instance_id=` | Idle response, or a task with retry counters, optional `dispatch_attempt_id`, prompt, attachment, conversation URL, and project hint. |
| `POST /heartbeat` | Presence, capabilities, health, current task, and command reports | `{status:"ok", command?}`. The server leases at most one capability-compatible command. |
| `POST /ack` | Task identity, phase, optional `instance_id` and `dispatch_attempt_id` | `{status:"ok"}` or `{status:"cancelled"}`. |
| `POST /result` | Task identity, response, optional images/files and attempt fences | `{status:"saved"\|"saved_failed"\|"requeued"\|"ignored"}`. `requeued` means a pre-send browser failure consumed an infrastructure retry. |
| `POST /pin-conv-url` | Task identity, URL, optional attempt fences | `{status:"pinned"}`. |
| `POST /transcript` | Task identity, turns, URL, optional attempt fences | `{status:"imported"\|"ignored", imported_pairs}`. |
| `GET /bundle` | None | Worker-token-authenticated embedded worker source, version, SHA-256, and exact `playwright-core` version. |
| `GET /login-snapshots/{snapshot_id}` | None | The still end-to-end-sealed login envelope for the authenticated pool. |

A `task` poll carries `kind` (`"prompt"`, `"scrape"`, or `"extract"`): on
`"scrape"` the worker navigates to `conversation_url`, extracts the full
transcript, and POSTs `/transcript` instead of injecting a prompt; on
`"extract"` it navigates to an arbitrary `target_url` and POSTs the page's
readable main text back as the `/result` (see the SSRF note under Security).

`ack` doubles as the cancellation back-channel. An acknowledgement for a task
that was cancelled or replaced returns `{status:"cancelled"}`. The worker then
abandons that attempt and polls again.

Legacy userscripts omit `instance_id`, `dispatch_attempt_id`, capabilities,
and heartbeats. The server preserves their old claim, acknowledgement, result,
pin, and transcript behavior. It sends commands only to workers that advertise
the command's required capability, so legacy and unknown clients never receive
a protocol shape they cannot parse.

---

## Queue semantics

- **FIFO per pool** uses MongoDB `find_one_and_update` with a `created_at`
  sort. There is no in-memory queue, so any backend instance can serve a poll.
- **Lease and heartbeat** give a claimed task a `task_timeout_secs` lease.
  The default is four hours. `ack` refreshes the lease.
- **Bounded infrastructure retry** requeues an expired lease at the front by
  preserving `created_at`. The lease expiry increments `retry_count`. New and
  legacy task rows default to `max_retries = 3`; older rows deserialize without
  a migration. A worker can also report `browser_recovery_exhausted` before it
  attempts prompt delivery. The server requeues that task immediately and uses
  the same retry budget. If the next retry would exceed the budget, the server
  marks the task failed with `infrastructure_retry_exhausted` and starts its
  retention TTL. Model failures and content failures do not use this budget.
- **Visible attempts** increment `attempt_count` on each fresh dispatch.
  Idempotent reclaims by the current worker do not increment it.
- **Idempotent reclaim** returns a worker's current leased task, including the
  known ChatGPT conversation URL. This lets the CDP worker recover after a
  process restart, Chrome restart, or tab replacement.
- **Attempt fencing** gives each fresh dispatch a `dispatch_attempt_id`.
  Capable workers echo it on acknowledgements, results, conversation pinning,
  and transcript settlement. Traffic from a replaced attempt is ignored.
  Stable installation IDs also stop two new CDP installations from sharing one
  label. Legacy clients omit both fences and retain the previous behavior.
- **Quotas**: `max_queue_length` caps queued tasks per pool (`429`
  `oracle_queue_full`); `per_user_max_inflight` caps queued+dispatched per
  submitter (`429` `oracle_quota_exceeded`); `max_workers` caps concurrent
  dispatch.
- **Idempotency**: a submit carrying a `client_ref` already used by the
  same submitter returns the original task instead of enqueuing a
  duplicate.
- **Extraction-failure detection**: an empty or `ERROR:`-prefixed worker
  result marks the task `failed`, mirroring the local oracle servers.
- **Retention**: terminal tasks (prompt, response, generated image, and
  generated file bodies and filenames) are TTL-expired after
  `ORACLE_TASK_RETENTION_DAYS` (default 30). Queued/dispatched tasks are never
  auto-expired.

---

## Worker recovery

The CDP worker keeps a mode `0600` JSON state file. It contains the stable
installation ID, current task ID, dispatch attempt ID, conversation URL,
phase, and transcript baseline. It never contains a prompt, response,
transcript, cookie, storage value, signed image URL, generated file body, or
generated filename.

HTTP failures use capped exponential backoff with jitter. Transient fetch,
timeout, rate-limit, and server failures do not terminate the process. The
worker tracks consecutive HTTP, CDP, tab, and task-recovery failures. It
reconnects over CDP, replaces a crashed or navigated-away tab, restores the
project URL, and relaunches Chrome with the configured executable, debug port,
and profile after repeated failures. Task recovery stops after a bounded count.
A pre-send failure reports `browser_recovery_exhausted` for server retry. A
post-send failure reports `prompt_delivery_uncertain` and cannot trigger a
prompt replay.

Prompt delivery uses a conservative recovery rule:

1. Before clicking Send, the worker records the transcript baseline and writes
   `send_attempted` to disk.
2. After recovery, it reclaims the same task and opens the known conversation.
3. If the matching user turn has a completed assistant turn, the worker
   extracts and submits that answer.
4. If the matching user turn is present but still running, the worker waits.
5. If the task was still in a pre-send phase, the worker sends it once.
6. If `send_attempted` was durable but the transcript cannot prove delivery,
   the worker fails with `prompt_delivery_uncertain`. It never guesses by
   sending the prompt again.

This rule prefers an explicit failed task over a duplicate user message in an
existing conversation. The server's bounded lease retry handles a worker that
does not recover before its lease expires.

The supervised worker keeps the Chrome debug port fixed because launchd or
systemd pins both `CHROME_CDP_URL` and `CHROME_DEBUG_PORT`. Silently probing a
fallback inside the worker would split runtime state from persisted supervisor
configuration and could attach to an unrelated local browser. A later port
collision appears as `Chrome: no` in `nyxid oracle worker list <pool>` and as a
sanitized connection error in `worker show`. Close the dedicated Chrome window
if it is still running without CDP, then run
`nyxid oracle worker install --force --pool <pool>` with the original CLI
`--profile` when applicable. Forced installation retains the label, token, and
browser profile; it keeps the existing port when a live Chrome still serves
CDP there (so a running, logged-in Chrome is never stranded) and selects a new
free port only when the endpoint is dead or squatted, then rewrites the paired
settings and restarts supervision.

## Worker presence and control

`nyxid oracle worker list <pool>` shows the worker label, bundle version,
online or last-seen status, login state, current task, Chrome state, and desired
state. `worker show <pool> <label>` also shows the sanitized last error,
platform, and recent command results.

Managers can queue these commands:

| CLI command | Worker behavior |
|---|---|
| `worker drain <pool> <label>` | Finish the current task, then stop claiming. |
| `worker resume <pool> <label>` | Resume claims. |
| `worker restart <pool> <label>` | Finish the current task, report, then exit for supervisor restart. |
| `worker relaunch-browser <pool> <label>` | Recreate the dedicated Chrome process and tab. |
| `worker forget <pool> <label> [--force]` | Remove a stale worker from `worker list` (presence + commands; releases its session affinity). Live or busy workers are refused without `--force`; a live worker re-registers on its next heartbeat anyway. |
| `worker relogin <pool> <label>` | Open the ChatGPT login page on the worker's own screen (someone at that machine must finish it). For remote login from your computer use `oracle login`, which pushes the session to the pool. While logged out or on a login page the worker leaves its tab untouched and claims no tasks. |
| `worker upgrade --pool <pool>` | Upgrade the installed local profile. The CLI waits for task drain, verifies the local source, version, dependency manifest, and restarted worker presence. |
| `worker upgrade --pool <pool> --label <label>` | Queue an asynchronous remote upgrade. The worker drains, verifies, replaces the bundle, and exits for supervisor restart. |

The server audits the actor, pool ID, worker label, command ID, command kind,
and result metadata. It never audits command payload bodies or session
material. A command has a 60-second delivery lease, at most 10 deliveries, a
24-hour deadline, and seven-day terminal retention. The worker journals command
IDs before side effects, so redelivery returns the stored result.

Local service controls mirror `nyxid node daemon`:

```bash
nyxid oracle worker start|stop|status|logs|uninstall --pool <pool> [--profile <name>]
```

The default install is under `~/.nyxid-oracle/<pool>/`. Named CLI profiles use
`~/.nyxid-oracle/<pool>/profiles/<profile>/`. `uninstall` removes the launchd
or systemd service but retains the worker files, Chrome profile, and token.

## Pool-wide ChatGPT login

`nyxid oracle login <pool>` performs the human login only on the CLI machine.
It does not ask the user to visit each worker.

1. The CLI obtains the existing raw pool worker token from a local install,
   `--worker-token-file`, an environment variable, or hidden input. It never
   rotates the shared token.
2. The CLI installs the checksummed capture worker and opens a dedicated local
   Chrome profile. Password, OTP, SSO, and Cloudflare interaction stay local.
3. After the DOM verifies authentication, the capture worker collects cookies
   for ChatGPT and OpenAI domains plus allowlisted local and session storage for
   `https://chatgpt.com` and `https://auth.openai.com`.
4. The CLI derives a 256-bit key with HKDF-SHA256 from the raw worker token and
   a random 32-byte salt. It encrypts the capture with AES-256-GCM and protocol
   AAD. Plaintext is capped at 350 KiB; the sealed envelope is capped at 512
   KiB.
5. The manager endpoint compares the supplied token SHA-256 with the pool hash,
   then stores only the sealed envelope. `EncryptionKeys` adds the normal
   server-side envelope encryption. The row expires after one hour.
6. The server queues `session_import` only for workers that advertise both
   `commands_v1` and `session_import_v1`.
7. A worker with an active task defers the import until settlement. If the task
   cannot proceed because the worker is logged out, it imports immediately and
   then reclaims the task. The worker decrypts locally, injects only allowlisted
   cookies and storage through CDP, reloads ChatGPT, and reports a stable result
   code after DOM verification.
8. The CLI polls each command and prints one result per worker. The command
   exits unsuccessfully if any target does not verify, expires, or times out.

After the upload, the CLI terminates the capture Chrome and deletes its entire
temporary workspace, including the profile and plaintext capture file. Error
paths use the same cleanup guard.

The server's persisted state cannot derive the session key because it stores
only the worker-token hash. The backend does see the raw bearer token while it
authenticates live worker requests, so this design does not protect against a
malicious live backend process. It protects session plaintext at rest, in
MongoDB, in audit records, and in logs.

ChatGPT can bind a session cookie to device or risk context. An import that the
site rejects reports `session_import_verification_failed`; it never reports a
successful login based only on cookie injection. The worker list continues to
show that worker as logged out.

## Bundle distribution and trust

The backend embeds `integrations/oracle/cdp-worker/worker.mjs` at compile time.
Its version is the backend package version plus the first 12 characters of the
source SHA-256. Both the manager and worker-token endpoints return the source,
version, full SHA-256, and an exact `playwright-core` version.

The CLI trusts its authenticated NyxID base URL and TLS connection to select
the bundle, then verifies the returned bytes against the returned SHA-256. It
also checks that the manager and worker-token endpoints agree. A pushed upgrade
downloads through the worker-token endpoint and verifies SHA-256. If the
installed `playwright-core` already matches the manifest pin (the common,
bundle-only upgrade) it skips npm entirely; otherwise it runs the install-time
absolute npm (`NYXID_NPM_EXECUTABLE`, with node's directory on the daemon
`PATH`, since launchd/systemd start the worker with a minimal one) with a
five-minute timeout, and restores `package.json` if that fails
(`upgrade_npm_unavailable`, `upgrade_dependency_install_failed`,
`upgrade_dependency_install_timeout`, `upgrade_dependency_version_mismatch`).
It then replaces `worker.mjs` and exits; the supervisor starts the new source.
npm validates the registry package integrity when it runs. The bundle checksum detects transport
or storage corruption. Neither check is an independent code-signing authority
beyond the NyxID backend, the npm registry, and TLS.

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
- Login captures exist as plaintext only in a mode `0600` temporary file on
  the CLI machine and in zeroizing CLI memory. The CLI deletes the temporary
  profile and file after upload, including on error paths. The server accepts
  only the end-to-end-sealed envelope. Audit and tracing record the snapshot ID, byte
  count, target count, and outcome codes. They never record the sealed blob,
  cookies, storage, raw token, prompt, response, transcript, conversation URL,
  attachment filename, signed image URL, generated file body, or generated
  filename.
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
| 11011 | `oracle_worker_not_found` | 404 |
| 11012 | `oracle_worker_capability_unsupported` | 409 |
| 11013 | `oracle_worker_command_not_found` | 404 |
| 11014 | `oracle_worker_label_unavailable` | 409 |
| 11015 | `oracle_login_snapshot_not_found` | 404 |

---

## Compatibility

All added MongoDB fields use serde defaults or optional fields. Existing
`oracle_pool`, `oracle_task`, `oracle_session`, and `oracle_worker` rows remain
valid. The new `oracle_worker_commands` and `oracle_login_snapshots` collections
use UUID-string `_id` values and TTL indexes for terminal or expired rows.

The deployed userscript remains unchanged. The server accepts requests without
installation IDs or attempt IDs, preserves the legacy acknowledgement shape,
and omits commands unless the worker advertises a matching capability. The CDP
worker and userscript can continue to share one pool.

## Relationship to the local oracle servers

The relay generalizes the local `bedc_oracle_server.py` / `oracle_server.py`
bridges (Python HTTP server on loopback + Tampermonkey userscript) into a
hosted, multi-tenant, authenticated service. The userscript at
`integrations/oracle/nyxid_oracle.user.js` is a direct fork of the
bedc-deep bridge: the DOM-automation core is verbatim; only the config +
networking layer was retargeted from `http://localhost:8767` (no auth) to
the NyxID worker API over HTTPS with a Bearer worker token. The CDP worker uses
the same extraction rules but adds durable recovery, supervised Chrome, health
presence, commands, upgrades, and login import. Existing local pipelines can
migrate by pointing their consumer at `/api/v1/oracle` instead of the local
server. The submit and poll shapes remain close to the local servers.
