---
name: firecrawl-via-nyxid
description: Teach an aevatar agent to run Firecrawl web-research/agent jobs through NyxID (submit, poll, then read the result).
version: "1.2"
metadata:
  category: plain
  tag: [firecrawl, web-scraping, web-research, nyxid, aevatar]
---

# Firecrawl via NyxID

This skill teaches an aevatar agent how to drive **Firecrawl** through **NyxID**. NyxID brokers Firecrawl as a connected service and injects the user's Firecrawl API key automatically, so the agent never handles the raw `fc-...` key.

## What Firecrawl is and when to use it

Firecrawl turns the live web into data. It offers:

- **scrape** — fetch one URL as clean markdown/HTML/structured content.
- **crawl** — walk a whole site and return its pages.
- **search** — web search that returns scraped results.
- **map** — discover all URLs on a site.
- **extract** — pull structured data from pages against a schema.
- **agent** — an *autonomous* web-research worker. You give it a natural-language prompt (optionally a list of `urls` to focus on and a JSON `schema` for structured output) and it browses, searches, and extracts on its own. It can work even without a URL.

Reach for Firecrawl whenever you need **fresh web data** or **structured extraction** that the model cannot answer from its own knowledge — current prices, recent events, competitor pages, documentation lookups, or assembling a structured dataset from across the web. The **agent** operation is the most powerful: hand it a question and it figures out where to look.

This skill focuses on the **agent** operation (async submit -> poll -> result), but the same NyxID service slug reaches every Firecrawl endpoint listed above.

## Prerequisite: the owner must connect Firecrawl in NyxID

Firecrawl is brokered as NyxID service slug **`api-firecrawl`** (base `https://api.firecrawl.dev`, bearer auth). Before any agent can call it, the **scope owner** (a human, not the agent) connects it once with their Firecrawl API key:

```
POST /api/v1/keys
{ "service_slug": "api-firecrawl", "credential": "fc-..." }
```

This is a one-time setup step performed by the owner — **the agent does not do this** and never sees the key. After connecting, NyxID injects the key on every brokered request.

**How the agent checks it is available.** Call `nyxid_service_inventory` with `{}` and confirm an
exact connected UserService whose slug is `api-firecrawl`. In a channel sender turn this tool is
list-only and accepts no hand-written ID; elsewhere use only the schema emitted for that request.

Look for an entry whose slug is `api-firecrawl`. If it is **absent**, Firecrawl is not connected yet — the agent cannot call it until the owner connects it (above). Surface that to the user rather than failing silently.

## How to call it

There are three distinct invocation modes. All go through NyxID and inject the key, but their
caller-owned fields are different. Never translate one mode by copying its route fields into
another.

### (a) Interactive request-local operation tools

NyxID publishes an `x-aevatar-tool` OpenAPI overlay that Aevatar discovers for each exact
UserService. The final request's dynamic schema and tool catalog are the authority. Operation tool
names are request-local, do not embed the slug, and require an enumerated `user_service_id`:

- **submit** → `nyxid_service_operation__agent`
- **poll** → `nyxid_service_operation__agent_status` when that exact name is emitted

For the submit operation whose dynamic schema requires `user_service_id` and `body`, call:

```json
{
  "tool": "nyxid_service_operation__agent",
  "arguments": {
    "user_service_id": "us-fc-7",
    "body": {
      "prompt": "Find ACME Cloud pricing tiers",
      "model": "spark-1-mini",
      "maxCredits": 2500
    }
  }
}
```

Pass only fields in the emitted operation schema. Take the returned `id`, then use the emitted
poll operation schema until the status is terminal.

### (b) Raw one-off `nyxid_proxy`

Raw calls own their route. They require the exact connected `service_id`, `slug`, and relative
`path`; method, body, allowed non-sensitive headers, and response mode are optional.

Submit:

```json
{
  "tool": "nyxid_proxy",
  "arguments": {
    "service_id": "us-fc-7",
    "slug": "api-firecrawl",
    "path": "/v2/agent",
    "method": "POST",
    "body": {"prompt": "...", "model": "spark-1-mini"}
  }
}
```

Poll (substitute the `id` returned from submit):

```json
{
  "tool": "nyxid_proxy",
  "arguments": {
    "service_id": "us-fc-7",
    "slug": "api-firecrawl",
    "path": "/v2/agent/<id>",
    "method": "GET"
  }
}
```

The same slug also reaches the other operations for non-agent use — e.g. `path: "/v2/scrape"`, `/v2/crawl`, `/v2/search`, `/v2/map`, `/v2/extract`.

### (c) Compiled workflow admitted operation

The workflow step calls `nyxid_proxy`, but its copied step-level `capability.nyxid_operation` and
server-owned admission proof own the UserService, slug, operation ID, method, path template,
contract digest, schemas, and response policy. Runtime arguments contain only admitted
`path_params`, `query`, `headers`, `body`, and `response_mode`. A compiled submit is:

```yaml
- id: submit
  type: tool_call
  capability:
    nyxid_operation:
      user_service_id: us-fc-7
      operation_id: agent
  parameters:
    tool: nyxid_proxy
    arguments: '{"body":{"prompt":"Find ACME Cloud pricing tiers","model":"spark-1-mini","maxCredits":2500}}'
```

Do not put `service_id`, `user_service_id`, `operation_id`, `slug`, `path`, `method`, a digest, or
schema fields in runtime `arguments`.

### Request and response shapes (the agent operation)

These are the shapes the typed tools and the proxy wrap. Match them exactly.

**Submit** — `POST /v2/agent`:

```json
{
  "prompt": "<string, REQUIRED, <= 10000 chars>",
  "model": "spark-1-mini",            // default, cheaper | "spark-1-pro"
  "urls": ["<string>"],                // optional — focus/constrain the search
  "schema": { "<JSON schema>": "..." },// optional — structured output
  "maxCredits": 2500,                   // optional, default 2500
  "strictConstrainToURLs": false        // optional
}
```

Returns:

```json
{ "success": true, "id": "<jobId, a UUID>", "status": "processing" }
```

**Poll** — `GET /v2/agent/{id}`:

```json
{
  "success": true,
  "status": "processing",        // "processing" | "completed" | "failed" | "cancelled"
  "data": { },                    // present when status == "completed"
  "creditsUsed": 0,
  "expiresAt": "<iso timestamp>"
}
```

Keep polling while `status` is `processing`. Stop on the terminal states `completed` (read `data`), `failed`, or `cancelled`. Results are retained for roughly **24 hours**.

## The async submit → poll pattern

The Firecrawl agent is **asynchronous**: a submit returns immediately with a job `id`, and the job finishes in **minutes — sometimes longer**. You must submit once, then poll the status until it reaches a terminal state.

### In a single agent turn

1. Call the emitted `nyxid_service_operation__agent` or a correctly shaped raw `nyxid_proxy`
   submit with your prompt → get `id`.
2. Call the emitted poll operation or a correctly shaped raw `nyxid_proxy` poll with that `id`.
3. If `status` is `processing`, wait briefly and poll again. When `status` is `completed`, read `data`; on `failed`/`cancelled`, report the failure.

### In an aevatar workflow

Model it as: a `tool_call` submit → a `while` loop that polls with a `delay` between iterations → a `switch` on `status` to a terminal branch. **Do not hand-roll the loop from memory** — the engine's exact step syntax is easy to get wrong. Start from the canonical templates below and adapt them.

For an admitted NyxID operation, copy the exact listed selector into step-level `capability` and
put only current operation values under `parameters.arguments`. Interpolation uses `${...}`, and
you read a JSON field of a prior step's result with `${steps.<id>.json.<field>}`:

```yaml
- id: submit
  type: tool_call
  capability:
    nyxid_operation:
      user_service_id: us-fc-7
      operation_id: agent
  parameters:
    tool: nyxid_proxy
    arguments: '{"body":{"prompt":"Find ACME Cloud pricing tiers","model":"spark-1-mini","maxCredits":2500}}'
  next: poll
# the submitted job id is then available as ${steps.submit.json.id}
```

Other real-DSL facts you will need: branching is `switch` with `parameters: { on: "...", branch.<key>: <step-id> }` plus a `branches:` map; `delay` takes `parameters: { duration_ms: "5000" }` (string value); an admitted poll step puts the returned ID in the exact runtime slot declared by its operation schema, commonly `path_params`. For the **full** submit → poll-loop → branch wiring, reuse the canonical templates rather than rewriting the loop.

### Canonical templates and engine caps

aevatar already ships canonical templates for exactly this pattern, registered in its `WorkflowDefinitionCatalog` — **prefer them** over hand-rolling:

- `firecrawl_agent_async_submit.yaml`
- `firecrawl_agent_async_poll.yaml`

Engine caps to respect: `delay` ≤ **5 minutes** (`duration_ms`), `while` ≤ **1,000,000 iterations**, `wait_signal` ≤ **24 hours**. For **multi-hour** jobs that could exceed a single run's budget, do not block — decompose the wait across runs with `self_reschedule`.

## Cost guidance

- A **submit spends Firecrawl credits** (default cap **2500 per job** via `maxCredits`). **Status polling is free** — poll as needed.
- Prefer **`spark-1-mini`** (the default): it is roughly **60% cheaper** than `spark-1-pro`. Only reach for `spark-1-pro` when a job genuinely needs the stronger model.
- Always set a **sane `maxCredits`** to cap spend per job.

## Quick recap

1. Owner connects `api-firecrawl` once in NyxID (`POST /api/v1/keys`).
2. Agent confirms an exact `api-firecrawl` UserService through the available inventory schema.
3. Select raw, interactive, or compiled mode and use only that mode's fields; submit a natural-language prompt (`model: spark-1-mini`, a `maxCredits` cap, optional `urls`/`schema`) → get `id`.
4. Poll `agent_status` / `GET /v2/agent/{id}` until `status` is terminal; read `data` on `completed`.
5. In workflows, use a `tool_call` submit → `while` + `delay` poll → `switch` on `status`, or reuse the canonical `firecrawl_agent_async_*` templates.
