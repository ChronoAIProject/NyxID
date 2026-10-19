# Security remediation W1–W4

Baseline `f96cf877834445d4976f62691aa192e534535c1f`; [accepted validity/scope/acceptance plan](https://gist.github.com/ctkm-aelf/49087f48b677a312fa308e686503e2f5). The original issues are [#1928–#1940](https://github.com/ChronoAIProject/NyxID/issues/1928). Draft [NyxID PR #1946](https://github.com/ChronoAIProject/NyxID/pull/1946) remains incomplete, unmerged and undeployed. Primary owns independent review, packaging and publication. Compiler/Mongo ownership passes only through explicit handoffs; the worker edits source only while primary owns runtime. No new agents, Git/index mutations, merge or deployment are authorized for the worker.

## Current verdict: checkpoint17 compiled; independent runtime pending

Checkpoint17 compiled successfully in **12m17s, zero warnings**, from the immutable frozen source. Both manifests were reverified with zero mismatches: 4,195 content entries (`33d434c0215c5b3156719c65ebdedc266cce0179313887d1224a233aef3a4402`) and 4,196 native entries/modes/link blobs (`8a74f6967b5a33fe383d1df54eedc9bc3acab7cafde804ef66accf56bc3d7cc2`). The immutable test executable has SHA256 `9bc99ea80f3992ce89675dc047a2b52983151ecfe2aacd91826e05c8a845e312`. [Durable build provenance](security/CHECKPOINT17_BUILD_PROVENANCE.json).

Primary's 74-scope runner resolves to 2,347 unique tests in this executable. OAuth-service (14) and tools-manager (34 listed, including two separately handled vendor cases) remain supplemental scopes. Runtime was explicitly released after compiler and verifier exit; the worker ran no backend/Mongo tests. Independent17 runtime is pending. The source-only platform holder and broker work described below postdates the frozen17 manifests and is uncompiled.

## Most recent completed independent runtime: checkpoint16 failed

Checkpoint16 compiled successfully in **14m15s, zero warnings** on main `ad233bb1`. Primary published it to draft PR #1946 at `3c927dd126b3a9d3d5b59c74f25531cd5e5a8c25`, preserving newer workspace edits. The PR remains unmerged and undeployed. No production migration, writer cutover, strict-read activation or key retirement has run.

| Frozen16 artifact | Verified evidence |
|---|---|
| Content manifest | 4,190 entries; SHA256 `a6888d429c728ec59648c1425781d5ab1750ad2934f1267d6ce48b17df446a9d` |
| Native entries, modes and symlink blobs | 4,191 entries; SHA256 `64e7d60e487af8e57c7e34f5ab3fdb4c9adb60646a6c827b31986ad8f92beb73` |
| Immutable backend executable | SHA256 `cea761c8a9b9b5995ecdd2321087d94c5f66f6f661ace23b12510f9ee85b0f74` |
| Exact provenance | `/tmp/nyxid-security-build16-handoff.json`; primary independently verified all sources and executable |
| Backend selection | 67 primary scopes: 2,295 passed, 15 failed; supplemental/vendor results below |
| Independent clients | 6,153/6,153 web tests across 582 files, production build/TypeScript/credential acceptance passed; ESLint 0 errors, 34 existing warnings; all 2,015 client entries unchanged |
| Mobile | No drift from15; prior 67/67 tests and typecheck apply, not rerun for16 |

Primary explicitly released compiler and Mongo runtime after its16 runners completed. The worker completed the next preflight/build, then explicitly released runtime for17. No post16 backend runtime result is claimed yet. Frozen16 remains immutable. Additional primary HTTP/CLI/vendor checks are separate from the 67-scope selection; do not add overlapping totals.

Final independent backend result: **2,788 passed, 22 failed, 0 ignored across 2,810 unique attempted tests**. The main 67 scopes produced 2,295 passed / 15 failed; the supplement produced 492 passed / 6 failed / 2 ignored, with those two vendor cases explicitly replaced by separate actual attempts (1 passed / 1 failed). [Durable exact failure list and artifact identity](security/CHECKPOINT16_RUNTIME_VERDICT.json).

The reported failures and source repairs are: All have source repairs excluded from16 and remain unexecuted in the backend harness:

- Disconnect restart fixture returned empty200 to a parser requiring JSON. It now returns `{ "ok": true }`; the durable completion/capability-removal assertions remain. Production parsing is unchanged; explicit negative malformed/empty/`ok:false` wire coverage remains.
- X ownership transfer expected unchanged ciphertext despite legitimate contextual owner rebind. The test now verifies unchanged access/refresh plaintext under the new context, old-owner authentication failure, callback-handle rotation and unchanged credential epoch.
- Assistant cascade-delete fixture used invalid ciphertext. It now uses authenticated material under matching state keys and checks local teardown plus retained broad remote obligations requiring reconciliation.
- The registry identity test searched for a downstream-services adapter that is no longer pending. It now verifies rejection against a different concrete adapter for the same collection.
- Two catalog migration tests and one SSH CA migration test failed at migration. Source tracing identified Mongo's root `_id` reordering against an exact raw-document predicate constructed before insertion. The predicate now normalizes only that justified root ordering, preserving nested values, absence/null distinctions and concurrent-addition protection. An actual BSON regression is drafted; runtime confirmation remains required. Frozen16 already contains the SSH BSON byte helper.
- Six MCP resource tests directly moved contextual key ownership or replaced raw encryption under a surviving contextual marker. Fixtures now use the actual credential rebind/journal transaction and normal replacement service. The parity fixture lacked its linked catalog row; it now seeds the real matching no-auth row, retaining approval policy assertions. These repairs await compilation/runtime.
- The held pre-activation writer test treated an early definite conflict as though activation had succeeded. The revised test rejects any successful activation, requires no committed marker before the held writer commits, and allows an explicit retry only after a definite abort. It retains the below-cursor insertion and holder assertions.

Separate validation and failures must not be folded into the main runner's totals:

- Primary's supplemental 15 backend scopes completed **492 passed, 6 failed, 2 ignored**. The four OAuth failures had missing protected-consent references; fixtures now obtain real consent and preserve resource-grant assertions. The two tools failures used an obsolete raw decrypt and header version assertion; fixtures now use the catalog adapter and current v4 header contract, retaining winner/freeze/context/noninheritance assertions. Repairs await backend execution.
- The two ignored supplemental vendor E2Es were explicitly attempted separately: tools passed; catalog failed its positive revocation scheduling assertion. The catalog test remains open until reviewed token-only metadata and complete production holder activation can support its actual wire assertions. No test-only activation waiver is acceptable.
- Frozen16 cloud-auth tests failed compilation after guarded credential types changed. Primary independently validated the repaired fixtures: **8 passed**, no warnings. Subsequent formatting changed the file hash; the semantic diff still needs checkpoint verification.
- A real CLI defect converted successful remote deletion acceptance into failure during unrelated local profile cleanup. The repair conditionally preserves successor/unrelated profiles, warns on local I/O failure after acceptance, and retains the server result. Primary independently ran **1,675 passed, 0 failed, 6 existing ignored** against the repaired source, plus **strict CLI/cloud Clippy for all targets with `-D warnings` passed** after moving redacted Debug implementations before tests. These are separate repaired-source results and do not change frozen16's failure verdict.

## Live source after16: preflight passed; new harness/runtime pending

- Async-operation result storage and channel connect-link receivers now have service-owned lifecycle adapters, actual reader validation, exact raw-source CAS migrations and registry entries. Result cleanup clears its matching version; receiver tests cover real signed staging/strict delivery, context swaps and empty/dangling markers. Frozen17 has **15 Pending entries**; frozen16 contains 19. The subsequent live broker adapter reduces the source registry to14, pending compilation/runtime. Oracle profile/snapshot outer adapters also preserve their existing AAD and validate the actual readers; inner recipient-sealed material is not server-readable token inventory. All four new entries passed the shared backend preflight; their new harness/runtime verification remains pending.
- Actual disconnect/manual-import regression covers both ordering directions under different owners/provider labels and unrelated-token progress. Startup provider/connection copy writers now pass authenticated keys through the shared journal boundary; X transfer does the same. The complete writer audit and late repeated-token issuance regression remain open.
- Token-discovery controls draft explicit admin-only first-party-human activation and steps, operator-readable metadata, an atomic chained activation audit, at most32rows per step, a blocked exact cursor and repair/resume. There is no skip/force-complete action. Completion binds an exact coverage version/digest; historical complete markers cannot authorize a newer registry.
- **Production activation remains blocked by incomplete holder coverage.** User API keys and provider-token rows alone do not cover imported catalog/master credentials, configured headers, channel/platform execution tokens and other supported server-readable capabilities. Oracle inner sealed browser envelopes are recipient controlled and excluded from token parsing; their outer encryption remains in W4. Broker bindings retain NyxID-issued refresh tokens, not provider OAuth tokens. A real encrypted cross-owner catalog-copy regression exercises the safe refusal. Remaining stores require actual decoders, backfill and atomic admission in their writers. This is implementation incompleteness, not a deployment acknowledgement an operator can waive.
- Holder decoding now includes catalog master credentials, all configured catalog/instance header values, and legacy connection credentials. Catalog HTTP creation/twins/mutations, owner transfer, instance header edits, connection commits and catalog/header storage migrations join token admission in the source draft. Actual raw installed BSON supplies holder source hashes. Other holder families and remaining raw writers still block activation; this is not complete writer coverage.
- After a complete matching index, preparation can use a finite captured-ceiling walk without a shared global writer revision. Owner/SA revisions still fence its own changes; dispatch retains the live holder/closure gate. Readiness never certifies absence of a future cross-owner import. The draft continuous-unrelated-writer test checks progress and retains a matching-copy blocker. The production coverage gate keeps this liveness path unavailable until holder coverage is complete.
- Four client notices now say “Upstream revocation is not confirmed” when scheduling is false. These wording edits postdate independently validated16clients.

The post16 backend preflight (`cargo check --locked -p nyxid --bin nyxid-server --tests`) passed in 6m02s. It reported two unused test-only authorization wrappers after fixtures moved to protected-consent issuance; those obsolete wrappers were removed. Strict backend Clippy passed for all targets with both `aws-kms,gcp-kms` features and `-D warnings` in 7m39s, with no diagnostics. The immutable checkpoint17 harness subsequently compiled in12m17s with zero warnings; primary independent runtime is pending. No repaired-source runtime pass is claimed here.

Post17 source adds atomic platform holder writes/deletes/rewraps and an owner/client-bound broker refresh adapter. Neither slice is included in frozen17 or its prior preflight/Clippy result. The final catalog/header holder acceptance still requires normal HTTP/service writers after production activation, with both provider wire calls retained; helper discovery tests do not replace that proof.

## Acceptance crosswalk

| Work | Concrete acceptance | Current implementation and remaining work |
|---|---|---|
| W1 reporting / #1928 | Private reporting survives managed-template reconciliation | Local policy/link/checker implemented; companion draft6196 tested/published, not merged/deployed. Maintainer SLA/support commitments remain operational decisions. |
| W1 current-truth docs / #1940 | All27claims below corrected without unsupported guarantees | Current tracker separates immutable evidence, live source and history. Final privacy/security/deployment reconciliation still follows verified implementation. |
| W2 consent / #1929 | Default-off persisted permission; fresh majority revision gate, queued withdrawal, pre-login/local/DNT/account-switch controls | Shared service/clients implemented with earlier runtime evidence;16 client full suite passed; frozen16 backend verdict failed as above. |
| W2 personal erasure / #1930.2 | Durable accepted intent, bounded leases/CAS/retry, complete owner/reference inventory, no shared-resource loss or false completion | Erasure36passed in16. Main's assistant-resource admission/inventory integrated. Complete token-holder coverage, production backfill, writer audit and late issuance/copy races remain blocking. Broad/unknown grant operations remain operator reconciliation, including delayed first dispatch. |
| W2 remote obligations | Atomic source claim/removal plus durable encrypted intent; crash/restart, partial receipts, equivalent-authority coalescing | Implemented in16 with reported fixture repairs pending. Global token fingerprints differ from authority-bound confirmations; failed/unknown results retain capability. Duplicate waiting does not consume provider attempts. Active-user obligations have operator APIs. |
| W3 MFA / #1931 | Bounded Argon2, atomic single-use recovery, all login paths, enrollment/confirm races and notifications | Implemented;16 MFA 15 passed; primary HTTP supplement passed the auth/MFA suites. Final full integration remains required. |
| W3 OAuth / #1939 | Live ordinary-token authority and generation at actual readers, logout/disable/erasure fences, retained rollback semantics | Implemented core;16token105/OAuth12/usercredentials40passed. Broader handler/native/proxy and direct/late issuance races remain in current validation and open source work. |
| W3 JWT/HMAC lifecycle | Reader overlap; exact independent stable HMAC adoption before source changes | Compiled in16 with prior evidence. No arbitrary HMAC rotation, historical re-signing or chain-retention change is supported. Preserve audit-derived token-index keys/backups. |
| W4 secret lifetime / #1933 | Guarded owners through actual readers, transport and errors; redacted metadata-only diagnostics | Core/cloud/node/proxy changes compiled; final propagation audit remains open. No universal zeroization/SDK-memory guarantee. |
| W4 context/migration / #1934 | Every service-owned family has lifecycle validator, actual caller conversion and exact CAS migration; per-field actionable readiness | Runner/implemented adapters compiled in16; 19 Pending then, 15 in frozen17 after async result, channel receiver and two Oracle outer adapters; 14 live after the source-only broker adapter. Missing adapters are implementation blockers. Reader-first rollout, E2E exclusions, backups and KMS internal-version uncertainty remain explicit. |
| Deployment/legal dependencies | Truthful retention, export, vendors, regions/contracts and upstream rollout status | No invented SLA, legal basis or deletion deadline. Self-service export/passkeys/org-required MFA remain separately scoped. W8 audit/billing retention and external witnessing remain separate; never delete/re-sign immutable history. |

## Prior frozen evidence and external dependency

- **15:** 1,649passed,11assertionfailures,1hungtestterminated,0stackaborts,0ignored, all1,661selected attempted. [Durable verdict](security/CHECKPOINT15_RUNTIME_VERDICT.json). Full web6,140passed/1docs-searchrace; that race is repaired and16full webpassed. Mobile67passed/typecheckpassed.15is historical, not current source validation.
- **14:** 1,538passed,26assertionfailures,2stackaborts,0ignored across1,566selected. [Durable verdict](security/CHECKPOINT14_RUNTIME_VERDICT.json). Its70CodeQL annotations received specific primary dispositions:53testfixtures,17falsepositives; [per-alert evidence](security/CODEQL_CHECKPOINT14_REVIEW.json). No queries disabled or analysis history removed.
- [Archived implementation journal](security/REMEDIATION_CHECKPOINT_HISTORY.md) retains earlier checkpoint observations and evidence. Its current-tense language refers only to those old boundaries.
- Companion [fkst-hosted draft6196](https://github.com/ChronoAIProject/fkst-hosted/pull/6196), head `852e7f055534352f38a6ad39d4434d3a99039aab`, base `934914502a2fb740c54c030826acd9b7f8128e92`: all 7 remote jobs passed, all 7 blobs/modes independently verified. Local159GitHub(including17template/22installer)+10reconcile+13audit, fmt/strictalltargetClippy passed. [Artifact/evidence](security/fkst-hosted/README.md). Unmerged/undeployed; installed NyxID marker remains13.

## Incomplete source families

Live migration registry still requires adapters/callers for agent-key login delivery; learning proposals; auth-device v1/v2; device codes; channel bots; four NyxBot collections; OAuth client webhooks; gateway transports; native reply handles; and platform setups. These 14 live entries remain explicit blockers, never evidence of completion. See [encrypted-field inventory](security/ENCRYPTED_FIELD_INVENTORY.md), [erasure inventory](security/PERSONAL_ERASURE_INVENTORY.md) and [key lifecycle](security/KEY_LIFECYCLE.md).

## #1940 factual correction crosswalk

All 27 cited statements have corresponding edits in the initial W1 documentation batch and were checked against the assessed source and parent review. This is documentation correction only; underlying W2–W4 fixes remain pending. Deployment facts and complete encryption inventory are not claimed verified.

| Item | Disposition and actual behavior | Code owner / related issue |
|---|---|---|
| 1. API-key prefix | Correct `nyx_k_` to the actual generic `nyx_` and agent `nyxid_ag_` formats | W1; `crypto/token.rs` |
| 2. No API-key scope enforcement | Remove: middleware and management-scope checks exist | W1; `mw/auth.rs` |
| 3. Rate limits are all in-memory | Correct: production global/per-IP/per-agent enforcement has Mongo-backed paths; describe any remaining local mechanisms separately | W1; `mw/rate_limit.rs:1024` |
| 4. Proxy body limit 10 MB | Correct the default to 100 MiB and distinguish per-endpoint limits | W1; `config.rs:1365` |
| 5. Header allowlists | Synchronize with actual allowlists, including tracing/range/idempotency behavior and removal of caller `content-length` | W1; `proxy_service.rs` |
| 6. RSA always 4096-bit | Qualify as generated-key size; imported-key enforcement is separate | W3 / #1939 |
| 7. All decrypted secrets zeroized | Remove the universal guarantee until wrappers propagate end to end | W4 / #1933 |
| 8. v2 minimum length 92 | Distinguish local-provider overhead (92) from generic parser minimum (33); wrapped-DEK length varies | W4 / #1934 |
| 9. Access JWTs never checked in DB | Correct by token class: user-active check, first-party session checks, plus Machine/relay/delegated-specific live checks; ordinary client tokens retain the identified gap | W3 / #1939 |
| 10. Encryption inventory incomplete | Add unified credentials and other implemented encrypted fields from a maintained inventory | W4 / #1934 |
| 11. “Run a re-encryption job” | Describe the explicit bounded runner now implemented, its incomplete registry and deployment gates; no production activation or retirement has run | W4 / #1934 |
| 12. Rewrap as records are touched | Describe only explicit service-owned CAS migrations and staged readers; do not promise universal lazy rewrap | W4 / #1934 |
| 13. No service can persist a plaintext credential | All newly written configured default-header values encrypt at rest; the sensitive flag controls management masking. Legacy public plaintext is reader-stage compatibility only; retain limits for remaining stores | W4 / #1934 |
| 14. Every Debug implementation redacts | Remove the universal claim, fix identified types, and describe the enforced scope | W4 / #1933 |
| 15. KMS removes all long-lived key material | Limit claim to KMS-held KEKs; local JWT/private signing material and other secrets remain | W3/W4 |
| 16. Node signing secret stored only hashed | Correct: the node model also has `signing_secret_encrypted` for server signing needs | W1; `models/node.rs:148` |
| 17. Only old timestamps described | Add future-skew rejection: implementation uses absolute skew above 300 seconds. Existing wording is incomplete, not proof future requests are accepted | W1; `cli/src/node/signing.rs:39` |
| 18. Delegation token on every MCP call | Make conditional on configured `inject_delegation_token`; it defaults off | W1; `mcp_service.rs:5360` |
| 19. Error bodies can be logged | Correct the documented diagnostic behavior; current code includes a regression test asserting upstream-body content is absent from captured logs | W1; `handlers/proxy.rs` (frozen16 proxy scopes are under independent runtime validation) |
| 20. Four injection modes | Expand the table or label examples. The page says “several,” so incompleteness is more precise than claiming it explicitly promises only four | W1; `proxy_service.rs` includes body/path and other modes |
| 21. Bodies never parsed/modified | Correct: body injection and other explicit transforms exist; describe exceptions without promising universal pass-through | W1; `proxy_service.rs:4513` |
| 22. Granular approvals “no code yet” / no MCP approval | Mark historical baseline as historical and describe current implementation | W1; current MCP preparation/approval paths |
| 23. cua 0.30.4 | Update the pinned-version reference to the code’s 0.31.0, ideally generated from the authoritative pin | W1/W9; `cli/src/node/machine/cua.rs` |
| 24. Azure Key Vault | No Azure provider implementation found. The cited passages are provider comparisons; label Azure as comparative/planned rather than implying shipped support | W1/W4 |
| 25. “M1 draft” title | Align title with status and incomplete rollout; editorial consistency, not a runtime defect | W1/W9 |
| 26. Missing `k8s/` manifests | Remove invalid installation commands or provide maintained manifests as a separately scoped deployment task | W1/W7 |
| 27. Telemetry CORS “not yet landed” | Resolve the contradictory current heading/history; retain historical evidence with explicit dates | W1/W2 |
