---
name: aevatar-codex-exec-workflow-sample
description: Mount and run harmless Aevatar workflows that prove codex_exec works through either the operator-managed chrono-sandbox/gVisor target or a private NyxID node-backed SSH target. Use after managed eligibility and required NyxID UserServices are ready, or after configuring a personal SSH node; also use when diagnosing typed managed or private-route failures before real tasks.
version: "3.1"
metadata:
  category: mixed
  output-type: text
  runtime:
    - aevatar-workflow
  tool-list:
    - aevatar_start_workflow
    - codex_exec
  tag:
    - aevatar
    - codex-exec
    - nyxid
    - chrono-sandbox
    - verification
    - workflow
---

# Verify Aevatar codex_exec

## Choose exactly one proof

Mount this skill from Ornn and run one bundled workflow for the configured target:

- `codex-exec-check`: canonical managed chrono-sandbox/gVisor proof with no caller routing.
- `codex-exec-private-ssh-check`: private NyxID node-backed SSH proof with only service and principal routing.

Configuration, health, UserService, node-online, and direct SDK/SSH checks are prerequisites, not completion evidence. Report `codex_exec` as usable only after the selected Aevatar workflow returns exact `CODEX_EXEC_READY` under that target's result contract.

## Guardrails

- Fetch the public skill from Ornn. Do not validate only a local workflow copy.
- Keep the fixed probe prompt. Do not replace it with a real task.
- Never place tokens, keys, `auth.json`, `CODEX_HOME`, local paths, model flags, images, providers, or sandbox flags in workflow input.
- Do not mix target fields. Managed requires `target.kind=managed_sandbox` plus `workspace.kind=empty_git`; private requires nested `target.private_ssh` and no workspace.
- Managed callers cannot choose a repository, workspace path, image, model, provider, credential, shell, Codex profile, approval policy, or sandbox flags.
- Run through Aevatar as the native NyxID account being verified. Never pass a raw token. Before
  the managed proof, use the explicit authenticated credential lifecycle: `POST
  /api/managed-codex/credential`, then `GET /api/managed-codex/credential`. Continue only when
  `execution_ready=true` and `execution_readiness_reason=ready`; `status=active` alone is
  insufficient.
- Normal `codex_exec` execution is credential-read-only. It never provisions, reconciles, rotates,
  repairs, or retries credentials.

## Mount

Call `use_skill` with workflow mounting enabled:

```json
{
  "skill": "aevatar-codex-exec-workflow-sample",
  "mount_workflows": true
}
```

Wait for the mount command to be accepted. Read-model visibility can propagate asynchronously. If mounting is unavailable, fetch this exact public Ornn version and submit the corresponding YAML under `assets/` as explicit inline draft-run input; state clearly that it was an inline run.

## Managed proof

Prepare and prove in this order:

1. Authenticated `POST /api/managed-codex/credential` idempotently provisions or reconciles the
   committed credential descriptor.
2. `GET /api/managed-codex/credential` reads projected status. Stop unless
   `execution_ready=true` and `execution_readiness_reason=ready`, even when `status=active`.
3. Mount and run the canary below. Do not loop ordinary `codex_exec` calls as a repair mechanism.

Start the canonical workflow without caller-controlled routing:

```json
{
  "workflow_id": "codex-exec-check",
  "inputs": {
    "prompt": ""
  },
  "wait": "stream"
}
```

The workflow owns this exact `codex_exec` payload:

```json
{
  "target": { "kind": "managed_sandbox" },
  "workspace": { "kind": "empty_git" },
  "prompt": "Reply with exactly CODEX_EXEC_READY",
  "timeout_secs": 180
}
```

Success requires all of:

- `status` equal to `succeeded`;
- `target` equal to `managed_sandbox`;
- `output` equal to `CODEX_EXEC_READY` after trimming;
- `exit_code` equal to `0`;
- a non-empty sanitized `diagnostic_id`.

`elapsed_ms` may be present but is not required for success. Treat a missing required field, extra model text, or any typed failure as a failed verification.

Keep the complete deadline chain ordered so each outer layer can observe the inner timeout:

- chrono-sandbox/Codex execution: `timeout_secs=180`;
- Aevatar managed request: `300` seconds;
- NyxID/ingress: at least `315` seconds;
- NyxID client: `330` seconds;
- workflow canary step: at least `360` seconds (`timeout_ms=360000` in the bundled workflow).

In short: `180s < 300s < >=315s < 330s < >=360s`. Do not shrink the workflow canary to the old
200-second budget.

## Private SSH proof

Start the private workflow with exactly the environment-owned service and Unix principal:

```json
{
  "workflow_id": "codex-exec-private-ssh-check",
  "inputs": {
    "prompt": "{\"service\":\"your-service-slug\",\"principal\":\"your-unix-user\"}"
  },
  "wait": "stream"
}
```

The workflow owns this target shape and deliberately has no `workspace`:

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

Pass the SSH UserService slug or UUID, never a node ID. Success requires the NyxID SSH response to have `exit_code=0`, `timed_out=false`, and stdout equal to `CODEX_EXEC_READY` after trimming. Private SSH output is returned as the original NyxID SSH response; it is not converted into the managed `status/target/diagnostic_id` JSON shape.

## Diagnose by boundary

Preserve the exact typed error and sanitized `diagnostic_id`, if present. Do not request raw tokens, raw upstream bodies, `auth.json`, agent-key values, or an unredacted runner environment.

- `target_not_configured` or `managed_target_disabled`: the Aevatar host does not expose the selected managed target; repair host configuration.
- `managed_feature_not_enabled`: the native NyxID subject is outside the current managed rollout; repair eligibility, not a sandbox.
- `managed_user_services_unavailable`: the user does not have the required directly owned active `chrono-sandbox` and usable `chrono-llm-public` UserServices; repair NyxID service readiness.
- `managed_credential_unavailable`: Aevatar could not resolve its per-user managed invocation
  credential. Re-run the explicit authenticated credential `POST`, read `GET`, and continue only
  when execution readiness is true; ordinary execution does not repair it. Inspect descriptor and
  secret-vault readiness without exposing the secret.
- `managed_proxy_authorization_denied`: NyxID denied the exact proxy request; inspect user/service ownership and the current internal delegation scope.
- `managed_proxy_target_unavailable`: the exact `chrono-sandbox` UserService or `/codex/execute` route is unavailable.
- `managed_proxy_timeout`: the NyxID proxy/chrono-sandbox request timed out. Preserve the diagnostic evidence and inspect that transport boundary first.
- `managed_proxy_unavailable`: the proxy/chrono service is temporarily unavailable or capacity-limited.
- `managed_response_invalid` or `managed_response_too_large`: chrono-sandbox violated the bounded terminal-response contract.
- `managed_execution_nonzero_exit`: the one-shot Codex runner reached a terminal non-zero exit; inspect sanitized runner diagnostics.
- Private `node_offline`, service, target, principal, key, or host-key errors belong to the user's NyxID SSH route.
- Private Codex PATH, login, Git-root, wrapper, or timeout errors belong to the selected host and fixed runner wrapper.

The managed workload runs under gVisor, which is the current isolation boundary. Codex runs inside it with the fixed runtime profile. Landlock, Bubblewrap, sandbox-side Credential Vault substitution, and a TLS credential proxy are not deployed in this runtime and are not repair paths for these failures.

Use `aevatar-codex-exec-node-setup` for detailed setup or repair after locating the boundary. Never declare readiness from configuration inspection alone.
