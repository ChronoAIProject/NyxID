# Verification and Troubleshooting

## Managed failures by boundary

Preserve only the typed code and sanitized `diagnostic_id`. Never request a raw upstream response, NyxID key, bearer, delegation token, or process environment.

### Host and admission

| Code | Meaning | Check |
|---|---|---|
| `target_not_configured` | This Aevatar host did not register the managed target | Ask operations to verify host composition |
| `managed_target_disabled` | Managed Codex is disabled in deployment configuration | Ask operations to verify `Enabled` and the internal rollout configuration |
| `managed_feature_not_enabled` | The authenticated native NyxID user is not eligible | Compare the verified user ID with the configured `Allowlist`, or confirm the internal `All` policy |
| `managed_identity_unavailable` | Aevatar lacks an exact native NyxID identity for this call | Re-authenticate through the normal Aevatar/NyxID path; do not infer identity from another ID |
| `managed_user_authorization_unavailable` | The explicit credential mutation lacks the current user's authorization | Call the credential lifecycle from an authenticated user request; do not supply or log the bearer manually |

### NyxID readiness and credential

| Code | Meaning | Check |
|---|---|---|
| `managed_user_services_unavailable` | The user's required `chrono-sandbox` or `chrono-llm-public` UserService is absent, ambiguous, inactive, or unusable | Inspect only the user's service inventory and ownership; do not provision a key manually |
| `nyxid_identity_mismatch` | The authenticated identity does not own the authorization used for readiness | Stop and sign in as the intended native NyxID user |
| `managed_credential_untracked_key_exists` | NyxID has conflicting active managed invocation keys | Escalate for controlled reconciliation using the sanitized diagnostic evidence |
| `managed_credential_mutation_in_progress` | Another credential mutation owns the per-user lease | Retry after the in-flight mutation reaches a terminal state |
| `managed_credential_commit_timeout` | The explicit credential mutation did not commit within its bounded deadline | Read status before deciding whether a retry is safe; if repeated, correlate the sanitized diagnostic with credential lifecycle logs |
| `managed_credential_cleanup_pending` | Obsolete key cleanup has not completed | Retry later; do not create another key |
| `managed_credential_persistence_pending` | A successful remote change is not yet durably recorded | Retry later and preserve the typed code for reconciliation |
| `managed_credential_vault_unavailable` | Aevatar's internal `ISecretVault` could not persist or resolve the invocation key | Ask operations to repair Aevatar secret storage; this is not sandbox-side injection |
| `managed_credential_unavailable` | The committed invocation credential cannot be used | Authenticated `POST /api/managed-codex/credential`, then `GET`; continue only when `execution_ready=true` and reason is `ready` |
| `managed_credential_invalid` | The committed credential descriptor violates the managed contract | Escalate for controlled credential reconciliation; never edit or expose raw secret material |

### Proxy and chrono transport

| Code | Meaning | Check |
|---|---|---|
| `managed_proxy_authorization_denied` or downstream `UNAUTHORIZED` | NyxID or chrono-sandbox rejected the invocation authority; a common cause is `forward_access_token=false` on the caller's exact `chrono-sandbox` UserService | Read the authoritative `GET /api/v1/keys` inventory. If that exact account-owned UserService reads false, send `PUT /api/v1/user-services/{userServiceId}` with `{"forward_access_token":true}`, then read back the same ID before retesting. Never substitute its different `catalog_service_id`; catalog resync is admin-only |
| `managed_proxy_target_unavailable` | The exact personal `chrono-sandbox` proxy target cannot be resolved | Verify the user's active `chrono-sandbox` UserService and deployment route |
| `managed_proxy_timeout` | NyxID proxy or chrono-sandbox did not return within the bounded transport wait | Correlate the sanitized diagnostic at NyxID/chrono-sandbox; do not repair local sandbox tooling |
| `managed_proxy_unavailable` | NyxID proxy or chrono-sandbox is temporarily unavailable or capacity-limited | Retry later and escalate repeated failures with the sanitized diagnostic |

### Terminal contract

| Code | Meaning | Check |
|---|---|---|
| `managed_response_invalid` | chrono-sandbox returned a malformed or incomplete terminal contract | Inspect bounded chrono-sandbox diagnostics without requesting raw secrets |
| `managed_response_too_large` | The bounded response limit was exceeded | Reduce task output; use a private artifact path for large results |
| `managed_execution_nonzero_exit` | The fixed Codex process exited unsuccessfully | Use the sanitized diagnostic to inspect runner/model failure; do not change caller-controlled flags |
| `managed_execution_cancelled` | The caller cancelled the managed execution | Retry only if the original task is still wanted |
| `managed_execution_failed` | Aevatar caught an unclassified terminal managed failure | Correlate the sanitized code and diagnostic across Aevatar, NyxID, and chrono-sandbox |

## Private SSH end-to-end probe

Construct the same fixed command Aevatar uses and call it through NyxID:

```bash
prompt='Reply exactly CODEX_EXEC_READY. Do not inspect files, run tools, or modify anything.'
encoded_prompt="$(printf '%s' "$prompt" | base64 | tr -d '\n')"
remote_command="p='$encoded_prompt'; { printf '%s' \"\$p\" | base64 --decode 2>/dev/null || printf '%s' \"\$p\" | base64 -D; } | codex exec -"

nyxid ssh exec "$service_slug" \
  --principal "$PRINCIPAL" \
  --output json \
  -- "$remote_command"
```

Expect `exit_code: 0`, `timed_out: false`, and `CODEX_EXEC_READY` in stdout.

Also send `id -un` through the same service. Expect the forced wrapper to reject it with exit code `126`.

## Failure Map

| Symptom | Meaning | Check |
|---|---|---|
| `No NyxID access token available` | Aevatar did not receive the caller token | Log into Aevatar with the same NyxID account |
| node absent from `node list` | Node belongs to another account/profile | Compare account identity and register a new isolated profile |
| `node_offline` | Node daemon is disconnected | `nyxid node daemon status --profile ...` and node logs |
| `No node agent is bound` | Workflow value is wrong or service lacks binding | Pass the SSH service slug, inspect `nyxid service show` |
| `target_not_allowed` | Node SSH allowlist excludes host/port | Fix `[ssh].allowed_targets`, restart profile |
| `SSH key missing` | Credential was stored in another node profile or selector | Match service slug and principal; set `NYXID_PROFILE` while adding |
| host-key mismatch | Target identity changed | Investigate; rotate the pin only after verifying the new fingerprint |
| exit `126` for a Codex request | Wrapper pattern no longer matches | Compare the exact Aevatar command contract and wrapper suffix |
| `codex: command not found` | Noninteractive SSH PATH lacks Codex | Use an absolute `CODEX_BIN` in the wrapper |
| login/auth failure | Target principal lacks valid Codex auth | Run `codex login status` as that principal |
| git repository required | Wrapper workspace is wrong | Set `WORKSPACE` to an absolute git root |
| request times out | NyxID SSH limit reached | Keep synchronous work under 300 seconds or use long-running handoff |
| output truncated | Stream exceeded NyxID capture limit | Store artifacts separately; keep stdout/stderr below 1 MiB each |

## Inspect Without Leaking Secrets

- Read node logs, but redact values beginning with `nyx_` before sharing them.
- Inspect node config structure, but do not print encrypted token/key fields.
- Use `codex login status`; never print `auth.json`.
- Use `nyxid service show`; never publish node registration or SSH private keys.
