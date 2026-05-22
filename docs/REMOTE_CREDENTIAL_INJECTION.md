# Remote Credential Injection

Design document for issue [#773](https://github.com/ChronoAIProject/NyxID/issues/773). Status: **proposed** — eng-reviewed (9 issues raised + resolved). Ready for implementation.

## Context

Today, supplying a secret to a node-managed service is a strictly two-party flow:

1. Org admin runs `nyxid node-credential push <node> --slug <s> ...` from anywhere. NyxID creates a pending credential record with display metadata but **no secret value**. The CLI explicitly says "do not send the secret value; it is entered on the VM."
2. **Operator on the node host** runs `nyxid node credentials accept <slug>` *on the machine where the node agent is installed*, pastes the secret at the prompt, and the node-side agent stores it in the local secret backend.

This separation enforces a critical invariant: **the secret value never traverses or persists on the NyxID server.** It also separates duties — the admin who decides *which credentials a service should have* is not necessarily the operator who *handles the secret material*.

Per [#773](https://github.com/ChronoAIProject/NyxID/issues/773), the step-2 friction has become a real blocker: org admins managing remote shared nodes (different sites, different timezones, no pre-existing SSH access) cannot supply the secret without either physical presence at the node or pre-existing SSH access. The issue's stated goal:

> The Nyx network is exactly the substrate that should make remote management possible **without growing an SSH dependency tree**.

This document proposes a protocol that lets an org admin supply the secret from any device on the public internet while strictly preserving the "secret never on NyxID server" invariant.

## Goals

- **G1.** Org admin can supply the secret to a node-managed pending credential from any device with a browser. No SSH to the node host required.
- **G2.** NyxID server only ever sees opaque ciphertext. The plaintext secret never traverses NyxID memory, never enters NyxID storage, never appears in NyxID audit logs.
- **G3.** Existing CLI two-party flow continues to work unchanged. The new flow is additive, not a replacement.
- **G4.** Forward secrecy: compromise of NyxID server data at rest does not reveal past secrets even if a node's long-lived state is later compromised.
- **G5.** Existing operator-on-node "accept" gate remains the default; the new flow is opt-in per node. Orgs running strict separation-of-duties today do not get a behavior change by default.
- **G6.** Protection against malicious code-substitution by a fully-compromised NyxID server. Achieved via Subresource Integrity (SRI), signed release channel, and admin verification UX (Phase 4.5 below).

## Non-goals

- Protection against a malicious node agent (the node holds credentials by design).
- Protection against an admin who is themselves malicious or compromised.
- Replacing the existing CLI push flow. The intent / metadata side stays the same.
- Per-credential audit of secret content. Audit remains metadata-only.
- HSM-backed node X25519 keypairs (uses existing local secret backend).
- Mobile native client support (mobile browser works against the browser flow).

## Threat model

Adversaries we defend against:

| # | Adversary | Defense |
|---|-----------|---------|
| T1 | Fully-compromised NyxID server (malicious operator, RCE, hostile fork) | E2E encryption + **Code-integrity controls (Phase 4.5)** — with an explicit operational caveat: NyxID serves the standalone HTML, the SRI hashes inside it, the displayed fingerprint, and the "verify" button. A fully-compromised server can substitute all of them in lockstep. Phase 4.5's defense is **detection assuming the admin independently verifies the displayed fingerprint out-of-band** (e.g., opens `releases.nyxid.dev` from a separate browser/device and compares). Without active admin verification, T1 degrades in practice to "T2 only" (passive-read protection). The signed manifest at a separate origin is what makes verification possible at all — see Phase 4.5 § "Admin verification UX" for the operational expectations the defense relies on. |
| T2 | Passive read access to NyxID storage or memory (DB dump, backup leak, future operator with archive) | E2E encryption alone: server only stores ciphertext bound to a single pending credential |
| T3 | Active MITM past TLS termination at NyxID (rogue middlebox, compromised CA proxy chain) | AEAD authenticity + freshness binding catch substitution |
| T4 | Replay of a captured ciphertext blob against the same pending credential | Atomicity guard: first-push-wins, returns 409 on second push (per §"Race protection") |
| T5 | Replay of a captured ciphertext blob against a *different* pending credential or node | AEAD associated data binds ciphertext to `(node_id, pending_credential_id, slug)` |
| T6 | Adversary later steals the node's persistent state and decrypts past pushes | Ephemeral X25519 keypair per pending credential, sealed at rest by the node's long-lived auth key, dropped on consume — gives forward secrecy |
| T7 | Race-to-brick: attacker submits garbage ciphertext faster than the legitimate admin | Per-pending-credential rate limit (1 successful POST, 3 failed in 60s, then 5-min lockout); pending IDs have ~256 bits of entropy |
| T8 | Node restart between pubkey post and ciphertext receipt | Ephemeral privkey persisted to node's local secret backend, sealed by long-lived auth key (survives restart, dropped on consume/decline/expire) |

Out of scope:

- Phishing of the admin (attacker tricks admin into supplying secret to attacker-controlled UI). Same risk surface as the current CLI prompt today.
- Malicious admin pushing a legitimately-encrypted credential pointing at an attacker URL. The operator-on-node accept step (manual-accept mode, default per §"Accept gate") and the existing cloud-metadata target_url block (#770) are the relevant defenses.

## Options considered

### Option A — Browser-driven e2e encrypted relay (CHOSEN)

NyxID brokers an opaque ciphertext blob from the admin's browser to the node over the existing WSS connection. The browser does the encryption; the node does the decryption; NyxID never touches plaintext. Detailed below.

### Option B — Time-windowed one-time link + admin-supplied passphrase

Rejected. Introduces a new distributed secret (the passphrase) that the admin must convey to themselves through some other channel, with its own replay / phishing surface. For no real security benefit over Option A.

### Option C — Push secret over the admin's existing SSH tunnel to the node

Rejected as primary. Reuses the existing `nyxid ssh` infrastructure but directly contradicts the issue's stated goal of removing SSH as a hard prerequisite. Retained as a *fallback* documented in `nyxid node-credential push` help text for admins who already have SSH set up.

## Proposed protocol (Option A)

### Cryptographic primitives

| Purpose | Primitive | Rationale |
|---------|-----------|-----------|
| Key exchange | X25519 ECDH | Compact, widely supported, suitable for ephemeral use |
| Key derivation | HKDF-SHA256 | Standard, takes ECDH shared secret + binding context as `info` |
| Authenticated encryption | XChaCha20-Poly1305 | Random-nonce-safe AEAD, good cross-stack support |
| Privkey sealing at rest | Stored via the existing `cli/src/node/secret_backend.rs::SecretBackend` trait (same mechanism that backs `store_auth_token` / `store_signing_secret` / `store_credential_value`) — keychain on macOS, `secret-tool` on Linux, encrypted file otherwise | No new sealing primitive; reuse the backend that already protects the node's long-lived auth token and per-service credentials |
| Encoding | Base64url for keys/nonces, raw bytes for ciphertext over WSS | URL-safe for HTTP, compact for WSS binary frames |

### Data model

A new optional sub-document on `NodePendingCredential`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CryptoBundle {
    /// Protocol version. "v1" for the protocol described in this document.
    pub version: String,
    /// Node's per-pending X25519 ephemeral public key (32 bytes, base64url).
    pub node_pubkey: String,
    /// Admin's per-push X25519 ephemeral public key. None until ciphertext is posted.
    pub admin_pubkey: Option<String>,
    /// AEAD nonce (24 bytes for XChaCha20-Poly1305, base64url).
    pub nonce: Option<String>,
    /// AEAD ciphertext + auth tag, opaque to NyxID. Capped at 16 KB.
    ///
    /// On the wire: base64url-encoded string when traversing JSON WSS
    /// frames (`pending_credential_ciphertext`) and JSON HTTP request
    /// bodies (`POST /ciphertext`). Stored as BSON binary in MongoDB.
    /// The `Vec<u8>` here represents the raw bytes after base64url
    /// decode; serde with a `#[serde(with = "base64url_bytes")]`
    /// adapter handles both transports.
    pub ciphertext: Option<Vec<u8>>,
}

// On NodePendingCredential:
pub crypto: Option<CryptoBundle>,
```

State transitions:

```
created                           crypto = None
  ↓ node receives pending event
  ↓ node generates X25519, seals privkey to local store
  ↓ node posts pubkey
pubkey_posted                     crypto.node_pubkey = <set>, rest = None
  ↓ admin submits ciphertext (POST /ciphertext)
  ↓ atomicity guard: find_one_and_update { crypto.ciphertext: null }
ciphertext_received               crypto fully populated
  ↓ NyxID forwards over WSS to node
  ↓ node decrypts, validates AAD
  ├── success → consumed (existing accept path) → privkey dropped from sealed store
  └── failure → decrypt_failed state, admin can cancel + re-create (NOT retry on same pending)
```

### Flow

```mermaid
sequenceDiagram
    autonumber
    participant A as Admin Browser
    participant N as NyxID Server
    participant W as Node Agent (WSS)
    participant LS as Node Local Sealed Store
    participant CS as Node Credential Store

    Note over A,W: Phase 1 — push intent (unchanged from today)
    A->>N: POST /nodes/{id}/credentials/push<br/>{slug, injection_method, label, ...}<br/>NO secret value
    N->>W: pending_credentials_available {} (existing plural nudge, no metadata)
    W->>N: GET /api/v1/node-agent/pending-credentials (existing pull)
    activate W

    Note over W,LS: Phase 2 — node posts ephemeral pubkey
    W->>W: for each pending credential lacking a sealed privkey:<br/>generate X25519 keypair
    W->>LS: seal privkey via SecretBackend (keychain / secret-tool / encrypted file), keyed by pending_id
    W->>N: pending_credential_pubkey {pending_id, node_pubkey} (NEW frame)
    N->>N: store CryptoBundle{node_pubkey} on NodePendingCredential

    Note over A,W: Phase 3 — admin encrypts + posts
    A->>N: GET /nodes/{id}/credentials/pending/{pending_id}
    N->>A: {pending_id, crypto.node_pubkey, slug, node_id, ...}
    A->>A: verify frontend SRI hash + signed-release fingerprint<br/>(Phase 4.5 gate)
    A->>A: generate ephemeral keypair<br/>ECDH(admin_priv, node_pubkey) → shared<br/>HKDF(shared, info=node_id‖pending_id‖slug‖"v1")<br/>XChaCha20-Poly1305 encrypt secret<br/>AAD=node_id‖pending_id‖slug‖"v1"
    A->>N: POST /nodes/{id}/credentials/pending/{pending_id}/ciphertext<br/>{admin_pubkey, nonce, ciphertext (≤16 KB)}
    N->>N: find_one_and_update guard: 409 if ciphertext already set<br/>per-pending rate limiter check
    N->>W: pending_credential_ciphertext {pending_id, admin_pubkey, nonce, ciphertext}

    Note over W,CS: Phase 4 — node decrypts and (optionally) accepts
    W->>LS: load sealed privkey for pending_id
    W->>W: ECDH(node_priv, admin_pubkey) → shared<br/>HKDF(shared, info=...)<br/>AEAD decrypt + verify AAD
    alt Decryption succeeds AND node has auto_accept enabled for this binding
        W->>CS: store secret via existing CredentialStore::accept
        W->>LS: drop sealed privkey
        W->>N: pending_credential_consumed {pending_id, ok}
        N->>N: mark pending consumed, emit metadata-only audit event
    else Decryption succeeds AND manual-accept (default)
        W->>W: hold decrypted secret in volatile ready-to-accept queue with TTL
        W->>N: pending_credential_decrypted {pending_id} (operator confirmation required)
        Note over W: Operator runs `nyxid node credentials accept` on the node<br/>(no paste required — just confirmation)
    else Decryption fails
        W->>LS: drop sealed privkey (single-use)
        W->>N: pending_credential_error {pending_id, code=8006 decrypt_failed}
        N->>A: 4xx with reason
    end
    deactivate W
```

### Multi-node fan-out (Phase 3.5)

For services with `fallback_node_ids`, the admin can fan out one logical push to N nodes:

1. Frontend issues N independent `pending_credential_pubkey` requests; each node posts its own ephemeral pubkey.
2. Browser encrypts the same plaintext secret N times — once per node's pubkey, with N distinct ciphertexts.
3. All N ciphertexts posted in a single POST `/credentials/push/fan-out` request, atomically.
4. Each node decrypts independently; per-node state tracked on the pending credential.
5. **All-N semantics with retry:** the logical pending credential is marked `consumed` only when all N nodes successfully decrypt + accept. Partial success is held in `partial_decrypted` state with a per-node breakdown; admin sees which nodes succeeded and which failed.
6. **Retry-only-failed nodes:** the frontend exposes a "retry failed nodes" action that re-runs Phase 3 against the subset of nodes still in `decrypt_failed` state — fresh ephemeral keypairs on those nodes, fresh ciphertext from the admin's browser. Successful nodes are untouched (their credentials are idempotent — see Design Review Feedback §2). Logical credential transitions `partial_decrypted → consumed` only when all N nodes have accepted.
7. **Expiry while in `partial_decrypted`:** when `NodePendingCredential.expires_at` fires and the logical credential is still in `partial_decrypted`, the logical state moves to `expired`. Previously-accepted node-level credentials are **not** rolled back — by the idempotency principle from §2 they are safe to leave in place, and the operator can rotate them via a fresh push if needed. The admin is shown which nodes succeeded so they can decide whether to start over or live with the partial state.

### Race protection

POST `/ciphertext` is atomic: backend uses the Rust `mongodb` driver's `find_one_and_update` (MongoDB `findAndModify` semantics) with the guard `{ "crypto.ciphertext": null }` to claim the slot. The first successful push wins; subsequent POSTs return `409 Conflict` until the pending expires or admin cancels. The handler also checks `crypto: { $exists: true, $ne: null }` so legacy pending credentials (no `CryptoBundle` at all) cannot accidentally satisfy the guard.

Per-pending-credential rate limit: 1 successful POST, max 3 failed POSTs in a 60-second window. After 3 failed attempts, the pending is locked for 5 minutes. **Mechanism:** an in-memory `DashMap<PendingId, RateLimitState>` keyed by `pending_credential_id` (TTL-ephemeral, never persisted to MongoDB) — distinct from the global per-actor `RATE_LIMIT_PER_SECOND` middleware, which still applies on top. Decrypt failures from the node are NOT counted against this limit (they happen after NyxID has already forwarded the ciphertext); they trigger an immediate `decrypt_failed` state which is terminal for that pending credential.

### Sync vs async response semantics

`POST /ciphertext` must reflect the node's decryption outcome in its HTTP response — implementers cannot fire-and-forget. The pattern already exists at `backend/src/services/node_ws_manager.rs:1558` (`send_credential_update_and_wait`):

1. Handler allocates a `request_id`, registers a `oneshot::channel` waiter on the node connection's `credential_acks` map.
2. Sends the `pending_credential_ciphertext` WSS frame with the `request_id`.
3. Awaits the node's reply (`pending_credential_consumed {request_id, ok}` or `pending_credential_error {request_id, code, reason}`) with a configurable timeout (suggest 15s, matching the existing pattern).
4. HTTP response shape:
   - 200 on `consumed`
   - 4xx with error code 8006-8011 on `pending_credential_error`
   - 503 `NodeOffline` (code 8010) on timeout / WSS disconnect

Reuse the `oneshot` + timeout machinery — do not invent a new mechanism. The handler MUST block on the ack before returning; committing the ciphertext to MongoDB and returning 200 without confirmation would leave admin and node out of sync (same failure class as the historical issue captured in the `send_credential_update_and_wait` comments at lines 1599-1632).

### Backward compatibility & version detection

Two separate signals — do not conflate them:

**1. Feature detection (synchronous, cached).** Node agents advertise `supported_features` (a new field added in Phase 1; current node agents do not advertise it). The new flow contributes `crypto_v1` to that set. NyxID persists `Node.supported_features` (new model field; populated from the existing in-memory `record_capabilities` path in `node_ws_manager.rs:1705` if the codebase already has it, or added in Phase 1 if not). The frontend queries it before rendering UI:

- `crypto_v1 ∈ supported_features` → show browser accept UI
- `crypto_v1 ∉ supported_features` (older agent or unupgraded node) → show legacy "SSH to node and run `nyxid node credentials accept`" instructions

Feature detection is fast and cached. No polling for this step.

**2. Per-pending pubkey readiness (asynchronous).** Once the admin pushes a pending credential, the node — if it supports `crypto_v1` — must generate its X25519 keypair and post the pubkey. This is async (the node may be currently busy, briefly disconnected from WSS, etc.). The frontend polls `GET /credentials/pending/{id}` with exponential backoff up to ~30s:

- Pubkey arrives → proceed with the encrypt+POST flow
- Times out → surface a clear "node not responding for crypto exchange" error (distinct from "legacy node" — this is a supported but unresponsive node)
- Error code `8009 PendingCredentialPubkeyAwaiting` covers the in-flight 404 state

Polling is bounded to a `crypto_v1`-supporting node. There is no polling against legacy nodes — feature detection already steered the UI to the SSH instructions.

The existing two-party CLI flow keeps working regardless. A pending credential with `crypto: None` continues to accept secrets via the legacy CLI path.

### WSS frame classification

The new `pending_credential_pubkey` and `pending_credential_ciphertext` frames are **internal node-control protocol traffic** — same class as `node_metrics`, `proxy_request`, `proxy_response_*`, `pending_credential_available`, and `pending_credential_consumed`. They bypass the `ws_frame_injections` rules on `DownstreamService` / `UserService` (which apply only to downstream-service WS passthrough). Implementation note for Phase 1: register the new frame types in `node_ws.rs` alongside the existing node-control variants, not via the injection plumbing in `cli/src/node/ws_frame_injector.rs`.

### Accept gate

Per-node config flag `enable_remote_credential_injection: bool` (default **false** — opt-in). Even when enabled, the node defaults to manual-accept:

- **Manual accept (default when `enable_remote_credential_injection: true`):** decrypted secret sits in a volatile ready-to-accept queue. Operator on the node runs `nyxid node credentials accept` to confirm — no paste required, just a `y/N` prompt. Preserves separation-of-duties.
- **Auto-accept (per-binding opt-in, second flag):** decrypted secret is stored immediately. Skips operator confirmation. Recommended only for orgs that explicitly want unattended remote rotation.

Default behavior of any node remains "legacy two-party CLI flow" until an admin explicitly opts in.

## Code-integrity infrastructure (Phase 4.5)

Defends T1 ("fully-compromised NyxID server"). The protocol's e2e encryption is only as strong as the JS that performs it; if NyxID can substitute the JS, it can capture plaintext before encryption. This phase makes that substitution detectable.

### Standalone credential-accept page

The credential-accept page is served as a **minimal standalone HTML document**, not as a route inside the main SPA bundle. Goal: minimize the "what else could be subverted" surface on this high-value page.

- Strict CSP: `default-src 'none'; script-src 'self' 'sha384-...' 'sha384-...'; connect-src 'self'; style-src 'self' 'unsafe-inline'` — no third-party origins, no inline scripts, no remote fetches except back to NyxID.
- No main SPA bundle, no shared dependencies. Just: a tiny form-handling shim (~5 KB), `@noble/curves` x25519 (~6 KB minified), `@noble/ciphers` xchacha20-poly1305 (~5 KB minified), and the NyxID crypto-wrap module (~2 KB). Realistic page weight: **≤30 KB gzipped total**, not 5 KB. If the SubtleCrypto X25519 polyfill is needed (older browsers), lazy-load `@noble/curves` only on miss to keep the cold-cache bundle leaner for the majority case where SubtleCrypto works natively.
- Page bundle is itself part of the signed release manifest (see below); its top-level HTML hash is published the same way as the JS bundles.
- Route: `/credentials/pending/.../accept` is served by a dedicated handler that emits this standalone HTML, separate from the SPA's catch-all route.

### SRI hashes on crypto JS bundles

The standalone HTML page carries SHA-384 SRI hashes on every `<script>` tag for `@noble/curves`, `@noble/ciphers`, and the NyxID crypto-wrapping module. Browsers refuse to execute scripts that don't match the hash. NyxID server cannot silently substitute the JS without changing the HTML's SRI attribute, which is itself loaded over TLS.

### Signed release channel

Frontend builds publish SHA-384 hashes of every crypto-related JS bundle to a separate, signed channel:

- GitHub Releases, signed by the release pipeline's GPG key
- A `releases.json` manifest published to a domain *separate* from the NyxID server (e.g., `releases.nyxid.dev`), CDN-cached, content-hash-pinned

Admins can verify the hash of the JS they're about to run against the published manifest.

### Admin verification UX

Before the credential-accept form renders the secret input:

1. The page computes the SHA-384 of the currently-loaded `@noble/*` + crypto-wrap bundles (using SubtleCrypto on the bundle content).
2. Displays the hash as a short fingerprint (first 12 hex chars) above the form.
3. Provides a one-click "verify" button that opens `releases.nyxid.dev/manifest` in a new tab. Admin compares the fingerprint.
4. Below the form: a checkbox "I verified the fingerprint" gates the submit button. Pre-checked for orgs that explicitly opt out of verification (per-org policy flag).

Verification is per-session, not per-push. Session expiry is 30 minutes (matches `JWT_RELAY_REPLY_TTL_SECS`).

### Operational story

- Each release pipeline run publishes a signed `releases.json` to the separate domain.
- Rollback paths: if a release is found compromised, the manifest is invalidated and admins see a "manifest mismatch" error gating new submits.
- Bundle changes require a coordinated release (HTML + JS + manifest update in lockstep).

### Phase 4.5 runbook items (must be answered before this phase starts)

These are infrastructure questions that should be scheduled, not discovered late:

| Question | Owner |
|---|---|
| Who holds the GPG signing key for the release manifest? Is it a hardware-backed key (YubiKey, HSM) or a software key in CI? | Infra |
| Key rotation procedure: when, how, who has the authority, how is the rotation announced to admins (so they know to refresh their trust anchor)? | Infra + Security |
| Where does `releases.nyxid.dev` actually live? DNS records, CDN provider, TLS cert chain — and is it operationally separable from the main NyxID server (i.e., compromise of the main server should not enable manifest tampering). | Infra |
| How does the frontend build pipeline publish the manifest? Is it part of the same CI job that builds the bundle, or a separate signing step requiring human approval? | Release engineering |
| Manifest format: schema, signature scheme (GPG detached signature? signify? cosign?), versioning. | Security |
| How long is a manifest valid? Does it expire (forcing periodic re-signing) or live indefinitely until superseded? | Security |

Implementer should document the chosen answers in `docs/RELEASE_INTEGRITY.md` or equivalent before Phase 4.5 lands.

## Error codes

Reserved range **8006–8011** for remote credential injection errors. The next-available slot in the existing node-error block: `backend/src/errors/mod.rs` already assigns 8000-8005 (8000 `NodeNotFound`, 8001 `NodeOffline`, 8002 `NodeProxyTimeout`, 8003 `NodeRegistrationFailed`, 8004 `NodeCredentialMissing`, 8005 `WsProxyDownstream`).

| Code | Name | Returned by | Meaning |
|------|------|-------------|---------|
| 8006 | `PendingCredentialDecryptFailed` | Node → NyxID | AEAD decrypt or AAD verify failed. Terminal for this pending. |
| 8007 | `PendingCredentialVersionUnsupported` | Node → NyxID | `crypto.version` not recognized. Likely a protocol drift. |
| 8008 | `PendingCredentialCiphertextTooLarge` | NyxID POST handler | `len(ciphertext) > 16 * 1024`. Returns 413. |
| 8009 | `PendingCredentialPubkeyAwaiting` | NyxID GET handler | Node has not yet posted the per-pending pubkey. Returns 404. Frontend polls with backoff up to ~30s (see §"Backward compatibility & version detection") — distinct from feature-detection fallback to legacy SSH UI. |
| 8010 | `PendingCredentialNodeOffline` | NyxID POST handler | Node currently not connected via WSS. Ciphertext queued; admin sees "waiting for node" state. |
| 8011 | `PendingCredentialQueueFull` | NyxID POST handler | Per-node ciphertext queue at the 5-pending-per-node cap (per Design Review Feedback §3). Returns 429. |

The authoritative error-code listing lives in `AGENTS.md` under the node-error bullet (currently states "Error codes 8000-8003 are reserved for node errors" — that text is itself out of date: 8004 `NodeCredentialMissing` and 8005 `WsProxyDownstream` already exist in `backend/src/errors/mod.rs`). Phase 1 must update that bullet to reflect the actual occupied range (8000-8005) plus the new RCI range (8006-8011), so the documentation source of truth stays consistent with the code.

## Test Strategy

Mandatory test coverage, locked at design time. Implementer must produce tests for every path below before merging.

### Node-side crypto (Rust, `cli/src/node/credentials/crypto.rs`)

| Test | File | Asserts |
|------|------|---------|
| `roundtrip_encrypt_decrypt_succeeds` | `crypto::tests` | Generate keypair, encrypt secret, decrypt, plaintext matches |
| `wrong_aad_fails_decryption` | `crypto::tests` | Encrypt with AAD=A, decrypt with AAD=B, returns Err |
| `wrong_nonce_fails_decryption` | `crypto::tests` | Encrypt, flip one nonce byte, decrypt returns Err |
| `wrong_admin_pubkey_fails_decryption` | `crypto::tests` | Encrypt with admin_priv_A, decrypt with admin_pub_B, returns Err |
| `persist_and_reload_privkey_across_restart` | `crypto::tests` | Seal privkey, drop in-memory state, reload from sealed store, verify decrypt still works |
| `privkey_evicted_on_consume` | `agent::tests` | After accept, sealed privkey is removed from local store |
| `privkey_evicted_on_decline` | `agent::tests` | After decline, sealed privkey removed |
| `privkey_evicted_on_cancel` | `agent::tests` | After admin cancels, sealed privkey removed |
| `privkey_evicted_on_expire` | `agent::tests` | After TTL passes, sealed privkey removed by sweep |

### Backend endpoints (Rust, `backend/src/handlers/node_admin.rs`)

| Test | File | Asserts |
|------|------|---------|
| `post_ciphertext_rejects_over_16kb` | `node_admin::tests` | 16385-byte ciphertext returns 413 with code 8008 |
| `post_ciphertext_first_push_wins` | `node_admin::tests` | Two concurrent POSTs: one returns 200, the other returns 409 |
| `post_ciphertext_per_pending_rate_limit` | `node_admin::tests` | 4 failed POSTs within 60s: 4th returns 429 + 5min lockout |
| `get_pubkey_404_until_node_posts` | `node_admin::tests` | Pre-pubkey-post, GET returns 404 with code 8009 (`PendingCredentialPubkeyAwaiting`) |
| `crypto_bundle_serde_roundtrip` | `models::node_pending_credential::tests` | CryptoBundle serialize → BSON → deserialize, all fields preserved |

### Frontend (TypeScript, `frontend/src/lib/crypto/`)

| Test | File | Asserts |
|------|------|---------|
| `noble_curves_x25519_ecdh_roundtrip` | `crypto.test.ts` | Browser-side keygen + ECDH produces same shared secret in both directions |
| `noble_ciphers_xchacha_roundtrip` | `crypto.test.ts` | Encrypt + decrypt round-trip succeeds with various plaintext sizes |
| `sri_hash_mismatch_blocks_submit` | `credential-accept.test.ts` | Tampered bundle hash blocks the submit button |

### Interop (cross-language fixture tests)

| Test | File | Asserts |
|------|------|---------|
| `js_encrypt_rust_decrypt_fixture` | `tests/interop/js_to_rust.rs` | Fixed-input encryption in JS produces ciphertext that Rust decrypts to expected plaintext |
| `rust_encrypt_js_decrypt_fixture` | `frontend/test/interop.test.ts` | Fixed-input encryption in Rust (test binary) produces ciphertext that JS decrypts to expected plaintext |

These two fixtures catch encoding-mismatch bugs (e.g., base64 vs base64url, big-endian vs little-endian length encoding) early.

### Integration / E2E

| Test | File | Asserts |
|------|------|---------|
| `e2e_full_push_accept` | `tests/e2e/credential_push.spec.ts` | Admin pushes, browser encrypts, node decrypts, operator accepts, secret reaches credential store |
| `e2e_manual_accept_gate` | `tests/e2e/credential_push.spec.ts` | Default opt-in flag set, decrypted secret waits for operator confirmation |
| `e2e_auto_accept_path` | `tests/e2e/credential_push.spec.ts` | With auto-accept flag enabled, secret stored without operator confirmation |
| `e2e_node_restart_mid_flight` | `tests/e2e/credential_push.spec.ts` | Push pubkey, restart node agent, restart loads sealed privkey, then ciphertext decrypts successfully |
| `e2e_legacy_node_fallback` | `tests/e2e/credential_push.spec.ts` | Node without `crypto_v1` feature flag: frontend shows legacy SSH instructions |
| `e2e_multi_node_fan_out_all_succeed` | `tests/e2e/credential_push.spec.ts` | 3 nodes, all decrypt, all accept, logical consumed state reached |
| `e2e_multi_node_partial_failure_then_retry` | `tests/e2e/credential_push.spec.ts` | 3 nodes, 1 decrypt failure → state `partial_decrypted` with per-node breakdown; admin retries the failed node only; succeeds → logical state `consumed`; idempotency: previously-accepted nodes unchanged |

### Audit / security regression

| Test | File | Asserts |
|------|------|---------|
| `audit_no_plaintext_in_any_event` | `audit_service::tests` | All event types emitted during the flow contain only metadata; grep for plaintext fixture string returns nothing |
| `audit_no_plaintext_in_error_log` | `audit_service::tests` | Decrypt failure path produces audit event without ciphertext or any derived value |
| `regression_legacy_cli_flow_unchanged` | `agent::tests` | `nyxid node credentials accept` on a pending with `crypto: None` works identically to today |

### Eval (LLM/protocol consistency)

| Test | File | Asserts |
|------|------|---------|
| `eval_sri_hash_format` | `tests/eval/sri.spec.ts` | Generated SRI tag matches the published manifest format |

Implementer should run `cargo test`, `npm run test`, and the e2e suite before requesting review. Coverage target: 100% of paths in the table above.

## Implementation phases

| Phase | Module touched | Effort (human/CC) | Depends on |
|-------|----------------|-------------------|------------|
| 1 — Backend protocol stubs | `backend/handlers/`, `backend/services/`, `backend/models/`, error codes | 1d / 2h | — |
| 2 — Node crypto | `cli/src/node/credentials/crypto.rs`, `cli/src/node/agent.rs`, sealed privkey persistence | 3d / 4h | Phase 1 |
| 3 — Frontend UI | `frontend/src/pages/credential-accept.tsx`, `frontend/src/lib/crypto/` | 3d / 4h | Phase 1 |
| 3.5 — Multi-node fan-out | All three layers (per-node ephemeral pubkeys, fan-out endpoint, partial-state UI) | 2d / 4h | Phases 1, 2, 3 |
| 4 — Audit + observability | `backend/services/audit_service` | 1d / 2h | Phase 1 |
| 4.5 — Code-integrity infrastructure | `.github/workflows/release-publish.yml`, `frontend/index.html` SRI tags, `releases.nyxid.dev` host setup, admin-verification UX | 1w / 1d | Phase 3 |
| 5 — CLI parity | `cli/src/commands/node_credential.rs accept-remote` subcommand | 2d / 3h | Phase 2 |
| 6 — Hint rewrites | `cli/src/commands/service.rs`, `cli/src/commands/node_credential.rs` | <1d / 30m | All others |

**Worktree parallelization:** Phase 1 alone first. Phases 2 + 3 + 4 in parallel lanes. Phase 3.5 after 1+2+3. Phases 4.5 + 5 + 6 in parallel after their deps. See "Parallelization" section below.

## Worktree parallelization

```
Phase 1 (backend protocol stubs)
     │
     ├──── Phase 2 (node crypto)
     │         └──── Phase 5 (CLI parity)
     │
     ├──── Phase 3 (frontend UI)
     │         └──── Phase 4.5 (code-integrity infra)
     │
     └──── Phase 4 (audit)

After Phases 1+2+3:
     └──── Phase 3.5 (multi-node fan-out)

Phase 6 (hint rewrites) last
```

Parallel lanes:

- **Lane A** (sequential): Phase 1 → Phase 4 — both backend, share services directory
- **Lane B** (sequential): Phase 2 → Phase 5 — both node/CLI
- **Lane C** (sequential): Phase 3 → Phase 4.5 — both frontend
- **Lane D** (after A+B+C): Phase 3.5 — touches all three layers, must wait
- **Lane E** (independent): Phase 6 hint rewrites, can land any time after Phase 1

Phase 1 alone gates everything; once it lands, B and C can launch as parallel worktrees with no conflicts. Phase 4.5 (code-integrity) needs Phase 3 done first but doesn't touch anything Lane A or B is working on.

## Failure modes

| Codepath | Realistic failure | Test? | Error handling? | User-visible? |
|---|---|---|---|---|
| Node generates X25519 keypair | OS entropy source temporarily unavailable | ✓ | retry once, then surface error | ✓ clear error state |
| Node posts pubkey via WSS | WSS drops between generation and post | ✓ | regenerate on reconnect | eventually consistent |
| NyxID stores pubkey | Concurrent push for same pending_id (race) | ✓ | 409 first-push-wins | ✓ clear conflict message |
| Browser fetches pubkey | Node hasn't posted yet (legacy node) | ✓ | 404 + `supported_features` flag fallback | ✓ legacy SSH-UI fallback |
| Browser encrypts | `@noble/curves` keygen fails on weak entropy | ✓ | fail loud, retry | ✓ |
| Browser SRI verify | Bundle hash mismatch (tampered or stale) | ✓ | block submit, surface mismatch | ✓ clear warning |
| NyxID forwards ciphertext | Node offline at forward time | ✓ | queue + retry on reconnect (code 8010) | ✓ "waiting for node" state |
| Node decrypts | Wrong AAD (cross-credential replay) | ✓ | emit error 8006 | ✓ decrypt_failed state |
| Node accepts + stores | Secret backend write fails | ✓ | preserve ciphertext, retry | ✓ recoverable state |
| Multi-node fan-out | Partial decryption (some succeed, some fail) | ✓ | partial_decrypted state with per-node breakdown | ✓ |

No silent-failure gaps. All paths have explicit error handling and user-visible state.

## NOT in scope (for this PR)

- HSM-backed node X25519 keypairs
- Mobile native client support (mobile browser works)
- Replacing the existing CLI two-party flow
- Verifying decrypted secrets against downstream services before accepting
- Per-credential audit of secret content

## What already exists (reused, not rebuilt)

- Two-party CLI push/accept flow — kept for legacy nodes
- `NodePendingCredential` model + service layer — extended with `crypto` sub-document
- WSS bidirectional protocol — adding two frame types
- Node-side `SecretBackend` / `CredentialStore` — reused for ephemeral privkey persistence
- `org_service::resolve_owner_access` + `node_access_can_read` — reused for access checks
- `RATE_LIMIT_PER_SECOND` middleware — extended with per-pending limit
- `AuditLog` + `audit_service::log_async` — reused for metadata-only events
- Node `agent_version` field — augmented with `supported_features` set

## Open questions for review

- **Browser compatibility floor — sample real analytics before Phase 3.** SubtleCrypto X25519 is Chrome 123+, Firefox 130+, Safari 17+; `@noble/curves` covers older browsers but adds ~30 KB to the standalone page bundle. Before Phase 3 starts, sample admin-population user-agent analytics (or a small canary slice) and report: *"X% of sessions get native SubtleCrypto, Y% fall back to @noble."* If `X` is overwhelmingly high (e.g. >95%), consider lazy-loading the noble polyfill only on miss to keep the standalone page leaner.
- **Releases domain.** `releases.nyxid.dev` (or chosen alternative) needs DNS, CDN, and signing-key infrastructure. Coordinate with infra ahead of Phase 4.5; see the runbook items table under §"Code-integrity infrastructure".
- **Auto-accept vs manual-accept default discoverability.** Per-org policy needs admin UI to flip the flag; covered in Phase 3 scope but worth scoping the per-org settings page changes explicitly during implementation.

## Design Review Feedback & Best Practices

During the architecture review of the remote credential injection proposal, the following best practices were established to address critical edge cases:

1. **Local Sweep Mechanism (Hybrid Local + Push Eviction)**
   - The node agent independently enforces a local TTL on any sealed ephemeral private keys, derived from the `NodePendingCredential.expires_at` value at the time of pubkey generation (so the local TTL stays aligned with the server-side TTL even if the server default changes).
   - A periodic background sweep task in the node daemon cleans up expired keys, and a cleanup check runs on daemon startup. This handles scenarios where the node goes offline or is abruptly shut down (SIGKILL, hardware failure) and never receives the consume/decline/cancel WSS frame that would normally evict the key.

2. **Multi-Node Fan-Out Idempotency**
   - Ensure the node-side `CredentialStore::accept` is idempotent (i.e., overwriting an existing credential for the same slug is a safe operation, and writing the same secret value is a no-op). This supports retry logic when a fan-out fails on some nodes and the admin re-pushes to all nodes.
   - The frontend will display a per-node status and support retrying only the failed nodes to avoid redundant re-encryptions.

3. **Queue Size & Resource Limits**
   - Enforce strict quotas on queued ciphertexts for offline nodes to prevent database bloating. Limit the maximum pending credentials in the "waiting for node" state to 5 per node.
   - The server will shorten the TTL of the queued ciphertext (e.g., 15 minutes) compared to the standard metadata TTL (1 hour).

4. **Admin Verification Security Policy**
   - Disabling or bypassing the "I verified the fingerprint" verification checkbox via the per-org policy flag requires fresh MFA confirmation at the moment the flag is flipped. NyxID already exposes the MFA verification endpoints (`/auth/mfa/verify`) and the existing MFA factor on the admin's account is the gate. Multi-admin approval is not used here because NyxID does not currently have a multi-admin approval queue subsystem; introducing one would be a separate project. The MFA gate ensures that a single compromised admin's session cannot silently lower the organization's defense-in-depth against code-substitution attacks.

## References

- Tracking issue: [#769](https://github.com/ChronoAIProject/NyxID/issues/769)
- This issue: [#773](https://github.com/ChronoAIProject/NyxID/issues/773)
- Already shipped on the tracking issue: [#775](https://github.com/ChronoAIProject/NyxID/issues/775), [#770](https://github.com/ChronoAIProject/NyxID/issues/770), [#771](https://github.com/ChronoAIProject/NyxID/issues/771), [#772](https://github.com/ChronoAIProject/NyxID/issues/772), [#774](https://github.com/ChronoAIProject/NyxID/issues/774)
- Related architecture docs: [ENCRYPTION_ARCHITECTURE.md](./ENCRYPTION_ARCHITECTURE.md), [NODE_PROXY_PROTOCOL.md](./NODE_PROXY_PROTOCOL.md), [SECURITY.md](./SECURITY.md)
