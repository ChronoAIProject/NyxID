---
name: aevatar-codex-exec-node-setup
description: Configure and prove Aevatar codex_exec for one NyxID account, choosing operator-managed chrono-sandbox/gVisor for bounded empty-workspace work or private NyxID node-backed SSH for a fixed host workspace. Use for managed eligibility and UserService readiness, private node/SSH hardening, mandatory public-sample verification, or diagnosing typed managed and private-route failures.
version: "4.2"
metadata:
  category: tool-based
  tool-list:
    - shell
    - use_skill
    - aevatar_start_workflow
    - codex_exec
  tag:
    - aevatar
    - codex-exec
    - nyxid
    - chrono-sandbox
    - gvisor
    - credential-node
    - workflow
  depends-on:
    - aevatar-codex-exec-workflow-sample@3.1
compatibility: NyxID CLI and an Aevatar deployment that exposes codex_exec; private SSH additionally requires macOS or Linux, OpenSSH, Codex CLI, and Git
disable-model-invocation: false
user-invocable: true
---

# Configure Aevatar codex_exec

## Invocation gate

Continue only when the user explicitly asks to configure, repair, or prove a `codex_exec` target,
or explicitly invokes this skill. If this skill was selected speculatively, stop before mutation:
route feasibility questions to `aevatar-feasibility-advisor` and failure attribution to
`aevatar-triage`.

Choose exactly one target before changing state:

- `managed_sandbox`: use for bounded one-shot work in an operator-selected, empty, ephemeral Git
  workspace. Explicitly prepare the eligible native NyxID user's invocation credential before
  normal execution; Aevatar then calls that user's exact `chrono-sandbox` UserService and receives
  a structured terminal result. No personal node or local Codex login is needed.
- `private_ssh`: use when work must access a user-owned fixed Git workspace, host files, or host Codex configuration. The user owns the private NyxID node, SSH service, principal, forced-command wrapper, and Codex authentication.

Never mix fields between targets. Do not report either target as ready until its public Ornn sample succeeds through Aevatar with exact `CODEX_EXEC_READY`.

## Guardrails

- Never print, copy, or persist NyxID tokens, SSH private keys, Codex credentials, or `auth.json` into workflow input, logs, issue comments, or image layers.
- Never let a workflow choose an image, provider, model flag, shell fragment, approval policy, workspace path, or sandbox bypass.
- Never ask a caller to supply a raw key or token. Use the authenticated managed-credential
  lifecycle explicitly: `POST /api/managed-codex/credential`, then
  `GET /api/managed-codex/credential`. Normal `codex_exec` is credential-read-only and never
  provisions, reconciles, rotates, repairs, or retries credentials.
- Keep a private node and SSH service personal unless the user explicitly authorizes organization ownership.
- For private SSH, use a dedicated SSH key, pinned host key, forced command, fixed Git workspace, and Codex `workspace-write`; never add a dangerous sandbox bypass.

## Managed target

### 1. Required readiness

Require only these managed prerequisites:

1. The caller is authenticated as the exact intended native NyxID user.
2. Managed Codex is enabled with `RolloutBoundary=InternalOnly`, and the user is eligible through `Eligibility.Mode=Allowlist` or the internal `All` policy.
3. That user directly owns exactly one active, usable `chrono-sandbox` UserService and has exactly one usable `chrono-llm-public` route.
4. Operations deployed the approved immutable runner digest under gVisor with fixed resource, output, timeout, and cleanup bounds.
5. The explicit credential lifecycle reports `execution_ready=true` with
   `execution_readiness_reason=ready`; `status=active` alone is insufficient.
6. The public `aevatar-codex-exec-workflow-sample@3.1` managed proof returns exact
   `CODEX_EXEC_READY` through Aevatar.

Confirm only the user-facing identity and service inventory before the proof:

```bash
nyxid --version
nyxid whoami
nyxid service list
```

Do not require a personal node, local Codex login, caller-managed OpenSandbox key, separate LLM
consent, Landlock installation, or sandbox-side credential injection. Prepare the managed
credential through the authenticated lifecycle API, then classify any typed failure in
[troubleshooting.md](references/troubleshooting.md).

### 2. Credential and delegation boundary

Prepare, read, and prove in this order:

1. Authenticated `POST /api/managed-codex/credential` idempotently provisions or reconciles the
   committed descriptor.
2. `GET /api/managed-codex/credential` reads status. Stop unless `execution_ready=true` and
   `execution_readiness_reason=ready`, even if `status=active`.
3. Run the managed public canary. Do not loop normal execution hoping it will repair readiness.

- Aevatar keeps its persistent per-user NyxID invocation key behind `ISecretVault`. The key is never placed in the chrono request body.
- The user's exact `chrono-sandbox` UserService terminates that invocation key at NyxID and injects a five-minute internal delegation token.
- chrono-sandbox passes only that request-local delegation token to the one-shot Codex process as `NYXID_LLM_TOKEN`.
- The current internal rollout validates exact `proxy:*` delegation and remains `InternalOnly`. Do not present it as a public-ready security contract or ask callers to handle either credential.

### 3. Fixed runner and isolation

chrono-sandbox writes the prompt as data and runs only:

```bash
codex --ask-for-approval never exec --ephemeral --json \
  - < /workspace/.aevatar/prompt.txt
```

The runtime-written Codex profile fixes the provider, model, retry bounds, approval policy, and `sandbox_mode="danger-full-access"`. The caller cannot override them. Codex's inner sandbox is deliberately disabled because the one-shot gVisor workload is the managed isolation boundary. Sandbox-side Credential Vault substitution, a credential proxy, Landlock, and Bubblewrap are not deployed in this managed runtime and are not repair steps.

### 4. Managed workflow proof

Mount the public dependency and run `codex-exec-check` with no caller-controlled routing. Follow its evaluation contract exactly. Success requires `status=succeeded`, `target=managed_sandbox`, trimmed `output=CODEX_EXEC_READY`, `exit_code=0`, and a non-empty sanitized `diagnostic_id`; `elapsed_ms` is optional. Health and direct chrono-sandbox checks are prerequisites, not this Aevatar identity-path proof.

Preserve the deadline hierarchy: chrono-sandbox/Codex `180s` < Aevatar managed request `300s` <
NyxID/ingress `>=315s` < NyxID client `330s` < workflow canary `>=360s`
(`timeout_ms=360000`).

## Private SSH target

Build this ownership chain:

`NyxID user -> private SSH UserService -> owned node -> SSH target -> fixed wrapper -> Codex CLI -> one Git workspace`

The workflow receives the service slug or UUID, never `node_id`.

### 1. Collect inputs and preflight

Choose `NODE_PROFILE`, `NODE_NAME`, `SERVICE_LABEL`, `PRINCIPAL`, absolute `WORKSPACE`, `SSH_HOST`, `SSH_PORT`, and dedicated `SSH_KEY_FILE`. Show these values and visibility before mutating state.

```bash
nyxid --version
nyxid whoami
codex --version
codex login status
git -C "$WORKSPACE" rev-parse --show-toplevel
command -v jq sshd ssh-keygen base64 codex
```

Stop if the Codex login is absent, the workspace is not a Git repository, or the NyxID identity is not the intended owner.

### 2. Register an isolated node profile

Preserve any default profile. Mint and consume the one-time token without emitting it:

```bash
NODE_STORAGE_FLAG=--keychain
registration_json="$(nyxid node register-token --name "$NODE_NAME" --terminal --output json)"
registration_token="$(printf '%s' "$registration_json" | jq -r '.token // .registration_token // .data.token // empty')"
test -n "$registration_token"
nyxid node register \
  --token "$registration_token" \
  --url wss://nyx-api.chrono-ai.fun/api/v1/nodes/ws \
  --profile "$NODE_PROFILE" \
  ${NODE_STORAGE_FLAG:+$NODE_STORAGE_FLAG}
unset registration_token registration_json

nyxid node daemon install --profile "$NODE_PROFILE" --log-level info
nyxid node daemon start --profile "$NODE_PROFILE"
nyxid node daemon status --profile "$NODE_PROFILE"
nyxid node list --output json
```

Use `--keychain` only where a supported secret service exists; otherwise explicitly select NyxID's encrypted-file backend. Continue only when this profile's node is online.

### 3. Prepare the fixed SSH target

Read [forced-command-wrapper.md](references/forced-command-wrapper.md) and install the wrapper first. On macOS follow [macos-loopback-sshd.md](references/macos-loopback-sshd.md); on Linux follow [linux-ssh-target.md](references/linux-ssh-target.md).

Allow only the selected host and port in this node profile's `[ssh].allowed_targets`, then restart only that profile. Do not replace its server, node, signing, or credential sections.

### 4. Create and bind the private service

Create the service without `--org` unless organization ownership was explicitly requested:

```bash
node_id="$(nyxid node list --output json | jq -r --arg name "$NODE_NAME" '.nodes[] | select(.name == $name) | .id')"
test -n "$node_id"

service_json="$(nyxid service add-ssh \
  --label "$SERVICE_LABEL" \
  --host "$SSH_HOST" \
  --port "$SSH_PORT" \
  --via-node "$node_id" \
  --node-key \
  --principals "$PRINCIPAL" \
  --output json)"
service_slug="$(printf '%s' "$service_json" | jq -r '.slug // .service_slug // .data.slug // empty')"
test -n "$service_slug"

NYXID_PROFILE="$NODE_PROFILE" nyxid node ssh-credentials add \
  --service "$service_slug" \
  --principal "$PRINCIPAL" \
  --key-file "$SSH_KEY_FILE" \
  --host "$SSH_HOST" \
  --port "$SSH_PORT"
```

Investigate a host-key mismatch or explicitly rotate the verified pin. Never disable host verification.

### 5. Verify the private route

Use [troubleshooting.md](references/troubleshooting.md) to confirm a generic command is rejected and the exact Aevatar command profile succeeds through `nyxid ssh exec`.

Private workflow arguments use the nested contract:

```json
{
  "target": {
    "kind": "private_ssh",
    "private_ssh": {
      "service": "your-service-slug",
      "principal": "your-unix-user"
    }
  },
  "prompt": "Reply with exactly CODEX_EXEC_READY",
  "timeout_secs": 300
}
```

Do not use the obsolete root-level `service` and `principal` shape.

### 6. Private workflow proof

Mount the public dependency and run `codex-exec-private-ssh-check` with the actual service slug and principal. A direct SSH success is insufficient. The Aevatar workflow must return `exit_code: 0`, `timed_out: false`, and stdout exactly `CODEX_EXEC_READY` after trimming.

## Mandatory public-sample procedure

For either target, load the declared public dependency, `aevatar-codex-exec-workflow-sample@3.1`, from Ornn:

```json
{
  "skill": "aevatar-codex-exec-workflow-sample",
  "mount_workflows": true
}
```

Use `codex-exec-check` for managed or `codex-exec-private-ssh-check` for private SSH. If workflow mounting is unavailable, fetch the same published Ornn version and run it as explicit inline draft input; do not substitute an unverified local copy.

## Completion criteria

For managed, all must be true:

- the exact native NyxID user is eligible under the internal rollout policy
- that user has the required active `chrono-sandbox` and `chrono-llm-public` UserServices
- operations deployed the approved immutable runner under gVisor with bounded output and strict cleanup
- the public managed workflow returned exact `CODEX_EXEC_READY`
- no raw persistent key or request-local delegation token appeared in output

For private SSH, all must be true:

- the node is online under the intended NyxID user
- the service is private, active, node-bound, and uses node-held key auth
- the target is restricted and arbitrary commands are rejected
- local Codex succeeds through the fixed NyxID SSH route
- the public private-SSH workflow returned exact `CODEX_EXEC_READY`

Anything less is partial configuration, not a usable `codex_exec` route.
