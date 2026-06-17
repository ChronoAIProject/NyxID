# Compute Pool Service

This integration is a reference external service for shared GPU / Mac / lab
compute. The queue and worker protocol live outside NyxID core. NyxID
manages the service as a normal user/org service: auth, agent API keys,
credential injection, node routing, proxying, and audit metadata stay in
NyxID; compute-specific task state stays in this service.

This is not a NyxID service-pool framework. Cross-service counting, quotas,
metering, and load balancing should be handled by a future generic NyxID
service-pool design rather than by a compute-specific core API.

This integration does not require a NyxID org model change. To share it with
company members, create the NyxID service under the existing org owner and
use the current org membership/admin checks that already apply to services.

## Architecture

```text
agent / org user
  -> NyxID proxy / service governance
  -> optional NyxID Credential Node
  -> compute-pool-service
  -> trusted GPU/Mac/Slurm workers
  -> local OpenAI-compatible backend
```

NyxID core does not store compute tasks, worker tokens, local backend URLs,
or local backend credentials.

## Security Boundary

This shares controlled task execution capacity, not host access.

- NyxID does not SSH into worker hosts.
- NyxID does not execute shell commands.
- NyxID does not expose worker filesystems or environment variables.
- NyxID does not store worker-local model endpoint URLs.
- NyxID does not store worker-local backend bearer tokens.
- If routed through a Credential Node, the service API token can stay on the
  node host and be injected locally.

The standalone service stores task input/output in its own local store. The
default store is a JSON file intended for smoke tests and small trusted
deployments, not production durability. A production version should replace
the store with Postgres, Redis, MongoDB, or another managed queue backend.
NyxID-level metering and quota decisions should count proxied calls to this
service the same way they would count any other registered service.

## Start The Service

Generate two independent tokens:

```bash
export COMPUTE_POOL_API_TOKEN="$(openssl rand -hex 32)"
export COMPUTE_POOL_WORKER_TOKEN="$(openssl rand -hex 32)"
```

Start the queue service on the private host:

```bash
cd integrations/compute-pool-service
node server.mjs
```

For local throwaway testing only:

```bash
COMPUTE_POOL_DEV_INSECURE=1 node server.mjs
```

## Add To NyxID As A Service

Recommended: run a NyxID Credential Node on the host that can reach this
service, then register the service through that node.

```bash
nyxid service add --custom \
  --slug chrono-compute \
  --label "Chrono Compute Pool" \
  --endpoint-url "http://127.0.0.1:8787" \
  --auth-method bearer \
  --auth-key-name "Authorization" \
  --via-node <node-id>
```

Then store the service API token on the node:

```bash
nyxid node credentials add \
  --service chrono-compute \
  --url "http://127.0.0.1:8787" \
  --header "Authorization" \
  --secret-format bearer
```

Agents and org members call it through NyxID like any other service:

```bash
nyxid proxy request chrono-compute /v1/tasks \
  -m POST \
  -d '{"model":"codex-local","input":{"messages":[{"role":"user","content":"ping"}]}}'
```

The returned `task_id` can be polled:

```bash
nyxid proxy request chrono-compute /v1/tasks/<task_id>
```

## Run A Worker

Start a local OpenAI-compatible backend first, bound to localhost. Then run a
worker on that same trusted host:

```bash
export COMPUTE_POOL_WORKER_TOKEN="..."

node integrations/compute-pool-service/worker.mjs \
  --service-url http://127.0.0.1:8787 \
  --worker home-4060-a \
  --endpoint-url http://127.0.0.1:8000/v1/chat/completions \
  --backend vllm \
  --host-kind linux-nvidia \
  --gpu-name "RTX 4060" \
  --model codex-local
```

Use `--model '*'` only for workers that should accept any submitted model.
Workers with no advertised model do not claim model-routed work.

If the local backend needs a token, keep it on the worker host:

```bash
export LOCAL_BACKEND_TOKEN="..."

node integrations/compute-pool-service/worker.mjs \
  --service-url http://127.0.0.1:8787 \
  --worker home-4090-a \
  --endpoint-url http://127.0.0.1:8000/v1/chat/completions \
  --backend-token-env LOCAL_BACKEND_TOKEN \
  --model codex-local
```

`LOCAL_BACKEND_TOKEN` is sent only to the local endpoint. It is not sent to
NyxID and is not sent to compute-pool-service.

## API Summary

Consumer API, called through NyxID service proxy:

- `POST /v1/tasks`
- `GET /v1/tasks/{task_id}`
- `POST /v1/tasks/{task_id}/cancel`
- `GET /v1/status`

Worker API, called directly by trusted workers:

- `POST /worker/task?worker=<label>`
- `POST /worker/ack`
- `POST /worker/result`

The OpenAPI spec for the consumer API is in `openapi.yaml`.

## Current Limitations

- Local JSON store only; not multi-process safe.
- One task per worker process at a time.
- No built-in Slurm adapter yet.
- No NyxID catalog seed yet; add as a custom service for now.
- No NyxID-level service pool yet. This service exposes `/v1/status` as a
  capacity signal, but generic service-instance load balancing, org/agent
  quotas, usage counting, and metering belong in NyxID's future service-pool
  layer.

## Why This Is Not NyxID Core

NyxID is the control plane: identity, org membership, agent API keys,
credential brokering, node routing, proxying, audit, and service governance.
Compute pools are a data-plane service. Keeping the queue outside core keeps
NyxID from becoming a runtime for every business-specific worker protocol.
