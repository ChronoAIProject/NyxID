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
