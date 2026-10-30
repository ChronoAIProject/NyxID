# Platform BotFather live diagnosis

Observed on 2026-10-16 through the existing authenticated NyxID CLI account and
the configured API at `https://nyx-api.chrono-ai.fun`. No bot message was sent,
runtime enabled, credential rotated, or deployment changed.

## Reproduced failure

The native manager is `@nyxid_manager_bot` (Telegram ID `8631737144`). Its channel
is active and reports a registered webhook. Its converted platform profile is
`0b4949e8-be13-4abc-8b65-7b1d33d4aa11`, generation 1, with `enabled: false`,
`guide: null`, and no retirement marker.

`GET /api/v1/admin/platform-bots/{profile_id}/runtime` returned HTTP 200 with
`supported: true`, `runtime: null`, and readiness reason `runtime_uninitialized`.
Thus neither the new public nor personal runtime has been initialized.

The current admin UI's initial discovery request was reproduced against the live
server. `POST /api/v1/admin/platform-bots/{profile_id}/public/profiles` used:

```json
{
  "initialize": true,
  "expected_profile_generation": 1,
  "expected_manager_authority_epoch": 0,
  "expected_personal_control_revision": 0,
  "expected_public_policy_revision": 0,
  "public_policy": {
    "source": "nyxagent",
    "total_daily_credits": "10",
    "sender_daily_credits": "0.5"
  }
}
```

The sponsor and public default were left to the existing server-owned selection.
These budget values are the current UI defaults for discovery, not saved policy.
The response was HTTP 200:

```json
{
  "profiles": [],
  "default_profile": null,
  "readiness": {
    "status": "unavailable",
    "reason": "public_contract_unsupported",
    "checked_at": null,
    "expires_at": null,
    "evidence_digest": null,
    "policy_digest": null
  }
}
```

Discovery does not perform the paid readiness model call. No readiness probe or
runtime update was submitted. This is live evidence of the discovery failure,
not evidence of Telegram end-to-end delivery or a completed fix.

## Source reconciliation

The handoff checkout started at NyxID `647ce172a`. That revision still had the
unconditionally unavailable public producer. The clean branch was fast-forwarded
to `5aed82c57`, including PR #1914 (`0c3297cf1`), which replaces that producer with
the signed runtime handshake. Do not attribute the old hard-coded rejection to
the current live server: the observed `public_contract_unsupported` response is
from the newer contract-aware path.

NyxAgent main was independently inspected at
`57ab8be89618a61b98e2ebc76331d121f96d731b`. Its `src/api.rs` advertises image and
steering capabilities and lacks the public control-plane routes required by
NyxID. The exact running upstream binary was not identified. The live discovery
failure and inspected source agree that NyxID cannot establish a public contract.

## Remaining implementation

The upstream work is in `ChronoAIProject/NxyAgent`. Implement the already-defined
[public runtime deployment contract](2026-10-16-platform-public-runtime-contract.md)
and use [the wire fixture](../fixtures/platform-public-v1.json):

1. Validate the dedicated signed NyxID authority and configured issuer/JWKS;
   add the strict public profile discovery and readiness endpoints.
2. Bind public execution to the verified profile, runtime, caller tuple, bounded
   model callbacks, and exactly the three approved public tools. Cover every
   model attempt and compaction in the declared execution and cost limits.
3. Enforce absolute public-session expiry and exact cleanup after execution-key
   revocation, including cleanup-before-execution races.
4. Run cross-service contract and containment tests, then deploy the compatible
   upstream build through a separately authorized rollout. Repeat discovery and
   the bounded readiness probe before enabling the reviewed manager settings.

Personal access can be explicitly initialized independently of public readiness.
That changes the available product behavior and must not be silently substituted
for the combined Enable bot action. The original onboarding guide has its own
configuration and also remains paused on this profile.
