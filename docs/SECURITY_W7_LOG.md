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
