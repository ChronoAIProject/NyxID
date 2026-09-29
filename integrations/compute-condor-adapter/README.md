# Compute HTCondor Adapter

This directory documents the production-oriented adapter shape for exposing an
HTCondor pool through NyxID as an external service. It intentionally does not
add NyxID backend routes, MongoDB models, org changes, or a fake partial
HTCondor implementation.

NyxID remains the control plane: identity, org membership, agent API keys,
credential brokering, node routing, proxying, audit, and service governance.
HTCondor remains the scheduler: execute points, submit queue, resource
matching, preemption, retries, and idle-machine policy. The adapter is the
thin data-plane service between them.

## Architecture

```text
agent / org user
  -> NyxID proxy / service governance
  -> optional NyxID Credential Node
  -> compute-condor-adapter
  -> HTCondor submit host
  -> HTCondor execute points
  -> MacBook / Linux GPU / CPU workers
```

The adapter should run on a trusted host that can submit to the HTCondor pool.
Common placements are the HTCondor submit host itself or a host reachable from
a NyxID Credential Node. Agents and org users should not SSH into the submit
host or execute points.

## Consumer API Contract

The adapter should expose the same consumer-facing API as the smoke-test
worker-pull service:

- `POST /v1/tasks`
- `GET /v1/tasks/{task_id}`
- `POST /v1/tasks/{task_id}/cancel`
- `GET /v1/status`

The OpenAPI contract is in `openapi.yaml`. Keeping this contract stable lets
agents call `chrono-compute` through NyxID without knowing whether the backend
is the local worker-pull reference, HTCondor, Slurm, Ray, or another scheduler.

## HTCondor Mapping

`POST /v1/tasks` should validate the request, create a NyxID-facing task id,
materialize the job input in an adapter-controlled working directory, submit a
Condor job, and persist the mapping:

```text
nyxid_task_id -> condor ClusterId / ProcId / working directory / output paths
```

`GET /v1/tasks/{task_id}` should read the adapter mapping and inspect Condor
state using `condor_q`, Condor history, event logs, or HTCondor bindings. It
should return queued/running/completed/failed/cancelled in the NyxID compute
contract shape.

`POST /v1/tasks/{task_id}/cancel` should translate to `condor_rm` when the job
is still active. Completed or failed tasks should return the terminal state.

`GET /v1/status` should summarize queue depth and capacity from Condor state,
for example idle/running job counts, available execute points, advertised GPU
types, and scheduler health.

## Security Boundary

This shares controlled task execution capacity, not host access.

- NyxID does not SSH into HTCondor hosts.
- Agents do not receive submit-host SSH access.
- Agents do not receive Condor admin credentials.
- Adapter-local working directories and result artifacts stay behind the
  adapter API.
- Service credentials should be injected by NyxID or a Credential Node rather
  than distributed to agents.
- Prompt/input/output retention belongs to the adapter backend, not NyxID core.

The adapter should use per-task working directories, avoid shell interpolation
with untrusted input, and prefer structured submit-file generation over string
concatenation. If jobs need container isolation, that should be configured in
HTCondor and documented as part of the deployment profile.

## Deployment Through NyxID

Register the adapter as a normal custom service:

```bash
nyxid service add --custom \
  --slug chrono-compute \
  --label "Chrono Compute" \
  --endpoint-url "http://127.0.0.1:8787" \
  --auth-method bearer \
  --auth-key-name "Authorization" \
  --via-node <node-id>
```

Store the adapter API token on the node or in NyxID-managed credentials:

```bash
nyxid node credentials add \
  --service chrono-compute \
  --url "http://127.0.0.1:8787" \
  --header "Authorization" \
  --secret-format bearer
```

The adapter process itself must be deployed and kept alive outside NyxID, for
example with systemd, launchd, Docker, Kubernetes, or an internal service
manager. Registering it in NyxID makes it governable and callable; it does not
make NyxID run the adapter process.

## Rollout Plan

1. Pilot the smoke-test worker-pull service with a small number of trusted
   machines to validate NyxID service registration, credential injection, agent
   access, and the `/v1/tasks` contract.
2. Stand up a small HTCondor pool with a submit host and a few MacBook / Linux
   GPU / CPU execute points.
3. Implement this adapter against the same OpenAPI contract.
4. Move production idle-machine scheduling to HTCondor while preserving the
   NyxID-facing service API.
