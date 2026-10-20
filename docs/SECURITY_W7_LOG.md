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
