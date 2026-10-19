# Managed security-reporting template change

The companion change is published as draft [fkst-hosted PR #6196](https://github.com/ChronoAIProject/fkst-hosted/pull/6196), head `852e7f055534352f38a6ad39d4434d3a99039aab`, against reviewed `develop` base `934914502a2fb740c54c030826acd9b7f8128e92`. All seven published Git blobs and modes were independently verified against the tested source. The PR is unmerged and undeployed. NyxID's installed template marker remains 13; upstream merge and rollout remain the W1 dependency.

The patch advances the upstream bundle to 14 and generates a security-policy contact from validated GitHub owner/repository components. Generic repositories route through `/security/policy` without assuming private advisory reporting is enabled. NyxID's root policy retains its direct private advisory link. Only fixed bundled content and validated path components enter the automatically merged template PR. Existing path and PR provenance guards remain in place. The unmanaged bug-report form remains outside the writer's allowlist.

The same patch repairs strict-lint failures exposed by upstream CI: it updates the `async-trait` lockfile entry from 0.1.89 to 0.1.92, removes an unnecessary closure borrow, and replaces deprecated atomic `fetch_update` with an equivalent saturating compare-exchange loop. It preserves atomic ordering and avoids requiring the newer replacement API. No lint suppression was added.

Root `AGENTS.md` was absent in the reviewed snapshot; a read-only Contents request at that revision returned 404. The snapshot's `CLAUDE.md` was read. No additional workers were used.

From a clean upstream checkout at the reviewed base:

```sh
git apply --check /path/to/NyxID/docs/security/fkst-hosted/security-reporting-9349145.patch
git apply /path/to/NyxID/docs/security/fkst-hosted/security-reporting-9349145.patch
cd backend
cargo fmt --all -- --check
cargo test --locked --lib github_app::
cargo test --locked --lib reconcile::templates
cargo test --locked --lib audit::worker::tests::
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Independent final-source validation passed: 159 GitHub tests (including 17 template and 22 installer tests), 10 reconciliation tests, and 13 audit-worker tests, with zero failures or ignored tests. Formatting and Clippy for the workspace and all targets passed locally with Rust 1.96. All seven remote CI jobs passed at the exact published head, including full Rust tests, strict lint, builds, Docker, frontend, secret scanning and deployment gates. Primary recorded the check results in `/tmp/nyxid-fkst-ci-repair-checks.json`; the PR remains draft and unmerged. The regenerated patch applies cleanly to the reviewed base and reproduces all seven published files.

The installer test captures and decodes actual Contents writes, asserting the generated NyxID contact and unchanged managed path set. Invalid components fail before reconciliation effects. Existing tests cover pending PR reuse, conflict recovery, provenance/path restrictions and protected-branch deferral. Independently rendered NyxID chooser output also passed the repository's reporting-contact reconciliation unchanged. No deployment behavior is claimed verified.
