# Historical W1–W4 implementation journal

Archived on 2026-10-19 while checkpoint16 independent runtime was in progress. This is historical evidence, not current status: phrases such as “current,” “pending,” and “uncompiled” below refer to their original checkpoint. Later repairs and validation supersede them. Consult [the current acceptance tracker](../SECURITY_REMEDIATION_W1_W4.md) for the live boundary. Temporary paths identify original local evidence; durable verdicts and review reports are linked from the tracker.

# Security remediation W1–W4

Baseline: `f96cf877834445d4976f62691aa192e534535c1f`. Assessment: [published validity/scope/acceptance plan](https://gist.github.com/ctkm-aelf/49087f48b677a312fa308e686503e2f5), cached locally at `/tmp/nyxid-security-plan.md`. Original issues: [#1928](https://github.com/ChronoAIProject/NyxID/issues/1928) through [#1940](https://github.com/ChronoAIProject/NyxID/issues/1940), cached locally as `/tmp/nyxid-security-issues/{number}.json`. The scope/correction crosswalk is retained here so review does not depend on temporary files. Sole implementation worker; parent supplies independent review. Implementation worker makes local source changes; primary owns commits and publication. Draft PR [#1946](https://github.com/ChronoAIProject/NyxID/pull/1946) is published and explicitly incomplete. No merge, issue closure, deployment, or production migration is authorized.

## Status and evidence rules

**In progress.** No unchecked scope is complete. Documentation corrections describe current behavior and do not close underlying defects. Tests below list actual execution separately from planned acceptance. Parent source/visual review is active, with accepted slices and outstanding findings recorded below. This file is the durable handoff/checklist and must advance with every implementation batch.


### Checkpoint16 compiled; independent runtime pending

Checkpoint16 compiled successfully in 14m15s with zero warnings. The frozen source has 4,190 content entries (manifest SHA256 `a6888d429c728ec59648c1425781d5ab1750ad2934f1267d6ce48b17df446a9d`) and 4,191 native entries (manifest SHA256 `64e7d60e487af8e57c7e34f5ab3fdb4c9adb60646a6c827b31986ad8f92beb73`), all reverified after compilation with zero mismatches. The immutable test executable SHA256 is `cea761c8a9b9b5995ecdd2321087d94c5f66f6f661ace23b12510f9ee85b0f74`. Provenance is `/tmp/nyxid-security-build16-handoff.json`. The primary's 67 prepared scopes select 2,310 actual tests without namespace corrections. No backend runtime tests were run by the worker. Compiler/Mongo/runtime ownership was explicitly released to primary after the owned compiler and verifier exited. Further worker work is source-only until primary releases runtime.

The next source boundary includes the checkpoint15 failure repairs, the reviewed main rebase, catalog credential owner rebind, SSH CA storage/signing coverage, guarded proxy propagation and assistant-resource erasure fences. Direct disconnect now commits encrypted remote obligations atomically with exact-source removal. Global token closure and authority-specific result receipts prevent a confirmation from another provider/client from settling the wrong obligation. Equivalent duplicate effects wait durably and can settle from confirmed receipts without another HTTP request or provider-attempt consumption. The duplicate settlement sweep has a matching partial index and an explain regression. Present malformed capability-version markers fail closed. Active users' disconnect effects remain visible and reconcilable through operator routes without inventing an erasure job. These frozen16 changes compiled; runtime validation remains pending.

**Implementation remains incomplete.** Token-index production activation/control, the complete writer-admission audit and held direct-disconnect/import/late-issuance races remain open. Discovery helpers are exercised by tests but are not a production activation path; no production cutover is claimed. The global preparation revision still has a liveness limitation under unrelated credential writes. Broad or unreviewed grant effects require external reconciliation because neither delayed first dispatch nor retry is proven safe against successor grants. Nineteen registered encrypted families and their actual callers, startup-migration race coverage, final plaintext propagation, feature/CLI/Clippy validation and final documentation reconciliation remain open. E2E recipient-sealed exclusions and backup/KMS retirement limitations remain explicit. No production migration, strict activation or key retirement has run.

Independent frozen16 client validation passed all 6,153 web tests across 582 files, the production build including TypeScript and credential acceptance, and workspace formatting. ESLint reported zero errors and 34 existing warnings. All 2,015 client entries remained unchanged; mobile has no source drift from15, so its prior 67 tests and typecheck apply without a fresh run. Primary evidence: `/tmp/nyxid-primary-build16-client-results.json`.

### Post16 live source, excluded from frozen16

The async-result family now has a service-owned migration/readiness adapter and shared bounded UTF-8 validation used by the actual reader. Expiry, cancellation, delivery and platform cleanup retire the payload marker with the payload; a dangling marker remains unresolved. Source tests cover the actual ready writer, legacy array/current Binary storage, strict reads, context swaps, exact-CAS successor preservation and lifecycle cleanup. This reduces the live registry to eighteen pending families; frozen16 still has nineteen. No post16 Rust compilation or runtime validation is claimed.

A new direct-disconnect/manual-import regression exercises both race orderings under distinct owners/provider labels and unrelated-token progress. It remains uncompiled. Client disconnect notices now say “Upstream revocation is not confirmed” when scheduling is false, covering unknown, expired and durable pending obligations without asserting their upstream state. These four wording edits are outside the independently validated frozen16 client source.

### Checkpoint15 compiled; independent runtime complete

Checkpoint15 is committed as `ea8b4c11c081c3df65a758da2d88620fb48468da`, tree `2ff41558df1ea8cdacb84e4704ffb915d1ce5eeb`. Frozen15 preserves 4,170 content entries (manifest SHA256 `11a42b3b244296326e47cdab749f61c3e24d9c5297d21c4303a04a0c4163d957`) and 4,171 native entries including symlinks/modes (manifest SHA256 `6fc3669f95eb7db82376636c05da3837945f02befc476dd179d871d8cf68cb23`). Compilation passed in 12m13s with zero warnings/errors. Immutable executable SHA256 is `898baae2e4f27de8e236f1ac72e11378120469ba1b690a880a2b8fc96c10cca5`. Primary independently verified source, executable and provenance. The [final runtime verdict](security/CHECKPOINT15_RUNTIME_VERDICT.json) records **1,649 passed, 11 assertion failures, one hung test terminated, zero stack aborts, zero ignored and zero unexecuted**, across all 1,661 selected tests in 56 scopes plus three exact remainder tests. Primary subsequently released runtime; checkpoint16 ownership is recorded above.

Passing runtime coverage includes both prior GCP and Aurinko stack-abort paths, GCP malformed expiry, assistant credential/upload migration, MFA, native entry, machine activity and operator actions. Remaining failures include credential/transfer fixtures, a platform-connection header path missing encryption keys, Aurinko erasure lifecycle fixtures and trigger live/retention tests. Post15 repairs are outside the tested snapshot. Full frontend had 6,140 passes and one docs-search timing failure; its live controlled-timer repair independently passed all eight focused tests. All 67 mobile tests, mobile typecheck and web production build passed independently.

The remote draft remains at checkpoint15 and conflicts with main; no new CI runs have started for it. Primary completed an isolated rebase and installed local HEAD `f42a205cb82b14bcc3a0d59001fd5e142576461c` on main `ad233bb1`, preserving all post15 source changes. The reviewed combined source tree was `536a9092087a791b9e8d0ab64bf64036420e2037` before subsequent source edits. The rebased checkpoint has not been force-pushed or compiled. New main's `assistant_resources` owner fences, bounded erasure inventory and contextual image writer are now drafted with held-writer coverage; execution is pending. No PR merge or deployment has occurred. Primary alone owns Git/index and publication operations.

Post15 catalog master and SSH CA adapters, startup connection migration, catalog owner rebind and guarded proxy propagation are uncompiled. The global credential inventory fence and external-reconciliation dispatch gate remain temporary safety measures. Final W2 acceptance requires token-specific import/dispatch fencing and bounded discovery, including direct disconnect and reused late issuance. Remaining adapters/callers, old-cascade retirement checks, feature/CLI builds, Clippy and final documentation are open. No production migration, strict activation or key retirement has run.

The W1 managed-template dependency is published as unmerged, undeployed draft [fkst-hosted PR #6196](https://github.com/ChronoAIProject/fkst-hosted/pull/6196), head `852e7f055534352f38a6ad39d4434d3a99039aab`, based on `934914502a2fb740c54c030826acd9b7f8128e92`. Primary verified all seven remote blobs/modes against the final tested source. Independent local validation passed 159 GitHub tests (including 17 template and 22 installer tests), 10 reconciliation tests, 13 audit-worker tests, formatting and workspace/all-target Clippy. All seven remote CI jobs passed at the exact published head. NyxID's installed template marker remains13. See the [companion evidence](security/fkst-hosted/README.md).

### Frozen build14 compiled checkpoint; independent runtime complete

Primary committed exact frozen14 as `c74fc1edcd82a9a5be41b16cc1991883730eb4a4` (tree `2128e150989271a0500efcd501fac3942ab38a73`) and published draft PR #1946. Manifest `/tmp/nyxid-security-build14-source-sha256.json` has **4,163 entries**, SHA256 `53c7f75235db76824dc740f98dbf0bfad8d1ad6c0cd03eca825454f65e805eba`. Compilation completed successfully in **78m38s**, with three known warnings. The immutable test executable SHA256 is `204554783f76d8c29e6d3b6ef841331751d48fc6747ba89e1b0ed14868435c5d`. Primary independently verified all source hashes with zero mismatches and the executable hash, then accepted exclusive compiler/Mongo/runtime ownership for the 49-scope runner. Primary completed all 49 scopes plus 18 previously unexecuted exact tests: **1,538 passed, 26 assertion failures, two stack aborts, zero ignored, 1,566 selected tests**. The [durable verdict](security/CHECKPOINT14_RUNTIME_VERDICT.json) records this boundary. All owned test processes exited and primary confirmed Mongo cleanup before releasing compiler/runtime ownership. Post14 repairs are excluded from checkpoint14 and are being compiled separately as checkpoint15.

Fresh frozen14 client validation: 119 targeted web tests, 67 mobile tests, web production build including TypeScript, mobile TypeScript, and native sync passed; ESLint has zero errors and 34 baseline warnings. CI found nine additional frontend fixture failures; five repaired fixture suites subsequently passed **60/60** in an isolated primary copy. Backend CI is blocked by three known frozen14 warnings; cloud feature checks and Clippy cleanup, wizard bundle regeneration, and full source validation remain outstanding.

Frozen14 findings include missing standalone lineage fixture initialization, provider-less OAuth provenance rejection, a zero-match issuance-install mutation error, two migration fixtures using staged writer keys, an MFA test that regenerated a different TOTP code, and GCP/Aurinko stack overflows. Checkpoint15 source repairs cover these findings plus native-entry fixture contexts, human node-registration issuers, malformed GCP expiry, and operator action-audit contention. Boxing production future boundaries still requires normal-stack runtime verification. The supported wizard build regenerated its embedded bundle; checkpoint15 preflight found no type errors; executable compilation is active. New shared-copy findings also keep automatic erasure revocation and final readiness incomplete.

Primary reviewed and recorded dispositions for all 70 CodeQL annotations against the published checkpoint: **53 used-in-tests, 17 false-positive**. The durable [per-alert evidence](security/CODEQL_CHECKPOINT14_REVIEW.json) includes exact paths, rules, reasoning and recorded dispositions. No queries were disabled or analysis history removed. The aggregate configuration warning refers to a retained September21 pre-matrix analysis; all four current language analyses completed without errors. A separate real `RecordContext` Debug exposure was fixed only in post14 source and still requires compiled validation.

### Frozen build13 compiled checkpoint; independent runtime complete

Build13 compiled successfully with stable Rust 1.96.0 in **11m17s**, test profile,
one job, final `-C debuginfo=0`; three unused migration-helper warnings and no
errors. Source `/tmp/nyxid-security-build13-source`; manifest
`/tmp/nyxid-security-build13-source-sha256.json` contains **4,158 entries**, all
verified after compilation with zero mismatches. Manifest SHA256:
`18cdd5a7d789dceb11066e5c6d33de9e8c92410b4983407c9ccac31b394e7020`.
The source executable and immutable clone
`/tmp/nyxid-security-build13-server-tests` both hash to
`7ebe316d98b6eb6a629c1c10a5339b41b5f1b45d8d48481240ccb2b35217baa9`.
Exact handoff `/tmp/nyxid-security-build13-handoff.json`; successful compile log
`/tmp/nyxid-security-current-harness-build-13-retry7.log`. Exclusive compiler and
Mongo/runtime ownership was explicitly released to primary. Primary completed all 22 scopes: **946 passed, 10 failed, 0 ignored**. Results `/tmp/nyxid-primary-build13-results.json`. Failures included stale raw-decrypt/bogus-ciphertext fixtures, missed contextual UserApiKey readers, and routine-refresh history metadata classification. These findings prompted frozen14 repairs; their independent runtime validation is pending.

Frozen13 includes credential field adapters/live readers and exact BSON migration,
public-client None/empty/set/clear semantics, source-fenced token copies and
bounded startup migration, atomic issuance/install receipts, BYO dependency
revision invalidation, compatible encrypted Node writers, configuration mode
validation and Aurinko descriptor corrections. The newer OAuth-state, generic
binary and assistant adapters, exact-document CAS fixes and held formerly absent
field/relationship tests are **workspace-only and unvalidated**. Shared OAuth
grant lineage/blast-radius isolation, remaining field families, global bounded
migration/readiness and final plaintext propagation remain open. No production
activation, migration or key retirement has run.

Build13 primary runtime completed: **22 scopes, 946 passed / 10 failed / 0
ignored**, `/tmp/nyxid-primary-build13-results.json`, logs `00..21`. Production
findings: the channel credential reader still used unbound decryption; GCP routine
mint emitted a UI history event solely from new storage-version metadata. Source
now converts that reader (plus separately reviewed node push, Lark setup, assistant
reconciliation and native-entry snapshot readers) and classifies storage/provenance
metadata independently of credential-authority invalidation. Remaining failed
assertions use bogus ciphertext/legacy raw-decrypt expectations or the pre-source
refresh gate. Repairs and new held-response tests require the next snapshot.
Primary released compiler/Mongo/runtime after cleanup. Workspace-only assistant,
OAuth-state and grant-lineage work remains unvalidated.

Additional unresolved lifecycle findings: GCP service-account mint needs durable
pre-dispatch/response ownership and explicit expiry/unsupported-revocation handling
for provider-less keys; a late mint response must survive an erasure/lost-CAS race.
Active accounts reaching the analytics dispatch uncertain cap need an authenticated,
generation-fenced, audited operator repair path without deleting the account. These
are completion-critical, alongside shared provider grant scope, global runner and
remaining W4 families; no unsupported completion claim is made.

### Previous frozen validation boundary (build12 runtime complete)


Primary build12 ran 18 scopes: **567 passed, 2 failed, 0 ignored**. Results are `/tmp/nyxid-primary-build12-results.json` and `/tmp/nyxid-primary-build12-provider-results.json`. All 22 failures from build11 are repaired and passed their corresponding build12 scopes. The two new failures are real Aurinko regressions: the platform credential adapter incorrectly restricted descriptor-selected provider storage to `provider_type:oauth2`, while Aurinko remains `api_key`. Source removes that incorrect discriminator while retaining descriptor identity/context and composite transactions; that repair awaits the next immutable validation.

Frozen source is `/tmp/nyxid-security-build12-source`, with **4,157** entries in `/tmp/nyxid-security-build12-source-sha256.json` (SHA256 `23e68d939ba8870a57afb45b511069f4ac406ecbc23d981bc76fe98c8e8e744c`). Every entry was verified unchanged after successful stable Rust 1.96.0 compilation (11m19s, one job, test profile, final `-C debuginfo=0`, two unused migration-runner helpers). The cloned harness `/tmp/nyxid-security-build12-server-tests` SHA256 is `d1f380e8ff8c57fe6403f4ad8d4044e9092a8cd916319d3c3c4e4e29e00513cd`, identical to the source harness. Primary independently verified both. Compiler/runtime ownership returned to the implementation worker after primary cleanup confirmed only admin/config/local databases remain.

Build12 validates the repaired integrated-erasure graph/held writers, initial effect fairness/backoff/corruption handling, raw BSON array migration for MFA/saved-login/node, staged provider/client adapters and composite consumers except the two Aurinko failures. It does **not** validate post12 atomic issuance/install receipts, stronger destination/response binding, preparation adapter dispatch, actual BYO dependency revision, Node compatible-writer correction, later key/token conversions or retry-budget/Debug/HMAC-buffer changes. Full shared-grant lineage/isolation, all remaining field adapters, migration/readiness runner and final owned-plaintext propagation remain incomplete. No production migration, writer activation or key retirement is authorized.

Historical primary evidence remains `/tmp/nyxid-primary-build11-results.json` (**433/22/0**) and `/tmp/nyxid-primary-build10-results.json` (**1,178/3/2**). The build10 concern test passed separately using replica discovery; build11 repaired that fixture. The two CLI-backed ignored E2Es still require the matching frozen CLI executable before final validation. Current client evidence: web/mobile TypeScript pass, 85 targeted web tests pass, 67 mobile tests pass, pagination identity test passes, web lint has zero errors and 34 warnings in 22 baseline-unchanged files, and isolated production web build passes with static include assets present.

## Work boundaries and original subclaims

| Work | Original subclaim | Planned deliverable | Status / dependency |
|---|---|---|---|
| W1 | #1928 reporting channel | Root policy, issue chooser/bug warning, managed-template reconciliation support | In progress; reporting already enabled in assessment; maintainer SLA/version policy not supplied |
| W1 | #1940 all 27 statements | Crosswalk below, corrected current-truth docs | In progress |
| W1/W2 | #1930.1 audit 90-day purge | Remove unsupported promise | Code retention belongs to W8; no audit TTL, historical mutation, or re-signing |
| W2 | #1929 backend consent, direct track, pre-login | Persist default-off account permission; fresh fail-closed dispatch gate, no positive cache, queued withdrawal | Implemented; build5 consent6 + dispatch1 + telemetry64 passed; parent reviewed; final-source validation pending |
| W2 | #1929/#1930.3/.5 controls | Web/CLI/mobile account permission plus local/DNT narrowing and account-switch isolation | Implemented; web/mobile suites and visual review recorded below; CLI27 distinct passes; parent accepted reviewed consent slices |
| W2 | #1930.2 erasure | Complete collection/reference inventory; durable bounded resumable personal erasure; revoke/fence first, preserve org resources and immutable evidence | In progress; vendor erasure build5 9 passed; integrated accepted-intent/domain worker, routes and client draft in frozen build11; build12 erasure22 passed; full primary567/2 with Aurinko repairs unvalidated; shared-grant isolation and remaining review acceptance open |
| W1 | #1930.4 export | Truthful contact channel and no claim of self-service endpoint | Manual fulfillment operationally unverified; self-service export separately scoped |
| W1/W3 | #1930.6/.7 token storage/rotation | Correct hashing/JTI wording and supported rotation mechanics | Implemented JWT/HMAC lifecycle draft; build5 lifecycle evidence below; final snapshot and rollout dependencies remain |
| W1 | #1930.8 vendors/transfers | Implementation data-flow inventory; remove unsupported contracts/regions/legal claims | Deployment-specific register and legal confirmation unresolved |
| W3 | #1931 recovery | All login MFA surfaces, bounded off-runtime Argon2 work, atomic single-use, notifications, concurrent tests | Implemented; build5 MFA service12 + HTTP11 + approval9 passed; web/mobile/CLI evidence below; final-source validation pending |
| W3 | #1939.1 ordinary OAuth | Durable live consent generation across token entrypoints, revoke/regrant/issuance races | Implemented core live authority; build5 authority4 + exchange20 passed; build7 OAuth93/broker38/auth124/token13 + authority5 passed; broader issuance/catalog acceptance matrix in progress |
| W3 | #1939.2 JWT | Active signer + retained kid-selected verification/JWKS, PEM strength/correspondence validation | Implemented; build5 ring/JWKS8 + actual assertions1 + consent-request retained-key paths passed; final-source pending |
| W3 | #1939.3 HMAC | Independent lifecycle with retained chain verification; no accidental invalidation on signing rotation | Implemented independent-domain adoption; build5 key-material4, HMAC fixtures/preparation/history passed; coordinate retained W8 evidence and operator rollout |
| W3 | #1939.4 passkeys and org MFA | Precisely scoped separate capability design, not claimed shipped | Separate capability: WebAuthn RP/origin, enrollment/recovery and org enforcement across sessions/OAuth/API keys/SAs/break-glass |
| W4 | #1933 Debug | Explicit redaction for AwsCredentials and CachedToken, distinctive-secret regression tests | In progress |
| W4 | #1933 plaintext lifetime | Propagate zeroizing decrypt buffers, proxy credentials, exchange cache and consumers | In progress; owned cache token propagation and expiry drafted; no universal library-memory guarantee |
| W4 | #1934.1 binding | Canonical collection/record/owner/field context, authenticated new version with no downgrade; preserve existing AAD formats | In progress; contextual primitive + Node adapter/transfer/strict-reader setting passed build7 contextual nodes5; remaining callers/rollout open |
| W4 | #1934.2 headers | Encrypt sensitive defaults at rest, preserve masked edit and HTTP/WS/node transport | In progress; dedicated storage/DTO/resolved adapter, exact masked CAS, zeroizing merge/node owners, transfer rebind and transport sensitivity drafted; compile/runtime pending |
| W4 | #1934.3 migration | Complete ciphertext field inventory, bounded durable resumable CAS runner and retirement readiness | In progress; complete field inventory and first Node CAS adapter drafted; durable runner/readiness pending; no production migration |
| Excluded | #1932 W5, #1935 W6, #1936 W9, #1937 W7, #1938 W8 | No broad delivery/updater/build/defaults/audit implementation | W1 corrects misleading related docs only |

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
| 11. “Run a re-encryption job” | State that the primitive exists but no production runner was found; do not give an unavailable command as an incident step | W4 / #1934 |
| 12. Rewrap as records are touched | Remove until an actual migration/lazy-writeback path ships | W4 / #1934 |
| 13. No service can persist a plaintext credential | Qualify/remove; sensitive default headers contradict it | W4 / #1934 |
| 14. Every Debug implementation redacts | Remove the universal claim, fix identified types, and describe the enforced scope | W4 / #1933 |
| 15. KMS removes all long-lived key material | Limit claim to KMS-held KEKs; local JWT/private signing material and other secrets remain | W3/W4 |
| 16. Node signing secret stored only hashed | Correct: the node model also has `signing_secret_encrypted` for server signing needs | W1; `models/node.rs:148` |
| 17. Only old timestamps described | Add future-skew rejection: implementation uses absolute skew above 300 seconds. Existing wording is incomplete, not proof future requests are accepted | W1; `cli/src/node/signing.rs:39` |
| 18. Delegation token on every MCP call | Make conditional on configured `inject_delegation_token`; it defaults off | W1; `mcp_service.rs:5360` |
| 19. Error bodies can be logged | Correct the documented diagnostic behavior; current code includes a regression test asserting upstream-body content is absent from captured logs | W1; `handlers/proxy.rs:11252` (test inspected, not run) |
| 20. Four injection modes | Expand the table or label examples. The page says “several,” so incompleteness is more precise than claiming it explicitly promises only four | W1; `proxy_service.rs` includes body/path and other modes |
| 21. Bodies never parsed/modified | Correct: body injection and other explicit transforms exist; describe exceptions without promising universal pass-through | W1; `proxy_service.rs:4513` |
| 22. Granular approvals “no code yet” / no MCP approval | Mark historical baseline as historical and describe current implementation | W1; current MCP preparation/approval paths |
| 23. cua 0.30.4 | Update the pinned-version reference to the code’s 0.31.0, ideally generated from the authoritative pin | W1/W9; `cli/src/node/machine/cua.rs` |
| 24. Azure Key Vault | No Azure provider implementation found. The cited passages are provider comparisons; label Azure as comparative/planned rather than implying shipped support | W1/W4 |
| 25. “M1 draft” title | Align title with status and incomplete rollout; editorial consistency, not a runtime defect | W1/W9 |
| 26. Missing `k8s/` manifests | Remove invalid installation commands or provide maintained manifests as a separately scoped deployment task | W1/W7 |
| 27. Telemetry CORS “not yet landed” | Resolve the contradictory current heading/history; retain historical evidence with explicit dates | W1/W2 |

## Validation actually run

- **Primary build8 runtime: 9 suites,195 passed/0 failed/0 ignored.** Node context9,org authority6,org33,admin25,personal erasure4,channel admission4,provider/cache37,GCP10,AES67. `/tmp/nyxid-primary-build8-results.json`, logs00..08. Owned fixture DBs cleaned; compiler/runtime handoff released. This validates frozen build8 only.

- **Build8 compiled successfully in 10m47s** from immutable `/tmp/nyxid-security-build8-source`, using official rustup stable Rust1.96.0, one job, test harness, final crate-only `-C debuginfo=0`. All1,882 source/asset hashes rechecked against `/tmp/nyxid-security-build8-source-sha256.json`. Cloned executable `/tmp/nyxid-security-build8-server-tests`, SHA-256 `10dd09cfc5d1cec4fe38c7fdecd6abdfe2098a9c5f0421b1ec21489cd7664ddf`. Log `/tmp/nyxid-security-current-harness-build-8.log`. Five warnings reflect unfinished worker integration. Primary runtime handoff pending; no runtime pass claimed. Newer header/context/Debug/node semantic-validation/adapter durability edits are outside this snapshot. Earlier wrong-toolchain/stopped attempts are not passes.

- Parent's repaired **build5-only** runtime total: **676 passed / 17 failed / 0 ignored** across the original 22 suites, broker/auth and repaired MCP/admin/org suites. Repaired suites: MCP203/203, admin_user_service25/25, org_service33/33; `/tmp/nyxid-primary-build5-repaired-results.json`. The 17 earlier OAuth broker/handler/refresh and middleware fixture failures remain recorded as build5 evidence and were all fixed in build7 runtime. Mongo file-descriptor exhaustion was infrastructure failure and was not counted as application failure.
- **Build7 completed successfully** in 14m55s with no warnings/errors. Primary runtime: **16 suites, 470 passed / 0 failed / 0 ignored**, including all 17 previous failures. Token13/OAuth93/broker38/auth124; actual userinfo/introspection/manual MCP in authority5; AES67; contextual nodes5; org fences4/org service33/admin25; personal acceptance2; push9/Telegram9/GCP10/provider exchange32/refresh containment1. Results `/tmp/nyxid-primary-build7-results.json`, logs `/tmp/nyxid-primary-build7-{00..15}.log`, cloned harness `/tmp/nyxid-security-build7-server-tests`. Compile log `/tmp/nyxid-security-current-harness-build-7.log`, initial source manifest `/tmp/nyxid-security-build7-source-sha256.txt`. This is development snapshot evidence only: later cache/registration/disable/header/worker changes are not validated. Final acceptance requires frozen source throughout compilation or an immutable source copy.

### Post-build7 source work (build8 compile only; runtime pending)

- Reversible administrative disable writes the shared person lifecycle fence and directly invalidates issuer/personal pending registration tokens. It does not scan memberships, require another admin, or impose the permanent-departure cardinality bound. Permanent erasure/removal/demotion retain recovery checks; disabled people do not count as recovery authority. Sole-admin and >256-membership regressions drafted.
- Partial personal-erasure worker uses majority leases/checkpoints and MongoDB `$$NOW`/`$dateAdd` for update/completion/retry scheduling. Operator retry records metadata-only actor audit and cannot attest completion. Platform/Oracle adapters drafted; remaining domain adapters, API integration and worker startup are incomplete. No full-erasure completion claim. Crash-after-effect adapter tests remain required.

- Node transfer checks name collisions against the name loaded inside the transaction. Held KMS regressions cover rename and successor rotation without overwriting current credential material.
- Node registration stores person issuer separately from polymorphic owner. Admin role is checked at issuance only; majority transactions fence mint and consume+insert against owner/issuer/org lifecycle and exact token invalidation on demotion/departure. Demotion/revoke/departure removes pending issuer registrations; unknown-issuer legacy org registrations fail closed and are cleared conservatively. Established org nodes and another admin's attributed tokens survive. Held-KMS redemption regression drafted; no runtime pass claimed.
- Generic token-exchange cache has periodic expiry, bounded residency/admission and zeroizing stored values. Credential replacement/disconnect clears matching local cached material and increments a generation so held exchanges cannot repopulate. Matching-source authorized owners can immediately refetch through a stable single-flight slot; cache cleanup is not authorization revocation. Shared-source HTTP and held-fetch regressions drafted. Exchange/cache/Lark owned token returns now use Zeroizing<String>; request-building copies and wider credential lifetime propagation remain open.
- Header adapter design explicitly preserves frozen v1/v2 plaintext authority digests through a bounded, local authority-only decoder after policy/ACL gates. No upstream/provider/OAuth refresh or generic materialization is permitted in that decoder. Storage adapter and denied-policy/frozen-digest integration tests remain unfinished; see the encrypted-field inventory.

- Initial branch/status inspection: clean `nyxid-security-remediation` at assessed baseline.
- Native `mongod` available. Docker daemon unavailable. Parent provisioned MongoDB 8.0.17 replica set on port 27284; URI `mongodb://127.0.0.1:27284/?replicaSet=nyxid_security` confirmed. Existing MongoDB 9 on port 63201 is unrelated and must not be touched.
- `python3 -m unittest discover -s scripts -p test_security_reporting.py`: 3 passed (generated chooser restoration/idempotence, malformed generator shape, repository routing).
- `python3 scripts/reconcile-security-reporting.py --check`: passed.
- `cargo fmt --all -- --check`: passed.
- `npm exec tsc -- -b` in frontend: passed. `npm test -- src/pages/settings.test.tsx`: 8 passed.
- `cargo test --locked -p nyxid-cloud-auth --target-dir /tmp/nyxid-security-cloud-auth-target`: 7 unit tests passed; 0 doc tests. Covers credential and signed-header Debug, malformed credential parse error redaction, serialization and actual signing/session-token injection. Backend token-cache/Wrapper Debug tests are still compiling and are not counted as passed. No skipped Mongo test counts as validation.
- Parent baseline before implementation: frontend telemetry/consent tests 38/38 passed; mobile `pnpm typecheck` passed. These are baseline evidence, not validation of later changes.

## Ordered implementation batches

1. W1 current-truth disclosure/privacy/docs and narrow W4 Debug changes; test and commit for parent review.
2. W2 central consent and all client controls; W3 complete MFA recovery and adjacent state races.
3. W3 ordinary OAuth generation and entrypoints; W2 inventory and resumable erasure.
4. W4 encrypted headers, propagated secret lifetimes, canonical binding and complete migration/retirement checks.
5. W3 JWT/HMAC lifecycle, documentation final alignment and full local integration/concurrency validation; address parent findings throughout.

## Unresolved external dependencies

- Maintainer acknowledgement/support commitment; no SLA or support matrix invented.
- fkst-hosted must adopt/invoke repository reconciliation support in its managed template generator; local support cannot prove external reconciliation behavior.
- Hosted vendor roles, regions, contracts, transfer basis, backup and external retention/erasure fulfillment require operator confirmation.
- W8 integrity-aware retention/external audit witnessing remains separate. Immutable audit/billing rows must not be deleted, scrubbed, or re-signed to imitate original evidence.
- Passkeys, org-required MFA and self-service export remain separate capabilities per assessment, pending explicit scope change.

## Managed issue-template reconciliation contract (W1)

Read-only upstream snapshot: fkst-hosted `develop` at `9349145` (parent review). `backend/src/github_app/templates.rs::bundled_templates()` returns fixed `config.yml` and three fkst Markdown templates. `template_install.rs::open_install_pull` writes exact bundled content, with no repository customization hook. The bug form is not managed and its private-report warning persists. The local version marker remains 13.

`scripts/reconcile-security-reporting.py` restores the NyxID contact link while preserving the generator marker, other links and public bug reporting. The always-running CI change-detection job checks this contract and fails if a later reconciliation removes the notice. **Detection/local restoration is not durable upstream reconciliation.** fkst-hosted must either merge this repository-owned contact during template generation or add an equivalent private-reporting link to its bundle and advance its own version through its normal process. No other repository was edited and no upstream behavior was verified at runtime; this acceptance item remains open.

Parent review findings incorporated in this batch: redact SignedHeader values and WrappedKey identifiers/material; sanitize AWS credential JSON parse errors; correct response-header source and rotation staging/rollback; label dated approval baseline distinctly from later execution-authority contract; consolidate web privacy onto the canonical document shared with mobile. Re-align interim policy limitations as code lands.

## Additional W2/W3/W4 review acceptance (pending)

- W2 vendor erasure: `$delete_person` sent to an ingest DSN is not supported deletion evidence. Replace with authenticated PostHog personal-API-key person lookup/deletion with `delete_events=true` and `delete_recordings=true`, plus asynchronous deletion-status confirmation. Missing configuration stays pending/operator-action; distinguish accepted from completed. Never log the personal key. Official reference: https://posthog.com/docs/privacy/data-deletion (parent cached `/tmp/nyxid-posthog-deletion.txt`).
- W2 erasure jobs need bounded fenced lease reclaim after crash, central admin/self deletion intent even when telemetry is currently disabled, and dispatch only after durable account/consent fencing. Historical analytics destinations need explicit handling.
- W2 collection inventory includes models AND service-layer literal collections, reference/embedded IDs. Discovery aids `/tmp/nyxid-erasure-model-candidates.txt` (129 files) and `/tmp/nyxid-security-encrypted-fields.txt` (32 model files) are not authoritative delete rules. Creator attribution is not ownership; preserve shared resources and immutable evidence.
- W3 include MFA pending-challenge single-use/live-account checks and enrollment activation/deactivation CAS races.
- W4 `node_service::transfer_node_owner_with_expected_state_version` must rebind signing-secret ciphertext transactionally on legitimate owner transfer. Inventory other transfers. Account erasure must fence node WS/proxy authority across replicas and preserve remote-machine guards before deleting node records.
- W4 inventory must include platform credential maps, encoded OAuth verifier bytes and nested transport/NyxBot fields; exclude node-pending E2E ciphertext explicitly by encryption contract. Uploads share `assistant_attachments`; there is no separate `assistant_uploads` collection.
- W2 admission captures persisted consent revision, dispatch freshly reads primary/majority and requires the same revision; changes use majority writes. Wall clocks and same-millisecond ordering are not consent authority.

## Committed batches

- `6171e8d6` — initial W1 current-truth docs, disclosure guidance and local managed-template regression support. Reporting helper 3 tests, frontend Settings 8 tests, frontend TypeScript and Rust formatting passed. Maintainer SLA/support policy and upstream generator durability remain open. Parent review follow-ups are being applied in subsequent commits.
- Narrow cloud-auth W4 — custom Debug for credential IDs/secrets and all signed-header values; sanitize malformed JSON diagnostics. All 7 cloud-auth unit tests passed using an isolated target; full output distinguishes 7 unit tests from 0 doc tests. Broader backend plaintext, binding and migration scope is still pending.

## Continuation checkpoint: consent implementation draft

W1 parent follow-ups now correct the compromised-key incident procedure, staged-reader encryption concept, stale mitigation-table claims and the shared upload/attachment collection. Reporting helper regression suite rerun: 3 passed; repository `--check` passed. The primary backend consent draft is not yet validated: it adds random consent revisions, explicit primary/majority reads and majority writes, bounded 250ms admissions, a bounded dispatch queue, direct-track checks, device/DNT narrowing and pre-auth denial. Actual HTTP-egress regression and command-concern tests have been added; their execution is pending. Full-source compilation is in progress; the earlier token-cache run (30 passed) predates the latest WrappedKey regression and does not validate it.

Client contract decision for the next batch: account permission is global and device choices only narrow it. Direct vendor SDK dispatch with a cached positive cannot enforce remote withdrawal. Route client product events through authenticated NyxID dispatch, binding admission and delivery to the same account consent revision. No pre-auth anonymous analytics; no account switch may carry another identity's device choice or queued event. Existing replay/autocapture must not bypass this gate. This is planned, not yet shipped. Update policy and TELEMETRY sections 3/4.2 when each implementation lands; the old ingest deletion event is still unsupported and erasure remains open.

W1 upstream deliverable is now concrete: after primary source batches prepare a separate local patch against reviewed fkst-hosted develop `9349145`, reading its AGENTS first. Generate deterministic repository-aware private reporting/policy contacts from validated owner/repo, bump the upstream bundle version there and regression-test NyxID chooser/reconciliation. Do not merge arbitrary repository content into its auto-merged PR. No upstream edit/publish has been performed; application remains an external dependency.

W1 review follow-up: the web and mobile legal renderers suppress their metadata date when the canonical Markdown already displays the same date; raw/mobile document readability is preserved. Frontend TypeScript and mobile `pnpm typecheck` passed on this change. Parent visually loaded `/privacy` at local port 5197: all 11 sections and links render with no horizontal overflow; duplicate-date correction awaits parent visual recheck. The reporting regression no longer pins bundle version 13: well-formed future markers are accepted, and deterministic upstream `/security/policy` contacts are accepted without duplicate private links. Reporting helper: 4 tests passed and repository `--check` passed. Upstream generator patch and its regression remain pending.

W3 entrypoint inventory addition from parent: direct decoding consumers include OAuth ConsentRequestClaims, platform public-contract Assertion and remote-machine bootstrap verification. Keyring selection must cover these, relay replies/callbacks and independently built JWKS. HMAC domains include auth-device, CLI pairing, audit, billing, internal dispatch and startup-derived domains; preserve historical canonical fixture bytes and verification keys. No lifecycle implementation claimed yet.

### Additional parent acceptance constraints (implementation pending)

- W4 default headers: redact `execution_authority::HeaderProjection` and enclosing diagnostics. Preserve frozen v1/v2 effective-header approval projections across storage-only encryption/rewrap; never bind randomized ciphertext or move credential materialization ahead of policy/authority checks. Test storage-only migration versus actual header/policy changes, masked edit preservation, direct/node HTTP and WebSocket parity. Catalog lookup and encrypted-header decode failures must fail closed rather than silently omit required headers.
- W2 permission authorization reads now use primary **linearizable** concern with bounded maxTimeMS; majority writes persist withdrawal. Display reads may use majority. Final-source tests remain pending.


### Current continuation evidence (consent slice; broader work remains pending)

- Mobile final-source `pnpm test`: **64 passed, 0 failed, 0 skipped**, including real delayed HTTP refresh/transport boundaries, serialized SecureStore/orphan cleanup, account-bound relay and captured-account persistence failure/retry. `pnpm typecheck` passed on the same source. Device persistence errors are exposed; a failed negative remains session-only until retry succeeds, with no claim that disk changed.
- Web seven-suite run: **70 passed** (telemetry, consent store, consent mutations, API client, auth store, Settings, mock fixtures). Current-source TypeScript and targeted ESLint passed. Parent browser verification confirms privacy date appears once, 11 policy sections, and Settings default-off/account-on/local-off/account-off states without desktop overflow.
- Current backend `cargo check --locked -p nyxid --bin nyxid-server --tests -j 1 --target-dir /tmp/nyxid-security-check-target` passed. Backend final-source regression binary remains compiling; none of those new tests are counted as passed yet. CLI check passed; final CLI tests are pending.
- CLI profile tests now target actual `Cli::command()` parsing plus production `selected_profile`, including nested commands, environment/default and telemetry profile placement. Added actual HTTP prefix/surface/relay and disable-on-network-failure regressions; execution is pending.
- W3 all-login-surface inventory explicitly includes mobile `AuthHomeScreen` password login: TOTP/recovery continuation, primary-factor/account binding and pre-auth denial must be implemented and tested alongside web MFA and approval forms. No W3 recovery implementation is claimed yet.
- Consent implementation is being prepared as one coherent reviewed batch; no positive consent caches or direct client vendor dispatch remain in this draft. Erasure, MFA, OAuth/JWT/HMAC lifecycle, canonical encryption/header storage and migration remain unfinished. Original subclaims remain individually open above.

### W3 active implementation checkpoint (not yet tested)

Recovery draft now mints 32 hex characters (128 bits), retaining legacy 8-character redemption. Shared login verification, active/person/non-deleting account fence, atomic hash pull, enrollment/disable serialization and duplicate-factor denial are in progress. TOTP uses the matching ±1-step counter with an atomic monotonic replay fence; enrollment consumes its proof counter. New first-factor challenges are being moved into `mfa_challenges` with explicit purpose, physically separate from completed `sessions` so old ordinary-session readers cannot accept newly minted challenges. New readers also reject legacy `mfa_pending` sentinel sessions for cookie/JWT/refresh/MCP authority. Existing old replicas remain vulnerable to legacy challenges until upgraded/drained and those five-minute challenges expire; deployment prerequisites must state this accurately. No deployment or external disclosure occurred.

All W3 regression execution is still pending. The new actual HTTP bypass regression must prove a password-only challenge cannot access `/users/me` or protected management, then prove completed MFA creates a usable session. Inventory also covers mobile password continuation and both web login surfaces. Recovery notification uses existing configured push/Telegram channels and metadata-only audit; no claim of universal delivery where no channel is configured.

### Current MFA/UI validation and next backend snapshot

- Web MFA suites rerun after fixing the approval input label: **84 passed, 0 failed** across `mfa-verify-form`, `mfa-setup-dialog`, `login-device`, and shared auth schemas. Actual approval continuation covers six-digit TOTP, legacy eight-character recovery and new 32-hex-character recovery. Setup uses full 32-character fixtures and verifies exact clipboard content. Recovery display is a wrapping single-column list; parent visual review is pending.
- Mobile full suite after challenge continuation/outcome fencing: **66 passed, 0 failed, 0 skipped**. The same dispatcher used by AuthHome drops late successful/error outcomes after session-generation changes or unmount; its regression holds an MFA-required response across both boundaries. No challenge token/body enters diagnostics.
- Final TypeScript/ESLint and backend Rust source checks are still running. No additional heavy Rust build was started while the three existing host compiler processes finish. Earlier build snapshots do not validate the current MFA changes.
- Backend draft additionally tests TOTP first use/replay/concurrent exactly-one/newer step, ±1-step matching, pending-cookie/JWT-sid/refresh/MCP authority denial, legacy sentinel denial, and revoke-all invalidation of dedicated challenges. New `mfa_challenges` is included in personal erasure cleanup. All these Rust tests remain **unexecuted** until the final snapshot builds.
- Narrow W4 draft now redacts all `HeaderProjection` values through enclosing Debug output while preserving serialization and approval digest semantics; regression execution pending. This does not implement encrypted header storage or complete plaintext lifetime propagation.

Parent visual verification of recovery layout: all ten 32-character code fixtures fit a forced 320px dialog width with wrapping, visible copy icons and scrolling, without horizontal overflow. Browser clipboard observation was interrupted and is **not** counted; exact 32-character clipboard content is verified by the passing setup-dialog unit test. The temporary browser harness was removed by parent and is excluded from commits.


### Continuation: completion transactions and vendor reconciliation (unvalidated Rust)

- Parent independently reran the current mobile suite: **67/67 passed**. The local full suite also passed 67 with no failures/skips, followed by TypeScript with no diagnostics. Web MFA remains 84/84; current TypeScript and targeted ESLint passed. Parent verified all ten 32-character recovery fixtures at 320px without horizontal overflow; clipboard evidence is unit-only.
- The redundant stale backend check was stopped by parent under host swap pressure. **It is not a passing final-source check.** The long-running analytics test build and queued CLI tests predate current MFA/erasure source. Only one subsequent current-source Rust compile/test job will run at a time.
- MFA review confirmed consumption/session insertion were split. Current draft holds an uncommitted verification transaction through browser/token session insertion and browser approval verification, commits before notifications, and rechecks TOTP/challenge time at commit. Revoke-all/password reset write the same account fence transactionally with session/refresh/challenge/browser-approval invalidation. Enrollment/disable invalidate earlier pending challenges/approvals so they cannot authenticate a successor factor. Held-boundary integration/concurrency validation remains required and unexecuted.
- Narrow Debug draft covers AppError/ErrorResponse and auth/MFA/OAuth secret-bearing DTOs, all default-header values and execution projections. No final Rust regression results yet; broader plaintext lifetimes/header encryption/AAD/migration remain open.
- Authenticated vendor erasure draft replaces unsupported capture events with personal-API-key lookup/DELETE/asynchronous status, retained destination registry and fenced reclaimable jobs. **Not complete or tested:** durable parent intent and resumable historical-destination fan-out must cover absent/late config and prior legacy/completed jobs; missing-person reconciliation needs a supported auditable operator path.
- Erasure must also account for capture already in flight across replicas. A fresh permission check alone cannot prevent a request already handed to the vendor from arriving after DELETE. Durable bounded dispatch fences/drain and a held-provider regression are required; exact provider asynchronous guarantees must be documented without claiming universal erasure.
- Full account collection/reference inventory and bounded resumable account workflow, ordinary OAuth revocation generation, JWT/HMAC lifecycle, all encryption callers and migration remain unfinished. W8 immutable evidence is retained unchanged.


Latest backend checkpoint (unexecuted): factored bounded same-transaction commit retry into neutral `transaction_service`; no erasure/token path depends on MFA error wording. MFA HTTP regressions now force session-insert failure (expect rollback of factor/challenge), hold a completed response across account-wide revoke, and reject pending completion after revoke/reset/disable→successor enrollment. Auth/admin development reset-token tracing values were removed; broader logging regression remains pending.

Vendor erasure draft now includes retained parent intents, bounded restartable legacy-parent discovery, registry-revision fan-out/reopening, audited local `bind-person`/`attest-no-data` reconciliation, post-delete lookup, and a held-capture fixture regression. Dispatch accepts no proof from elapsed lease time: unresolved transport blocks erasure; provider processing requires separate generation-bound operator drain evidence. Confirmed accepted event rows are removed, retaining one account checkpoint rather than an unbounded event history; uncertain rows/reconciliation evidence are retained. `status --user-id` applies to both jobs and dispatches. User/status, global-status and erasure-claim indexes are included. All new Rust regressions remain unexecuted.

Parent stopped the stale analytics test compiler under swap pressure; it produced **no test result**. The stopped check and stopped analytics build are not passes. Queued CLI validation is now building. Next backend validation will compile only the current backend crate with reduced debuginfo (`cargo rustc --locked -p nyxid --bin nyxid-server --profile test -j 1 -- -C debuginfo=0`) and run its actual test harness against the required isolated MongoDB URI, sequentially with other local Rust validation.


### Continuation: password proof and personal-owner boundaries

- CLI consent validation actually ran: `cargo test --locked -p nyxid-cli --bin nyxid telemetry --no-fail-fast` **22 passed**, and the compiled harness production `selected_profile` parser filter **1 passed**. Evidence: `/tmp/nyxid-security-cli-consent-tests.log` and `/tmp/nyxid-security-cli-profile-tests.log`. These results predate the new CLI password/MFA continuation and do not validate it.
- Current backend harness build uses `cargo rustc --locked -p nyxid --bin nyxid-server --profile test -j 1 -v -- -C debuginfo=0`; verbose rustc output confirms `--test`. An earlier command using both `--tests` and `--bin` was rejected for selecting multiple targets; it did not run tests. Only one owned Rust build runs at a time. Source changes after its snapshot require a subsequent compile before their regressions count.
- Password proof now binds the verified password hash and MFA state to the live personal account inside challenge/session/browser-approval installation transactions. Held-after-password-verification HTTP regressions cover reset against pending challenge, inline MFA, ordinary login and browser approval. **Draft, Rust tests pending.**
- Existing CLI `login --password` now has a bounded hidden TOTP/legacy/new recovery continuation with sanitized diagnostics, selected-profile/full-base preservation and real HTTP regressions. **Draft, compile/tests pending.**
- Parent identified that personal cascade accepted organization `User` rows. The shared service now rejects organizations before vendor enqueue, admission closure, security-state changes or deletion. A mandatory-Mongo regression exercises direct admin service, self service and actual admin handler with complete before/after collection snapshots. **Draft, compile/test pending.** Organization deletion remains the separate guarded lifecycle documented in `ORG_MODEL.md`; `created_by` alone never establishes personal ownership.
- Remaining W3 lifecycle inventory includes assistant action receipt fingerprints: retain the legacy `assistant-action-fingerprint` derivation from ENCRYPTION_KEY **hex string bytes**, preserving persisted matching. W4 binding migration also preserves native platform entry/reply `binding_digest` AAD, alongside broker/Oracle formats. Neither lifecycle migration is implemented yet.


### Current continuation: shared MFA admission and upstream artifact

- The active reduced-debuginfo backend test-harness build is a snapshot **before** the shared account limiter below. No runtime backend tests from that snapshot have run yet. The first build failed with Send lifetime diagnostics; the replacement uses a Send-safe neutral same-session commit abstraction and is still compiling. Neither stopped/failed compilation is counted as passing.
- Shared factor admission now uses the existing durable `RateWindowStore` before input parsing and expensive recovery verification, with majority writes, ten admissions per account in each DB-clock five-minute fixed window. Inline password MFA, pending challenges and browser approvals share this entrypoint. Failed attempts and aborted completions do not refund admission. Adjacent fixed-window boundaries can admit two bursts; this is a finite online budget, not a sliding-window claim. Database failure denies completion. New cross-client concurrency/isolation/reset and real HTTP inline/new-challenge regressions are **unexecuted** and require recompilation.
- The new CLI production-seam regression snapshots selected/default/sibling credential files, holds them unchanged through failed MFA (including a distinctive-secret response), and checks successful replacement affects only the selected profile. This and the new CLI password/MFA exchange suite are **unexecuted**; earlier CLI consent/parser passes predate them.
- Durable upstream artifacts: [patch](security/fkst-hosted/security-reporting-9349145.patch), [application/validation notes](security/fkst-hosted/README.md). Parent independently passed final patch `git apply --check`, actual renderer **4/4**, and actual rendered NyxID output through the repository checker: unchanged, exactly one policy contact. Separate-copy Rust formatting passed. Installer/reconcile runtime tests remain pending, as does external upstream adoption/deployment. NyxID local marker remains 13.
- Personal deletion accepted-intent semantics remain unfinished: existing cascade currently enqueues vendor intent ahead of effect-conflict guards. The bounded durable workflow must accept/fence only after guards, preserve the usable account when deletion is rejected, and prove vendor dispatch cannot erase a rejected deletion. This is an open W2 acceptance item, not completion.
- W4 plaintext inventory explicitly includes TOTP library-owned clones, `TotpSetupResult`, recovery `Vec<String>` and request/response serialization buffers; current wrappers do not cover these lifetimes. Broader W2 personal erasure, W3 ordinary OAuth/JWT/HMAC and W4 header/AAD/migration work remains open.

Parent lifecycle review additions: imported JWT key pairs must match, a partial development PEM pair must fail without overwriting the surviving file, and generated private files must be restrictive from first creation (no write-then-chmod exposure). Regression coverage is required with the keyring batch. Enable the installed totp-rs zeroize support and handle owned `Secret`/temporary strings in W4; feature support alone is not full plaintext propagation. These are pending.

Vendor/dispatch follow-up draft (tests unexecuted): per-account unresolved dispatch cap is 32, serialized on the source account transaction; accepted per-event rows are deleted and uncertain records never expire to free budget. New DB regression exercises saturation, independent clients and acceptance cleanup. Explicit failed/unknown vendor status now becomes visible operator action, with evidenced bind-person restart and a fresh baseline; new HTTP-vendor/DB regression covers failure→reconcile→new accepted request→verified completion. These edits postdate the active harness snapshot.

Additional personal-erasure ownership contract: `docs/ORACLE_RELAY.md` and `oracle_pool_service/lifecycle.rs` require owner tombstone, worker credential invalidation, durable cleanup and mandatory audit. Active queued/dispatched tasks and queued/delivered commands block without implicit abandonment. Reuse that lifecycle for owned pools; revoke a personal contributor worker under an org pool while preserving the pool. Oracle snapshot `created_by` is attribution, not ownership. Header masking sensitive→public changes require explicit safe replacement/preserved masked semantics, and migration readiness must authenticate/decrypt supposedly-current rows and count malformed/undecryptable data as failures. All remain W2/W4 acceptance work.

W3/W4 follow-through draft after the active compiler snapshot: the JWT loader refuses partial key pairs, validates RSA private parameters, a 3072-bit minimum and private/public correspondence, accepts PKCS#1/PKCS#8 imports, and holds private PEM text in a zeroizing owner. Generation uses exclusive `create_new` and Unix mode 0600 from first open; collisions never truncate another file. New weak/mismatched/invalid/partial-pair/restrictive-creation regressions are **unexecuted**. This is loader hardening only; retained-key selection/JWKS overlap and HMAC lifecycle remain unimplemented.

MFA plaintext propagation draft now enables totp-rs `zeroize`, wraps setup seeds/otpauth URLs, returned recovery vectors and MFA request secrets in zeroizing owners, keeps encrypted-secret decode bytes zeroizing, and propagates through assistant MFA response DTOs with redacted Debug. Constructor errors use fixed diagnostics. Existing TOTP internal copies now use the library's zeroize-on-drop; serializer/network/library allocations remain outside a universal memory-erasure claim. Cargo locked offline metadata and formatting pass, but these Rust edits require the next compile/runtime snapshot.

Ordinary delegation inventory refinement: delegated access may have no `client_id` while `act.sub` identifies the exchanging client. Applicable source and acting/receiving-client consent revisions must be carried through access and refresh, including third-party source tokens; do not gate only on `client_id`, require Machine scope for ordinary clients, or equate delegated execution scopes with consent scopes. Current ordinary delegation uses openid consent plus the client's delegation scope policy. Revoke/regrant, source-token liveness and existing catalog authority contracts remain explicit upcoming regression cases.

JWT keyring draft now includes bounded retained public PEM imports (`JWT_RETAINED_PUBLIC_KEY_PATHS`), strict RS256/known-kid selection, active+retained JWKS and the direct consent/public-assertion/relay consumers. Added production-consumer regressions for actual consent request validation, public assertion scope/body binding, and the mounted JWKS HTTP handler, including retirement and SPKI import/JWK compatibility. **All postdate the running backend harness and remain uncompiled/unexecuted.** External remote-machine bootstrap/provider verifiers use dedicated configured external issuer/key trust roots; their NyxID key references are fixtures, and those production trust roots are deliberately excluded from the NyxID ring.

HMAC review inventory remains open: all `audit_chain_hmac_key` consumers, not only chain verification, must retain matching semantics. This includes persisted instruction context, call binding/coordinator/custom LLM, Aurinko callbacks, voice credential revisions, assistant skill/learning authority, Machine/Oracle lifecycle and service history fingerprints. Pinning the existing audit-derived material independently would preserve these subdomains; rotating that material requires explicit historical and delayed-retry handling for every consumer. Internal dispatch and assistant action use legacy encryption **hex-string bytes**, while the other four domains decode the hex. No HMAC seed has been changed.

Independent HMAC adoption draft adds an authoritative restricted six-domain keyset file, exclusive local export command and a majority-durable consistency pin before new writers. First adoption requires exact unchanged legacy-domain matching and bounded latest audit/billing row MAC evidence before writing the pin; this is **not a full history audit** and does not synchronously scan chains or require head stability at ordinary startup. Existing full Integrity verification remains separate. Regression source covers wrong first candidates leaving no pin, both audit and billing evidence, retained v1/v2 canonical hashes and a persisted pending fingerprint through export/adoption/source change, plus independent-client mismatch rejection. These edits and all new JWT consumer tests **remain uncompiled/unexecuted**. [Operator adoption/overlap runbook](security/KEY_LIFECYCLE.md) is a draft awaiting runtime validation. This preserves HMAC keys independently; it does not support arbitrary HMAC-key rotation, historic re-signing or W8 retention changes.


### Build2 runtime checkpoint and current review follow-through

The reduced-debuginfo build2 harness finished in 40m01s. Against isolated native
MongoDB 8.0.17 at port 27284, its earlier source snapshot ran **114 tests: 110
passed and 4 failed**, none ignored. This is not a passing batch. Evidence:
`/tmp/nyxid-security-build2-runtime.log`. Individual passing suites: telemetry
63, HTTP analytics ingress/account binding 2, login approvals 9, vendor erasure
6, neutral transaction commit 1, WrappedKey/key-provider 8, private CORS 1.
Analytics consent was 5/6; MFA service 7/9; MFA handlers 8/9. In particular the
new WrappedKey redaction regression actually ran and passed in this binary.

Defects exposed by that run are being corrected without weakening assertions:
BSON maxTimeMS accepts Int32 or Int64 but must equal 200; rejected MFA
transactions must await definite abort before the next valid operation; the
actual get_me HTTP fixture must seed normal platform roles. All corrections
and the newer shared account limiter, dispatch bound/vendor failure, TOTP
zeroizing ownership, JWT/HMAC and ordinary OAuth code still require a new
source-snapshot compile and runtime validation.

Parent independently validated the separate upstream patch's actual **library**
suites: renderer/templates 17, installer 22, reconcile 10, all passed with no
ignored tests. Durable upstream README now uses `--lib`; the earlier bin run
executed zero tests. Applying/rolling out that external patch and reporting
policy SLA/support acceptance remain open dependencies.

New reviewed backend draft changes (not yet runtime-accepted):

- MFA disable captures the verified password hash, mfa_enabled and exact MFA
  generation and fences them in the mutation transaction. Held proof across
  password reset or factor-generation change must preserve successor factor
  and pending authority; fresh proof succeeds. Definite failure aborts are
  awaited; unknown commit outcomes retain same-transaction commit semantics.
- Recovery notices use at most eight admitted background tasks with a five
  second total timeout, with no pending spawn queue. Transactional recovery
  audit is durable; channel delivery remains best effort, including process
  termination/admission saturation. Stalled-channel regression pending.
- Native Sec-GPC:1 narrows the central request context, alongside DNT, before
  authenticated egress. New actual-egress case is pending the next binary.
- No-data vendor attestation requires transactionally fenced current transport
  settlement and provider-drain generation; no evidence/completion write on
  failure. Bind-person stays preparatory. Operator transaction uses majority.
  Destination ID `unresolved` is reserved in configuration, registry and child
  creation, retaining original parent IDs. New regressions are unexecuted.
- HMAC export/preparation runs before strict signer import and normal server
  writers. A legacy 2048-bit PEM may supply unchanged HMAC bytes for preparation,
  but the strict active/retained JWT ring still rejects it. Weak-key cutover
  requires planned token invalidation/reauthorization, documented in the key
  lifecycle runbook; no weak-verifier fallback. Regression pending.
- OAuth draft binds all direct tokens to their original consent revision and
  anchors RFC8693 delegation by JTI with source/receiving-client revisions and
  source session. Primary linearizable bounded reads and majority grant/revoke
  writes are required across REST/MCP/introspection/exchange/refresh. Signed
  service-origin delegation is separate so Aevatar/proxy/NyxBot callback
  contracts survive without OAuth consent. Unversioned OAuth delegation has
  no missing-row fallback and cannot acquire a later regrant. Full endpoint,
  callback, cross-client and racing revocation tests remain pending.

Full W2 personal-erasure orchestration/inventory and W4 encrypted headers,
zeroizing plaintext propagation, canonical context/owner-transfer rebinding and
bounded CAS migration/retirement are still unfinished. Review requirements
retained: reuse unified-key DisconnectPlan/OAuthAppLineage remote revocation
and durable retry outcomes, avoid org/shared grant collateral, preserve
ChannelBot webhook cleanup; Oracle tombstone/active-work guards and contributor
worker ownership; node/Machine cross-replica authority fences; DownstreamService,
ChannelBot, X OAuth token/key and node transfers must rebind contextual data in
the same fenced mutation. Creator attribution is never ownership. Preserve
existing broker/Oracle/native AAD formats and immutable audit/billing evidence;
W8 retention integrity remains a separate dependency.

### Continuation checkpoint: current OAuth and MFA source, build3

- The [OAuth authority rollout](security/OAUTH_AUTHORITY_ROLLOUT.md) requires coordinated admission closure, old-writer drain and replacement of all authority readers/writers before reopening. Legacy unbound/unknown-origin tokens fail closed and affected clients must reauthorize; mixed replicas cannot claim immediate revocation. JWT/HMAC preparation does not resolve this separate compatibility boundary. Deployment remains unperformed.
- Fixed derived service-callback lineage for every signed OAuth delegation origin, including catalog receiving-client tokens and ordinary exchanges without a source session or prior lineage. Added database/real AuthUser regression coverage using production proxy/Aevatar token builders and the real NyxBot `/users/me` path. Runtime pending; these fixtures do not yet cover the complete OAuth entrypoint/race matrix.
- Removed erroneous unrelated SkillActor field additions; retained authenticated lineage in MCP account-tool conversion. Updated expired consent-request fixtures to sign with the current kid, preserving their intended expiry assertions. Explicit missing/unknown-kid tests remain separate.
- Added awaited abort on definite token preparation/password-proof/approval precommit failures; account-wide session revocation now requests majority commit. Broader MFA source tests remain pending build3.
- Started one current-source reduced-debuginfo backend test harness build (`/tmp/nyxid-security-current-harness-build-3.log`). Build2 remains **110 passes / 4 failures** on its older snapshot; none of its failures are claimed resolved until current runtime verifies them.
- Build3 exited with exactly two fixture compile errors (moving from a zeroizing recovery-code vector and a stale disable-proof test call). Both were corrected. Build4 (`/tmp/nyxid-security-current-harness-build-4.log`) now validates the later snapshot, including precommit browser-cookie/approval-response preparation and the invalid-cookie/no-spent-recovery HTTP regression across standalone, inline and approval completion. No build3 runtime pass is claimed. Token insertion helpers leave abort/commit ownership with their enclosing transaction owner; definite preparation failures explicitly await abort, while unknown commits retry only the same commit.
- Parent independently ran current CLI password_mfa: **4 passed**, telemetry: **22 passed**, selected_profile: **2 passed** (one overlaps password_mfa), totaling **27 distinct passes**, zero failed/ignored. Logs: `/tmp/nyxid-primary-cli-mfa-tests.log`, `/tmp/nyxid-primary-cli-telemetry-runtime.log`, `/tmp/nyxid-primary-cli-selected-profile-runtime.log`. No subsequent CLI source changes at this checkpoint.
- Full personal-erasure inventory/job, complete ordinary/delegated OAuth acceptance matrix, contextual encrypted headers, full encrypted-field migration/transfer and plaintext/error-containment work remain open. W8 integrity retention and external upstream adoption remain explicit dependencies.

Remaining personal-erasure acceptance refinements from parent: reject last-active-org-admin deletion transactionally across membership mutation races; require transfer/promotion/org lifecycle instead of dissolving orgs. Accepted/pending/complete must be distinct in self/admin HTTP responses, audit events, CLI/web/mobile copy and authenticated operator progress. Preserve usable accounts and existing data when prerequisite guards reject. Header ciphertext/version/context remain server-owned storage, with dedicated API input/output DTO conversion; existing shared model/request/output types cannot expose or accept migration material. Expanded draft [encrypted inventory](security/ENCRYPTED_FIELD_INVENTORY.md) now records nested maps/enums, encoded fields and explicit E2E exclusions; it remains an implementation checklist, not completed migration evidence.

The [personal-erasure inventory and acceptance contract](security/PERSONAL_ERASURE_INVENTORY.md) preserves the 129-model discovery appendix, lifecycle/retention decisions, service-layer literal discovery requirement, last-admin fencing and accepted/pending/complete requirements. It is explicitly unfinished implementation work; the existing synchronous cascade does not satisfy the contract.

### Build5 runtime and subsequent source checkpoint

Build5 compiled successfully in 18m15s from the recorded manifest
`/tmp/nyxid-security-build5-source-sha256.txt` (log
`/tmp/nyxid-security-current-harness-build-5.log`). Parent is running isolated
MongoDB8 runtime suites sequentially. Confirmed so far: analytics consent6,
dispatch1, telemetry64, vendor erasure9, MFA service12, MFA HTTP11: **103 passed,
0 failed, 0 ignored**. This includes the previous enrollment, TOTP and HTTP
fixture failures, stale-password disable, shared rate admission, cross-client
proof consumption, invalid-cookie recovery across three completion surfaces,
challenge authority denial and stalled-notification response. Remaining suites
are running; no full backend acceptance or final-source pass is claimed.

Post-build5 source, awaiting the next compiler/runtime window:

- Explicit AES/aes-gcm zeroize feature requirements (already enabled in the
  resolved dependency graph). Provider/notification finite diagnostics and
  actual transport fixtures have advanced beyond the compiled snapshot.
- Node signing-secret adapter uses a server-owned explicit storage version,
  trusted collection/record/owner/field context and exact legacy decoding.
  Registration, device provisioning, managed bootstrap and rotation write v3;
  transfer authenticates/rebinds in the same owner mutation. Rotation CASes the
  loaded owner and auth hash. New regressions cover registration/legacy transfer,
  swaps, malformed current data, legacy nonce magic collisions and rotation
  held across transfer/successor rotation. Runtime remains pending.
- `ALLOW_LEGACY_RECORD_READS` is a trusted compatibility switch (default true),
  consumed by converted adapters. Setting false makes Node's absent-version
  path fail closed even if a database writer substitutes an old blob and its
  version marker. It does not retire unconverted fields, old providers, backups
  or rollback readers. Authenticated migration readiness and coordinated
  reader/writer drain remain required before a local cutover; no production
  cutover or migration has run.
- Node Debug redacts credential hashes/ciphertext. Current record-envelope and
  legacy rewrap changes authenticate before no-op and enforce provider bounds;
  equal-ID fallback rewrap cannot preserve a fallback-only dependency.

The parent compiler hold remains active during build5 runtime. Full personal
accepted-intent/worker integration, all lifecycle adapters, encrypted headers,
remaining contextual callers/migration and the OAuth acceptance matrix remain
open implementation work.

Build5 complete runtime checkpoint: parent executed 22 suites, **260 passed / 10
failed / 0 ignored**. Results `/tmp/nyxid-primary-build5-results.json`. The ten
failures were two refresh fixtures and eight OAuth handler fixtures with obsolete
unbound-authority setup/expectations. Subsequent source binds real original
consent revisions; explicit legacy denial stays. The stale incremental fixture
now distinguishes an untouched stored row from a superseded grant that must be
denied. Broader parent runtime additionally found six successful broker-exchange
fixtures and one auth-denial fixture needing current consent/client authority;
these are corrected in build6 source, with no pass claimed yet.

Build6 is the sole heavy compile, log
`/tmp/nyxid-security-current-harness-build-6.log`, manifest
`/tmp/nyxid-security-build6-source-sha256.txt`. Explicit `--tests` and per-crate
`-C debuginfo=0`; no second check/build. This snapshot includes the Node contextual
adapter/CAS/readiness tests, provider diagnostics, org fences and initial personal
erasure acceptance. The new accepted-intent service is not yet routed into HTTP
or backed by complete domain workers; it does not close personal erasure.

Build6 produced five URL-seam `&&str: IntoUrl` diagnostics and a Node transfer
HRTB/Send diagnostic; it was stopped after those diagnostics, **not passed**.
URL seams now pass borrowed str directly. Node transfer prepares its encrypted
successor before the transaction and compares the exact live source blob,
version, auth hash and signing hash inside the transaction before changing
owner/ciphertext. Any concurrent credential change rejects the prepared rebind.
Build7 is now the sole compiler, with the same explicit test/per-crate debug
settings; log `/tmp/nyxid-security-current-harness-build-7.log`, manifest
`/tmp/nyxid-security-build7-source-sha256.txt`. It is a development snapshot,
not final frozen-source acceptance. No Mongo runtime is started by the worker
while parent finishes repaired-instance runtime groups.

### Post-build8 source work (not compiled)

- Platform owner retirement/setup now use scoped primary majority evidence reads and nontransactional writes. Their lifecycle transactions and Oracle cleanup use snapshot reads and majority commits. Oracle personal-erasure scans read majority evidence. Command-monitoring tests model crash after effects/before scheduler checkpoint and independent retry; no real replica failover test claimed. No global concern/W8 audit change.
- Header storage uses distinct API input/output, storage-only ciphertext/version and zeroizing resolved representations. Exact masked edits decrypt only preserved fields; clear/explicit replacement work with corrupt old values. Canonical case-insensitive header identity, strict legacy-reader setting, exact BSON array/owner CAS and catalog ownership-transfer rebind drafted. Frozen v1/v2 hashing streams canonical JSON into SHA-256 and recursively clears owned temporary JSON. Header tests, actual injected Request Debug, URL-free real query/path transport failure tests drafted; runtime pending.
- Focused Debug candidate inventory converted to explicit redaction, preserving wire serialization: user/hash/reset models, encrypted credential models, OAuth/node/service-account secret-bearing DTOs, decrypted provider token, node proxy/SSH/terminal frames and stream/response envelopes. Representative distinctive-secret tests drafted. This is scoped formatting hardening; plaintext ownership propagation and remaining full-field adapters are still incomplete.
- Full personal-erasure worker adapters/routes/client accepted/pending/complete semantics remain completion-critical. CLI profile deletion, assistant human-confirmation action/dialog and admin detail toast are included with web/mobile consumers; authentication failure after acceptance is not completion evidence.

Post-build8 header validation checkpoint: build9 failed compilation (22 diagnostics), with no runtime claim. Its workspace fixes and the seeded-header opaque raw-array/owner CAS, reserved required-header capacity checks, typed admin twin header inheritance, contextual writer activation guard, extra Debug containment and scoped telemetry-erasure durability were frozen into build10 (`/tmp/nyxid-security-build10-source`, 1,933-file SHA256 manifest `/tmp/nyxid-security-build10-source-sha256.json`). Build10 compiled successfully in 11m01s (four unfinished-erasure integration warnings); its log is `/tmp/nyxid-security-current-harness-build-10.log`. No build10 runtime pass is claimed yet. Later provider-erasure adapter work is outside that immutable snapshot and remains unvalidated.

The personal-erasure provider adapter now has a draft encrypted effect journal that preserves exact token/app material before removing source secrets, uses token-scoped disconnect planning and bounded delivery, and does not count missing token outcomes as confirmation. Full source-writer fencing, shared-grant dependency proof, usable operator reconciliation, all remaining domain adapters and live routes/clients remain completion-critical. Raw BSON legacy-header source snapshots and queued node JSON owners remain plaintext-lifetime inventory items; parsed DTO wrappers alone do not close those boundaries.

Build10 harness was copied to `/tmp/nyxid-security-build10-server-tests`; SHA256 `84ccc234e674201d6232d0deb748925479c20eb22e3299d07c627f04618f3d5f`. The primary owns exclusive Mongo runtime for the 24-suite run. Initial runtime found a positive proxy fixture with a nonexistent catalog link (206/207 passed in that suite); source now seeds its real catalog and tests missing-link failure. This edit, provider reconciliation parent-fence/rotated-secret regressions, and bounded provenance preparation are outside frozen10 and unvalidated. No new compiler/runtime is started during primary ownership.


Build10 primary runtime completed: **24 suites, 1,178 passed / 3 failed / 2 intentionally ignored**, 921 seconds. Evidence: `/tmp/nyxid-primary-build10-results.json`, logs `00..23`. The strict concern test passed with the identical immutable harness and `directConnection=false` (`/tmp/nyxid-primary-build10-durability-discovery.log`); the ordinary local fixture's Single topology caused its primaryPreferred command. A dedicated discovery fixture is now drafted, preserving the strict primary assertion. The two remaining failures were the nonexistent catalog positive fixture and absent-to-null catalog header transfer shape; source corrections await validation. The ignored catalog/tools-manager CLI E2Es still require a matching frozen CLI and explicit execution. Build10 validates only its frozen bytes, not subsequent erasure/preparation adapters.

Post-build10 source: vendor personal checkpoints now bind a majority-snapshot proof of current registry revision, capture generation and per-destination receipts; status reads and bounded background revisits invalidate stale completion. Channel cleanup precedes provider revocation, including retained jobs with the earlier domain ordering. Machine evidence uses bound native/never-submitted contracts and bounded keyset pages of historical machines. These edits and their regressions are uncompiled/unexecuted. Full personal erasure integration, dependency-writer freshness/fences, all local/source adapters, routes/clients and full W4 contextual migration remain open; no completion, production migration or retirement is claimed.

Integrated erasure draft after build10 (uncompiled): acceptance now rejects
unsettled Ornn publications under the same owner fence as publication claim,
dispatch and attachment. Provider material remains usable on rejection; the
worker checks the same prerequisite before cleanup. Unknown/landed target locks
are retained independently of personal agent bodies. New group creation,
first-use NyxBot creation and saved-login writes now share the persistent owner
barrier. These changes, the issuance journal, domain adapters and client response
integration await the next immutable compiled checkpoint; build10 evidence does
not validate them.

Provider issuance journal routine receipts contain no token capability after a
confirmed local installation or explicit no-issuance protocol outcome. Those
receipts retain same-attempt response fingerprints for seven days, then expire
using a server-time TTL field; eligible routine receipts are also removed by
personal local cleanup. Unresolved dispatches, returned capabilities, operator
attestations and revocation exception receipts have no routine TTL. A real late
returned capability removes any routine expiry atomically and reopens an
existing erasure intent. The thirty-second upstream timeout bounds ordinary
response delivery; elapsed time never supplies no-issuance evidence.
