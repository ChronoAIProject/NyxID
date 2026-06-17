# Compute Pools

Compute pools turn trusted GPU / Mac hosts into an org-visible task queue.
They deliberately follow the Oracle Relay's pool/worker/task safety shape
without reusing browser-specific semantics.

## Security Model

- Workers authenticate with a pool worker token (`nyx_cwk_...`), stored as
  SHA-256 on the server and shown once at create/rotate time.
- Workers pull tasks from NyxID. NyxID never SSHes into arbitrary hosts or
  sends shell commands to execute.
- Worker hosts are outbound-only for task execution. They do not need an
  inbound firewall hole, public IP, exposed SSH server, or exposed model
  port.
- Worker hosts should run a narrow daemon that calls a configured local
  backend such as vLLM, Ollama, MLX, OpenClaw, or LiteLLM.
- The worker's local backend URL is local machine configuration. It is not
  stored in NyxID and is not shown to task submitters.
- Local backend credentials are local-only. `--backend-token-env` values are
  sent only to the local backend, never to NyxID.
- Consumers authenticate with normal NyxID sessions or scoped agent API
  keys. Pool visibility can be `private`, `org`, or `platform`.
- Task input/output lives on the task document and is TTL-expired using the
  existing task-retention setting. Audit events are metadata-only.

## Exposure Contract

Compute pools share controlled task execution capacity, not host access.
Each machine exposes only:

- a stable worker label;
- scheduling metadata such as host kind, backend, models, accelerator name,
  memory/concurrency hints, and worker version;
- task results for tasks it accepted.

Compute workers must not expose these through NyxID:

- SSH, shell, or arbitrary command execution;
- local filesystems or environment variables;
- local backend URLs, host IPs, private network routes, or open ports;
- local backend tokens, SSH keys, API keys, or model-provider credentials.

Task kinds in v1 are narrow request/response jobs such as chat completion,
completion, embedding, or batch. `shell_exec`, `read_file`, `write_file`,
port proxying, and general remote execution are explicit non-goals.

## Create an Org Pool

```bash
nyxid compute pool create chrono-gpu \
  --name "Chrono GPU Pool" \
  --visibility org \
  --org chrono \
  --scheduling-policy model_fit \
  --max-workers 64 \
  --max-queue 1000 \
  --per-user-inflight 4
```

Save the printed worker token. Install it only on trusted worker hosts.

## Run a Worker

Start a local OpenAI-compatible backend on the GPU/Mac host first, then run
the NyxID worker daemon against it. The worker token is used only to poll
NyxID; it is not forwarded to the local backend.

```bash
export NYXID_COMPUTE_WORKER_TOKEN='nyx_cwk_...'

nyxid compute worker run \
  --base-url https://auth.example.com \
  --worker home-4090-a \
  --endpoint-url http://127.0.0.1:8000/v1/chat/completions \
  --backend vllm \
  --host-kind linux-nvidia \
  --gpu-name "RTX 4090" \
  --model codex-local \
  --model local-chat-large
```

If the local backend requires a bearer token, keep it local to that host:

```bash
export VLLM_API_KEY='local-only-token'

nyxid compute worker run \
  --worker home-4090-a \
  --endpoint-url http://127.0.0.1:8000/v1/chat/completions \
  --backend-token-env VLLM_API_KEY \
  --model '*'
```

The MVP worker executes one task at a time and heartbeats while the local
request is in flight. If the task is cancelled in NyxID, the daemon aborts
the local request and does not submit a result.

## Submit a Task

```bash
nyxid compute submit chrono-gpu \
  --model codex-local \
  --input '{"messages":[{"role":"user","content":"ping"}]}'
```

Fire-and-forget:

```bash
TASK=$(nyxid compute submit chrono-gpu \
  --model codex-local \
  --input @request.json \
  --no-wait --output json | jq -r .task_id)

nyxid compute result "$TASK"
```

## Worker API

Worker endpoints are mounted outside JWT middleware:

```text
POST /api/v1/compute/worker/task?worker=<label>
POST /api/v1/compute/worker/ack
POST /api/v1/compute/worker/result
```

All requests use:

```text
Authorization: Bearer nyx_cwk_...
```

Poll body:

```json
{
  "capabilities": {
    "node_id": "optional-nyxid-node-id",
    "host_kind": "linux-gpu",
    "gpu_name": "RTX 4090",
    "backend": "vllm",
    "models": ["codex-local", "local-chat-large"],
    "vram_total_mb": 24576,
    "vram_free_mb": 18000,
    "max_concurrency": 2,
    "current_inflight": 0,
    "avg_tokens_per_sec": 92.5,
    "worker_version": "0.1.0"
  }
}
```

Task response:

```json
{
  "status": "task",
  "task_id": "...",
  "kind": "chat_completion",
  "model": "codex-local",
  "input": { "messages": [] },
  "priority": 0,
  "assigned_worker": "home-4090-a",
  "submitted_at": "..."
}
```

Ack:

```json
{
  "task_id": "...",
  "worker": "home-4090-a",
  "phase": "running",
  "phase_detail": "local vLLM request in flight"
}
```

Result:

```json
{
  "task_id": "...",
  "worker": "home-4090-a",
  "output": {
    "choices": [
      { "message": { "role": "assistant", "content": "pong" } }
    ]
  }
}
```

Failure:

```json
{
  "task_id": "...",
  "worker": "home-4090-a",
  "failure_reason": "local backend timeout"
}
```

## Scheduling

The first backend version supports:

- `fifo`: priority then created-at order.
- `model_fit`: only dispatch to workers that advertise the requested
  model. Workers advertising `*` are treated as accepting any model; workers
  advertising no models do not claim model-fit tasks.
- `least_busy`: reserved as a policy label; the current queue claim path is
  still worker-poll based, so the worker should report accurate
  `current_inflight` and `max_concurrency` for future scheduler scoring.

## Practical Host Setup

NyxID intentionally does not install CUDA, vLLM, Ollama, MLX, or model
weights for you. Each worker host needs:

1. A trusted OS user or service account to run the daemon.
2. A local model server bound to localhost or a private interface.
3. The pool worker token in an environment variable or service secret.
4. A stable worker label so operators can identify and drain hosts.

SSH is still useful for installation and operations, but not as the task
execution plane. The runtime path is pull-based: worker -> NyxID queue ->
local model endpoint -> NyxID result.

## Manual QA

Before opening a PR or connecting real workloads, run the mock backend and
privacy checklist in `docs/COMPUTE_POOLS_QA.md`.
