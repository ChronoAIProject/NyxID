# Stream C batch log

> Bundle note: this is the W5, W6, W7 and W9.1–W9.5 subset of this log at
> `e68e4276e`. W8 and W9.6 entries remain with draft PR #1944. Entry statuses
> are historical; current bundle verification is recorded in its PR.

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

## Hosted CI repair — run 39362062126
- Serve a distinct CA-signed CA:false/serverAuth leaf with loopback/gateway SANs; trust only the disposable CA; Node/Python use the same generator.
- Complete the isolated Playwright runtime import closure; fixture imports cannot write Python bytecode into authenticated source exports.
- Publication-plan identity reuses exact-checkout isolated Git trust; the Focal rustup HOME diagnostic was nonfatal, while bare Git was the blocker.
- Derived APT prefix tests consume the real desktop package prefix inside the same BuildKit build, without resolving an engine-local tag remotely.
- Local commands: test-machine-docker-cli.py17/17; node --test scripts/test-machine-docker-fixture.mjs7/7; test-built-image-csp.py13/13; isolated Playwright --list2/2 required cases.
- Local commands: test-apt-inputs.py22/22; test_publication_contract.py11/11; test-export-build-source.py16/16; test-build-inputs.py20/20.
- Local commands: .github/scripts unittest discovery28/28; test-packaging-workflows.py14/14; test-workflow-yaml.py9/9; check-build-inputs.py and diff --check passed.
- Total: 157 unit/policy/protocol tests passed, 0 final failures; Docker execution remains hosted evidence. No Cargo build or D harness/probe changes.
- APK durable-present is PR continue-on-error and excluded from apk-pass; CI Pipeline remains gated on APK policy/replay/layers.
- Required PR evidence: Artifact Packaging Policy; Machine Docker CLI E2E; Built Image Worklet CSP (both); CLI Archive Identity (aarch64 GNU); APT Policy/Dockerfile Layers (both); Detect Changes.

## C-6 — SBOMs from shipped artifacts
- Catalog the exact staged OCI layout with pinned Syft; validate raw manifest/config/platform/squashed scope and complete converted package/file/distro inventory.
- Extract each final binary and frontend/credential assets from the immutable image; bind compiled metadata, ABI, source identity, embedded wizard and fresh locked JS/CSS membership.
- Bind archive SBOMs to actual archive/member digests; publication replay reopens every final archive and checks all planned release-file digests.
- Validate exact amd64/arm64 platform composition and all ten platforms/five indices; reject extra/duplicate/missing subjects; keep auxiliary evidence outside the publication set.
- Require verified SBOMs before platform copies, index writes, tag promotion and release hosting; attach OCI referrers/release assets only in publishers; operator wrapper gates tags and records results.
- PR/weekly jobs validate and retain evidence with contents:read; no secrets/OIDC/registry writes. Native inspection avoids Python schema tools in the Focal builder.
- Local commands: test-sbom-evidence.py20/20 (real pinned Syft/regctl OCI/referrer replay); test-sbom-workflows.py3/3 (mutation matrix); test-build-local-image.py19/19; test-image-vulnerabilities.py14/14.
- Local commands: test_rust_binary_sbom.py7/7; test_dist_artifacts.py5/5; test_oci_subjects.py8/8; test_publication_contract.py11/11; test_cyclonedx.py3/3.
- Regressions: packaging14/14, scanners7/7, Detect Changes28/28, credentials21/21, source16/16, build inputs20/20, Node policy4/4, YAML9/9; total209 passed, 0 final failures.
- Standalone workflow YAML (14 workflows/22 Rust actions), reusable permissions (8 calls), build-input/Node policy and diff checks passed; real fresh locked credential composition10 components and wizard membership passed.
- Required PR jobs: Shipped SBOM Policy; Image SBOM Publication Closure; CLI Archive SBOM Composition; Backend Credential Packaging and Shipped Image Identity (both arches); CLI Archive Identity (all four targets).
- Actual image/archive acceptance remains hosted PR evidence; no local Cargo/Docker build or remote publication. C-7 mandatory signing is still pending; D native acceptance workflow awaits its PM-forwarded harness contract.


## Hosted CI repairs — C-5/C-6 integration
- Replace whole-lock advisory bindings with canonical affected package/consumer subgraphs (exact registry versions/checksums and edges); workspace release versions and unrelated entries are excluded.
- All six graphs match the accepted C-5 revision a0ca2a97c; no reviewed scope, advisory, expiry or source/patch proof was broadened. Full lock hashes still reject install/export/audit mutation.
- Scanner log's synthetic npm finding and missing-tool messages were passing negative controls, not new advisories/bootstrap failures; capture expected operator-fixture diagnostics while preserving all refusals.
- Real regctl containerd dual-view replay reproduces the image failure; select the unique staged alias, then require its digest, config, platform and every blob hash. Add PR tag/staging mutation tests.
- Downloaded APT evidence proves timeout in archive 508 of the desktop prefix, not Rust/private builds. Use direct verified HTTPS and reuse identical desktop/Chromium prefixes in the fresh job-local builder; no external cache or timeout increase.
- APT still tests every real package instruction and offline receipt; log elapsed times/tails within the unchanged 2400-second cold-prefix bound. Tamper proxy downgrades only its disposable wrapper's isolated sources.
- Local: disposition14/14, patch22/22, APT23/23, real regctl staging3/3, operator19/19; image14/14, Detect Changes29/29, secrets17/17 and scanner wiring7/7 (148 total).
- PR jobs: Security Scans / Scanner Policy, Every Dependency Root Audit; Artifact Packaging / Shipped Image Identity (all components/arches), Backend Credential Packaging; APT Policy, APT Snapshots and APT Dockerfile Layers (both arches); Detect Changes and CI Pipeline.
- APK durable archives present remains outside apk-pass/CI required dependencies and continue-on-error on PRs; missing provisioning still fails its scheduled job.
- No local Docker or Linux replay claimed; final image, signed snapshot/tamper and timing acceptance must come from these PR jobs. No remote mutation performed.

## C-7 — mandatory official signing and distinct OIDC rehearsal
- Derive signed official manifests from the complete final-byte scan/SBOM closure; reverify before CLI hosting/upload, platform/index publication and tag promotion.
- Require protected official-release authority and exact configuration; missing/failed signing, verifier, trust metadata or subject/provenance binding blocks publication.
- PR/weekly nonpublishing OIDC rehearsal must pass exact W6 cryptography, then fail specifically under each production workflow identity; no production trust source changed.
- Approved producer lock: sign/bundle 3.0.0, core 3.2.1, protobuf 0.3.2; 91 SHA-512 packages, zero hooks, pinned npm ci --ignore-scripts, npm audit 0 vulnerabilities.
- Local scripts/JS: 187 passed, 0 failed across signing, publication, dependency-root, scanner, packaging, SBOM, build-input, YAML and CI-helper suites.
- Shared-target cargo test --locked -p nyxid-cli --example release-verifier: 58 passed, 0 failed, 1 existing live-mirror ignored; locked example build passed.
- Shared-target cargo clippy --locked -p nyxid-cli --all-targets -- -D warnings passed; all required shared build settings used.
- Actual pinned cargo-dist plan exposes only nyxid in executable archives; metadata marks verifier as example, and publication negatives reject its addition.
- Workflow YAML, permission ceilings, pin policy and diff checks passed; signing root is automatically audited and receives weekly Dependabot updates.
- PR evidence: Release Signing Policy / W6 Signing Compatibility (amd64/arm64); Signing Rehearsal / OIDC Rehearsal and Production Rejection; Every Dependency Root Audit; Rust Clippy; CI Pipeline.
- RELEASE_SIGNING.md specifies admin rulesets/environment/reviewers/variable/OIDC permissions and public-trust prerequisites; no remote provisioning or publication performed.

## W9.4 D-5 — required hosted native and full-image acceptance wiring
- Require Machine Native Acceptance / Native Machine Separation (amd64) on every PR and in CI Pipeline; repeat weekly without secrets or publication permissions.
- Invoke D's unchanged root/disposable harness with locked CLI/context-spike builds, pinned CUA, 100 samples and full native/adversarial checks.
- Fail with kernel/LSM/AppArmor diagnostics unless native Linux amd64 root has Landlock ABI >= 6 and real AppArmor userns restrictions; never skip or relax policy.
- Snapshot-install native display/venv/NSS dependencies and /usr/bin offline toolchain; native Google Chrome and actual image Debian Chromium cover both managed policy layouts.
- Machine Container E2E additionally runs D's 100-sample probe with --init, 512m shm and committed seccomp; no token env or explicit state/workspace mounts; assert container/anonymous-volume cleanup.
- Preserve fixed-label per-sample timing, first observation, p99/max and review bounds as machine-native-amd64-probe-timings and machine-container-amd64-probe-timings artifacts.
- Local commands: test-machine-acceptance-ci.py 9/9; test-packaging-workflows.py 14/14; test-signing-workflows.py 5/5; CI helper discover 29/29; test-workflow-yaml.py 9/9; test-build-inputs.py 20/20 (86 passed, 0 failed).
- Actual pinned amd64 CUA archive SHA/size and safe complete driver/SDK extraction passed; shell/Python syntax, workflow mapping/permissions, pin policy and diff checks passed.
- Native root/browser and Docker execution remain hosted-only evidence; no local destructive acceptance or image builds attempted. D's 16/16 native and 44/44 adversarial results are separate PM-provided evidence.
- PR jobs: Machine Native Acceptance / Native Machine Separation (amd64), Machine Container E2E, existing Artifact Packaging / Machine Docker CLI E2E, Workflow Permissions, Detect Changes, CI Pipeline.

## W7 release readiness checklist
- Consolidated 13 admin/maintainer prerequisites in RELEASE_SIGNING.md; each states missing-setup effects and completion evidence.
- Covers APK durability, Renovate/Dependabot/private CMA, scheduled refresh and bot-PR CI, protected signing custody/OIDC/permissions, expiry reviews and required hosted acceptance.
- Includes conditional private hosted-image receipts/digest pinning and independent credential-manifest hosting; distinguishes availability gaps from publication blockers.
- Local documentation audit: 13/13 entries have effects and verification, 4/4 relative links resolve, required workflow/input/environment/scope names match implemented gates; git diff --check passed.
- Documentation only; no new tests or hosted jobs required. W7 source acceptance remains conditional on the existing hosted jobs turning green.

## W7 hosted follow-up — GitHub workflow semantic validation
- Reproduced CI's zero-job failure with checksum-verified actionlint 1.7.12: runner context is invalid in job-level env; initialize the target through GITHUB_ENV in the first native step.
- Pin official actionlint archives/SHA-256 for both macOS/Linux architectures, use the existing verified bootstrap and add weekly Renovate coverage.
- Mandatory semantic validation runs in Detect Changes and Workflow Permissions; real negative fixtures cover context, expressions, needs, local calls, input/secret declarations and permission syntax.
- Local: exact CI-helper discovery 45/45; workflow policy 10/10; all 17 workflows and 14 permission call edges pass; default actionlint also passes.
- All 17 pinned-input/publication checks plus native/packaging/signing/SBOM/scanner workflow suites pass: 22 commands, 163 tests; 218 tests total, 0 failures; diff check passes.
- GitHub annotations show Release and Release Build Cache did create jobs: remaining secret/dependency audit and arm64 cache runtime failures are separate from CI's invalid-workflow blocker.
- PR jobs: Detect Changes, Workflow Permissions, Machine Native Acceptance / Native Machine Separation (amd64), CI Pipeline; no push, workflow dispatch or remote provisioning.

## W7 hosted follow-up — Release scanners and arm64 cache
- Read Release 39397124174 and Release Build Cache 39397124126 job logs; the reusable PR/Release scanner invocation and full-history checkout are identical.
- Add one exact monthly non-secret prose disposition for the W8 inventory's slash-separated writer-state words; same-line new-key and all existing rule/path/content/expiry negatives remain blocking.
- Review only four newly committed AES-GCM decrypt lines for the conservative RSA guard; retain all exact RSA/SSH source, graph, feature, advisory and expiry bindings; report changed paths on failure.
- Bind whole canonical Cargo manifests apart from application package release numbers, fixing the v0.72.0-to-v0.73.0 drift; missing/moved bindings, JWT provider and RSA dependency changes fail tests.
- Install missing jq through the existing signed APT snapshots in root and sudo release/cache paths; test update, download, offline install and all upgrade failures with the actual helper.
- Require the shared dependency step before release/cache builds and select real cache PR builds when its installer or APT inputs change; regression tests reject omissions and late/conditional wiring.
- Local: CI helper discovery 48/48; secret policy 18/18; dependency dispositions 17/17, patches 22/22, audits 7/7; scanner workflows 7/7, packaging 15/15.
- Local: build inputs 20/20, APT inputs 23/23, APK inputs 43/43, workflow YAML 10/10; total 230 Python tests passed, 0 failures; actionlint 1.7.12, 17 workflow mappings, 14 permission edges and diff checks pass.
- Actual scan: 216 history + 112 committed-tree findings, 328 exact reviewed, 0 unresolved; actual isolated committed-source audit: 13/13 roots passed, 7 registry findings, 7 reviewed, 0 unresolved.
- Audit also passed 8/8 mobile patch regressions, 5/5 compatibility cases, 119 installed files, four vulnerable-original proofs and a 3,691-module app export with zero forbidden modules.
- Local Python 3.9 uses a temporary verified Tomli 2.3.0 compatibility alias for tomllib; hosted Python 3.12 remains authoritative. No Cargo source, manifest, lock or other stream's document edited.
- PR jobs: Detect Changes; Workflow Permissions; Security Scans / Scanner Policy, Full History Secret Scan, Every Dependency Root Audit, Security Scans Passed; Release Build Cache / warm (aarch64-unknown-linux-gnu); CI Pipeline. Real Linux cache build remains hosted-only.

## W7 CI82 follow-up — audit guard, update coverage, toolchain and durability
- RSA refresh reads only immutable committed HEAD blobs, prints changed path/hash/count bindings, and rewrites only rsa_guard; dirty/index/untracked sources are excluded by regression.
- Bind the high-churn test_utils fixture to counted RSA-reference lines; retain every private-operation line and production RSA context; review f3d78472f's new AES-GCM recovery decrypt alongside PM-reviewed MFA/Mongo edits.
- Add the already-managed actionlint pin to the exact Renovate coverage expectation.
- Artifact Packaging explicitly requests cargo/rustc/rust-std as well as Clippy/rustfmt, repairing partial existing toolchains rather than relying on profile defaults; missing-component workflow negatives added.
- APK availability absence/incompleteness warns on PRs, fails schedules/other direct invocations, and names the exact post-merge provisioning dispatch; API/network errors and all archive verification remain fatal.
- Local: dispositions 19/19, catalog maintenance 29/29, packaging workflows 16/16, APK archives 35/35, scanner workflows 7/7: 106 tests passed, 0 failures; build-input policy and pinned actionlint over all workflows passed.
- Committed-source guard matches f3d78472fe13cd8a5a36a8bd6f165d0be358dc06; no dirty source used. Python3.9 local tests reuse the existing verified Tomli compatibility alias.
- Hosted evidence: Scanner Policy, Every Dependency Root Audit, Catalog Maintenance Policy, Backend Credential Packaging (both arches), APK durable archives present, Workflow Permissions and CI Pipeline. Image vulnerability repairs follow separately.

## W7 CI82 follow-up — runtime image package repairs
- Refresh all 12 APT InRelease pins to 20261020T000000Z; verify Debian native-index hashes and inspect candidate package versions; native signed replay stays mandatory in PR CI.
- Glama final runtime now upgrades the inherited base through the snapshot installer; trixie-security contains fixed liblzma5 5.8.1-1+deb13u2.
- Remove build-essential from machine, hosted desktop and node-only runtimes; retain separate Rust builders. Final scan rejects linux-libc-dev/build-essential even without reported CVEs.
- Preserve hosted GitHub CLI with upstream 2.102.0 verified archives for both arches, replacing vulnerable Ubuntu gh 2.45.0; Renovate manager and checksum-failure tests cover the new pin.
- Local: APT inputs 23, catalogs 29, image vulnerabilities 15, build inputs 22, source artifacts 16, local image wrapper 19, packaging workflows 16, staging 3, hosted context 20, scanner workflows 7: 170/170 passed, 0 failed.
- Freshness/build-input gates, checksum-pinned actionlint, diff check and both real gh archive SHA-256 downloads pass. No local Docker or Cargo compilation used.
- docs/RELEASE_IMAGE_REMEDIATION.md records 104 remaining exact no-Debian-fix findings across 14 advisories, without enabling any suppression; PM clarification pending on upstream-source-fix versus distro-package-fix eligibility.
- Hosted: all APT Snapshot and APT Dockerfile Layers matrices, Shipped Image Identity (every component/arch), Backend Credential Packaging, publication final-image gates, Scanner Policy and CI Pipeline. Final green remains unproven until rebuilt scans and the remaining no-fix decision.

## W7 CI82 follow-up — restore machine workstation tooling
- Restore build-essential in the three Ubuntu machine workstation images; keep the checksum-pinned gh upgrade and snapshot pins.
- Remove the incorrect package-presence prohibition and its test; all actual HIGH/CRITICAL/UNKNOWN finding gates remain required.
- Correct the runtime documentation against MACHINE_CONTEXT_SPIKE.md's C/C++, make, npm-native and pip-wheel contract.
- Local: image vulnerability 14/14, build-input 22/22 and APT policy 23/23; 59 passed, 0 failed; diff check passes. Native image builds/scans remain PR CI evidence.

## W7 CI82 follow-up — shell-free static MCP demo
- Replace the Glama Debian runtime with scratch and a musl executable from the pinned Alpine Rust builder; remove the superseded liblzma5 package upgrade.
- Share the explicit updater/MCP scratch-component set between scan and SBOM inventory checks; real rustbinary coverage remains mandatory for both.
- Check both PT_INTERP and DT_NEEDED before accepting static binaries; retain source identity and exact subject/digest checks.
- Record the PM's residual-CVE eligibility and unconditional CRITICAL-blocking policy; no active image dispositions added.
- Local: ELF parser 2/2, source artifacts 16/16, image vulnerabilities 14/14 and APT policy 23/23; 55 passed, 0 failed; packaging workflows 16/16 and SBOM workflows 3/3; build-input gate passed.
- PR: Shipped Image Identity (mcp-demo, amd64/arm64), Shipped SBOM Policy, Artifact Packaging Policy, APT Policy, Scanner Policy and CI Pipeline; actual final-image scan counts remain hosted evidence.

## W7 CI82 follow-up — minimal node rootfs exporter and measured linkage
- Export exactly libc6/libgcc-s1/base-files/netbase/tzdata from snapshot-installed packages, retaining honest dpkg status/file lists; copy only the same snapshot's CA bundle.
- Reject absent runtime files, uninstalled/wrong packages, traversal, escaping write paths and builder-tool leakage; record only slim-excluded documentation/metadata omissions.
- Both actual CI Linux CLI archives link libdbus-1.so.3. Native arm64 execution in the exact pinned base fails at the loader; this reproducer is distinguished from an actual CI image execution.
- Disposable arm64 signed-snapshot export passed; actual Trivy rootfs scan detected exactly five OS packages and 0 HIGH/CRITICAL/UNKNOWN findings with the local DB. Full native image evidence still required in PR CI.
- Local exporter tests 5/5 passed; no Cargo source/manifest or node entrypoint edited. Coordinated CLI entrypoint and container feature are required before wiring the reduced Dockerfile.
- PR: Artifact Packaging Policy; eventual node image wiring must pass Shipped Image Identity (node-agent, both arches), native execution, final scan and SBOM closure.

## W7 CI82 follow-up — closed residual image-CVE eligibility
- Implement only the reviewed kernel-UAPI negative-test kind; no generic VEX/ignore input and no active image dispositions.
- Bind CVE, source/binary package versions, component/arch, full advisory content, owner, named affected API, rationale and at most 30 days; validate every record for expiry.
- Require the actual build-essential/libc6-dev/linux-libc-dev dependency chain, exact final package file-list/content and the unchanged signed-snapshot catalog.
- Verify native security-index checksum/size from the pinned InRelease; fixed versions, newer candidates, changed source/content and resolved/unused records fail, including renewals.
- Inspect the stopped final image with host-side dpkg-list/tar parsing, refusing executable/ELF/non-header payloads; publication reruns that proof.
- No real header records activated: the strict /usr/include-only negative still rejects the two package documentation files pending the explicit PM clarification.
- Local: eligibility 13/13; all existing image-vulnerability tests retain HIGH/CRITICAL/UNKNOWN behavior. Required Scanner Policy and Artifact Packaging Policy run the new negatives on PRs/schedules.

## W7 CI82 follow-up — execute every final image
- Mandatory identity verification now runs immutable-image executable/default-entrypoint smoke checks before publication or operator tagging.
- All CLI images execute version/help and default-entrypoint help; backend/frontend, MCP stdio ping and updater's safe invalid-command path also execute.
- Node ELF dependencies must fit the five-package runtime; the confirmed current DBus link remains a hard failure until the coordinated CLI fix.
- Machine Dockerfiles dispatch sole help/version flags before existing entrypoints; ordinary startup arguments/environment are preserved; D's entrypoint source stays unchanged.
- Probes have no network/registration input, read-only roots, dropped capabilities and resource/time bounds; cleanup removes probe containers and anonymous volumes.
- Local: runtime 9/9, shell dispatcher 3/3, packaging 17/17, operator wrapper 19/19, build/APT policy 45/45; native scratch MCP passed this exact read-only/network-less runtime gate.
- PR: Artifact Packaging Policy; Backend Credential Packaging and all Shipped Image Identity matrix cells; publication/operator checks inherit the same gate.

## W7 CI82 follow-up — measured runtime and security-candidate evidence
- Reconstructed current node final stage with the real CI arm64 executable and actual snapshot CA install; docker nyxid --version exits 127 for missing libdbus-1.so.3; dpkg confirms the library package is absent.
- Native MCP scratch fixture passes the committed runtime gate; rootfs and MCP scans report 0 blocking findings with the existing local database, without claiming full new hosted-image evidence.
- Both authenticated October 20 noble-security native indexes still offer linux/source and linux-libc-dev 6.8.0-147.147; checksums and sizes match pinned InRelease.
- Record last hosted package/finding counts per component and distinguish new native fixtures; machine header findings remain blocking until the exact documentation-path ruling and final-image proof.
- CLI ownership/manifest coordination is pending for a shell-free node entrypoint and a non-DBus container feature; no A/D CLI files edited.

## W7 CI82 follow-up — exact approved kernel-header records
- Allow only headers plus changelog.Debian.gz/copyright at their exact linux-libc-dev paths and parent directories; require regular non-ELF/non-executable content and bind every byte.
- Activate 309 exact records per machine component/architecture (2,472 total): 298 HIGH + 11 CRITICAL; owner recorded, issued 2026-10-20, exclusive expiry 2026-11-19.
- Authenticated native package replay: amd64 988 headers + 2 docs; arm64 960 headers + 2 docs; unchanged signed-security candidate 6.8.0-147.147.
- Real retained report + exact native package proof replay: 8/8 component/arch mappings remove only 309 approved identities; 8/8 injected new-CVE cases remain blocking. This is not a rebuilt final-image scan.
- Local: eligibility 15/15, vulnerabilities 14/14, scanner workflows 7/7, packaging workflows 17/17, SBOM workflows 3/3, runtime 9/9; diff whitespace passes.
- PR: Security Scans / Scanner Policy; Artifact Packaging Policy; every workstation Shipped Image Identity matrix cell. Private hosted builds rerun final proof through the operator gate.

## W7 CI82 follow-up — pure Rust keyring and shell-free node
- Use async-io Secret Service plus RustCrypto; upstream requires a crypto feature, so the approved D-Bus session changes Plain→Dh; no active Linux libdbus parent or OpenSSL dependency.
- Replace shell registration/start with the real CLI paths; keep file custody, config/URL defaults, pre-parse forwarded args and redact registration failures.
- Node exports five signed-snapshot runtime packages, snapshot CA and scannable dpkg records into scratch; no shell or DBus runtime.
- Require real CLI Tokio preflight/set/get/delete in isolated gnome-keyring/D-Bus on CLI Test; missing tools/test, timeout or failure blocks. Native Lima production-backend fixture passed the same roundtrip.
- Linux archives reject libdbus DT_NEEDED; final node executes version/help plus the entrypoint's no-config refusal.
- Targeted entrypoint 5/5 and existing keychain 5/5 pass using nyxid-387c312571c94c44; all-target node-proxy-test Clippy has only A’s cutover.rs unused_unit/chunks_exact_to_as_chunks lints; PM accepts the gate with no compile/owned diagnostics.
- Cargo audit: only the two existing RSA findings; 3/3 exact lock-subgraph dispositions validate. No new exception or RSA source refresh.
- PR: CLI Test; Rust Clippy; both Linux CLI Archive Identity; node-agent Shipped Image Identity both arches; Artifact Packaging Policy; Every Dependency Root Audit.

## W7 CI82 follow-up — backend minimal rootfs and binary health probe
- Replace the 106-package Debian runtime (71 retained baseline findings) with the five signed-snapshot packages, snapshot CA and real dpkg inventory; no backend dispositions.
- Preserve non-root 999:999, key/TUF directories, independent TUF state and exact credential bytes; enforce the final ELF's declared C-runtime linkage.
- Add early shell-free health-check dispatch: PORT-only fixed loopback /health, no proxy/redirect, three-second bound, 2xx only and stable one-line failures before config/keys/DB/tracing.
- Production Compose uses the binary; ENV.md documents the curl/shell removal, custom-probe upgrade and Kubernetes HTTP probe.
- Native arm64 offline package-rootfs: base-files 12.4+deb12u15, libc6 2.36-9+deb12u14, libgcc-s1 12.2.0-14+deb12u1, netbase 6.4, tzdata 2026c-0+deb12u1; Trivy 0.76.0 reports zero HIGH/CRITICAL/UNKNOWN (26 LOW/MEDIUM).
- This package-only proof does not replace current final-image evidence; the both-KMS Linux graph has no openssl-sys, with no backend TLS feature edit.
- Reuse D's nyxid_server-91bd2f1ee51e6e06 (driver 21:09 after last source edit 21:07): health tests 5/5. Reuse ID's both-KMS all-target Clippy /tmp/nyxid-w8-id-oauth-corrected-clippy.log (drivers 21:25): exit 0 at 21:32:44.
- Local policy: rootfs 7/7, backend runner 2/2, runtime 10/10, vulnerabilities 15/15, packaging 19/19, credential 21/21, scanner 7/7, SBOM 3/3; pinned actionlint and diff whitespace pass.
- PR: Artifact Packaging / Backend Credential Packaging amd64+arm64 must start the real final server and require the binary probe with invalid application/key env; Artifact Packaging Policy; APT Dockerfile Layers; Scanner Policy.
