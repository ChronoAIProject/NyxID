---
title: nyxid nyxbot
description: Run your NyxBot from a local terminal, follow conversations, inspect durable work, and cancel an invocation.
---

`nyxid nyxbot` gives your existing NyxBot work from a terminal or script. The
CLI runs on your computer and calls your configured NyxID server. NyxBot's
execution, memory, conversation history, service access, and billing stay on
that server, using the same agent engine as the web interface.

## Get started

Install the [CLI](/docs/cli/getting-started/install), then log into a dedicated
profile. Choose **full account access** on the approval page:

```bash
nyxid login --profile nyxbot-owner --base-url https://nyx-api.chrono-ai.fun
nyxid nyxbot run --profile nyxbot-owner \
  --text 'Summarize my pending work' --output json
```

The instance must support and enable owner invocations. Admission defaults off;
administrators must upgrade every backend replica before enabling
`assistant:owner-invocations`. A new CLI alone does not enable this server feature.

Owner sessions and first-party account access tokens work. Agent Keys, service
accounts, delegated tokens, and developer OAuth credentials cannot invoke
NyxBot through this resource. Explicit credential arguments or environment
variables take precedence over a saved profile; a rejected credential never
switches to the owner's saved session automatically. See
[Authenticate](/docs/cli/getting-started/authenticate) for profile and credential
selection.

Every command accepts `--profile`, `--base-url`, `--access-token-env`, and
`--output table|json`. Use the saved profile for routine local work.

## Submit work

```bash
nyxid nyxbot run --profile nyxbot-owner --text 'Review my open tasks'
nyxid nyxbot run --profile nyxbot-owner --file prompt.txt --output json
cat prompt.txt | nyxid nyxbot run --profile nyxbot-owner --file - --no-wait --output json
```

Choose exactly one of `--text`, `--file PATH`, or `--file -` for stdin. Prompts
must contain non-whitespace text and fit 32,768 Unicode scalar values and
131,072 UTF-8 bytes. File and stdin contents become prompt text sent to NyxID.

`run` waits by default, for up to 300 seconds. `--timeout SECONDS` changes the
total wait deadline; `--no-wait` returns after durable acceptance. Timing out
locally does not cancel server work.

The CLI prints an invocation UUID recovery notice to stderr before submission.
With `--output json`, stdout contains one result object. Save its
`invocation_id` to inspect or cancel that work later.

## Continue a conversation or choose a specialist

```bash
nyxid nyxbot agents --profile nyxbot-owner --output json
nyxid nyxbot models --profile nyxbot-owner --output json
nyxid nyxbot conversations --profile nyxbot-owner --limit 50 --output json
nyxid nyxbot history nyxa-CONVERSATION --profile nyxbot-owner --limit 50 --output json

nyxid nyxbot run --profile nyxbot-owner --agent SPECIALIST_ID \
  --text 'Work on the task within your existing grants' --output json
nyxid nyxbot run --profile nyxbot-owner --conversation nyxa-CONVERSATION \
  --text 'Follow up on the previous answer' --output json
```

An invocation is one submitted task and its tracked follow-up work. A
conversation is the thread that retains its messages. Use the returned
`conversation_id` for a later request and the `invocation_id` to inspect or
cancel the original request.

Omitting `--agent` starts a conversation with your NyxBot. Use an owned
specialist ID from `agents` to start with that specialist. `--model MODEL_ID`
selects an available model for a new thread. `--conversation` cannot be
combined with `--agent` or `--model`.

`conversations` also supports `--agent`, `--cursor`, and `--limit`;
`history` supports `--before-seq` and `--limit`. Group, channel, voice,
automation, and platform delivery threads use their existing interfaces.

## Inspect, wait, and cancel

```bash
nyxid nyxbot show INVOCATION_UUID --profile nyxbot-owner --output json
nyxid nyxbot wait INVOCATION_UUID --profile nyxbot-owner --timeout 300 --output json
nyxid nyxbot cancel INVOCATION_UUID --profile nyxbot-owner --output json
```

`show` returns the durable status and bounded replies. `wait` follows the
invocation until it settles, needs an owner decision, or reaches the local
deadline. `cancel` stops that invocation family and requests cancellation of
its tracked work; it cannot stop an unrelated later turn.

`requires_action` means review an owner card or complete setup in NyxID.
The CLI preserves existing grants and confirmations and cannot approve cards
or change permission modes. `requires_review` means the server cannot safely
claim completion, such as an unconfirmed external cancellation. Already
dispatched external effects cannot be undone by this API.

After a lost response, use `show` or `wait` with the recovery UUID first.
`run --idempotency-key UUID_V4` lets you choose that identity before submission.
Repeating the exact request with the same UUID returns its existing receipt;
a changed request conflicts. No uncertain transport outcome automatically
submits a second task.

| Exit | Meaning |
| --- | --- |
| 0 | Completed, or accepted and active for `run --no-wait`, `show`, or `cancel` |
| 1 | HTTP request rejected |
| 3 | Login required |
| 4 | Session renewal temporarily uncertain |
| 20 | Execution failed |
| 21 | Invocation cancelled or its root conversation deleted |
| 22 | Owner action or review required |
| 23 | Local wait deadline reached; server work may continue |
| 24 | Transport outcome unknown; inspect the same invocation UUID |
| 25 | Invalid or mismatched server response |

Argument, input, and local profile setup errors use the existing CLI error
behavior. Inspect `cli_status`, `invocation.status`, and `terminal` when
consuming JSON; an accepted request is not necessarily completed.

## Local services and MCP

For an existing self-hosted NyxID instance, use its API URL at login, such as
`http://localhost:3001`. The CLI starts no listener, backend, or offline
NyxAgent engine. Model and service requests still follow your server's
configuration.

Running the command in a directory does not grant NyxBot access to its files,
shell, or desktop. Only the explicitly supplied prompt is sent. Local machine
work requires a separately configured machine node and its existing grants
and confirmations.

No MCP setup is required for this command. [nyxid mcp](/docs/cli/reference/mcp)
configures external AI clients to call your connected services; it does not
expose the owner invocation API to those clients. NyxBot's own tool calls keep
the shared authorization and cancellation checks.

For HTTP request/response fields, bounded reply limits, and the mandatory
rollout and recovery contract, see the
[owner CLI/API reference](https://github.com/ChronoAIProject/NyxID/blob/main/docs/chat/09-owner-cli-api.md).
