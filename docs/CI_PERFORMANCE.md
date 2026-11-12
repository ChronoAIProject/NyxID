# CI performance and validation contracts

Measured on 2026-11-12–13 at `31f3d8969e051e1fe6ed2f42a5594ded90606fe5`.
Local compiler measurements used stable Rust 1.98.1, an 18-core/128 GiB Apple
Silicon Mac and four Cargo jobs. This is a shared machine. These observations
are not Linux runner benchmarks or confidence intervals. Peak RSS below is
`/usr/bin/time -l` maximum resident set size, converted from bytes to GiB.

## Why the hosted gates were slow

Read-only reproduction commands:

```sh
gh run view 43210236259 --json jobs
gh run view 43210236259 --job 131155970860 --log
gh run view 43174155560 --json jobs
gh run view 43210236077 --json jobs
gh run view 41641023323 --json jobs
```

| Run and work | Observed elapsed |
| --- | ---: |
| PR 43210236259, ordinary backend archive compilation | 50.50 min |
| Same PR, instrumented backend compilation | 51.05 min |
| Same PR, image-input binary compilation | 38.17 min |
| Same PR, Clippy job | 22.85 min |
| Same PR, three linked KMS feature jobs | 15.57–19.45 min |
| Same PR, Machine Container E2E | 27.98 min |
| Publish 43174155560, amd64 / arm64 backend builds | 39.80 / 36.48 min |
| CodeQL 43210236077, Rust Analyze | 65.88 min |
| Release 41641023323, macOS arm64 / Intel local artifacts | 187.88 / 148.55 min |

The ordinary archive job had one rustc process for most of compilation. Between
minutes 6 and 37, mean load was 1.715; minimum sampled available memory was
about 154 MiB and peak swap use was 26.533 GiB. The monolithic compiler exceeded
16 GiB while doing substantial serial work. These samples establish memory
pressure; they cannot separate elapsed time spent swapping from useful work.
The backend has 1,646 Rust files and 1,350,556 lines, including 854,933 service
and 391,332 handler lines. Counting test-named paths alone is not a sound measure
of test-only code.

Publish's backend manifest was available about 40 minutes after run start.
The approximately 73-minute workflow duration includes its later, nonblocking
coverage baseline. The existing exact-tree gate reuse can avoid repeated CI
only for jobs actually completed successfully on the matching tree. Skipped
or missing PR jobs still run on main; image publication remains fully gated.

### Three independent cache failures

An incremental cache download did not imply a reusable backend graph:

1. Six lanes saved only `target/debug/incremental`, while every reader requested
   both that path and `target/llvm-cov-target/debug/incremental`. Actions hashes
   the exact path list into its cache version. The one-path version starts
   `a625e3a14900daf5`; the two-path version starts `f98f77cfa579f3b1`. Clippy,
   three KMS feature lanes, CLI test and permissions therefore could never
   restore their own saved graphs.
2. Ordinary backend saves in runs 43174155488 and 43247779268 failed with
   `No space left on device` during compression. A successful nonblocking warm
   job conclusion did not prove cache publication.
3. Coverage restored its graph, then cargo-llvm-cov's default archive cleanup
   deleted the workspace compiler artifacts before building.

Run 43247779268 job 131284989812 saved the one-path GCP cache at 15:41:48 UTC;
run 43266680960 job 131351986848 missed at 17:11 on the same `04dadb45` manifest
prefix. CLI jobs 131284990513 and 131351986713 show the same mismatch. Inspect:

```sh
gh api 'repos/ChronoAIProject/NyxID/actions/caches?per_page=100&key=v0-rust-incremental'
gh run view 43247779268 --job 131284989812 --log
gh run view 43266680960 --job 131351986848 --log
gh run view 43174155488 --job 131034239644 --log
gh run view 43247779268 --job 131284990234 --log
```

The archive build in PR 43210236259 missed the exact manifest prefix and
restored a 0.76.0 backend graph while compiling 0.78.0. That download took
178 seconds for 7.7 GB. A local version-only fixture confirmed different
`-C metadata`, `-C extra-filename` and incremental directories. This was not
proof that backend incremental reuse worked.

## Cache contract and pre-merge measurements

`ci-rust-cache` now defines the incremental key and ordered path list once.
Its restore consumes that contract, and every writer passes the same outputs
to `ci-rust-incremental-save`. Tests check each writer's key/path expression
and execute the contract's shell derivation. No writer restates those values.

The broad lane/OS/architecture restore fallback is retained. An unrelated
member manifest or lock entry can change the workspace hash without changing
the backend dependency graph. Removing the fallback would force those builds
cold. A package-version change may instead waste 3–6 minutes downloading an
unusable graph. That bounded cost is preferable to losing reusable graphs;
Cargo remains responsible for dependency and metadata validity. No global
package-version segment is added because unrelated member version changes
would invalidate it too.

Only `ci-cache-warm.yml` opts into ordinary dependency and incremental saves.
Push saves are main-only; manual dispatch can save on its selected branch.
Actions provides ref scoping. Readers remain read-only. Concurrency is per
ref, so a branch measurement does not cancel main warming. Retention protects
a branch cache until a compatible main replacement exists: dependency caches
must match prefix and cache version, incremental caches must match cache
version. The old one-path main caches cannot evict new two-path branch caches.
See [cache retention](ci/ACTIONS_CACHE_BUDGET.md) for the complete policy.

Each warm compilation runs under `ci_resources.py`, recording elapsed time,
child peak RSS, and sampled host/cgroup memory and swap. Resource artifacts are
uploaded even on failure. Hosted verification requires two manual dispatches
of the same branch: inspect cache save success in the first, restore hits and
compiler/resource durations in the second. This work does not trigger them.
A further small-edit branch run is needed to measure practical reuse after a
code change. Local subsecond no-change timings do not include hosted downloads.

Backend warm jobs release only the swap reserve they created after compilation
and before cache compression. On the inspected runner this returns roughly
37 GiB: the reserve supplements the runner's existing 3 GiB to reach 40 GiB.
The helper records ownership, disables exactly that swap file, and preserves
the file and ownership record if `swapoff` fails. Tests exercise these paths
without changing developer-machine swap. Compile-time capacity is unchanged.
Actual disk recovery and successful uploads still need Actions evidence.

## Plain Cargo freshness and coverage archives

Both build scripts resolve real Git paths with `git rev-parse --git-path`.
Relative results are resolved against the package working directory, without
`--path-format` (which Git 2.25 does not support). The Docker input guard uses
the same resolution against its repository working directory.
They watch existing HEAD and per-worktree HEAD reflog files, not nonexistent
package-relative `.git` paths or a shared refs directory. A loose current ref
is the fallback when a HEAD reflog is absent. If neither exists inside a Git
worktree, the scripts deliberately watch the missing HEAD reflog path. Cargo
then reruns them on every invocation until a reflog or loose ref is available.
This covers a packed branch with `core.logAllRefUpdates=false`: a commit creates
a loose ref without changing HEAD. Watching HEAD alone would retain the old
stamp indefinitely. The Docker input guard recognizes the missing reflog as
Git metadata replaced by the image's runtime stamp. If Git cannot return a
UTF-8 reflog path, the scripts use `build.rs/git-stamp-unavailable` instead of
failing the build. That child path cannot exist because `build.rs` is a file;
watching it forces a rerun without another fallible Git or environment lookup.
The guard exempts only that exact sentinel under the backend and CLI package
roots. Outside a Git worktree, the scripts emit no Git watch.

Linked worktrees, detached HEAD and packed refs are exercised by real Cargo
fixture builds. Commits, checkouts,
resets, rebases and pulls refresh the stamp; another worktree's commits and
fetches do not invalidate it. The backend's explicit `NYXID_GIT_HASH` wins;
a source context without Git still embeds `unknown`.

The dirty suffix is sampled when the build script runs, as documented in
CONTRIBUTING. No wrapper or environment stamp must be supplied before Cargo.
CLI builds still watch `DEVELOPER_DIR`, `TOOLCHAINS` and `SDKROOT`: these affect
`xcrun`'s Swift compatibility-library location on macOS. Without those watches,
a cached build could retain an obsolete linker search path after an SDK change.

| Unchanged build | Before | After, plain Cargo |
| --- | --- | --- |
| Ordinary backend test build | 49.62 s / 20.64 GiB | 0.51 s / 197 MiB |
| Instrumented archive | 648.92 s / 39.48 GiB | 0.97 s / 281 MiB |

The coverage result combines correct Git watches with `--no-clean`. Producers
first run `cargo llvm-cov clean --profraw-only` to remove old execution counters.
Nextest selects current Cargo executables; reports select objects from that
archive using `--nextest-archive-file`. They do not scan stale executables in
the target directory. A real-tool fixture verifies unchanged reuse, deleted
source, a renamed test binary and equality with a clean coverage report.
The package selection, `cfg(coverage)`, 73% threshold, coverage filters and
base/head comparison are unchanged. Ordinary and instrumented tests remain
separate because `cfg(coverage)` changes which tests exist.

The earlier cached-archive experiment produced a byte-identical 1,348,559,424
byte test executable before and after `--no-clean` (SHA-256
`77886501709378fad371e62df435a1a9235230f5631048a079b8b0280ca49105`). A warm
semantically equivalent constant edit took 95.30 s / 26.85 GiB. Instrumented
edits still exceed standard-runner RAM locally; this is a cache fix, not a
claim that every compilation now fits 16 GiB.

Reproduce using pinned nextest 0.9.146 and cargo-llvm-cov 0.9.1 on PATH:

```sh
export CARGO_TARGET_DIR="$PWD/target/ci-benchmark"
export CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=1 RUSTFLAGS=-Dwarnings
export CARGO_PROFILE_DEV_DEBUG=line-tables-only
export CARGO_PROFILE_TEST_DEBUG=line-tables-only
/usr/bin/time -l cargo +1.98.1 test --locked \
  -p nyxid -p nyxid-service-adapters --no-run --timings
# Repeat without edits to measure freshness.
cargo +1.98.1 llvm-cov clean --profraw-only
/usr/bin/time -l cargo +1.98.1 llvm-cov nextest-archive --locked \
  --package nyxid --archive-file target/ci-benchmark-tests.tar.zst --no-clean --timings
# Repeat the same archive command without edits.
python3 -m unittest discover -s scripts -p test_git_build_scripts.py
python3 .github/scripts/test_coverage_archive_cache.py
```

## Compiler costs and rejected alternatives

The following baseline observations used the test command above before fixing
the build scripts. The small edit rewrote a constant `5` as `2 + 3`, then
restored the source byte-for-byte. Dependencies were warm except where stated.

| Configuration | Cargo wall | Backend rustc | Peak RSS |
| --- | ---: | ---: | ---: |
| Incremental, cold dependencies and workspace | 563.07 s | 483.04 s | 39.35 GiB |
| Incremental, unchanged | 49.62 s | 48.38 s | 20.64 GiB |
| Incremental, small edit | 49.67 s | 48.53 s | 20.74 GiB |
| Non-incremental, cold workspace | 411.74 s | 408.34 s | 40.25 GiB |
| Non-incremental, unchanged | 411.60 s | — | 40.28 GiB |
| Non-incremental, small edit | 411.03 s | — | 40.25 GiB |

Disabling incremental did not reduce peak RSS and made invalidated warm builds
far slower. These numbers do not support disabling it on hosted test lanes.
The unchanged rebuilds were caused by invalid Git watches; they are not an
inherent cost of plain Cargo.

A measurement-only rustc wrapper added `-Ztime-passes` to `nyxid_server`, with
`RUSTC_BOOTSTRAP=1`. The backend compiler took 473.807 seconds:

| Phase | Elapsed | RSS at phase end, rustc's decimal MB |
| --- | ---: | ---: |
| Expansion | 13.493 s | about 4,900 |
| Type checking including coherence | 97.980 s | 16,894 |
| MIR borrow checking | 125.606 s | 25,803 |
| Monomorphization graph walk | 142.535 s | 36,777 |
| LLVM passes | 58.432 s | 39,911 |
| Linker | 1.497 s | 14,845 |

Phases can nest or overlap; their times must not be summed. The compiler
already exceeded hosted RAM before LLVM completed. A faster linker would
address only about 1.5 seconds here. The stable gate uses no bootstrap or
unstable flags. To reproduce diagnostics without affecting every dependency,
set `RUSTC_WRAPPER` to an executable that forwards all arguments and appends
`-Ztime-passes` only when its arguments contain `--crate-name nyxid_server`;
run the same test build with `RUSTC_BOOTSTRAP=1` in a separate target directory.

A development production build measured 395.214 s in rustc, with 57.350 s type
checking, 76.033 s borrow checking, 192.352 s monomorphization, 42.161 s LLVM
and 0.918 s linking. Peak RSS was 23.62 GiB. `-Zdump-mono-stats` counted
730,133 concrete generic instances; BSON serialization methods led estimated
MIR sizes. These are compiler costs, not runtime performance measurements.

| Rejected experiment | Measurement and reason |
| --- | --- |
| Full test debug info instead of existing line tables | 756.59 s / 31.97 GiB versus 663.88 s / 39.33 GiB in a paired clean-backend run. Line tables were 12.3% faster but did not reduce RSS. No new universal profile change. |
| Non-incremental release CGU 256 instead of 16 | Backend 1,207.73 s versus 884.31 s; RSS 30.06 versus 26.57 GiB; binary 452,088,992 versus 281,392,608 bytes. Worse in all three observed dimensions. |
| Docker compiler bump from image's Rust 1.93.1 to gate's 1.98.1 | Backend 903.19 versus 884.31 s; RSS 25.68 versus 26.57 GiB. About 2% time difference on a shared Mac does not justify a compiler change. |
| Compile one quarter of explicit test functions, keeping fixtures and production | 450.28 s / 29.88 GiB; still exceeds 16 GiB. Inventory union and coverage equivalence were not established, so not a replacement gate. |
| Extract 188 leaf model modules | Test build 689.87 s / 31.47 GiB, slower and still too large. Production 534.85 s / 17.39 GiB. |
| Expand extraction to 236 models plus AppError | Production 417.65 s / 26.94 GiB versus 23.62 GiB baseline; not a memory fix. |
| Split services/models/middleware core from handlers/routes | Development units 13.49 and 12.37 GiB, but optimized GCP core + binary took 522.96 + 635.46 s sequentially, 31.0% more than the monolith; peak 22.87 GiB still exceeds runner RAM. Required 230 source/manifest edits. |
| Hybrid production split with original monolithic tests | First test check failed with 53 trait/type errors. No successful timing, test-equivalence or coverage claim. Not shipped. |

The structural experiments were disposable source snapshots and are not part
of this change. Their results do not justify a large architecture migration as
a CI speed fix. Debug and CGU comparisons used the same command with
`--config 'profile.test.package.nyxid.debug=2'` or
`--config 'profile.test.package.nyxid.debug="line-tables-only"'`, and
`--config 'profile.release.package.nyxid.codegen-units=256'` respectively.
Release measurements used `cargo build --locked --release -p nyxid
--bin nyxid-server --features gcp-kms --timings`, four jobs and `-Dwarnings`.

## Backend image input validation

`Backend Image Inputs` keeps its job id, display name and gate-reuse position.
It now runs `cargo check --release --features gcp-kms` with `-Dwarnings`.
That retains release-profile parsing, expansion, type/borrow checking and the
Docker input proof. The linked `gcp-kms` feature job retains ordinary codegen
and linking. `Backend Release Image` retains real optimized linking for image
infrastructure PRs; publish retains optimized linking for both architectures.
Release checks do not prove optimized codegen or linking on every source PR;
that proof is supplied by the real image builds at those existing gates.

Two incremental release candidates were measured before choosing check:

| Cold backend variant | Total wall | Backend rustc | Peak RSS |
| --- | ---: | ---: | ---: |
| Existing non-incremental release, CGU 16 | 1,027.71 s | 884.31 s | 26.57 GiB |
| Incremental release, default CGU 256 | 1,138.34 s | 1,126.28 s | 29.02 GiB |
| Incremental release, explicit CGU 16 | 952.70 s | 837.18 s | 30.18 GiB |
| Non-incremental release check | 147.43 s | 125.30 s | 12.66 GiB |

| Variant | No-change wall / peak RSS | Small-edit wall / peak RSS |
| --- | --- | --- |
| Incremental release, default CGU 256 | 0.29 s / 189 MiB | 787.39 s / 24.12 GiB |
| Incremental release, CGU 16 | 0.29 s / 190 MiB | 458.48 s / 27.17 GiB |
| Release check | 0.30 s / 189 MiB | 173.09 s / 12.65 GiB |

The default incremental cold graph occupied 5.6 GiB and compressed to
2,172,469,559 bytes (2.02 GiB) with `tar | zstd -T4 -1`. Those bytes would
be additional cache transfer on a hosted runner. After the edited and restored
CGU-16 builds, its retained cache was 9,724,448,634 bytes (9.06 GiB), compressed
to 3,226,768,127 bytes (3.01 GiB) with the same command. No-change local builds do
not include transfer or the Git timestamp invalidation from a fresh checkout.
The small-edit comparisons reused their respective completed cold builds;
they ran concurrently on the shared host, so elapsed differences include
scheduling variation. Each source edit was restored byte-for-byte afterward.

Registry dependency states differ: compare compiler-unit time rather than
using total wall time as a controlled ratio. CGU 16 is 5.3% faster in the local
compiler, but uses 3.6 GiB more peak RAM than the original release build.
Both incremental candidates increase memory pressure in a compiler already
well above the hosted runner's 16 GiB. This fails the requirement that the cold
case not materially worsen that runner's memory constraint. The exact swap/time
penalty is a projection, not measured on hosted Linux. Release check stays
below 16 GiB locally and avoids the optimized codegen cost.

The collector consumes the successful current Cargo JSON stream, including
fresh artifacts, and resolves each workspace unit's `.d` file plus build-script
`rerun-if-changed` inputs. It refuses failed/incomplete streams, missing unit
inputs, test artifacts or a backend without release/GCP settings. It never
reads a stale aggregate `nyxid-server.d` after check. The original full-build
closure, both incremental release closures, and the cold, cached and edited
check closures contain exactly the same
**1,364 repository inputs**, with no missing or additional paths. Only the
specific Git metadata replaced by the image's runtime stamp is exempted.

A real Cargo fixture proves equality against a full release build, including
an embedded asset with a space in its name and a build-script watched asset.
It then removes a source while leaving stale `.d` files and proves they do not
enter the new closure. The existing Docker COPY and ignore checks still inspect
every selected repository input. Both CI and the warm lane use the same command
and collector; workflow tests enforce that contract.

```sh
export CARGO_TARGET_DIR="$PWD/target/image-input-benchmark"
export CARGO_BUILD_JOBS=4 RUSTFLAGS=-Dwarnings CARGO_INCREMENTAL=0
/usr/bin/time -l cargo +1.98.1 check --locked --release -p nyxid \
  --bin nyxid-server --features gcp-kms --timings \
  --message-format=json-render-diagnostics > target/image-inputs.jsonl
python3 scripts/check-backend-docker-embeds.py --cargo-messages target/image-inputs.jsonl
# Alternatives: replace check with build and set CARGO_INCREMENTAL=1;
# repeat with CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 in a separate target.
# Repeat unchanged, then change the main.rs interval constant 5 to 2 + 3;
# restore the exact original source after measuring the edit.
python3 -m unittest discover -s scripts -p test_backend_docker_embeds.py
```

## CLI release selection

`scripts/dist-build.sh` gives cargo-dist a Cargo adapter that adds
`--bin nyxid --locked` to its workspace `dist` build. It retains workspace
feature unification and lets cargo-dist package the artifacts. Both release
and release-cache use it. The adapter rejects unexpected package/target
selection and restores the real Cargo executable before entering build scripts.

`precise-builds=true` was rejected: 627 rather than 789 build units, but changed
retained dependency features, including HTTP/2, JSON insertion order, GIF
support and WebSocket TLS roots. Workspace binary selection retained 638 units
with identical features on every retained unit. Measurement-only unit graphs:

```sh
RUSTC_BOOTSTRAP=1 cargo +1.98.1 build --locked --workspace \
  --profile dist --target aarch64-apple-darwin -Z unstable-options --unit-graph
# Compare with the same command plus --bin nyxid.
```

Actual cargo-dist 0.30.0 packaging took 1,131.62 s / 23.07 GiB for the workspace
build and 115.19 s / 3.48 GiB with binary selection. Cache states differed;
this is not a controlled cold/cold ratio. After the final SDK-watch change,
repackaging took 81.68 s / 3.52 GiB. The original and final macOS arm64 packaged
CLI were byte-identical (SHA-256
`9c2a9d2e12d149e942b5f0bd6378460f36062f7bbabc7b469b3992a9866f1e66`), with identical
README/LICENSE members, version and help output. Other targets' retained unit
features matched; actual packaging on those platforms still needs Actions.

```sh
env RUSTUP_TOOLCHAIN=1.98.1 CARGO_BUILD_JOBS=4 RUSTFLAGS=-Dwarnings \
  CARGO_TARGET_DIR="$PWD/target/dist-benchmark" \
  /usr/bin/time -l scripts/dist-build.sh --artifacts=local \
  --target=aarch64-apple-darwin --tag=v0.78.0 --output-format=json
# Original selection: replace scripts/dist-build.sh with dist build.
```

The discarded backend/demo binaries were not release artifacts. Backend CI,
linked feature builds and real release image/publish builds retain validation.
No CLI feature flags, binary names or packaged contents change.

## Updater image cache portability

BuildKit registry exports do not export Cargo cache mounts. The old updater
Dockerfile kept dependencies only in those mounts and the PR lane omitted its
registry cache import. Broad source copies also invalidated compilation for
unrelated backend and CLI edits.

The revised Dockerfile uses the existing normalized dependency recipe and
ordinary layers. Production and test dependency builds feed separate production
and test compilations. PRs import the per-platform publish cache for both.
Publish exports the `build-inputs` target with both branches; this adds test
compilation on a cold publish runner and makes it reusable by later PRs.
The default image still contains only the static updater and CA bundle.

Local Docker Desktop had eight CPUs/32 GiB; Cargo used four jobs. On Linux
arm64 the original E2E + production builds took 344 + 2 seconds; revised builds
took 271 + 47 seconds. Base-image/cache states differed, so the 28-second total
difference is not a controlled speedup. The stronger evidence is that a fresh
builder imported an exported `mode=max` cache and reused all four Cargo steps
in 4.49 seconds. Registry transfer time and hosted hit rates remain unmeasured.

Before/after production and test executables and CA certificates were
byte-identical. Production SHA-256:
`581ef6bdfa37974a4968caf9d7a67982bd5ffdf25028d64d854a678d9d109690`;
test SHA-256:
`3fb21275c1af7ca130aef6cf3dc38f187cfe775fccd9ecf8b9ea71754cfcab57`.
Actual rustc dep-info verified 34 production and 8 additional test inputs.
The guard checks missing/excluded inputs, manifests and empty evidence, and
keeps test fixtures out of production. Real E2E passed in 356.07 seconds;
all eight production verification checks passed in 91.75 seconds, including
real TUF and image attestations, tmpfs variants and watch-triggered replacement.
The separate opt-in check of the already-published 0.41.0 updater was skipped;
the eight checks exercised the newly built production updater.

```sh
docker buildx build --builder "$BUILDER" -f cli/Dockerfile.machine-updater \
  --target e2e --load -t nyxid-updater-perf:e2e .
docker buildx build --builder "$BUILDER" -f cli/Dockerfile.machine-updater \
  --target production --load -t nyxid-updater-perf:production .
docker buildx build --builder "$BUILDER" -f cli/Dockerfile.machine-updater \
  --target build-inputs --output type=local,dest=target/updater-inputs \
  --cache-to type=local,dest=target/updater-cache,mode=max .
/usr/bin/time -l docker buildx build --builder "$FRESH_BUILDER" \
  -f cli/Dockerfile.machine-updater --target build-inputs \
  --cache-from type=local,src=target/updater-cache \
  --output type=local,dest=target/updater-imported-inputs .
python3 scripts/check-updater-docker-embeds.py \
  --dep-info target/updater-imported-inputs/production \
  --test-dep-info target/updater-imported-inputs/test
MACHINE_IMAGE=ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:0.78.0 \
  UPDATER_E2E_IMAGE=nyxid-updater-perf:e2e sh cli/tests/machine_updater_e2e.sh
UPDATER_PRODUCTION_IMAGE=nyxid-updater-perf:production \
  node cli/tests/machine_updater_production.mjs
```

Use unique builders/tags on a shared machine and remove the resources afterward.
Production verification uses the existing authenticated GitHub environment.
The staging guard requires Python 3.11+ (`tomllib`), provided by CI.

## Rust CodeQL scope

Rust PR analysis now excludes files reached only with `cfg(test)`. The
`codeql_cfg_test_files.py` generator walks Cargo roots, module declarations,
`#[path]`, `cfg_attr` and literal `include!` edges. It considers every value of
non-test feature/platform conditions, so `cfg(any(test, feature = "x"))`
remains in scope. A test-like filename is never sufficient. Mixed production
and inline-test files remain in scope, as do integration-test crate roots.

Unresolved production includes, external modules inside macro/block contexts,
or unreadable production inputs disable exclusions for the entire source tree.
This accounts for possible references across package boundaries. Regression
fixtures include real stable-rustc builds with nested comments, local modules,
macro-generated modules, alternate paths, automatic Cargo targets, packed
module layouts and files with misleading names. Literal paths containing glob
characters are retained. CI's Detect Changes job runs these tests on every PR.
CodeQL generates and uploads the config and exact exclusion list for inspection.

At the measured revision, the generator finds 410 test-only files among 1,896
Rust files. Its final list is identical to the measured list (SHA-256
`476b81bc8cf687d6b42193499e4a4fb84cb74ed2bde3e953680e067f8ca19ac2`). None
of those files appears in the separately collected 1,365-input production
closure from the linked build with both KMS features. That closure is a local
cross-check, not a substitute for reasoning about other platform configurations.

Both local CodeQL runs used CLI 2.27.4, Rust query pack 0.1.46, all 37 default
queries, four threads, a 14,570 MiB RAM budget, build mode `none`, and stable
Rust 1.98.1. Their extractor logs identify that exact project toolchain.
Analysis used `--expect-discarded-cache --min-disk-free=1024`, matching the
Actions cache policy. On the same shared Mac:

| Scope | Extraction / peak RSS | Analysis / peak RSS | Combined |
| --- | --- | --- | ---: |
| Full, 1,896 Rust files | 380.24 s / 13.36 GiB | 1,293.87 s / 10.38 GiB | 1,674.11 s |
| Exclude 410 test-only files | 283.74 s / 11.12 GiB | 456.37 s / 9.89 GiB | 740.11 s |

Combined time fell 55.8%; analysis alone fell 64.7%. This exceeds the 25%
selection threshold. It is a local result, not a promised 55.8% reduction on
Linux. Both runs retained extractor diagnostics: the full run reported five
files with extractor warnings, and the scoped analysis metric reported four
files with extraction errors. Neither run is described as complete extraction
of every source construct. No queries or existing diagnostic checks are removed.

The full analysis reports 182 alerts; the scoped analysis reports 177.
`codeql_compare_sarif.py` resolves positional file, rule and dataflow taxonomy
references before comparing results. All 177 retained results are identical,
including messages, locations, fingerprints, code flows and referenced rule
and model definitions. Exactly these five alerts disappear, with all source
locations inside the excluded files:

| Rule | File | Lines |
| --- | --- | --- |
| `rust/cleartext-logging` | `backend/src/services/billing/config_tests.rs` | 535, 551 |
| `rust/hard-coded-cryptographic-value` | `backend/src/services/node_service/tests/contextual_secrets.rs` | 111, 116 |
| `rust/hard-coded-cryptographic-value` | `backend/src/services/platform_native_direct_tests.rs` | 239 |

The comparison treats CodeQL's own model-definition paths separately from
finding source locations. It does not drop code flows or conceal changes to
model references. Tests change a model reference and a related source location
to verify that both are detected.

An earlier full analysis using the CLI's default persistent-cache policy took
1,655.54 seconds and produced the same 182 alerts. Changing that policy is not
a proposed Actions optimization: Actions already used the faster discard
policy. Query splitting is rejected because the hosted log has one shared
stage from 11m34s to 50m11s, before the security queries finish within about
five minutes. Splitting queries would repeat the dominant shared computation.

### Workflow triggers and result categories

`codeql.yml` always starts on its configured PR, push, schedule and manual
events. Its `changes` job uses `dorny/paths-filter@v4` with `token: ''`, which
selects Git-based detection. The action compares the PR base SHA with the
checked-out merge commit using `git diff --no-renames --name-status -z`.
Checkout fetches the merge commit and both parents; the action fetches a missing
base commit by SHA. A fetch or diff failure cannot produce a successful negative
Rust result.

This avoids both GitHub limits: workflow-level `paths` sees at most 300 changed
files, and the [REST PR-file listing](https://docs.github.com/en/rest/pulls/pulls#list-pull-requests-files)
returns at most 3,000. The action's Git mode does not call that listing or impose
a file-count cap. Its [PR dispatch](https://github.com/dorny/paths-filter/blob/ceb8a2b8f2d89434be7ff52d3de7ec3738c5cc9d/src/main.ts#L90)
and [Git diff implementation](https://github.com/dorny/paths-filter/blob/ceb8a2b8f2d89434be7ff52d3de7ec3738c5cc9d/src/git.ts#L55)
were inspected at v4 commit `ceb8a2b8f2d89434be7ff52d3de7ec3738c5cc9d`.
The real bundled action was also executed locally in disposable repositories:

| Changed set or error | Action result | Selected languages |
| --- | --- | --- |
| 3,100 Markdown files | success, `rust=false` | Actions, JavaScript/TypeScript, Python |
| Those files plus `z.rs` at position 3,101 | success, `rust=true` | All four, including Rust |
| Rename `z.rs` to `z.md` | success, `rust=true` from the deleted path | All four |
| Base SHA unavailable and fetch fails | failure, no Rust output | All four |

The checked test suite builds the same 3,101-file Git fixture, verifies that
neither the first 300 nor the first 3,000 includes Rust, and checks the full
diff and rename. Decision tests cover every combination of event, step outcome
and recognized/missing/invalid Rust output. Rust is omitted only for a PR whose
detection succeeds and explicitly returns `false`. Non-PR and unexpected events
always include Rust. A missing base SHA skips detection and includes Rust;
a failed detection job or missing matrix output also defaults to all four.

Filters cover every Rust source, Cargo manifest/lockfile, `rust-toolchain*`,
nested `.cargo/`, and CodeQL workflow/config/helper. All languages share job id
`codeql` with only the `language` matrix variable. There is no explicit category
input, so each retains its historical default category. There is one upload
per analysis and no duplicate full-scope category. Push to `dev`, weekly schedule
and manual runs analyze all four languages, with full Rust scope. Only Rust PR
analysis adds the generated test-only exclusions.

The generator uses the runner's `python3` without a package install. In run
43210236259, Detect Changes job 131155785727 ran 133 tests successfully on the
stock Ubuntu 24.04 image, including YAML-parsing workflow tests that require
PyYAML. Backend Image Inputs job 131155970598 ran all eight dependency-recipe
tests, whose top-level import requires `tomllib`. Neither job used setup-python
or installed Python packages. Recheck the evidence with:

```sh
gh run view 43210236259 --job 131155785727 --log
gh run view 43210236259 --job 131155970598 --log
```

CodeQL only generates its config; the helper suites already run in CI's Detect
Changes job. An exception from the generator fails the Rust analysis before
initialization, and a regression test verifies that no config is published on
that path. Conservative unresolved-module results are still logged and retain
full scope, as described above. Scope artifacts use `upload-artifact@v4` like
the other workflows.

A scoped PR analysis compared with a full base shows alerts in excluded test
files as fixed in that PR's view (five in the measured snapshot). PR analysis
does not change their state on the default branch; its full scans still report
them. The duplicate category prevented no additional failure and was removed.
A non-Rust PR can still receive GitHub's missing-Rust-analysis notice because
its base has a Rust category; [upstream discussion](https://github.com/github/codeql-action/issues/1179)
records neutral results for missing configurations. No new category is missing
on Rust PRs. Actual aggregate check conclusions and default-branch alert state
require the [hosted validation checklist](#github-actions-confirmation-checklist).

Reproduce the paired measurements with the pinned CodeQL bundle on PATH:

```sh
mkdir -p target/codeql-benchmark/source
git archive HEAD | tar -x -C target/codeql-benchmark/source
python3 .github/scripts/codeql_rust_scope.py config --event push \
  --output target/codeql-benchmark/full.yml
python3 .github/scripts/codeql_rust_scope.py config --event pull_request \
  --output target/codeql-benchmark/pr.yml
for scope in full pr; do
  /usr/bin/time -l env RUSTUP_TOOLCHAIN=1.98.1 codeql database create \
    "target/codeql-benchmark/$scope-db" --language=rust --build-mode=none \
    --source-root=target/codeql-benchmark/source --threads=4 --ram=14570 \
    --codescanning-config="target/codeql-benchmark/$scope.yml"
  /usr/bin/time -l codeql database analyze "target/codeql-benchmark/$scope-db" \
    codeql/rust-queries:codeql-suites/rust-code-scanning.qls \
    --threads=4 --ram=14570 --format=sarif-latest \
    --output="target/codeql-benchmark/$scope.sarif" \
    --rerun --expect-discarded-cache --min-disk-free=1024
done
python3 .github/scripts/codeql_compare_sarif.py \
  target/codeql-benchmark/full.sarif target/codeql-benchmark/pr.sarif \
  target/codeql-benchmark/pr.excluded.txt --output target/codeql-benchmark/comparison.json
```

Use a source snapshot containing the changes under measurement. `git archive
HEAD` reproduces a committed revision, so it omits uncommitted source edits.
Delete the benchmark source and database directories after collecting results.

## Local full validation

There is no single checked-in local full-gate command. `scripts/pre-commit`
runs formatting and workspace Clippy for staged Rust changes and ESLint for
staged frontend changes. CONTRIBUTING additionally requires tests and frontend
build/lint. WORKFLOW.md configures Symphony orchestration. Backend validation
also needs linked KMS builds, the Docker input guard, RCI boundary check and
coverage. A successful pre-commit hook alone does not establish these results.

The test Compose override provides a disposable replica set with the CI tmpfs,
2 GiB WiredTiger cache, zero snapshot-history window and fault-command settings.
Journaling and transactions remain enabled. Listening, published and advertised
replica ports match, including tests using replica discovery. CONTRIBUTING gives
startup and cleanup commands. MongoDB was not downloaded outside Docker.

A matched ABBAABBA experiment used fresh volume and tmpfs replica sets with
otherwise identical settings. The
`pool_proxy_legacy_round_robin_remains_single_attempt` test took
3.666/3.569/3.482/3.521 seconds on volume (mean 3.560), and
2.345/2.311/2.301/2.392 on tmpfs (mean 2.337). That 34.3% sample improvement is
not a full-suite speedup estimate. Reproduce by running that nextest filter
against two dedicated Compose projects, overriding only `/data/db` and
`/data/configdb` storage in one project, alternating in that order, then
removing both projects and volumes.

Before review round 1, ordinary execution passed 12,369 tests in 2,350.314 s
(14 ignored, zero retries); instrumented execution passed 12,353 in 2,483.566 s
(12 ignored, zero retries). `backend_shards.py --count 1` verified every runnable
binary/test ID exactly once. Archive-selected coverage was 688,734 / 768,399
lines (89.632339%), passing the unchanged 73% bar, with JSON and LCOV exports.
After replacing the Git-stamp wrapper, the ordinary rerun passed all 12,369
runnable tests in 2,545.168 seconds, with the same 14 ignored tests. The inventory
still contains exactly the original 12,383 binary/test IDs. One test,
`routes::machine_external_runtime_tests::local_journey_external_http_node_and_oauth_refresh`,
passed on its configured second attempt; its first attempt reported
`machine_permission_revoked` and a MongoDB duplicate-key command error. This was
not a zero-retry run. The fresh JUnit and `backend_shards.py` independently
confirmed the final successes and named that retry.

The instrumented rerun passed all 12,353 runnable tests in 2,252.382 seconds
with the original 12 ignored tests and exactly the same 12,365 inventory IDs.
`handlers::assistant_group::tests::shared_agents_in_personal_crews_list_route_handoff_and_revalidate`
passed on retry after a 67.5-second first attempt ended with `database_error`.
Fresh JUnit and shard verification identify that retry. The final report covers
688,737 of the same 768,399 lines (89.632730%); the 73% threshold, JSON export
and LCOV export all passed. The three-line difference from the earlier run is
execution coverage, not a changed denominator. Both owned Compose projects
were removed with their volumes after execution.

Some successful macOS links emit the existing `__eh_frame section too large`
diagnostic. Rust's `linker_messages` lint explicitly ignores `-Dwarnings`;
these are not warning-free logs. A dependency also reports future
incompatibility for proc-macro-error2. The `-Dwarnings` and Clippy
`-D warnings` bars remain enabled.

## Local verification

The first review round's Rust verification used stable 1.98.1, `RUSTFLAGS=-Dwarnings`, four
Cargo jobs and the existing line-table debug settings. These were warm local
checks, not cold hosted speedup estimates:

| Check | Elapsed | Peak RSS | Result |
| --- | ---: | ---: | --- |
| Workspace `cargo fmt --all -- --check` | 12.51 s | 0.92 GiB | Passed |
| Workspace Clippy, all targets, `-D warnings` | 78.75 s | 17.30 GiB | Passed |
| Linked backend `aws-kms` | 47.42 s | 11.40 GiB | Passed |
| Linked backend `gcp-kms` | 46.97 s | 11.38 GiB | Passed |
| Linked backend `aws-kms,gcp-kms` | 41.54 s | 11.42 GiB | Passed |

That round passed 182 GitHub helper tests and 36 script helper tests, including
real Cargo Git-layout and input-closure fixtures. The instrumented archive
inventory has the original 12,365 IDs, with no additions or removals. Ordinary
and instrumented execution were both rerun because the initial build-script
change changes the embedded identity and thus the test executable, even though
runtime sources are unchanged.

The second review round passed 185 GitHub helper tests in 21.770 seconds and
38 script helper tests in 22.265 seconds, with no skips. The separate
dependency-recipe suite passed eight tests. The reusable-workflow permission
check validated all four calls, the RCI boundary check passed, and actionlint
1.7.12 passed the complete workflow directory. Workspace formatting and
`git diff --check` also passed.

Only `backend/build.rs` and `cli/build.rs` changed among Rust files in the
second round. Their change replaces Git's newer path-format option with local
resolution of the same Git paths. The real Cargo fixtures verify the stamps
and rebuild behavior; no runtime code, features or test selection changed.
The full backend suites, linked feature builds and workspace Clippy were not
repeated in this round. Their first-round results above remain the evidence
for those checks.

The Git fixture suite passed all five tests on macOS in 23.783 seconds. It also
ran in the actual linux/arm64 `buildpack-deps:focal` image with Git 2.25.1 and
Rust 1.98.1: four tests passed in 4.059 seconds, and the macOS-only SDK test was
skipped. Both build scripts were exercised with plain Cargo through no-change
builds, HEAD movement, linked worktrees, detached HEAD, packed refs, no Git and
the backend override. A shim rejects `--path-format` to keep this regression
covered on developer machines with newer Git. The Docker input guard has its
own old-Git path-resolution fixture.

The third round added a regression with reflogs disabled before the first
commit and the branch ref packed before the first build. Both old build
scripts retained the original stamp after the next commit. Both corrected
scripts pass: they rerun while the ref is packed, refresh the stamp after the
commit, and reuse the next unchanged build once the loose ref exists. The
normal-worktree and no-Git no-change assertions also still pass.

The third round's real-workspace verification used the same stable toolchain,
warning bars, four Cargo jobs and line-table debug settings:

| Check | Elapsed | Peak RSS | Result |
| --- | ---: | ---: | --- |
| Real workspace Clippy, all targets, from an empty target directory | 327.21 s | 25.23 GiB | Passed |
| First real `cargo build -p nyxid-cli`, after Clippy | 63.63 s | 2.35 GiB | Passed; compiled `nyxid-cli` |
| Identical second CLI build | 0.21 s | 174 MiB | Passed; no `Compiling` lines |

The final helper runs passed 185 CI tests in 24.479 seconds and 39 script tests
in 26.828 seconds, with no skips. Actionlint 1.7.12, the four-call permission
check, workspace formatting, eight recipe tests, the RCI boundary check and
`git diff --check` passed. The existing `proc-macro-error2` future-compatibility
notice remains in the Cargo logs. No runtime Rust source changed.

The fourth round replaced the Git-path panic with the missing sentinel above.
The build-script suite passed seven tests in 33.526 seconds, including failed
Git commands and non-UTF-8 output for both packages, successful builds and
fresh stamps after a commit. The Docker guard suite passed four tests in
1.241 seconds, including rejection of other missing paths with the same
sentinel name. Formatting and `git diff --check` passed. Clippy was not repeated:
the only Rust change is the fallback expression and its comment, and the
fixtures compile both actual build scripts with stable 1.98.1 and `-Dwarnings`.

Reproduce the real-workspace checks from the repository root:

```sh
export RUSTUP_TOOLCHAIN=1.98.1 CARGO_BUILD_JOBS=4 RUSTFLAGS=-Dwarnings CARGO_INCREMENTAL=1
export CARGO_TARGET_DIR="$PWD/target"
export CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_PROFILE_TEST_DEBUG=line-tables-only
/usr/bin/time -l cargo clippy --workspace --all-targets -- -D warnings
/usr/bin/time -l cargo build -p nyxid-cli
/usr/bin/time -l cargo build -p nyxid-cli
```

Reproduction commands (Python 3.11+ with PyYAML 6.0.2; pinned nextest and
cargo-llvm-cov on PATH):

```sh
export RUSTUP_TOOLCHAIN=1.98.1 CARGO_BUILD_JOBS=4 RUSTFLAGS=-Dwarnings CARGO_INCREMENTAL=1
export CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_PROFILE_TEST_DEBUG=line-tables-only
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked -p nyxid --features aws-kms
cargo build --locked -p nyxid --features gcp-kms
cargo build --locked -p nyxid --features aws-kms,gcp-kms
python3 -m unittest discover -s .github/scripts -p 'test_*.py' -v
python3 -m unittest discover -s scripts -p 'test_*.py' -v
python3 scripts/test-docker-dependency-recipe.py
python3 scripts/check-reusable-workflow-permissions.py
bash scripts/check-rci-backend-boundary.sh
actionlint
git diff --check
```

Reproduce the Focal build-script check from the repository root. All tool
installation and fixture builds occur inside the disposable container:

```sh
docker run --rm -i --platform linux/arm64 \
  --mount "type=bind,src=$PWD,dst=/workspace,readonly" \
  buildpack-deps:focal sh -s <<'SH'
set -eu
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends python3
curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o /tmp/rustup-init.sh
export CARGO_HOME=/tmp/nyxid-ci-cargo RUSTUP_HOME=/tmp/nyxid-ci-rustup
sh /tmp/rustup-init.sh -y --no-modify-path --profile minimal --default-toolchain 1.98.1
export PATH="$CARGO_HOME/bin:$PATH"
git --version
rustc --version
python3 -m unittest discover -s /workspace/scripts -p test_git_build_scripts.py -v
SH
```

The measured image digest was
`sha256:2de32588f72f63229c83e3f08529cc15db231b2d95412ab4c30ad49d7cd99591`.
Remove the image afterward if the check was its only local user.

Use the dedicated Compose replica set from CONTRIBUTING for execution. With
`RUST_MIN_STACK=8388608`, run `cargo nextest run --profile ci --test-threads 4`
against the ordinary archive and `cargo llvm-cov nextest --no-report --profile
ci --test-threads 4` against the instrumented archive. Pass `--archive-file`
and `--workspace-remap "$PWD"` to both. Create the llvm-cov extraction directory
first when using a fresh `CARGO_TARGET_DIR`; nextest requires its destination
to exist. Save each inventory with `cargo nextest list --archive-file ...
--profile ci --message-format json`, collect the fresh CI JUnit, and verify it
with `.github/scripts/backend_shards.py --count 1`. Report with
`cargo llvm-cov report --nextest-archive-file ... --package nyxid
--fail-under-lines 73`, followed by JSON and LCOV exports. Remove the Compose
project with `down -v` even on failure.

Cleanup removed all measurement targets, CodeQL databases, tools and evidence
under the scratch directory, the nextest extraction output, and 368 untracked
coverage profiles. `target/` is now 4 KiB. No owned build/test processes,
temporary archive directories, Docker containers or volumes remain. The
dedicated builders, custom images and known test-only image download were
removed; shared Docker builders and pre-existing base images were preserved.
The second round's scratch tools/logs and newly pulled Focal image were also
removed after verification; its disposable container left no volumes.
The third round's 9.9 GiB of build output and scratch tools/logs were removed
after the final Clippy and CLI checks.
The measurements above and their reproduction commands are self-contained;
the document does not depend on the deleted logs.

## Change map

| Files | Purpose |
| --- | --- |
| `.github/actions/ci-rust-cache/action.yml`, `.github/actions/ci-rust-incremental-save/action.yml` | One save/restore key and path contract, explicit manual-ref save policy, manifest fallback. |
| `.github/workflows/ci-cache-warm.yml`, `.github/scripts/test_ci_cache_warm.py` | Matching producers, resource measurements, owned swap release before saves, contract regression checks. |
| `.github/scripts/cache_budget.py`, `.github/scripts/test_cache_budget.py`, `.github/workflows/cache-maintenance.yml`, `docs/ci/ACTIONS_CACHE_BUDGET.md` | Preserve ref-scoped caches until a restorable main replacement exists, including cache versions and prefixes. |
| `.github/scripts/prepare-backend-runner.sh`, `.github/scripts/test_prepare_backend_runner.py` | Release only the helper's own swap reserve after compilation. |
| `backend/build.rs`, `cli/build.rs`, `scripts/test_git_build_scripts.py` | Plain Cargo freshness, real per-worktree Git watches and SDK-sensitive macOS linker paths. |
| `.github/workflows/backend-tests.yml`, `.github/scripts/test_coverage_archive_cache.py` | Reuse instrumented compiler artifacts while resetting counters and selecting current archive objects. |
| `.github/workflows/ci.yml`, `scripts/check-backend-docker-embeds.py`, `scripts/test_backend_docker_embeds.py` | Release-check image inputs, fail-closed current-unit collection, real build/check closure fixture, Git-stamp fixture wiring and updater cache imports. |
| `Cargo.toml`, `scripts/dist-build.sh`, `.github/scripts/dist_cargo.py`, `.github/scripts/test_dist_cargo.py`, `.github/workflows/release.yml`, `.github/workflows/release-cache.yml` | Compile only cargo-dist's distributed binary while retaining workspace feature unification. |
| `cli/Dockerfile.machine-updater`, `scripts/check-updater-docker-embeds.py`, `scripts/test_updater_docker_embeds.py`, `.github/workflows/publish-images.yml` | Portable production/test build layers and exact Docker input validation. |
| `docker-compose.yml`, `docker-compose.test.yml`, `CONTRIBUTING.md` | Disposable local test replica set, Git-stamp semantics and current validation contracts. |
| `.github/workflows/codeql.yml`, `.github/codeql/codeql-config.yml` | Uncapped Git change detection, conservative language selection, generated Rust PR scope and historical default categories. |
| `.github/scripts/codeql_cfg_test_files.py`, `codeql_rust_scope.py`, `codeql_compare_sarif.py` and their `test_codeql_*.py` files | Conservative module-tree traversal, language/config contracts and alert-equivalence verification. |
| `docs/CI_PERFORMANCE.md` | Reproducible measurements, rejected alternatives, verification and limits. |

No backend or CLI runtime source is changed. The Git-stamp wrapper and its
workflow/local prefixes were removed; plain Cargo is the supported interface.
There is no crate split, binary rename, KMS feature change or test-ID migration.

## GitHub Actions confirmation checklist

No workflow was triggered during this work. This is the complete remaining
hosted checklist; the local measurements above are not hosted speedup claims.
Record the run IDs, cache save/restore messages, elapsed times, resource samples
and check conclusions for each applicable item. Windows is not a cargo-dist
target in this repository.

### Before merge

1. Dispatch **CI Cache Warm** twice on the same non-main ref. For all ten lanes,
   verify the first run's dependency and incremental saves where applicable,
   then the second run's actual hits and resource logs. Check the two-path
   incremental version and ordinary backend save after owned swap release.
   Repeat after a small Rust edit to measure useful incremental recompilation.
   Confirm the branch cache entries remain until compatible main entries exist;
   a successful job alone does not prove a successful cache save.
2. Run this PR's **CI** gate. All eight ordinary and eight instrumented shards
   must pass inventory verification; the 73% threshold and existing base/head
   comparison must run. Verify the unchanged **Backend Image Inputs** name,
   release/GCP check and successful complete-input guard on Linux. Verify all
   linked KMS builds, **Backend Release Image**'s optimized amd64 link, and
   **Machine Container E2E** including the production updater checks.
3. Run **CodeQL** on a Rust-changing PR and on a non-Rust-only PR carrying this
   workflow. Confirm the dynamic matrix includes Rust only in the first,
   the other three languages always run, and categories match historical base
   categories without explicit overrides. Confirm no blocking pending check;
   inspect new production alerts normally, plus the expected PR-only fixed
   test alerts. Download the Rust scope artifact and inspect its generated
   exclusion list. Measure extraction and Analyze time on the standard runner.
   The local large-diff and failed-fetch fixtures prove the selection logic;
   a hosted PR with over 3,000 changed files, if used, must also include a late
   Rust change and select Rust.
4. Dispatch **CodeQL** on the branch to verify non-PR full scope: all four
   languages, an empty added-exclusion list, all Rust queries, and only the
   historical category per language. This proves the full-scan workflow on a
   branch, not default-branch alert state or the schedule trigger.
5. Use this PR's **Release Build Cache** run (or dispatch it on the branch) to
   confirm actual cargo-dist builds for `aarch64-apple-darwin`,
   `x86_64-apple-darwin`, `aarch64-unknown-linux-gnu` in Focal, and
   `x86_64-unknown-linux-gnu`. Require executed **Build artifacts** steps and
   successful binary selection/packaging on each target; cache-hit-only jobs
   are not new packaging evidence. A skipped build without a prior successful
   build of this change leaves this pre-merge item unconfirmed.

Do not use **Publish Images** as a pre-merge dry run: its manual trigger really
publishes images and registry caches. Its main-only behavior is checked below.

### After merge

1. Inspect the first normal main **Publish Images** run. Gate reuse must accept
   only successful jobs on the matching tree; every unavailable validation
   must execute. Both backend architectures must complete optimized links,
   smoke checks and manifest publication. Verify the coverage baseline is
   published with the unchanged fingerprint/denominator contract. Cold
   monolithic release compilation still exceeds 16 GiB locally; no hosted
   cold-build speedup is claimed for it.
2. Confirm main **CI Cache Warm** publishes restorable dependency/incremental
   entries. Inspect trusted **CI Cache Maintenance** output: compatible main
   entries replace branch generations, and incompatible versions do not.
   After seven unused days from their last old-writer save/access, confirm
   that GitHub evicts the obsolete one-path entries. The approximately 9 GiB
   temporary cost and eviction policy are documented in
   [Actions cache retention](ci/ACTIONS_CACHE_BUDGET.md).
3. Confirm **Publish Images** exports the updater `build-inputs` registry cache
   for amd64 and arm64. On a subsequent build with unchanged updater inputs,
   verify import from a fresh runner/builder reuses all four Cargo layers on
   both architectures; confirm the next PR's machine lane imports its cache.
   This checks GHCR authentication and transfer behavior not measured locally.
4. Inspect the first scheduled **CodeQL** run on the default branch. All Rust
   files and queries must remain covered, test-file alerts must still exist
   on that branch, and no duplicate category may appear. A manual default-
   branch run can confirm alert state earlier; only the scheduled event proves
   schedule wiring.
5. At the next approved tagged **Release**, confirm the full platform matrix,
   global packaging, integrity/attestation artifacts and release upload. The
   branch build proves compilation/packaging; it does not prove tag-triggered
   publication permissions or artifact delivery.

The worktree remains uncommitted. No push, PR creation, workflow dispatch,
remote cache deletion or remote configuration change was performed.
