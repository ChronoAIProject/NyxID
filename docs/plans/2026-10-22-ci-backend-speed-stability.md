# CI backend speed and stability plan

Owner: PM (Claude Fable 5.1). Implementer: gpt-6.1-sol via heca. Branch: `optimize-ci-speed-and-stability`.

## Problem (measured on run 39633456234, 2026-10-21)

| Job | Duration | Breakdown |
|---|---:|---|
| Coverage (Backend) | 84.8 min | ~20 min instrumented compile, ~50-55 min tests |
| Backend Test | 72.1 min | 18m04s compile (16.5 min of it is the single `nyxid` rustc), 52.7 min tests (10,142 tests, 12 skipped) |
| Coverage (Backend Base) | up to 84 min | full recompile + rerun of the base commit on every PR whose exact-SHA cache entry is missing |

Facts that shape the plan:

- Test execution is fully parallel already: the sum of per-test wall time is 12,206 s, run on 4 slots in 3,163 s (3.9x). 395 tests take over 10 s each; 30 take over 30 s. These are real MongoDB / wall-clock durability tests and stay as they are.
- Dependency compilation is cheap (590 crates in ~2 min). The cost is the one `nyxid` test crate: 16.5 min in one rustc with a 30 GiB RAM+swap peak. Compile cache cannot help this; splitting the crate is out of scope for this round.
- The repository Actions cache is full (189 entries, 11 GiB, mostly buildkit blobs). Every PR branch saves a ~700 MiB Rust cache that evicts everything else, so most jobs see "No cache found".
- Both the ordinary and the instrumented suite execute the same CI tests: the only `cfg(not(coverage))` items are an `#[ignore]` local e2e test and its helpers. The ordinary run additionally executes `nyxid-service-adapters`.
- Organization plan is Enterprise: runner concurrency is not the constraint; per-runner CPU and memory are.

## Goals and non-goals

Goals: cut the backend critical path from ~85 min to roughly 40 min without reducing which tests run, what they assert, their process isolation, the no-retry policy, or the 73% line-coverage gate; keep each runner's CPU budget bounded (same `num-cpus` test slots, one MongoDB per runner, compiler alone on the build runner).

Non-goals this round: changing test code or timeouts, splitting the `nyxid` crate, larger runners, removing either the ordinary or the coverage suite (Phase 2 decision, with data).

## Phase 1: this PR

1. Compile once per build mode, execute on 4 isolated shards (`cargo nextest --partition hash:N/4`), verify completeness, merge coverage counters. Already implemented in `.github/workflows/backend-tests.yml` and `.github/scripts/backend_shards.py`; raise shard count from 3 to 4 (expected ~13 min of tests per shard plus setup).
2. Keep the completeness gate: every listed test runs exactly once across shards; failures, unexpected skips, duplicates and retries fail the PR; coverage merges all shard profiles before `--fail-under-lines 73`.
3. Split the build artifact so the ordinary verification job downloads only the inventory, not the multi-GB archive.
4. Stop PR branches from saving Rust caches (`save-if` on trunk refs only: `main` via publish-images, `dev`, `rollup-*`). PRs still restore trunk caches.
5. Base coverage delta: never rerun the base suite. Try the exact-SHA cache first, then look up the base commit's own successful CI/publish run by SHA and download its published baseline artifact (fingerprint-checked), else report N/A. Head coverage remains the gate.
6. Shard evidence staging and diagnostics must not mask the real failure (guard file copies; MongoDB dump on failure stays).
7. Hosted validation before merge (acceptance criteria below).

## Acceptance criteria

- Ordinary suite: 10,142 tests pass exactly once across shards, 12 pre-existing ignored tests reported; `nyxid-service-adapters` tests included.
- Coverage suite: same completeness gate; merged line coverage within noise of the current main measurement and above 73%; an intentionally lowered gate still fails (already proven locally).
- `CI Pipeline` fails when any shard, the build, or the verification job fails (reusable-workflow result propagation checked on a deliberately failing test run or by inspection of a red run).
- Base delta shows a number on a PR whose base has a published baseline; N/A otherwise; never a rerun.
- Backend critical path on two consecutive green hosted runs at or below ~45 min; no runner memory kills; resource samples uploaded.
- `python3 -m unittest` for all `.github/scripts/test_*.py`, `actionlint`, and `scripts/check-reusable-workflow-permissions.py` pass.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Artifact transfer failure | explicit `if-no-files-found: error`; verification job fails loudly |
| Hash partition imbalance | 4 shards; measure per-shard durations from junit; rebalance count if any shard exceeds 20 min |
| Order-dependent tests exposed by partitioning | nextest already isolates processes; report any such failure to PM without changing test code or retrying |
| Image rollout between build and shard runners | coverage identity recorded on every runner; mismatch marks the baseline non-reusable |
| Compile memory | unchanged compiler concurrency, swap reserve, MongoDB absent on the build runner |

## Phase 2 (separate PRs, decided from Phase 1 measurements)

- Consolidate to a single instrumented archive if instrumented shard times are within ~15% of ordinary ones (halves runner minutes; wall time unchanged). Requires adding `nyxid-service-adapters` to the instrumented run.
- Compile time of the `nyxid` test crate (16.5 min, 30 GiB): investigate codegen settings and splitting integration-heavy test modules into a separate test target.
- Other long jobs after the backend path: Backend Image Inputs (28 min), Machine Container E2E (26 min), Billing Smoke (21 min), Clippy (15-18 min) all recompile the backend independently.
- Slow-test audit: the 30 tests over 30 s, for deadlines that can be configured shorter in CI without changing what is asserted.

## Measurements log

Before: [run 39633456234](https://github.com/ChronoAIProject/NyxID/actions/runs/39633456234) took 72.1 minutes for ordinary backend tests and 84.8 minutes for backend coverage. Hosted measurements for this change are pending; the target is a backend critical path at or below approximately 45 minutes on two consecutive fully green PR runs.

| Run | Ordinary build min | Ordinary shards 1/2/3/4 min | Ordinary report min | Coverage build min | Coverage shards 1/2/3/4 min | Coverage report min | Merged line % | Backend critical path min | Ordinary passed / ignored | Coverage passed / ignored | Base delta |
|---|---:|---|---:|---:|---|---:|---:|---:|---|---|---|
| Hosted run 1 | pending | pending | pending | pending | pending | pending | pending | pending | pending | pending | pending |
| Hosted run 2 | pending | pending | pending | pending | pending | pending | pending | pending | pending | pending | pending |

Local validation (2026-10-22): all 45 Python unit tests, actionlint 1.7.12, the reusable workflow permissions validator, and `git diff --check` pass. The existing pinned tools and PyYAML environment in `/tmp/nyxid-ci-tools` were reused. The system Python lacks PyYAML, so verification uses that environment's `python3` on PATH.

The prior Rust fixture was reproduced with cargo-nextest 0.9.146, cargo-llvm-cov 0.9.1, Rust 1.98.1, and four hash partitions. Build outputs were removed before execution in separate remapped workspaces. Ordinary inventory verified 45 passed plus one ignored; instrumented inventory verified 44 passed plus one ignored. Both retained every expected test exactly once without compilation during execution. Traces for fixture tests with the exact production exclusive-override names showed no overlap with any other test, confirming that the checked-in CI profile and `threads-required = "num-cpus"` overrides load from the remapped workspace during archive execution. All 16 raw profiles survived staging. Merged totals exactly matched the unsharded fixture (58/62 lines, 93.54838709677419%); a deliberately unsatisfied 100% gate exited 1. This fixture checks archive mechanics; hosted results determine full-suite counts, performance, and stability.

Implementation decisions:

- Keep `SHARD_COUNT=4` once at workflow level and the explicit four-entry matrix with an agreement comment. Partitioning and completeness checks consume the shared value; the verifier rejects extra/missing shard directories or a mismatched count.
- Upload the archive and inventory separately. Ordinary verification downloads inventory and shard evidence; coverage verification also downloads and explicitly extracts the instrumented archive with GNU `tar --zstd` next to the merged counters.
- Preserve earlier setup/execution failures when JUnit is absent, while a successful nextest step without JUnit fails evidence staging. Upload evidence only when JUnit exists.
- Limit Rust cache writes to main, dev, and rollup push refs, including main's reusable publish gate. PRs retain restore access. No cache entries are deleted.
- Try the exact-SHA cache before API artifact lookup, as directed by PM. Accept only successful push/dispatch runs of CI or Publish Images at that SHA, skip absent/expired artifacts, and require both the current and recorded identities to be reusable with identical fingerprints. Missing/incompatible baselines remain N/A; the base suite never runs.
- Publish the baseline report plus recorded identity on non-PR coverage runs with 30-day artifact retention. Preserve the existing exact-SHA cache and coverage comment/merge behavior. Explicitly grant `actions: read` to the base job and its publish caller.
- Leave `coverage_base_lookup.py` and its unit tests out of `coverage_identity.INPUTS`: selecting and validating a prior report does not change the measurement recipe. The existing workflow/config/tooling fingerprint remains in force.
- Keep repository tests, package selection, nextest settings, retry policy, and the 73% head gate unchanged. Use the current checkout and requested branch only; keep the PR a draft for PM review.
