# Public NyxAgent runtime evidence

Read-only source inspection on 2026-10-15 examined
[`ChronoAIProject/NxyAgent` at `fe9745799cc1d46f1290e84effb366ff12600577`](https://github.com/ChronoAIProject/NxyAgent/tree/fe9745799cc1d46f1290e84effb366ff12600577).
This is the repository's current main commit, not a verified deployed build.
No upstream files, settings, accounts or services were changed. No real runtime
test was run. This evidence narrows the public implementation dependency; it
does not satisfy acceptance A1, A9, A13 or A15.

## Existing enforcement

| Boundary | Source observation |
| --- | --- |
| Native tool configuration | `src/runtime.rs:507` disables shell, unified exec, apply patch, code mode, JavaScript REPL, multi-agent, plugins, hooks and memory; skips host skill discovery and supplies an empty MCP-server map. A fresh temporary home and workspace are created per turn. |
| Tool declarations and invocation | The model bridge at `src/runtime.rs:367` retains only the six NyxID wrapper tools plus profile-enabled provider web search. Runtime RPC dispatch at line 560 permits only known dynamic tool calls and refuses other requests. NyxID must still restrict wrapper discovery and invocation for public keys. |
| Model credentials | `Bridge::new` at `src/runtime.rs:303` uses the incoming caller credential for `source: nyxid`. Both `/responses` and `/responses/compact` traverse that bridge, which pins the configured model and refuses redirects. `source: direct` instead uses a runtime environment credential, so public policy must exclude it. |
| Time and tool limits | `src/responses.rs:526` enforces the policy's overall turn deadline; `src/runtime/tool_calls.rs:194` enforces its tool-call count. These are global policy values, not a public-purpose request contract. |
| Session isolation | `src/nyxid.rs:74` authenticates through NyxID MCP initialize and derives session ownership by HMAC over the NyxID origin and exact incoming key. `src/responses.rs:76` uses owner-specific session storage paths and live expiry checks. Storage uses authenticated encryption. |
| Erasure | `Sessions::update` and `Sessions::sweep` clear runtime state and responses before writing an encrypted tombstone. The ordinary DELETE endpoint authenticates the original key through MCP initialize first. |

## Concrete gaps before public activation

1. **Public capability and profile binding.** `/v1/capabilities` currently reports
   images and steering only. The default `chat` profile enables provider web
   search, outside the three approved public tools. Profiles reload per request
   and may select direct credentials. Public execution needs a trusted,
   versioned restriction that disables web search and native abilities, requires
   NyxID model routing, and binds the reviewed effective profile. Merely choosing
   the same profile label does not establish those conditions.
2. **Every-call cost bounds.** Existing global deadline/tool limits do not bound
   model-call count or input/output tokens for each public call and compaction.
   NyxID must enforce per-dispatch cost reservations. The runtime must provide
   compatible hard limits and preserve the same public authority through
   retries/continuations; no direct-provider fallback is permitted.
3. **Cleanup after revocation.** All session DELETE requests use ordinary MCP
   authentication (`src/api.rs:239`, `src/nyxid.rs:74`). A fully revoked key
   therefore cannot authenticate deletion. Session TTL is idle-based and renews
   in `Session::touch`, so it does not establish an absolute 24-hour public
   lifetime. An exact cleanup-only capability or upstream-enforced absolute
   expiry is needed. Temporarily restoring execution authority is not a fix.
4. **Runtime evidence and drift.** The Codex runtime is resolved through the
   updater and the bundled policy follows `latest`. Pin and record the tested
   runtime/profile contract; readiness must reject incompatible changes. Source
   flags and synthetic fixtures alone do not prove the actual deployed runtime
   cannot bypass the allowlist or retain expired context.

The implementation should retain the existing owner API behavior and make public
restrictions additive and credential-bound. The likely upstream touch points are
`src/nyxid.rs` (trusted purpose discovery), `src/api.rs`/`src/responses.rs`
(capability binding and absolute session lifetime), and `src/runtime.rs`
(public profile restrictions and hard model bounds), with focused runtime and
session tests. Part 3 owns the precise cross-service proposal and NyxID policy.
Personal routing does not depend on these public changes.
