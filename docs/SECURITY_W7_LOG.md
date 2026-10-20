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
