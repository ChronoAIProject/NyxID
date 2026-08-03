---
name: aevatar-workflow-authoring
description: Author, preview, validate, and persist an executable aevatar workflow from a natural-language request. Use it when the user wants to create, build, set up, or automate a multi-step task as a runnable Aevatar workflow. It covers exact NyxID operation and authored-request admission, bounded YAML, file inputs, terminal run verification, and reusable publication. Not for blindly rerunning an existing failed workflow.
version: "2.2"
metadata:
  category: tool-based
  tool-list:
    - nyxid_services
    - list_external_workflow_capabilities
    - inspect_external_workflow_capability_readiness
    - aevatar_start_workflow
    - ornn_publish_skill
  tag:
    - workflow
    - authoring
    - automation
    - aevatar
    - create-workflow
    - pipeline
---

# Authoring an executable aevatar workflow

You turn a user's natural-language request into a **valid, test-run, reusable** aevatar workflow. A workflow is a YAML document of `roles` + `steps` that the engine executes; once validated you persist it as a skill so the user can re-run it and watch it in the observatory.

Everything you need is in this document — the DSL, the engine rules, the tools, and worked examples. Follow the protocol in order.

> **Two execution surfaces — know which one you are *before* step 3.** Steps 3 / 5 / 6 below call the *server-side agent tools* `nyxid_services`, `aevatar_start_workflow`, and `ornn_publish_skill`. Those exist **only** when you are the model running **inside** an aevatar session with the nyxid MCP connected. If instead you are an external **client** holding only a NyxID bearer token — driving the aevatar backend through the NyxID broker (`nyxid proxy request aevatar`), the same identity the sibling skills (`aevatar-team-builder`, `aevatar-service-publisher`, `aevatar-scheduler`) assume — **those three tools are not callable**, and you dry-run + publish over plain authenticated REST instead. Jump to **[Client path (no nyxid MCP)](#client-path-no-nyxid-mcp--dry-run--publish-over-rest)** at the end; the DSL, engine rules, and examples in between apply to both surfaces.

---

## Protocol (follow in order)

1. **Confirm the intent is authoring.** The user wants a *new* runnable workflow. If they want to run something that already exists, stop and search for it instead.
2. **Clarify just enough.** Pin down: the trigger/input, the ordered steps, the desired output, and which external services (if any) are involved. Ask only what you cannot reasonably infer; do not over-interrogate.
3. **Select the exact external capability (only if external calls are needed).** Prefer a published operation: call `list_external_workflow_capabilities`, copy its exact selector, then inspect readiness for `interactive` or `durable`. Use typed `capability.nyxid_request` only when no published operation represents a required HTTP request; it needs an exact UserService selected from authoritative `/api/v1/keys`, a typed method/path/body contract, and bind-time authenticated confirmation/grant. `/api/v1/user-services` is only a routing projection and is never execution authority.
4. **Author the YAML.** Apply the DSL below and obey every rule in **Engine rules (must obey)**. Prefer the reliable-core primitives; use advanced primitives only when the task truly needs them.
5. **Preview before execution.** Use the explicit-request preview or draft-run readiness surface. Confirm unique call sites, read/write classification, approval requirements, exact workflow/revision identity, and every blocker. Preview/readiness proves admission readiness only; it does not prove runtime credential propagation or downstream authorization.
6. **Run once when authorized, then observe the same run.** Never use another mutation as a status check. For a real acceptance run, require terminal completion, `lastSuccess=true`, non-empty required step outputs, and non-empty final output. On failure, read run detail and audit, find the first failed step, and diagnose before any rerun. Do not blindly retry a mutation or a failed workflow.
7. **Persist as a reusable workflow.** After structural validation and any required execution proof, call `ornn_publish_skill` with the final workflow in `workflow_yamls` (see **Persisting**). This creates a private skill in the user's account containing the workflow.
8. **Report evidence honestly.** Separate preview-ready, accepted, completed, business-verified, blocked by policy, and failed. Do not call `accepted` or an opened SSE stream a successful run.
9. **Iterate on request.** To change an existing workflow: load it with `use_skill`, edit the YAML, repeat preview and validation, and publish a new version.

> Keep tool rounds bounded, but do not trade away terminal evidence. Start one authorized run, record its identity privately, and observe that same run through the run/read-model surfaces.

---

## Engine rules (must obey)

These are the failure modes that break generated workflows. Check every one before validating.

- **Single terminal step.** A run ends at the step that has no `next` **and is last in document order**. Make the final step the last line of the document.
- **Bounded YAML.** Each YAML document is limited to 1 MiB UTF-8, 10,000 parsed nodes, and collection nesting depth 64. Aliases do not bypass these limits. Treat a resource-limit rejection as a document defect; reduce the document instead of retrying it.
- **Fall-through is by document order, not id order.** A step with no `next` falls through to the *next step written in the file*. So every branch must reach the terminal step via an explicit `next`, and nothing should sit after the terminal step. Getting this wrong silently overwrites your output.
- **No clock.** The engine has no time source. If the workflow needs "today", a date, or a window, the caller must inject it via the run input (e.g. an early `assign`). Never assume the engine knows the date.
- **Role is not model.** `target_role` selects the actor, not a model — never put a model name in `target_role`. A role *may* carry `provider`/`model`, but set them only when the user explicitly wants a specific model; otherwise omit and let the session default apply.
- **`parameters` values are strings.** Bare words are read as strings (`op: trim`); quote anything numeric or boolean so it stays a string (`n: "50"`, `max_iterations: "5"`).
- **Determinism for money/counts/dedup.** Use `transform` (`sum`, `group_by`, `round`, …) for any arithmetic, totals, or deduplication. Never let an `llm_call` compute amounts or counts.
- **Side effects are at-least-once.** `tool_call` / `connector_call` may run more than once on retry. Keep them idempotent where it matters.
- **External calls go through tools, not raw hosts.** Use `nyxid_proxy` (or a typed tool) — never embed a vendor base URL as a direct target. See **Accessing external services**.
- **Files are typed inputs.** `input_file_refs` is not `$input` text and not an interpolation variable. Use `foreach` with `items_source: input_file_refs` to process multiple files; file tools are still invoked through `type: tool_call`.

---

## DSL quick reference

### Top-level shape

```yaml
name: my_workflow            # identifier
description: what it does     # optional
roles: [ ... ]               # actors
steps: [ ... ]               # ordered execution
```

### Roles

```yaml
roles:
  - id: analyst                       # referenced by step.target_role
    name: Analyst                     # optional display name
    system_prompt: "You are a strict analyst."
    # optional, usually omit and inherit session defaults:
    # provider: openai
    # temperature: "0.2"
    # allowed_tools: [web_search]     # ceiling of agent tools this role can see; [] = none
    # connectors: [my_api]            # whitelist for connector_call
```

`agent_kind` defaults to `workflow.role-agent`. Omit `model` (see Engine rules). `allowed_tools: []` means the role exposes no agent tools.

### Step shape

```yaml
steps:
  - id: step_a                 # unique within the workflow
    type: llm_call             # primitive type (see table)
    target_role: analyst       # which role runs it (alias: role); some types need none
    parameters:                # all values are strings
      prompt_prefix: "Analyze:"
    next: step_b               # explicit successor; omit only on the final step
    branches:                  # for conditional/switch/vote: branch key -> step id
      true: step_b
      false: step_c
    # compensation: undo_step  # only tool_call/connector_call/secure_connector_call
    # allowed_tools: [web_search]  # only llm_call: narrow tool scope (intersection with role)
```

### Reliable-core primitives (prefer these)

`llm_call` — run the target role's LLM.
```yaml
- id: analyze
  type: llm_call
  target_role: analyst
  parameters: { prompt_prefix: "Summarize the input:" }
```

`tool_call` — call a registered tool (incl. `nyxid_proxy`, `code_execute`, `document_extract`, `workflow_file_submit`). A JSON-object result is mirrored to `steps.<id>.json.<field>` for later branching.
```yaml
- id: fetch
  type: tool_call
  capability:
    nyxid_operation:
      user_service_id: <copied-user-service-id>
      operation_id: <listed-operation-id>
  parameters:
    tool: nyxid_proxy
    arguments: '{"query":{}}'
```

`code_execute` — run deterministic Python/JavaScript/TypeScript/Bash in the sandbox.
Do not call external services or LLMs from `code_execute`; use `nyxid_proxy` for external services and `llm_call` for LLM work.
```yaml
- id: build_payload
  type: tool_call
  parameters:
    tool: code_execute
    arguments: '{"language":"python","code":"import json\nprint(json.dumps({\"ok\": True}))"}'
```

`document_extract` — extract text from one current workflow file ref.
```yaml
- id: extract_file
  type: tool_call
  parameters:
    tool: document_extract
    arguments: "{}"
```

`workflow_file_submit` — upload an existing workflow file ref to a NyxID service.
```yaml
- id: submit_file
  type: tool_call
  parameters:
    tool: workflow_file_submit
    arguments: '{"file_ref":{"file_id":"<file-id>","owner_run_id":"<run-id>"},"slug":"my-upload-service","path":"/v1/upload"}'
```

`transform` — deterministic data ops: `trim`, `split`, `json_extract`, `json_parse`, and numeric `sum`/`subtract`/`multiply`/`divide`/`round`/`min`/`max`/`group_by`, plus `rss_extract_items`.
```yaml
- id: total
  type: transform
  parameters: { op: group_by, key: category, value: amount, aggregate: sum, precision: "2" }
```

`json_parse` — parse a JSON string selected by `path` into structured JSON.
```yaml
- id: parse_embedded_json
  type: transform
  parameters:
    op: json_parse
    path: "$.payload"
```

`assign` — write a workflow variable (often the final output step).
```yaml
- id: finalize
  type: assign
  parameters: { target: final_summary, value: "$input" }
```

`conditional` — two-way branch; set `branches.true`/`branches.false`.
`switch` — multi-way branch on a value; set `parameters.branch.<key>` and `branches`, include `_default`.
```yaml
- id: route
  type: switch
  parameters:
    on: "${steps.classify.json.category}"
    branch.urgent: handle_urgent
    branch._default: handle_normal
  branches: { urgent: handle_urgent, _default: handle_normal }
```

`foreach` — split input by delimiter, run a sub-step per item, merge.
```yaml
- id: per_item
  type: foreach
  parameters:
    delimiter: "\n"
    sub_step_type: llm_call
    sub_target_role: worker
    sub_param_prompt_prefix: "Process item:"
```

For multiple workflow input files, use `items_source: input_file_refs`; each child step receives exactly one current file ref.
```yaml
- id: extract_each_file
  type: foreach
  parameters:
    items_source: input_file_refs
    sub_step_type: tool_call
    sub_param_tool: document_extract
    sub_param_arguments: "{}"
```

Keep `items_source`, `sub_step_type`, and `sub_param_tool` under `parameters`; root-level `items_source` / `sub_param_tool` are not reliably lifted by the parser. Use `sub_param_arguments: "{}"` when the tool should read the per-item file ref instead of treating the file id input as arguments.

### Parallelism: concurrent fan-out → merge

Yes — the engine runs branches **concurrently**, and `foreach` / `parallel` / `map_reduce` / `race` are the primitives that express it. Each one dispatches its sub-steps **in parallel** (by default up to **20** at once, hard ceiling **200**; set `max_concurrent_workers` to change the cap and `min_concurrent_workers` for a steady-state floor). The parent step then waits for **all** sub-steps and merges their text outputs joined by `\n---\n`. That fan-out → fan-in *is* the aevatar equivalent of a tool like n8n where many source branches feed one merge node.

There is no free-form DAG: you do **not** hand-draw N parallel branches into a merge node. Instead you pick the primitive whose built-in fan-out matches your shape:

| You want… | Use | How each branch is fed |
|---|---|---|
| One input, run by **N workers** concurrently, then optionally vote a winner | `parallel` | every worker gets the **same** `$input` |
| A **list of items**, run the **same** sub-step on each, then concatenate | `foreach` | the input is **split into items**; each sub-step gets a **different** one |
| A list of items, process each, then **synthesize all into one** result | `map_reduce` | split → map each (different item) → reduce the merged outputs once |
| N alternative attempts, take the **first to finish** | `race` | every branch gets the **same** `$input`; first success wins, the rest are discarded |

The n8n "read N sources in parallel → merge" shape is a **list → fan-out → merge**, so it is `foreach` (concatenate the per-item results) or `map_reduce` (synthesize them into one) — **not** `parallel`. `parallel` and `race` feed the *same* input to every branch (ensemble / consensus / first-wins), not a different source per branch.

**Where the item list comes from** (`foreach` / `map_reduce`): the previous step's output, split by `delimiter` (default `\n---\n`) or parsed as a **JSON array**; or an explicit `items:` list; or `items_source: input_file_refs` (one file per item). Produce that list upstream — the run input, an `assign`, or `transform op: rss_extract_items`.

`parallel` — fan one input out to N `llm_call` workers, merge (optionally vote). Sub-steps are **always** `llm_call`; it needs either `workers` (distinct roles) or `target_role` + `parallel_count`.
```yaml
- id: critique
  type: parallel
  parameters:
    workers: "reviewer_a,reviewer_b,reviewer_c"   # one llm_call per role; each gets the SAME $input
    # parallel_count: "3"            # alternative: N copies of target_role instead of distinct workers
    # max_concurrent_workers: "20"   # default 20, ceiling 200
    # vote_step_type: vote           # optional: aggregate the N outputs via a consensus rule (see the vote primitive)
  next: finalize
```

`map_reduce` — split into items, map each concurrently, then reduce the merged results into one output. The map phase carries **no** per-step parameters, so map is best for `llm_call` analysis driven by the map role's `system_prompt`; `reduce_prompt_prefix` is prepended to the merged outputs before the reduce step.
```yaml
- id: analyze_all
  type: map_reduce
  parameters:
    delimiter: "\n---\n"             # how the input splits into items (or pass a JSON array)
    map_step_type: llm_call
    map_target_role: analyst         # analyzes every item concurrently — the fan-out
    reduce_step_type: llm_call
    reduce_target_role: synthesizer  # runs ONCE on the merged map outputs — the fan-in
    reduce_prompt_prefix: "Synthesize these analyses into one brief:"
  next: finalize
```
Omit the `reduce_*` fields and you get just the merged map outputs (no synthesis) — that is equivalent to `foreach`.

### Full primitive vocabulary (use advanced ones only when needed)

| Group | Types |
|---|---|
| AI | `llm_call`, `tool_call`, `evaluate` (score+threshold), `reflect` |
| Data | `transform`, `assign`, `retrieve_facts`, `cache` |
| Control | `guard`/`assert`, `conditional`, `switch`, `while`/`loop`, `delay`/`sleep`, `lease`/`mutex`, `wait_signal`, `checkpoint` |
| Composition | `foreach`, `parallel`/`fan_out`, `race`, `map_reduce`, `workflow_call`, `dynamic_workflow`, `vote` |
| Integration | `connector_call` (aliases: `http_get`, `http_post`, `http_put`, `http_delete`, `mcp_call`, `cli_call`), `emit`/`publish` |
| Human | `human_input`, `human_approval`, `wait_signal` |

Advanced notes: `human_approval`/`wait_signal` suspend the run until a resume/signal event — use them for approvals and long external waits instead of stretching a step past its 600s executor limit. `parallel`/`foreach`/`map_reduce` accept `min_concurrent_workers`/`max_concurrent_workers` (see **Parallelism: concurrent fan-out → merge** above for which one to pick and the concurrency defaults). Side-effecting steps may declare `compensation: <step_id>` for saga rollback.

### Interpolation

- `$input` — the current step's input (the previous step's output, or — for the FIRST step — the run prompt). This is how a value flows step-to-step.
- `${steps.<id>.output}` — a prior step's text output. **It is `.output`, NOT `.text`.** The engine registers `steps.<id>.output` and never `steps.<id>.text`, so `${steps.<id>.text}` silently resolves to an empty string — the run still shows every step "completed", but a tool/connector downstream receives an empty argument and fails.
- `${<name>}` — a workflow variable written by an `assign` step (`target: <name>`). This is the canonical way to read a captured value back in a later step; `${steps.<capture-id>.text}` does NOT work (use the bare `${<name>}`, or equivalently `${steps.<capture-id>.output}`).
- `${steps.<id>.json.<field>}` — a field from a prior step whose output was a JSON object (e.g. a `tool_call` result). Also: `${steps.<id>.success}`, `${steps.<id>.error}`, `${steps.<id>.annotations.<ns>.<key>}`.
- Expression functions (usable in any value, incl. `condition`): `if`, `concat`, `isblank`, `length`, `not`, `and`, `or`, `upper`, `lower`, `trim`, `json`, `add`, `sub`, `mul`, `div`, `eq`, `lt`, `lte`, `gt`. **There is no `contains`/substring function.**

> **Gotchas that silently break runs (verified against the engine — a clean test run does NOT catch these, because failed tool calls return their error as ordinary step output):**
> - **`${steps.<id>.text}` is always empty — use `${steps.<id>.output}`.** This is the #1 cause of "every step completed but the connector got an empty argument."
> - **Read an `assign`ed value with the bare `${<target>}`**, not `${steps.<capture-id>.text}`.
> - **`transform op: split` joins all parts with `\n---\n` and ignores any `index`** — it is for fan-out, not single-element extraction. To use one segment of `a/b` (e.g. an `owner/repo` in a path), pass the whole string where the `/` is already correct rather than splitting it apart.
> - **`conditional.condition`** is interpolated first; if the result is not literally `true`/`false`, the engine does a substring `$input.Contains(condition)`. Since there is no `contains` function, build "any/all contain token" checks around this: `concat` the inputs into one string in the prior step, then set `condition` to the literal token.
> - **`parallel` (and `race`) feed every branch the *same* `$input`** and each sub-step is always an `llm_call`. For *different* input per branch — the "N different sources" shape — split a list with `foreach` / `map_reduce` instead. All four merge sub-step outputs with `\n---\n`. `map_reduce`'s map sub-steps receive **no** per-step parameters (drive them via the map role's `system_prompt`); only `foreach` passes `sub_param_*` to each child, so per-item `tool_call` fetches must use `foreach`.
> - **`switch.on` that resolves to empty silently takes the WRONG branch.** `switch` evaluates `on` first; if it resolves to empty — e.g. `${steps.read.json.off_count}` when that field is absent, which is exactly what happens when the prior tool returned an *error envelope* (`{"error":true,"status":503,...}`) instead of the expected object — `switch` **falls back to matching the step's whole `$input`**, and matching is exact → **substring-contains** → `_default`. So a 503 error body substring-matches a branch key `"0"` (via `"503"`/`"8001"`) and quietly routes as if the value were `0`. **Always make `on` a clean, never-empty token:** `on: "${eq(steps.read.json.off_count, '0')}"` with branches `"true"` / `"false"` / `_default`. Never branch on a raw count/field that can go missing.
> - **A side-effecting workflow MUST read back and assert the real end state — a step "success" does NOT mean the side effect happened.** Because a failed tool call returns its error as ordinary step output (above), an action step like `turn_on` / `post` / `create` completes "successfully" even when nothing changed (e.g. the downstream returned 503). Pattern: after the action, add a `tool_call` that **reads the real state** (have the upstream API compute a scalar you can branch on — e.g. an HA `/template` returning `{"off_count":N}`), `switch` on `${eq(<count>,'0')}`; on the not-yet-satisfied branch `delay` + re-read for a bounded retry, and on final failure route to a `guard` with `on_fail: fail` so the **run actually fails** (red in the observatory) instead of a green run that did nothing. Do not trust `${steps.<action>.success}`.

---

## Accessing external services

There is a raw current-turn proxy surface, two compiled workflow capability shapes, and a separate host-connector subsystem. Do not mix their fields.

- **Raw current-turn `nyxid_proxy`.** First select an exact UserService instance. The call requires `service_id + slug + path`; method defaults to GET. Optional fields are `body`, non-sensitive `headers`, and `response_mode`:
  ```json
  {
    "service_id": "<selected-user-service-id>",
    "slug": "<service-slug>",
    "path": "/v1/resource",
    "method": "POST",
    "body": {"k": "v"}
  }
  ```
  This is a direct current-turn tool call, not workflow YAML.
- **No dynamic current-turn operation tools.** `nyxid_service_operation__*` and `nyxid_service_request` are retired. Never invent or cache one of those tool names. Current-turn management uses the typed connected-service tools; raw one-off HTTP uses `nyxid_proxy` only where that surface is explicitly available.
- **Compiled published operation.** Call `list_external_workflow_capabilities`, copy the exact descriptor `selector`, inspect it for the required execution mode, and persist it beside the step as `capability.nyxid_operation`:
  ```json
  {
    "selector": {
      "nyx_id_operation": {
        "user_service_id": "us-lark-7",
        "operation_id": "get-message"
      }
    },
    "execution_mode": "interactive"
  }
  ```
  Pass that whole object to `inspect_external_workflow_capability_readiness`; `nyx_id_operation` is required and must not be flattened.
  ```yaml
  - id: fetch_message
    type: tool_call
    capability:
      nyxid_operation:
        user_service_id: us-lark-7
        operation_id: get-message
    parameters:
      tool: nyxid_proxy
      arguments: '{"path_params":{"message_id":"m-42"}}'
  ```
  The compiler's admission proof owns UserService identity, slug, operation ID, method, path template, digest, schemas, and response policy. Runtime `arguments` may contain only admitted `path_params`, `query`, `headers`, `body`, and `response_mode`. Do not send `service_id`, `user_service_id`, `slug`, `path`, `method`, `operation_id`, or a contract digest in `arguments`.
- **Compiled authored HTTP request.** Use `capability.nyxid_request` only for an exact request contract that cannot be selected as a published operation. Author the typed method, relative path template, allowed request fields, and response policy; select one exact UserService from `/api/v1/keys`. Binding must preview the current request-contract digest and risk, then obtain the authenticated binder's explicit confirmation/grant. Saving a draft, preview success, or `/api/v1/user-services` cannot grant execution. Authored requests never rediscover inventory or OpenAPI at runtime; they execute only the committed proof and matching grant.
- **Registered workflow connectors.** If the capability is a connector registered in the workflow connector registry, call it with `connector_call` and authorize it on the role:
  ```yaml
  roles:
    - id: caller
      name: Caller
      connectors: [my_connector]
  steps:
    - id: call
      type: connector_call
      target_role: caller
      parameters: { connector: "my_connector", operation: "list", path: "/v1/items", timeout_ms: "10000" }
  ```
- **`allowed_tools` gotcha.** A role with no `allowed_tools` sees the full inherited tool catalog (including `nyxid_proxy`). But the moment you set `allowed_tools` on a role, you **must** list every tool its steps call (e.g. `allowed_tools: [nyxid_proxy]`) — otherwise the `tool_call` will not resolve the tool at run time.
- **Prefer a typed tool when one exists** for the capability (they expose stable control fields and validation) over a hand-built proxy path.
- **Missing service** → degrade gracefully (skip that source) or stop and ask the user to connect it. Never fabricate a slug or connector.

---

## Validating and proving a run

Dispatch **one** test run with `aevatar_start_workflow`, passing the draft inline:

```json
{ "workflow_id": "<name>", "workflow_yamls": ["<full yaml>"], "inputs": { "prompt": "<test input>" } }
```

`workflow_id` is required; `inputs` is an object (typically `{ "prompt": "..." }`, optionally `input_parts` / `headers`). File-bearing calls may carry normalized `inlineFile` or `fileRef` parts; member stream invocation and draft-run share the same ingress normalization, so preserve the typed file part instead of flattening it into prompt text.

Interpret evidence in order:
- `accepted`/`streaming` means only that dispatch started.
- A typed parse, admission, identity, or resource-limit error is a structural failure; fix the document before another run.
- For the same run, inspect terminal completion, run detail, and audit. Success requires completed terminal state, `lastSuccess=true`, expected non-empty step output, and non-empty final output.
- On failure, locate the first failed audit step and classify it before retrying. For NyxID HTTP auth failures, distinguish missing caller credential propagation, missing executor propagation, wrong exact route/credential selection, and downstream UserService authorization.
- Never start another run merely to check whether the previous run finished.

---

## Persisting (make it reusable)

Once the draft dispatches without a parse error, publish a private skill that carries the workflow:

```json
{
  "name": "<kebab-case-workflow-name>",
  "description": "<one line: what it does>",
  "version": "1.0",
  "category": "runtime-based",
  "instructions_markdown": "Runs the <name> workflow. Invoke with use_skill then aevatar_start_workflow; inputs: <list>.",
  "workflow_yamls": [ { "workflow_id": "<name>", "content": "<full yaml>" } ]
}
```

Choose a clear `name`/`description` so the user (and future searches) can find it. Publishing is private by default; the user can later promote it to public on the platform.

**Re-run later:** the user (or their model) loads it with `use_skill("<name>")` — which mounts the workflow into their scope — then calls `aevatar_start_workflow` with `workflow_id: "<name>"`. The run goes through the normal engine path and is visible in the observatory.

---

## Client path (no nyxid MCP) — dry-run + publish over REST

Use this whole section when you hold a **NyxID bearer token** but the server-side tools
(`aevatar_start_workflow` / `ornn_publish_skill` / `use_skill` / `nyxid_services`) are **not** in
your tool list. Everything here is plain authenticated REST against the same control-plane base
the sibling aevatar skills use. (All of it is verified live; none of it requires reading aevatar
source.)

### Bootstrap
```bash
# Drive the aevatar backend THROUGH the NyxID broker: it injects your scope_id claim AND
# auto-refreshes your token. A raw curl with ~/.nyxid/access_token resolves NO scope
# (scopeResolved:false) and the stored token expires — it is not a usable path.
# Prerequisite once: the `aevatar` service must be connected — `nyxid service add aevatar`.
aev() { nyxid proxy request aevatar "$@"; }   # aev "<path>" [-m POST|PUT|DELETE] [-d '<json>'] [--stream]
scopeId=$(aev "api/studio/context" | jq -r .scopeId)
```
No `jq`? Any JSON reader works, e.g.
`... | python3 -c 'import sys,json;print(json.load(sys.stdin)["scopeId"])'`.

### Connectors
The `nyxid_services` inventory tool is server-side. As a client, discover exact connected-service
instances through the NyxID CLI or the authenticated Aevatar preview surface. `/api/v1/keys` is the
authoritative instance/readiness inventory; `/api/v1/user-services` is not. Never invent a slug,
UserService identity, endpoint, method, or path.

### Dry-run (the client replacement for `aevatar_start_workflow`) — `draft-run`
`aevatar_start_workflow` is a **server-side agent tool dispatched through the engine, not a REST
endpoint** — a client cannot call it. The client dry-run is the **draft-run** endpoint, which
takes the YAML inline (long runs stream, so the broker holds the connection open):
```bash
aev "api/scopes/$scopeId/workflow/draft-run" -m POST --stream \
  -d "$(python3 -c 'import json;print(json.dumps({"prompt":"<test input>","workflowYamls":[open("workflow.yaml").read()]}))')"
```
Body (JSON, **camelCase**): `prompt` (string) + `workflowYamls` (array of YAML strings,
**required** — omitting it returns 400). The response is an **SSE stream** and the run executes
synchronously through the connection. Judge it like the server-side validate step:
- **HTTP 200 + lifecycle frames** proves only dispatch. For execution acceptance, continue to the terminal frame and read the resulting run detail/audit.
- A parse/validation/4xx error → fix the YAML and retry (cap **2**).

**Reading the SSE frames** (so a naive parser doesn't see "nothing"): each `data:` line is JSON,
and two kinds interleave — there is **no flat `type` field**:
- *Lifecycle*, keyed by a top-level field: `{"stepStarted":{"stepName":…}}`,
  `{"stepFinished":{"stepName":…}}`, `{"usage":{…}}`, `{"runFinished":{…}}`,
  `{"stateSnapshot":{…}}`. A matched `stepStarted`+`stepFinished` per step proves each step ran;
  `runFinished` marks the end.
- *Raw observation*: `{"custom":{"name":"aevatar.raw.observed",…}}` — these carry the actual step
  **output text** (search recursively under `output` / `content`).

`draft-run` is **not** observable in `/workflow/observatory` (it is a throwaway validation run).
For an observable run, publish + invoke (below / sibling skills).

### Publish the workflow skill to ornn (the client replacement for `ornn_publish_skill`) — REST zip
`ornn_publish_skill` is also server-side. The client publishes a **zip** through the nyxid proxy
(slug **`ornn-api`**, not `ornn`). Build this exact layout — a **root folder**, `SKILL.md` at the
root, and the workflow YAML under **`assets/`** (the validator **rejects a `workflows/` root dir**):
```
demo-skill/
  SKILL.md
  assets/
    my_workflow.yaml      # top-level `name:` + `steps:` → auto-extracted; its `name` is the workflow id
```
The platform's extractor scans `assets/*.{yaml,yml}` and treats any YAML having **both** a
top-level `name` and `steps` as a runnable workflow whose `workflow_id` equals that `name`.

`SKILL.md` frontmatter **must nest under `metadata:`** (flat top-level `category`/`output_type`/
`tool_list` is rejected). A workflow skill is **`category: mixed`** with these **kebab-case** keys —
all three are required for `mixed` (and for `runtime-based`):
```yaml
---
name: demo-skill
description: <one line — what it does>
version: "1.0"
metadata:
  category: mixed
  output-type: text                 # required for mixed / runtime-based
  runtime:                          # required; MUST be a YAML array — a bare string is rejected
    - aevatar-workflow              # the workflow runtime (NOT node/python)
  tool-list:                        # required for mixed
    - aevatar_start_workflow
  tag: [demo, workflow, aevatar]    # singular `tag`, ≤10
---
```
Then **validate first** (the format oracle — read every `violations[].rule`/`message` and fix),
then **upload** (re-uploading the **same `name`** later creates a **new version**):
```bash
cd <parent>; zip -r demo-skill.zip demo-skill                 # root folder MUST be included
# 1) validate → {"data":{"valid":bool,"violations":[{"rule","message"}]}}
nyxid proxy request ornn-api /api/v1/skill-format/validate -m POST \
  -H 'Content-Type:application/zip' -d @demo-skill.zip
# 2) publish (private by default; promote to public separately)
nyxid proxy request ornn-api /api/v1/skills -m POST \
  -H 'Content-Type:application/zip' -d @demo-skill.zip
# verify
nyxid proxy request ornn-api /api/v1/skills/demo-skill
```
The server normalizes the kebab frontmatter into its stored model
(`runtimes:[{runtime,dependencies,envs}]`, `tools:[{tool,type:mcp}]`, `outputType`).

### Run a published workflow skill as a client
The `use_skill` → `aevatar_start_workflow` mount path is server-side. As a client you take the
control-plane route instead: bind the workflow to a **team member**, then invoke the published
service. Binding a member **is** publishing a service; its `chat:stream` invoke runs the workflow
and shows in the observatory. See `aevatar-team-builder` then `aevatar-service-publisher`.

---

## Worked examples (generic — adapt, don't copy verbatim)

### A. Linear LLM chain

```yaml
name: summarize_then_title
roles:
  - id: writer
    system_prompt: "You are a concise writer."
steps:
  - id: summarize
    type: llm_call
    target_role: writer
    parameters: { prompt_prefix: "Summarize the input in 3 bullets:" }
    next: make_title
  - id: make_title
    type: llm_call
    target_role: writer
    parameters: { prompt_prefix: "Write a one-line title for this summary:" }
```
`make_title` is last and has no `next` → it is the single terminal step.

### B. Fetch → classify → branch → converge (single terminal)

```yaml
name: fetch_and_route
roles:
  - id: analyst
    system_prompt: "You classify and draft responses."
steps:
  - id: fetch
    type: tool_call
    capability:
      nyxid_operation:
        user_service_id: <copied-user-service-id>
        operation_id: <listed-operation-id>
    parameters:
      tool: nyxid_proxy
      arguments: '{"query":{}}'
    next: classify
  - id: classify
    type: llm_call
    target_role: analyst
    parameters: { prompt_prefix: "Reply with one word, 'urgent' or 'normal':" }
    next: route
  - id: route
    type: switch
    parameters:
      on: "$input"
      branch.urgent: handle_urgent
      branch.normal: handle_normal
      branch._default: handle_normal   # fallback for unexpected output
    branches: { urgent: handle_urgent, normal: handle_normal, _default: handle_normal }
  - id: handle_urgent
    type: llm_call
    target_role: analyst
    parameters: { prompt_prefix: "Draft an urgent response:" }
    next: finalize
  - id: handle_normal
    type: llm_call
    target_role: analyst
    parameters: { prompt_prefix: "Draft a standard response:" }
    next: finalize
  - id: finalize
    type: assign
    parameters: { target: final_summary, value: "$input" }
```
Both branches converge to `finalize` via explicit `next`; `finalize` is last → single terminal. (No step sits after it, so nothing fall-through-overwrites the output.)

### C. Per-item processing (foreach)

```yaml
name: process_each_line
roles:
  - id: worker
    system_prompt: "You process one item."
steps:
  - id: per_item
    type: foreach
    parameters:
      delimiter: "\n"
      sub_step_type: llm_call
      sub_target_role: worker
      sub_param_prompt_prefix: "Process this item:"
    next: collect
  - id: collect
    type: assign
    parameters: { target: final_summary, value: "$input" }
```

### D. Multiple files → extract → upload files

```yaml
name: extract_files_then_upload_files
description: Extract text from multiple uploaded files, submit each original file to a NyxID upload service, and return a structured upload summary.
steps:
  - id: extract_each_file
    type: foreach
    parameters:
      items_source: input_file_refs
      sub_step_type: tool_call
      sub_param_tool: document_extract
      sub_param_arguments: '{"maxChars":2000}'
    next: build_submit_requests

  - id: build_submit_requests
    type: tool_call
    parameters:
      tool: code_execute
      arguments:
        language: javascript
        code: |
          const raw = "${json(json(input))}";
          const requests = [];
          const fileRefKeys = [
            "file_id",
            "artifact_id",
            "source_kind",
            "source_message_id",
            "source_resource_key",
            "owner_run_id",
            "owner_scope_id"
          ];

          for (const [index, part] of raw.split("\n---\n").filter(part => part.trim()).entries()) {
            const extracted = JSON.parse(part);
            const file = extracted.file || {};
            const fileRef = {};
            for (const key of fileRefKeys) {
              if (file[key]) fileRef[key] = file[key];
            }

            const itemIndex = index + 1;
            requests.push({
              file_ref: fileRef,
              slug: "<upload-service-slug>",
              path: "<upload-endpoint-path>",
              method: "POST",
              file_field_name: "<file-form-field-name>",
              form: {
                file_name: file.file_name || "workflow-upload-" + itemIndex + ".bin",
                size: file.size_bytes ? String(file.size_bytes) : "",
                source: "multiple-files-" + itemIndex
              },
              output: {
                kind: "provider_file_token",
                selector: "<response-json-path-for-upload-token>"
              },
              max_file_bytes: 31457280
            });
          }

          console.log(JSON.stringify(requests));
    next: parse_submit_requests

  - id: parse_submit_requests
    type: transform
    parameters:
      op: json_parse
      path: output.stdout
    next: submit_each_file

  - id: submit_each_file
    type: foreach
    parameters:
      sub_step_type: tool_call
      sub_param_tool: workflow_file_submit
    next: build_upload_summary

  - id: build_upload_summary
    type: tool_call
    parameters:
      tool: code_execute
      arguments:
        language: javascript
        code: |
          const raw = "${json(json(input))}";
          const uploads = [];
          const submitResults = [];

          for (const part of raw.split("\n---\n").filter(item => item.trim())) {
            const submitted = JSON.parse(part);
            submitResults.push(submitted);
            if (submitted.output_code) {
              uploads.push({
                file_name: submitted.file && submitted.file.file_name ? submitted.file.file_name : "",
                output_code: submitted.output_code,
                output_kind: submitted.output_kind || ""
              });
            }
          }

          const summary = {
            upload_count: uploads.length,
            uploads,
            submit_results: submitResults
          };

          console.log(JSON.stringify(summary));
    next: parse_upload_summary

  - id: parse_upload_summary
    type: transform
    parameters:
      op: json_parse
      path: output.stdout
    next: finish

  - id: finish
    type: assign
    parameters:
      target: final_summary
      value: '{"run_tag":"multiple-files-upload","uploaded_files":${steps.parse_upload_summary.output}}'
```
`extract_each_file` produces one JSON result per file. `build_submit_requests` projects only the stable workflow file-ref identity keys into `file_ref`; keep display metadata such as file name and size in `form` only when the upload service needs it. `parse_submit_requests` turns `code_execute` stdout into a JSON array so `submit_each_file` can upload each original file with `workflow_file_submit`. `build_upload_summary` collects returned provider codes without writing to any vendor-specific record system. Replace the upload service slug, endpoint path, file field name, and response selector before validation.

### E. code_execute → json_parse

```yaml
name: code_execute_then_parse
steps:
  - id: build_json
    type: tool_call
    parameters:
      tool: code_execute
      arguments:
        language: javascript
        code: |
          console.log(JSON.stringify({"route":"approved","score":91}));
    next: parse_stdout
  - id: parse_stdout
    type: transform
    parameters:
      op: json_parse
      path: output.stdout
    next: route
  - id: route
    type: switch
    parameters:
      on: "${steps.parse_stdout.json.route}"
      branch.approved: finalize
      branch._default: finalize
    branches: { approved: finalize, _default: finalize }
  - id: finalize
    type: assign
    parameters: { target: final_summary, value: "${steps.parse_stdout.output}" }
```
`code_execute` returns a sandbox envelope; the business JSON is a string at `output.stdout`. `json_parse` promotes that string to structured output so later steps can read `steps.parse_stdout.json.route`.

### F. Fan-out in parallel → merge (the n8n "multiple sources → merge" shape)

```yaml
name: sources_digest
roles:
  - id: analyst
    system_prompt: "Summarize one source's content into 3 bullet highlights."
  - id: editor
    system_prompt: "Merge per-source highlights into one digest, deduping overlaps."
steps:
  # The N sources arrive as ONE list — a JSON array, or a "\n---\n"-delimited string —
  # produced upstream (the run input, an assign, or transform op: rss_extract_items).
  - id: digest
    type: map_reduce
    parameters:
      delimiter: "\n---\n"
      map_step_type: llm_call
      map_target_role: analyst         # every source analyzed concurrently — the fan-out
      reduce_step_type: llm_call
      reduce_target_role: editor        # one merge over all results — the fan-in
      reduce_prompt_prefix: "Combine these per-source highlights into one digest:"
```
One `map_reduce` step is n8n's "N source branches → merge node": the map phase analyzes every source concurrently (default ≤20 at once), the reduce phase merges them into one result. Want the per-source outputs concatenated with **no** synthesis? Use `foreach` (Example C) and drop the reduce. Need to **fetch** each source first (each item is e.g. a feed URL or file)? Do that with a `foreach` of `sub_step_type: tool_call` — its `sub_param_*` give each fetch its tool + arguments — then pipe the fetched text into this `map_reduce` to analyze-and-merge. (Don't use `map_reduce` for the fetch: its map sub-steps get no per-step parameters.)

---

## Self-check before publishing

- [ ] Final step is last in the document and has no `next`; every branch reaches it via explicit `next`.
- [ ] Any date/time the logic needs is injected via input, not assumed.
- [ ] No hardcoded `model:` unless the user demanded one.
- [ ] Arithmetic / totals / dedup use `transform`, not `llm_call`.
- [ ] Every workflow external call copied an exact listed selector, passed readiness for its execution mode, and puts only runtime operation values in `nyxid_proxy.arguments`; raw current-turn proxy calls use exact `service_id + slug + path`.
- [ ] One `aevatar_start_workflow` dispatch returned a `run_id` with no parse error — you did **not** wait for/poll `run_finished` (the run finishes async; report the `run_id` + observatory).
- [ ] Any parallel fan-out uses the right primitive: same input → `parallel` / `race`; a list of different items → `foreach` (concatenate) or `map_reduce` (synthesize). Per-item `tool_call` fetches use `foreach`, not `map_reduce`.
