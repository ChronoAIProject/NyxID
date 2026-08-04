# Client REST path

Read this reference only when the caller has the NyxID CLI or bearer-backed broker path but does not have the server-side `aevatar_start_workflow`, `ornn_publish_skill`, `use_skill`, or `nyxid_services` tools.

## Contents

- Bootstrap and capability discovery
- Draft-run validation
- Publishing a workflow skill to Ornn
- Running a published workflow

## Bootstrap and capability discovery

Drive Aevatar through the NyxID broker so the request receives the caller's `scope_id` and token refresh behavior:

```bash
aev() { nyxid proxy request aevatar "$@"; }
scopeId=$(aev "api/studio/context" | jq -r .scopeId)
```

The `aevatar` service must already be connected. Do not send the stored NyxID access token directly to the Aevatar backend: that route can authenticate while resolving no scope. `jq` is optional; use any structured JSON reader.

The `nyxid_services` tool is server-side. As a client, discover exact connected-service instances through the NyxID CLI or the authenticated Aevatar preview surface. `/api/v1/keys` is the authoritative UserService instance/readiness inventory; `/api/v1/user-services` is only a routing projection. Never invent a slug, UserService identity, endpoint, method, or path.

## Draft-run validation

`aevatar_start_workflow` is not an HTTP endpoint. Validate client-authored YAML with the scoped draft-run endpoint:

```bash
aev "api/scopes/$scopeId/workflow/draft-run" -m POST --stream \
  -d "$(python3 -c 'import json;print(json.dumps({"prompt":"<test input>","workflowYamls":[open("workflow.yaml").read()]}))')"
```

The JSON body uses camelCase and requires `workflowYamls`. The response is SSE. Interpret it as follows:

- HTTP 200 and lifecycle frames prove only dispatch.
- A parse, validation, or typed 4xx failure requires a document fix before another attempt.
- Root `RUN_FINISHED` or root `RUN_ERROR` is terminal. Role text, reasoning, tool-call, tool-result, and role terminal frames are progress only.
- Acceptance starts a 30-second deadline for the first projection-backed business frame. `: keepalive` does not satisfy or extend it.
- Root `RUN_ERROR(code=RUN_OBSERVATION_TIMEOUT)` closes the stream because observation stalled; it is not proof that the workflow failed. Query the same `actorId + commandId` for later state. Do not create a second run to check the first.
- After a real terminal frame, read run detail and audit. Require completed state, `lastSuccess=true`, expected non-empty step outputs, and non-empty final output before reporting execution success.

SSE `data:` lines contain JSON. Lifecycle shapes use top-level keys such as `stepStarted`, `stepFinished`, `usage`, `runFinished`, and `stateSnapshot`. Raw observation frames use `custom.name: aevatar.raw.observed` and carry step output under nested `output` or `content`. There is no universal flat `type` field.

A draft-run is throwaway validation and is not listed in the workflow observatory. Publish and invoke for an observable run.

## Publishing a workflow skill to Ornn

`ornn_publish_skill` is also server-side. A client publishes a zip through the NyxID proxy service slug `ornn-api`. The zip must contain one root directory, with `SKILL.md` at that root and workflow YAML under `assets/`:

```text
demo-skill/
  SKILL.md
  assets/
    my-workflow.yaml
```

The extractor recognizes `assets/*.{yaml,yml}` files that have top-level `name` and `steps`. Do not use a `workflows/` root directory.

`SKILL.md` frontmatter must nest Ornn fields under `metadata`. A workflow skill is
`category: mixed` and requires kebab-case `output-type`, an array-valued `runtime`, and
`tool-list`:

```markdown
---
name: demo-skill
description: Run the demo workflow when the user asks for its task.
version: "1.0"
metadata:
  category: mixed
  output-type: text
  runtime:
    - aevatar-workflow
  tool-list:
    - aevatar_start_workflow
  tag: [demo, workflow, aevatar]
---

# Demo workflow

Load and run `my-workflow` with the user's input.
```

Validate the zip before publishing, then publish it through the caller-scoped Ornn service. Send
the zip itself as `application/zip`, not multipart form data:

```bash
nyxid proxy request ornn-api "/api/v1/skill-format/validate" -m POST \
  -H 'Content-Type:application/zip' -d @demo-skill.zip
nyxid proxy request ornn-api "/api/v1/skills" -m POST \
  -H 'Content-Type:application/zip' -d @demo-skill.zip
```

Require `valid:true` from validation and fix every `violations[].rule`/`message`. Updating an
existing skill uses `PUT /api/v1/skills/{skillGuid}` with the same zip body; do not create a second
skill name. Read back the returned skill GUID, exact version, parsed JSON, and workflow list before
reporting publication. Publication is private by default unless a separate authorized visibility
operation changes it.

## Running a published workflow

Mount or select the exact published skill/version through the supported client surface, then invoke its workflow using the member or service contract created by the platform. Do not attempt to call `use_skill` as HTTP.

For a streaming invocation, apply the same root-terminal and 30-second first-projection rules as draft-run. For a non-streaming accepted receipt, retain `actorId + commandId` and observe/query that exact run. An accepted receipt, opened SSE stream, role completion, or keepalive is never execution success.
