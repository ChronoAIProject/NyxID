# Compute Pools QA

Use this checklist before opening a PR for compute pools. The goal is to
prove the worker poll path, routing, cancellation, token handling, and privacy
boundary before using real GPU workloads.

## Preconditions

- NyxID backend is running this branch.
- CLI binary is built from this branch: `cargo build -p nyxid-cli`.
- Test pool is private.
- Local model/mock backends bind to `127.0.0.1`, not `0.0.0.0`.

Use `./target/debug/nyxid` in the examples below.

## 1. Create A Private Pool

```bash
./target/debug/nyxid compute pool create my-4060-test \
  --base-url "$NYXID_URL" \
  --name "My 4060 Test Pool" \
  --visibility private \
  --scheduling-policy model_fit \
  --max-workers 2 \
  --max-queue 20 \
  --per-user-inflight 4 \
  --task-timeout 120
```

Save the printed `nyx_cwk_...` token only on trusted worker machines:

```bash
export NYXID_COMPUTE_WORKER_TOKEN='nyx_cwk_...'
```

## 2. Start Mock Backends

On 4060-A:

```bash
python3 scripts/compute-mock-backend.py --port 8001 --name 4060-a
```

On 4060-B:

```bash
python3 scripts/compute-mock-backend.py --port 8001 --name 4060-b
```

The mock server does not log prompts, responses, or Authorization values.

## 3. Start Workers

On 4060-A:

```bash
./target/debug/nyxid compute worker run \
  --base-url "$NYXID_URL" \
  --worker test-4060-a \
  --endpoint-url http://127.0.0.1:8001/v1/chat/completions \
  --backend mock \
  --host-kind linux-nvidia \
  --gpu-name "RTX 4060 A" \
  --model codex-test-a
```

On 4060-B:

```bash
./target/debug/nyxid compute worker run \
  --base-url "$NYXID_URL" \
  --worker test-4060-b \
  --endpoint-url http://127.0.0.1:8001/v1/chat/completions \
  --backend mock \
  --host-kind linux-nvidia \
  --gpu-name "RTX 4060 B" \
  --model codex-test-b
```

Check status:

```bash
./target/debug/nyxid compute status my-4060-test --base-url "$NYXID_URL"
```

Expected: both workers appear as active.

## 4. Verify Model Routing

Route to A:

```bash
./target/debug/nyxid compute submit my-4060-test \
  --base-url "$NYXID_URL" \
  --model codex-test-a \
  --input '{"messages":[{"role":"user","content":"route to A"}]}'
```

Expected: output says `hello from 4060-a`.

Route to B:

```bash
./target/debug/nyxid compute submit my-4060-test \
  --base-url "$NYXID_URL" \
  --model codex-test-b \
  --input '{"messages":[{"role":"user","content":"route to B"}]}'
```

Expected: output says `hello from 4060-b`.

## 5. Verify Wildcard Worker

Start one worker with:

```bash
./target/debug/nyxid compute worker run \
  --base-url "$NYXID_URL" \
  --worker test-4060-any \
  --endpoint-url http://127.0.0.1:8001/v1/chat/completions \
  --backend mock \
  --host-kind linux-nvidia \
  --gpu-name "RTX 4060" \
  --model '*'
```

Submit an arbitrary model:

```bash
./target/debug/nyxid compute submit my-4060-test \
  --base-url "$NYXID_URL" \
  --model arbitrary-model \
  --input '{"messages":[{"role":"user","content":"wildcard test"}]}'
```

Expected: wildcard worker completes the task.

## 6. Verify Cancellation

Start a slow mock:

```bash
python3 scripts/compute-mock-backend.py --port 8002 --name slow-4060 --delay-secs 60
```

Start a worker with a short heartbeat:

```bash
./target/debug/nyxid compute worker run \
  --base-url "$NYXID_URL" \
  --worker test-slow-4060 \
  --endpoint-url http://127.0.0.1:8002/v1/chat/completions \
  --backend mock-slow \
  --host-kind linux-nvidia \
  --gpu-name "RTX 4060 Slow" \
  --model slow-test \
  --ack-interval-secs 2
```

Submit and cancel:

```bash
TASK=$(./target/debug/nyxid compute submit my-4060-test \
  --base-url "$NYXID_URL" \
  --model slow-test \
  --input '{"messages":[{"role":"user","content":"cancel me"}]}' \
  --no-wait \
  --output json | jq -r .task_id)

./target/debug/nyxid compute cancel "$TASK" --base-url "$NYXID_URL"
./target/debug/nyxid compute result "$TASK" --base-url "$NYXID_URL"
```

Expected: task is cancelled and worker logs that the local request was
aborted.

## 7. Verify Token Rotation

```bash
./target/debug/nyxid compute pool rotate-token my-4060-test --base-url "$NYXID_URL"
```

Expected:

- old workers fail to poll with the old token;
- worker resumes after setting `NYXID_COMPUTE_WORKER_TOKEN` to the new token.

## 8. Privacy Checks

In MongoDB, inspect `compute_pools`, `compute_workers`, and `compute_tasks`.
Expected:

- no `endpoint_url`;
- no local backend bearer token;
- no SSH key or local credential;
- worker token is stored only as `worker_token_hash`.

Inspect audit logs for compute events. Expected:

- metadata such as `task_id`, `pool_id`, `model`, and `kind`;
- no prompt text;
- no response body;
- no worker token;
- no local backend token;
- no endpoint URL.

Inspect worker terminal logs. Expected:

- task ids and status messages only;
- no prompt body;
- no response body;
- no Authorization header values.

On each worker host, verify the mock/model backend binds only to localhost:

```bash
lsof -nP -iTCP -sTCP:LISTEN | rg '8000|8001|8002|11434'
```

Expected: `127.0.0.1:<port>`, not `0.0.0.0:<port>` or `*:<port>`.

## 9. Real GPU Smoke Test

After mock QA passes, replace the mock backend with a real local backend such
as vLLM, Ollama, MLX, or llama.cpp. Keep the backend bound to `127.0.0.1`.

For vLLM:

```bash
python -m vllm.entrypoints.openai.api_server \
  --host 127.0.0.1 \
  --port 8000 \
  --model <your-local-model>
```

Run the worker:

```bash
./target/debug/nyxid compute worker run \
  --base-url "$NYXID_URL" \
  --worker real-4060-a \
  --endpoint-url http://127.0.0.1:8000/v1/chat/completions \
  --backend vllm \
  --host-kind linux-nvidia \
  --gpu-name "RTX 4060" \
  --model codex-local
```

Submit a non-sensitive prompt first.
