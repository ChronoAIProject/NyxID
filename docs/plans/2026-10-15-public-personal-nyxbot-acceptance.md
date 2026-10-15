# Public and personal NyxBot acceptance evidence

The primary maintains this evidence record with Part 5. The converged design is
authoritative; an implementation, a passing test and a live observation are
different evidence states. Record the tested commit and actual commands/results
before changing any row to verified. Mocked upstream calls cannot satisfy A13
or the external portion of A1/A15.

Current checkpoint: producer proposals complete and implementation started;
no feature implementation tests run.

| Gate | Owners | Required evidence | Status |
| --- | --- | --- | --- |
| A1 Public tool boundary | 3, 2 | Direct/wrapped MCP, resources/prompts, proxy/LLM and native-tool denial tests; pinned real NyxAgent containment proof | Open |
| A2 Operator isolation | 1, 2, 3, 5 | Two owners plus operator/sponsor; no history reads, prompt/result logs or public account capabilities | Open |
| A3 Proof and consent | 1, 4, 5 | Wrong account/chat, old onboarding link, signup alone, expiry/replay, parallel confirm | Open |
| A4 Personal parity | 2, 3 | Existing Auto/Ask/Full behavior, explicit denies, actual connected-resource scope, separate user credentials | Open |
| A5 History | 2, 4 | Same personal thread in web/Telegram; no public or old-home import; all adoption/recovery paths exclude platform threads | Open |
| A6 Revocation | 1, 2, 5 | Transaction races; exact origin/turn stop; no late send/card; independent tasks and web turns continue | Open |
| A7 Replay and races | 1, 3, 5 | Retry across link/unlink and process restart has one durable audience/charge; concurrent claims have one winner | Open |
| A8 Billing | 2, 3, 5 | Sponsor public, person personal inference, existing org BYOK semantics; no fallback payer or duplicate ledger posting | Open |
| A9 Full-loop limits | 3 | Exact concurrent caps over all priced hops, streaming, continuations, timeout/crash and UTC rollover | Open |
| A10 Groups | 5 | Linked sender and copied confirmation in group yield only private-chat handoff | Open |
| A11 Simple setup | 1, 3, 4 | Browser source + two budgets + Enable journey; defaults/preserved limits; failed readiness remains disabled | Open |
| A12 Existing product | All, 5 integrates | Existing assistant, channel, BotFather child setup/activation, grant and billing regressions; required CI | Open |
| A13 Live proof | 5, primary | Pinned NyxID/NyxAgent builds; two real accounts and Telegram identities; FAQ through billing/history/unlink | Open; needs reviewed build and authorized test environment |
| A14 Stable consent/capacity | 1, 5 | Guide edits/onboarding expiry/same-bot rotation preserve consent; public exhaustion leaves personal capacity; emergency stop blocks both | Open |
| A15 Public retention | 3, 5 | Sender/session isolation; live 24h expiry; linking cleanup; exact upstream delete/expiry proof; cleanup key cannot execute | Open |
| A16 Signup completion | 1, 4 | Signup beyond ten minutes gets fresh challenge; invitation policy intact; stale/replayed/mismatched intents denied | Open |
| A17 Personal lifecycle | 1, 2, 4, 5 | Atomic thread deletion/recreation, web continuation/cards after unlink, no public fallback on personal failures | Open |

## Verification environment

Source baseline `b6f11c1d3`; CI pins Rust `1.98.1` and cargo-nextest `0.9.146`.
CI uses MongoDB 8.0 as a replica set with an explicit
`NYXID_TEST_DATABASE_URL`. New race tests must call
`test_utils::connect_transaction_test_database`, which verifies a real
write-and-abort transaction and fails if MongoDB is absent or standalone.

Relevant required checks, as defined by `.github/workflows/ci.yml`:

- `cargo +1.98.1 fmt --all -- --check`
- `cargo +1.98.1 clippy --workspace --all-targets -- -D warnings`
- `cargo +1.98.1 nextest run -p nyxid -p nyxid-service-adapters --profile ci`
  with transaction-capable MongoDB; CI wraps this with resource sampling and
  uses line-table-only test debug information.
- Backend billing smoke and other required jobs selected by changed paths.
- Frontend `npm ci`, `npm run lint`, `npm run test`, `npm run build`.
- Relevant Playwright journeys using this worktree's server. The existing
  configuration uses port 4611 and may reuse a server outside CI; verify checkout
  identity or disable reuse so another worktree cannot supply false evidence.

Run focused checks after meaningful slices, then the required integrated suite.
Do not rerun unchanged suites without a new failure, change or unresolved risk.
Backend builds are serialized by the primary; workers request the build slot.
At the opening environment check, unrelated worktrees were compiling and the
volume had approximately 12 GiB available. No unrelated processes were stopped
and no unrelated artifacts were removed. Reassess before granting a build slot.

The primary provisioned an isolated local MongoDB 8.0.34 replica set on
`127.0.0.1:27188` named `nyxbot_manager_test`. Its explicit test URI is
`mongodb://127.0.0.1:27188/?replicaSet=nyxbot_manager_test&directConnection=true`.
An actual transactional insert followed by abort left zero documents, and the
temporary probe database was dropped. This verifies the test environment only,
not feature behavior. The temporary data directory and process belong to this
task and must be shut down after validation; other local MongoDB instances are
unrelated.

## External evidence to resolve before activation

- Exact NyxAgent repository/build and public profile contract: native tools,
  extra MCP servers, bounded execution, credential propagation for every priced
  model call, and isolated upstream session deletion/expiry.
  A [pinned upstream source review](2026-10-15-public-personal-nyxbot-upstream-evidence.md)
  now records the existing restrictions and concrete missing public-contract,
  cost-bound and cleanup support. It does not identify the deployed version or
  satisfy real runtime acceptance. Permission for changes to the separate
  NyxAgent repository has been requested; NyxID implementation continues.
- Sponsor authorization/funding, enforced invitation policy, and two distinct
  test people/Telegram identities for A13.
- All serving auth/MCP/proxy/webhook/worker replicas support the new mode before
  enabling it. Additive deserialization alone is insufficient.

No production readiness, external proof, CI result or live acceptance is claimed
by this initial record. Each completed gate must link to its evidence.
