# Stream C batch log

## C-1 — immutable APT inputs
- Reviewed draft catalog/installer; all 12 HTTPS InRelease digest comparisons pass locally.
- Shared portable parser rejects missing requested roots, duplicate suites/roots, bad origins/hashes and mixed timestamps.
- All APT Dockerfile stages and Linux dependency wrappers use authenticated snapshots; images install downloaded archives offline.
- Preserve inherited package evidence by binding each request to its base installed inventory.
- PR CI calls APT Inputs, required by CI Pipeline; the identical workflow also runs weekly, with contents:read only.
- Hosted matrix: 4 distro roots × 2 native arches × 9 success/tamper cases; 12 Dockerfile package prefixes × 2 arches.
- Local verification: catalog/caller/harness tests 19/0; build-input tests 20/0; input, YAML and permission checks pass.
- Linux execution and final application-image behavior are pending PR CI; no local Docker or remote workflow execution.
- APK, final image packaging/SBOMs and mandatory signing remain C-2 through C-7.

## C-2 — immutable APK closures
- Reviewed/replaced APK drafts; six catalogs bind 152 signed archives to exact bases, architectures and before/after inventories.
- Frontend runtime and updater builder/E2E use checked downloads plus physical RUN --network=none installs; no mutable APK resolution in builds.
- Shared portable parser and build-input admission require complete architecture/stage coverage and exact requested packages.
- Candidate generator isolates target package state, disables scripts/self-reexecution and checks signed archive identities; bounded VM resolution only.
- PR CI requires APK Inputs; identical contents:read-only workflow runs weekly, without secrets, OIDC or publication.
- Hosted matrix: 3 closures × 2 native arches × 14 success/tamper cases; 3 real Dockerfile package prefixes × 2 arches.
- Local verification: APK tests 35/0; build-input tests 20/0; APT tests 19/0; YAML tests 6/0; policy, syntax and permission checks pass.
- Native replay, offline Dockerfile execution and final application-image behavior remain pending PR CI; no local image builds.
- C-3 retains weekly candidate-refresh/stale-catalog gates and the HTTPS-catalog/installer-only HTTP transport regression.

## C-1 follow-up — restore Detect Changes helper tests
- Root cause: legacy sudo/apt-get stub rejected the snapshot-installer delegation with exit 99.
- Preserve all four original cases using the real helper and installer with isolated filesystem/command fixtures; add download-failure coverage.
- Detect Changes exact discovery command: all four helper modules, 22 passed/0 failed.
- C-1/C-2 local suites: build inputs 20/0, APT 19/0, APK 35/0, workflow YAML 6/0; policy/YAML/permission checks pass.
- Production helper/installer behavior unchanged; hosted Detect Changes and downstream APT/APK evidence pending the next PR run.

## C-2 revision — archive layers and durable storage
- Separate pinned download stages supply read-only BuildKit mounts; installers write receipts elsewhere and retain no archive layer.
- Policy rejects same-stage download/install, writable mounts, downloader inheritance/copies and non-durable defaults.
- All-layer Docker-save inspection rejects archive payloads even when a later layer deletes them; native replay mounts inputs read-only.
- Catalog v2 adds repository-owned release URLs and preserves all 152 existing hashes/identities/provenance URLs/inventory bindings.
- Normal builds/schedules use durable assets only; explicit PR-only HTTP-404 fallback logs upstream use with unchanged hash/signature/identity checks.
- Maintainer-dispatched provisioning verifies native payloads in read-only jobs; only its trusted publisher has contents:write, with no clobber/secrets/OIDC.
- PR closure jobs rehearse exact release payload staging; Machine Container E2E receives explicitly built updater images with event-scoped source selection.
- Local suites: Detect Changes 22/0, build inputs 20/0, APT 19/0, APK callers/catalogs 43/0, archive/source/publisher/layers 27/0, workflow YAML 6/0 (137/0 total).
- Input, shell syntax, strict workflow YAML and reusable-permission checks pass; native replay/image-layer execution remains pending PR CI.
- Initial durable provisioning is an explicit maintainer prerequisite for strict schedules/publication; no remote operation or local image build was run.

## C-2 R2b — keep verified builds available before provisioning
- All builds prefer durable archives and permit only HTTP-404 fallback to recorded upstream bytes, with a release-tagged ::warning::.
- Checksums, Alpine signatures and signed identities remain mandatory; corrupt durable bytes, non-404 errors and foreign redirects reject without fallback.
- GitHub's HTTPS release-asset CDN is explicitly admitted; HTTP, credentials and hostname lookalikes reject before acceptance/fallback.
- Separate APK durable archives present job checks release/catalog/archive completeness and SHA-256 metadata: non-blocking on PRs, failing on schedule.
- Native replay adds fallback success, bad-signature and wrong-signed-identity cases for each stage/arch (102 total hosted cases).
- Local tests: Detect Changes 22/0, build pins 20/0, APT 19/0, APK inputs 43/0, archive/durability 33/0, workflow YAML 6/0 (143/0 total).
- Input, shell syntax, workflow YAML and reusable-permission checks pass; native fallback execution remains pending PR CI.
- Provisioning is documented as a post-merge durability step; no push, remote workflow or local image build was performed.

## C-1/C-2 hosted follow-up — transport trust and preflight completeness
- Hosted root causes: Ubuntu HTTP snapshots redirect to HTTPS without base CA trust; APK simulation accepts some incomplete closures.
- Vendor the exact public Mozilla/curl bundle, SHA-check it before APT, and require CaInfo plus TLS peer/host verification for every caller.
- All APT Dockerfile stages copy the bundle; CI/release wrappers use the same installer. System trust stores are not modified by bootstrap.
- APT Dockerfile Layers adds direct minimal-Ubuntu coverage without a system CA bundle before/after installation, on both native arches.
- APK preflight replays into an isolated database/keyring root without scripts/network/repositories/cache and requires the catalog inventory before live mutation.
- Native replay requires unchanged DB bytes for incomplete closures/wrong final catalogs; separate post-install drift still exercises the final guard.
- Local tests: Detect Changes 24/0, build pins 20/0, APT 21/0, APK inputs 43/0, scratch preflight 5/0, archive/durability 33/0, YAML 6/0 (152/0 total).
- Input, shell syntax, YAML/permission checks pass. Native replay is 18 cases × 3 closures × 2 arches; PR acceptance pending.
- No local image build, remote workflow or push; R2b already committed at 9514f3827 for the same next push.

## C-3 — weekly catalog maintenance and transport-CA provenance
- Pin Mozilla/curl bundle provenance, exact HTTPS sources, source/check dates and SHA-256 in one portable installer manifest.
- All APT stages carry the manifest/parser; invalid/missing provenance rejects before update; HTTPS-only snapshot catalogs remain required.
- Read-only PR maintenance resolves/replays APT candidates on four bases/two native arches and APK closures on both native arches.
- Main-only weekly publisher admits exactly catalog/CA files and checked APK payloads, opens a review PR and refuses branch/asset replacement.
- Only the scheduled publisher has write scopes; candidate/freshness checks are required by CI Pipeline, with no PR secrets or OIDC.
- Freshness rejects distro/CA-check age >14 days and CA-source age >90 days; recovery candidate generation is independent of stale-input failures.
- Dependabot adds hash-locked Python coverage; Renovate proposes standalone/tool/CUA/CMA pin updates weekly (admin App activation remains external).
- Hosted CMA patches require all three reviewed byte identities before ordered execution of private temporary copies; final ACP identity still required.
- Local: helpers 26/0, build 20/0, APT 21/0, APK 43/0, preflight 5/0, archives 33/0, catalogs 29/0, hosted 11/0, workflow YAML 8/0 (196/0).
- Policies, freshness, workflow YAML/permissions and syntax pass; native replay/image evidence awaits PR CI. No push or remote workflow run.
- Hosted npm lock root/wiring still awaits PM's explicit manifest approval; D's real Docker CLI/separation coverage is assigned to C-4.

## C-3 completion — approved hosted npm lock and required native checks
- PM-approved hosted root preserves Codex 0.147.0 / ACP 1.4.0; 24 registry packages have SHA-512 integrity and zero install hooks.
- Focused manifest commit b3d7f7ad7 adds the lock, ignore-scripts config, script-policy root and weekly Dependabot coverage.
- A separate pinned public hosted-tools Dockerfile stage uses the checked npm bootstrap and npm ci --ignore-scripts, with existing output hashes.
- Both consuming stages retain the complete locked node_modules tree, including hoisted transitive dependencies.
- Policy negatives reject global/unlocked installs, shell-substitution bypasses, hook execution/approvals, unbound archives, lock drift and partial runtime copies.
- Required Hosted Locked Tools PR/weekly jobs build that exact target on amd64/arm64 and check unchanged locks, hook absence and offline CLI startup.
- BUILD_SECURITY explicitly lists Renovate activation/private-CMA access, post-merge APK dispatch, Actions publication prerequisites and effects while absent.
- Local suites: helpers 26/0, build 20/0, APT 21/0, APK 43/0, preflight 5/0, archives 33/0, catalogs 29/0, hosted 20/0, YAML 9/0, Node policy 4/0 (210/0).
- Exact committed root clean-installs with scripts disabled and no lock drift; policies, syntax and workflow/permission checks pass.
- No local Docker/application build, push or remote workflow; hosted tool/maintenance/image acceptance remains pending PR CI.
- C-4 will propose D's local registration fixture, separation flags, credential non-creation assertions, restart persistence and probe cleanup contract.

## C-4a — backend-image credential packaging
- Replaced the cache-only backend image smoke with required native amd64/arm64 backend+frontend runtime builds and extraction/parity checks; weekly uses the same workflow.
- Extract from a resolved image ID without startup; reject archive traversal, duplicate members, links and special files; remove stopped containers and anonymous volumes.
- Require exact credential file closure, manifest hashes/sizes/SRI, recomputed fingerprint, HTML script metadata, source commit/version/epoch and composition/lock binding.
- Publication repeats extraction/parity on actual backend/frontend image digests before multi-architecture tag promotion; workflow negatives prevent dropping required gates.
- Local: 148 tests pass (credential21, wiring5, CI helpers26, inputs20, image caller3, workflow9, APT21, APK43); policies/YAML/permissions/syntax pass.
- A fresh locked scratch npm install and actual credential build pass; all five output files validate. Shared dependency symlinks were unsuitable for source attribution and were replaced only in scratch.
- Docker extraction and native image builds require Artifact Packaging Policy and Backend Credential Packaging (amd64/arm64) on the PR.
- Signature verification remains C-7; this check binds the actual credential manifest bytes that signing must consume.

## C-4b — final application source identity
- All application runtime images carry full revision/epoch labels; mcp-demo retains the shared versioned identity section after stripping.
- Frontend emits strict commit/epoch metadata; credential manifests share the same validated source identity and deterministic timestamp.
- Actual extracted executables must match package/version/commit/epoch/exported state and native architecture/ABI; scratch updater must be static.
- Actual frontend assets require matching metadata, initial assets, exact composition closure/hashes and committed source-lock binding.
- Required read-only PR/weekly jobs build eight public image subjects on both arches (backend/frontend together), plus four actual stripped CLI archives.
- Public image publication repeats final identity inspection before attestation/promotion; CLI release checks the packaged binary before upload.
- Local suites: identity16, credential21, helpers26, pins20, image caller3, APT21, APK43, hosted20, wiring10, YAML9, source7, inspector2, archives5, dist6, Node4 (213/0).
- Actual locked scratch frontend/credential builds and final-file verification pass; actual mcp-demo build/strip retains git-dirty local identity; no official artifact claim.
- YAML/permissions/input policies, Python syntax, Rust 2024 formatting and diff checks pass; no local Docker, backend suite, push or remote workflow execution.
- Private hosted final-image checker is ready; no private hosted publisher exists in this checkout or workflow inventory. Caller identification/integration remains pending PM.
- C-4c built-image worklet/CSP and C-4d real Docker CLI paths remain next; private CMA patch bytes and other streams' paths are untouched.

## C-4b follow-up — mandatory operator image gate
- Hosted deployment uses the local operator wrapper; all reviewed application targets now build untagged and verify their immutable image ID before tagging.
- Build the locked inspector from the exact source export; missing/failed checkers or inspectors, invalid IDs and source mutation fail closed with no new tag.
- Existing execution receipts record final status, checked image/component/platform, verification command/result and tag outcome; unknown stages reject.
- Backend operator verification includes actual credential-page extraction; the explicit updater e2e target remains test-only.
- PR/weekly hosted-desktop jobs exercise the real wrapper and receipt/tag-ID binding on native amd64/arm64 without private CMA inputs.
- Local: wrapper15, wiring11, identity16, credential21, exported runner7, CI helpers26, input policy20, YAML9 (125 passed/0 failed).
- Workflow YAML/permissions/input policies, AST syntax and diff checks pass; Docker/private final-image execution remains hosted/operator evidence.
- Documented mandatory passing receipt and registry digest provenance; raw Docker builds are unsupported for hosted deployment.

## C-4c — actual frontend-image worklet/CSP
- Required PR/weekly Built Image Worklet CSP jobs run on native amd64/arm64 with actual final frontend images and digest-pinned Playwright 1.62.0 browsers.
- Inherit the packaged default entrypoint/Cmd; runtime has only writable tmpfs paths, with no injected nginx config, application assets or renderer.
- Safe extraction and final frontend identity/seal checks bind the tested assets to the immutable image; HTTP checks require exact HTML/hashed-worklet bytes and enforcing microphone/CSP headers.
- Reuse the two existing real microphone/worklet/font/WebSocket and CSP negative cases; require exact case names, one pass each, no skips/retries/errors.
- Locked browser dependencies install without hooks; version/architecture resolution is recorded in existing input pins and covered by weekly frontend Docker/npm updates.
- Local: harness11, wiring12, identity16, credential21, CI helpers26, input policy20, YAML9 (115 passed/0 failed); isolated Playwright lists exactly two cases.
- Initial listing mixed scratch/shared Playwright modules; rerunning wholly within the locked scratch tree passed without repository dependency changes.
- Workflow YAML/permissions/input policies, syntax and diff checks pass; actual Docker/browser execution requires Built Image Worklet CSP (amd64/arm64) in PR CI.

## C-4d — real machine Docker CLI paths
- Add required read-only PR/weekly Machine Docker CLI E2E, building actual desktop/node-only images and extracting each real CLI.
- Browserless node-only setup deterministically tests omitted/automatic/require-separated refusal before local state, any fixture request or registration; no persistent/probe resources may remain.
- Measure desktop's real versioned Linux capability evidence without assuming the hosted kernel supports separation.
- Both images explicitly register shared-legacy once over ephemeral verified WSS, advertise live capabilities/heartbeats, and preserve encrypted custody/choice across restart and stop/start.
- Existing fixed official image refs, CA forwarding, unique profiles and D's self-hosted/separation contract are used; no production image override or D-owned source edit.
- Transparent Docker argv recording execs the real client unchanged; probe credential environments/mounts and leftover anonymous volumes fail acceptance.
- Use existing locked ws transport without hooks; fixture metadata excludes secrets, and startup failure diagnostics redact synthetic credentials/signing bytes.
- Local: CLI harness11, WSS7, wiring13, credential21, identity16, wrapper15, CSP11, Node identity4, CI helpers26, pins20, YAML9 (153 passed/0 failed).
- All Artifact Packaging policy commands, workflow YAML/permissions/input policies, Python/Node syntax and diff checks pass.
- Actual engine/image execution requires Machine Docker CLI E2E on the PR; no local Docker, Rust/backend build, push, tag or remote workflow run.

## Urgent CI repair — efa50f512 / run 39355352247
- Read the completed failing job logs: APT CA path traversal, root-owned replay downloads, APK 2 scratch cache layout, arm64 container Git ownership and Intel runner SDK.
- Copy the checksum-verified CA into the installer's 0755 work directory as 0644, recheck it, and retain mandatory TLS/signature/hash checks for all callers.
- Add real root/_apt acquisition below a 0700 input parent, signed metadata/native-index checks and unchanged host dpkg state; both PR APT layer jobs require it.
- Minimal Ubuntu Docker regression also hides original CA inputs behind 0700; no system trust-store injection or verification bypass.
- Return only writable replay downloads to the host UID/GID, including failures; never chown read-only installs. APK 2 scratch roots get an empty var/cache/apk.
- Resolve the initial archive commit through isolated exact-path Git trust; both Apple targets use the current runner, with an SDK >=26 gate matching locked Metal APIs.
- Local: helpers28, APT21, APK84, source16, packaging14, pins20, YAML9, catalogs29, exported-runner7, dist6 = 234 passed / 0 failed.
- Input/YAML/reusable-permission policies, Python/shell syntax and focused diff checks pass; original update/install failure cases remain required.
- Real Linux sandbox and Docker/image/archive execution require PR CI; local Lima access awaits PM coordination with D. No local application/backend build.
- Missing APK durable release confirmed loud but non-blocking on PRs; scheduled durability remains required. Provisioning is still a maintainer step.
- Commit excludes all C-5 scanner drafts and other streams' files; no push, merge, secrets, tags or remote workflow execution.

## C-5 — required discovery, exact dispositions and final-image scans
- Add read-only PR/weekly history/dependency scanners required by CI Pipeline and release publication; no inline, baseline, inherited-config or blanket ignores.
- Seven PM-proposed semantic placeholders replace only the owned lines. Exact rule/path/match/value/source-line dispositions include owner, expiry, rationale and provenance; all 123 entries have identity negatives.
- Real-detector negatives cover Firebase same-file new key, changed bytes/path/rule, expiry, policy metadata and deleted branch history; no Firebase key use/probe.
- Audit all 12 locks and every manifest/workspace, including machine/updater and independent tools/test roots; local result: 10 clean, Cargo 12 advisory occurrences, mobile 6.
- Scan actual runtime images on both native PR/weekly architectures, require OS/Rust binary inventories, block HIGH/CRITICAL/UNKNOWN, and require fresh APT/APK/CA catalogs.
- Publication waits for source scans, scans before the first registry write and copies only the passing staged OCI digest; operator wrapper fails before tagging on missing/failed scans.
- D's final contract now asserts exact volumes, retained container ID/state, explicit choice-change refusal, unchanged custody and hidden/dangling profile absence; no D-owned edit.
- Local: secret14, dependency7, image14, wiring6, wrapper18, catalogs29, pins20, helpers28, packaging14, YAML9, machine14, mobile8, WSS7 = 188 passed / 0 failed.
- Real Lima root/_apt regression on committed 8589cfd88: 1 passed / 0 failed in a disposable pinned Ubuntu container; guest dpkg hash unchanged and no container left.
- Full committed candidate with all 566 source refs: history215 + tree111, reviewed316, unresolved10 (six historical documentation values); gate correctly fails without provenance exceptions.
- Remaining dependency/history findings stay blocking. No manifest/lock edit, confirmed operational credential, rotation, local application build, push or remote workflow execution.
- PR evidence: Scanner Policy, Full History Secret Scan, Every Dependency Root Audit, Vulnerability Scan Catalog Freshness, Artifact Packaging Policy/image matrix, Machine Docker CLI E2E.

## C-5 approved Cargo remediation
- PM approved one focused Cargo commit: crossbeam-epoch 0.9.21, h2 0.4.20, self_update 1.3.1 with renamed gzip feature, backend zip 9.0.5.
- Retain tar/ZIP/TLS and backend deflate features; remove obsolete quick-xml 0.38 and zip 6 paths.
- zip 9 requires nix 0.31.3, which requires newer libc; resolver-only downgrade probes correctly reject old versions.
- Final PM D3: remove unused Swagger UI/zip 3 and migrate machine bootstrap RSA to AWS-LC; braces and remaining RSA uses receive separately verified dispositions.
- Validation: precise resolver updates pass; requested locked workspace check with both KMS features follows this commit using the exact shared-target environment.
- Fold the final Cargo decisions into the same focused commit before another stream commits; all private bootstrap RSA operations use AWS-LC with canonical/bounded DER and Zeroizing buffers.
- Preserve existing wire fixtures and add explicit RustCrypto-to-AWS-LC interop; machine rsa is test-only. No production private RustCrypto RSA remains in bootstrap.

## C-5 D1 — resolved historical secret decisions
- Add eight exact history-only finding identities covering ten findings / six value identities; five fabricated values use structural/provenance evidence.
- The single inert device-token identity requires exact code evidence: no redemption lookup, 24-hour hash-row TTL, and the introducing unshipped-feature statement.
- Annual history entries bind immutable commits and cannot cover current-tree/reintroduced copies; current-tree cadence remains monthly.
- Both device references explicitly state no endpoint currently redeems the refresh token; no token format/revocation mechanism changes.
- Local: real-detector policy17/17; full history215 + committed tree111, reviewed326, unresolved0; no credential probed.
- Required PR jobs: Scanner Policy; Full History Secret Scan.

## C-5 scoped YAML parser compatibility
- Add only js-yaml@3>argparse 2.0.1; remove argparse 1 / sprintf-js from the frozen mobile graph, retaining the legacy js-yaml 3 CLI API.
- Commit executable tests resolving the actual React Native → babel-jest → Istanbul → js-yaml 3 consumer, including NYC YAML config loading.
- Local: locked pnpm10.34.7 clean frozen install without hooks passes; actual consumer/CLI compatibility5/5.
- Full production bundle proof is implemented separately; a pre-existing undeclared Babel preset requires the pending per-change PM manifest approval.
- Required PR jobs: Mobile; Every Dependency Root Audit.

## C-5 dependency decisions — executable dispositions
- Implement exact monthly patch records for decode-uri-component, image-size and node-forge; retain complete raw audits.
- Prove all 119 installed source files against authenticated npm archives plus reviewed patches; 8 patched regressions and 4 vulnerable-original probes passed.
- Add monthly braces graph/archive/lock bindings and a real production iOS export proving no braces/micromatch modules; exact direct Babel preset 55.0.16 is committed as c55934a62 (minimal importer change).
- Bind 90-day RSA decisions to exact versions/checksums, Linux/KMS features, parent chains and reviewed source; new private-operation/import/context changes fail.
- Recommend Ed25519 for node SSH keys; rewrite remediation/scanner docs around the final PM decisions.
- Local: patch policy 21/21, use policy 12/12, audit 7/7, workflow 7/7, bundle negatives 5/5, secret policy 17/17.
- AWS-LC bootstrap 16/16 passed under the exact shared-target Cargo environment.
- ZIP fixes 53a10a33c/eb6251219/8aaf4616c cover all three fallible-name sites; locked workspace/all-targets/both-KMS check passed (exit 0), isolated production-function regressions 2/2; actual Backend Test execution remains PR evidence.
- Clean committed-source audit: all 12 roots passed (8 npm/3 Cargo/1 pnpm); 7 raw findings retained, 7 exact reviewed, 0 unresolved; bundle 1 map/3691 modules/0 forbidden.
- Final scan: 215 history + 112 tree, 327 reviewed, 0 unresolved; two new progress-prose identities receive exact monthly/current or annual/immutable dispositions; build-input policy passed.
- Reviewed committed A/B changes through 7c05214fb: retain 9723cedf9's test_utils binding; add only AES-GCM custody and Ed25519 witness-test operation entries, with no RSA-reference change; Linux/KMS graph/source checks passed.
- Required PR evidence: Scanner Policy, Full History Secret Scan, Every Dependency Root Audit, Vulnerability Scan Catalog Freshness, Mobile, Backend Test, CLI Test, Rust Features/KMS and Artifact Packaging identity/image scans.
