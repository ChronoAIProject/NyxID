# Workflow authoring examples

Read this reference when a concrete YAML shape is useful. Adapt names, roles, tools, selectors, and inputs; do not copy capability identities or routes from an example.

## Contents

- Linear LLM chain
- Branch and converge
- Per-item tool calls
- Multiple typed files
- Bounded template aggregation
- Fan-out and reduce

## Linear LLM chain

```yaml
name: summarize_and_title
roles:
  - id: analyst
    system_prompt: Summarize faithfully.
  - id: editor
    system_prompt: Produce one concise title.
steps:
  - id: summarize
    type: llm_call
    target_role: analyst
    parameters:
      prompt_prefix: "Summarize:"
    next: make_title
  - id: make_title
    type: llm_call
    target_role: editor
    parameters:
      prompt_prefix: "Title this summary:"
```

The last document step is the only step without `next`, so it is the single terminal step.

## Branch and converge

```yaml
name: classify_and_route
roles:
  - id: classifier
    system_prompt: Return exactly urgent or normal.
  - id: writer
    system_prompt: Draft the requested response.
steps:
  - id: classify
    type: llm_call
    target_role: classifier
    next: route
  - id: route
    type: switch
    parameters:
      on: "$input"
      branch.urgent: urgent_reply
      branch._default: normal_reply
    branches:
      urgent: urgent_reply
      _default: normal_reply
  - id: urgent_reply
    type: llm_call
    target_role: writer
    parameters:
      prompt_prefix: "Draft an urgent response:"
    next: finalize
  - id: normal_reply
    type: llm_call
    target_role: writer
    parameters:
      prompt_prefix: "Draft a normal response:"
    next: finalize
  - id: finalize
    type: assign
    parameters:
      target: final_summary
      value: "$input"
```

Both branches converge explicitly. Never put another step after `finalize`.

## Per-item tool calls

Use `foreach` when each item needs its own tool arguments. `${input}` and `${output}` inside any `sub_param_*` value both resolve to the current item after fan-out:

```yaml
- id: fetch_each_instance
  type: foreach
  parameters:
    delimiter: "\n"
    sub_step_type: tool_call
    sub_param_tool: nyxid_proxy
    sub_param_arguments: >-
      {"path_params":{"instance_id":"${input}"}}
  next: synthesize
```

Keep all `sub_param_*` fields under `parameters`. Do not use `map_reduce` for per-item tool calls because its map phase does not carry per-step parameters.

## Multiple typed files

Fan typed refs out directly. The child tool receives one current file ref; it does not receive a file ID string as prompt input:

```yaml
name: extract_files
roles:
  - id: editor
    system_prompt: Combine extracted text faithfully.
steps:
  - id: extract_each_file
    type: foreach
    parameters:
      items_source: input_file_refs
      sub_step_type: tool_call
      sub_param_tool: document_extract
      sub_param_arguments: "{}"
    next: summarize
  - id: summarize
    type: llm_call
    target_role: editor
```

When the start request has no explicit `fileRef`, deduplicated ambient refs may supply `input_file_refs`; empty `inputs` is valid in that case. If any explicit `fileRef` is present, ambient refs are suppressed. Role LLM tool loops receive the same typed refs.

For uploads, call `workflow_file_submit` in a `foreach` child with the current typed ref. Preserve the full ref fields supplied by the platform; do not manufacture an `artifact_id`, owner run, source message, or resource key.

## Bounded template aggregation

Use the typed `template` transform for deterministic multi-item aggregation or JSON/report rendering:

```yaml
name: aggregate_budget
steps:
  - id: aggregate
    type: transform
    parameters:
      op: template
      template: >-
        {{ total = 0 }}
        {{ for item in data.items; total = total + number(item.amount); end }}
        {{ json({ count: data.items.size, total: round(total, 2) }) }}
    next: finalize
  - id: finalize
    type: assign
    parameters:
      target: final_summary
      value: "$input"
```

The step input must be bounded JSON and is exposed as `data`. The template is a typed static program, not upstream `${...}` code. Only `append`, `date`, `get`, `json`, `keys`, `number`, and `round` are available; the limits and fail-closed rules are in `SKILL.md`.

## Fan-out and reduce

Use `map_reduce` for different items that need concurrent LLM analysis followed by one synthesis:

```yaml
- id: analyze_sources
  type: map_reduce
  parameters:
    delimiter: "\n---\n"
    map_step_type: llm_call
    map_target_role: analyst
    reduce_step_type: llm_call
    reduce_target_role: editor
    reduce_prompt_prefix: "Combine these analyses into one digest:"
  next: finalize
```

Use `foreach` instead when concatenated per-item outputs are sufficient. Use `parallel` or `race` only when every branch should receive the same input. To fetch different sources, first use `foreach` with `sub_step_type: tool_call`, then pass the fetched outputs into `map_reduce`.
