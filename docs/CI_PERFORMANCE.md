# CI performance and validation contracts

Initial local measurements were taken on 2026-11-12–13 at
`31f3d8969e051e1fe6ed2f42a5594ded90606fe5`. Hosted validation below uses
PR #2428 at `df20e4869` and its PR merge commit.
Local compiler measurements used stable Rust 1.98.1, an 18-core/128 GiB Apple
Silicon Mac and four Cargo jobs. This is a shared machine. These observations
are not Linux runner benchmarks or confidence intervals. Peak RSS below is
`/usr/bin/time -l` maximum resident set size, converted from bytes to GiB.

## Hosted validation and remaining measurements

These are observed standard-runner results, not projections. Read them with
`gh run view <run> --json jobs` and `gh run view <run> --job <job> --log`.
PR CI **43310337142** passed in about 79 minutes cold. CodeQL was
**43310337099**, Release Build Cache **43310337092**, and the two manual warm
runs were **43310342885** (cold) and **43323188823** (same commit). These
hosted measurements predate the round-six archive policy: the 301-second
same-commit coverage result is not the expected behavior of the final
non-incremental archive jobs. The other seven lanes retain graph reuse.

| Work | Cold / old | Exact-commit warm / new |
| --- | ---: | ---: |
| Backend Image Inputs, PR job | 26–39 min before this PR | 4.9 min |
| CodeQL Rust, job | 65.9 min full scope (43323193873) | 28.3 min PR scope (43310337099) |
| Warm ordinary backend compilation | 2,279 s wall / 1,476 s CPU | Restore failed with ENOSPC; no warm measurement |
| Warm backend coverage compilation | 3,440 s wall / 2,058 s CPU | 301 s wall / 175 s CPU / 14.3 GiB peak RSS |
| Warm Clippy | 1,606 s | 311 s |
| Warm GCP build | 1,103 s | 83 s |
| Warm AWS + GCP build | 1,110 s | 78 s |

Cold ordinary job **131504474372** and cold coverage job **131504474644**
saved their incremental archives successfully after releasing swap. Exact-hit
coverage **131548887185**, Clippy **131548887191**, GCP **131548887210** and
both-KMS **131548887217** establish reuse on the same commit. They do not
establish reuse for an arbitrary source change or a main-to-PR transfer.

Ordinary warm job **131548887478** exhausted disk during restoration; the
check-run annotation records `System.IO.IOException: No space left on device`
while writing the worker log, and the job log was never uploaded. The old
0.76.0 fallback archive was 8,096,551,974 compressed bytes; saving it alongside
the new 0.78.0 graphs produced about 10,036 MiB. The cold job reported a
145 GiB filesystem with 100 GiB used and 45 GiB free **after** releasing its
37 GiB swap file: only about 8 GiB had remained with that file present.
Coverage archives similarly grew from about 2,792 to 5,096 MiB. A successful
build does not establish that its next restore fits.

Maintenance **43312760683** ran during the first dispatch and deleted several
branch entries. Consequently CLI tests, AWS-only and permissions missed in the
second run, and CLI coverage fell back to main. Those are not warm benchmarks.
Release Build Cache executed packaging only for Linux arm64; three exact hits
skipped it. The revised PR conditions require all four builds to execute.

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

The final policy disables graphs for both backend archives after the
[eight-commit replay](#eight-commit-incremental-replay). Seven other lanes keep
graph reuse. The two archive warm jobs still compile and save dependencies.

`ci-rust-cache` now defines the incremental key and ordered path list once.
Its restore consumes that contract, and every writer passes the same outputs
to `ci-rust-incremental-save`. Tests check each writer's key/path expression
and execute the contract's shell derivation. No writer restates those values.

The broad lane/OS/architecture restore fallback is retained. An unrelated
member manifest or lock entry can change the workspace hash without changing
a KMS/Clippy dependency graph. Removing the fallback would force those builds
cold. A package-version change may instead waste 3–6 minutes downloading an
unusable graph. That bounded cost is preferable to losing reusable graphs;
Cargo remains responsible for dependency and metadata validity. No global
package-version segment is added because unrelated member version changes
would invalidate it too.

Only `ci-cache-warm.yml` opts into ordinary dependency and incremental saves.
Push saves are main-only; manual dispatch can save on its selected branch.
Actions provides ref scoping. Readers remain read-only. Concurrency is per
ref, so a branch measurement does not cancel main warming. Retention protects
a branch cache for at least 48 hours from creation, and longer when no
compatible main replacement exists. Both cache kinds must match prefix and
cache version. Missing, malformed or future timestamps are preserved. The old one-path main caches cannot evict new two-path branch caches.
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
Cold warm run 43310342885 confirmed the recovery and all nine incremental
uploads. Restore capacity is a separate contract described below.

### Restore disk budget and graph pruning

Archive jobs now skip graph restoration entirely. This admission budget still
protects the seven remaining incremental lanes; the archive jobs retain the
swap allocation/build-space check and dependency restore diagnostics.

Restoration now precedes swap allocation. Before dependency restore, only
jobs that allocate the swap reserve opt into `ci-rust-cache`'s `reclaim-disk`
input: backend archives, Clippy, KMS features and backend image inputs, including
their warm jobs. The input defaults to false. On hosted Linux runners it
removes unused Android, .NET, GHC and ghcup SDK directories and logs `df`.
Permissions, CLI tests, wizard freshness and CLI coverage skip this step;
developer machines never have their SDKs removed. Removal or reader `df`
failures log warnings and continue. The next check measures actual free bytes
**after** dependency extraction, so runner
image growth and the dependency target are charged to the budget rather than
assumed constant. On a 145 GiB filesystem, restoring a graph requires the
image, checkout, tools and extracted dependencies together to occupy at most
about **56 GiB** (145 minus 89). Their separate sizes vary with runner images;
there is no measured fixed image-baseline claim.

The first completed job available from the swap experiment was permissions,
run **43344390346**, job **131620090266**. Its old unconditional cleanup took
**47.65 s** between the `df` samples at `01:22:30.8348853Z` and
`01:23:18.4840545Z` on 2026-11-13. Available space rose from **86G to 107G**
(approximately **21 GiB**, rounded by `df -h`; used space read 59G then 39G).
This interval brackets SDK removal and the second `df`, not cache download.
The opt-in removes that work from this small lane. Reproduce with:

```sh
gh run view 43344390346 --job 131620090266 --log
# While the overall run is still in progress, a completed job's log is available via:
gh api --allow-escape-sequences repos/ChronoAIProject/NyxID/actions/jobs/131620090266/logs
```

| Reserved use after dependency restore | GiB |
| --- | ---: |
| Extra disk swap above the runner's existing 3 GiB | 37, rounded up to MiB plus 1 MiB |
| Extracted incremental graph, enforced at save | at most 16 |
| Compressed download while extracting | at most 17 |
| Build growth, including a successor graph and linked output | 24 |
| Nextest archive | 4 |
| Free-space floor | 8 |

The download disappears before compilation, so it is not added to build
usage. Required free space is the maximum of `16 + 17 + 8 = 41 GiB`
for extraction and `37 + 16 + 24 + 4 + 8 = 89 GiB` for compilation (plus
swap rounding to MiB and a 1 MiB margin). An insufficient budget **skips the optional incremental
restore before download**, then builds normally. The reserve script separately
requires swap plus 36 GiB remaining build/archive/floor capacity before
allocating. It still establishes at least 40 GiB total swap or fails before
compilation. After the reserve is released, a save needs 25 GiB for the
worst-case archive and free-space floor. These are enforced admission budgets;
24 GiB build growth is a provision, not a hard bound on arbitrary future Rust
source growth. Opted-in cleanup, extraction admission and save preflight print
`df`; resource samples include free bytes. The actual free-space guards remain
authoritative even when optional cleanup or a reader diagnostic fails.

The new graph prefix is `v1-rust-graph`; it never restores old unbounded
`v0-rust-incremental` archives. The dependency/artifact prefix remains
`v1-rust-incremental`; non-incremental lanes retain `v0-rust`. Native rustc
storage and compiler arguments do not require an artifact-cache generation
change. Release dist keys are unchanged. Only the bounded graph prefix needs
a cold seed. Main's old maintenance script does not recognize that prefix;
it can still delete branch dependency entries until the grace rule merges.

`incremental_cache.py mark` records a timestamp after restoration and before
any Cargo command. On a fresh CI checkout, source mtimes are newer than
restored dep-info, so selected workspace units finalize new sessions. Before
save, `prune` removes each native crate directory with no finalized session
newer than that marker. A single marker covers all Cargo commands in the job.
There is no rustc wrapper, path rewriting, or Cargo debug-log parsing.

Within each retained crate, pruning keeps only the finalized session with the
greatest timestamp encoded in its name, matching rustc 1.98.1 selection. It
acquires every session lock before deleting older or unfinished sessions; a
busy lock or unknown layout refuses the save. Rustc can leave its previous
input session after compilation because it was locked during GC. The real
Cargo fixture verifies removal of obsolete package versions, recompilation
from fresh-checkout mtimes, preservation across multiple Cargo commands, and
an unchanged build after pruning that compiles nothing. A new marker on a
locally fresh target produces no sessions and refuses an empty save; these
writers only run after fresh Actions checkouts.

Pruning measures logical bytes plus 4 KiB per entry, counting hardlinks more
than once and rejecting symlinks. It refuses missing markers, empty graphs
and saves above 16 GiB. That bound, with zstd overhead and tar headers covered
by the 17 GiB download allowance, makes extraction admission independent of
compression ratio. Inventories print native crate/session directories and
sizes after graph restore, after each wrapped build, and before/after pruning.
The restore inventory step is skipped when `incremental` is false. Reader
inventories are diagnostic only: malformed directories, symlinks, failed `df`
or filesystem errors produce warnings and preserve the compiler's exit code.
Pruning and writer preflight remain strict and refuse an unverified save.
Broad manifest fallback remains, but obsolete crate directories and prior input sessions
cannot accumulate across saved generations.

The native version-change fixture shrank from **1,964,788** to **982,394 bytes**
(including the conservative per-entry allowance), retaining two of four crate
directories. The instrumented fixture compares current test identities and
coverage against a clean build while obsolete binaries remain in the target;
its report is identical. A separate non-incremental rebuild preserves every
coverage field except compiler-generated function symbol names, and every
test ID. Run these checks with:

```sh
python3 -m unittest discover -s .github/scripts -p 'test_incremental_cache.py'
python3 .github/scripts/test_coverage_archive_cache.py
```

The full native graph was remeasured on `da70dddc9` with the reviewed build
scripts copied into the disposable snapshot, the same ordinary archive
command and four jobs. A cold workspace graph with warm dependencies took
**460.23 s / 39.34 GiB**; a comment edit plus fresh-checkout mtimes took
**50.28 s / 20.72 GiB**. After initializing the normal detached HEAD reflog,
a fresh-mtime rebuild took **49.38 s / 20.56 GiB**. Pruning then reduced
**17,574,359,360 bytes (16.37 GiB)** to **8,787,202,332 bytes (8.18 GiB)**.
The tar/zstd archive is **2,477,815,736 bytes (2.31 GiB)**. The following plain
Cargo archive command took **0.75 s / 272.78 MiB**, with no `Compiling` line.
Its **12,452 binary/test IDs** match both the pre-prune archive and the final
non-incremental reference, including `nyxid::bin/nyxid-server`.

The synthetic snapshot initially lacked a HEAD reflog. In that case an
unchanged build correctly took 48.10 s because the build scripts deliberately
rerun. The freshness result above uses a real HEAD reflog; pruning did not
invalidate the retained graph. These measurements verify the pruner even
though the final archive policy no longer uploads this particular graph.

Reproduce the size check in the disposable clone from the replay recipe.
Use the reviewed `backend/build.rs` and `cli/build.rs`, a normal Git checkout
with a HEAD reflog, and the same environment with `CARGO_INCREMENTAL=1`.
Build once, record a marker, then touch the tracked regular files and add one
comment to the snapshot's `backend/src/main.rs`. Build again, prune, and repeat
the identical command; the last invocation must compile nothing. For a custom
scratch target, call the real helper with its roots set explicitly:

```python
import importlib.util, os, shutil, subprocess
from pathlib import Path
study = Path('target/incremental-replay').resolve()
source, target = study / 'source', study / 'build'
subprocess.run(['git', '-C', str(source), 'reflog', 'exists', 'HEAD'], check=True)
for name in ('backend/build.rs', 'cli/build.rs'):
    shutil.copyfile(name, source / name)
spec = importlib.util.spec_from_file_location('cache', '.github/scripts/incremental_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)
cache.ROOTS = (target / 'debug/incremental',)
cache.MARKER = study / 'build-start'
env = os.environ.copy()
for name in ('RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'RUSTC_BOOTSTRAP',
             'CARGO_LOG', 'CARGO_ENCODED_RUSTFLAGS', 'NYXID_GIT_HASH', 'CARGO_BUILD_TARGET'):
    env.pop(name, None)
env.update(RUSTUP_TOOLCHAIN='1.98.1', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='1',
           CARGO_TARGET_DIR=str(target), RUSTFLAGS='-Dwarnings', RUST_MIN_STACK='8388608',
           CARGO_PROFILE_DEV_DEBUG='line-tables-only', CARGO_PROFILE_TEST_DEBUG='line-tables-only')
def build(label):
    subprocess.run(['/usr/bin/time', '-l', 'cargo', 'nextest', 'archive',
                    '-p', 'nyxid', '-p', 'nyxid-service-adapters', '--timings',
                    '--archive-file', str(study / (label + '.tar.zst'))],
                   cwd=source, env=env, check=True)
build('pruning-cold')
cache.mark()
for name in subprocess.check_output(['git', 'ls-files', '-z'], cwd=source).split(b'\0'):
    if name:
        file = source / os.fsdecode(name)
        if file.is_file() and not file.is_symlink():
            file.touch()
with (source / 'backend/src/main.rs').open('a') as output:
    output.write('\n// Incremental pruning measurement.\n')
build('pruning-edited')
cache.prune()
build('pruning-fresh')
```

`tar --zstd -cf <archive> -C <target>/debug incremental` measures compressed
size. Compare `cargo nextest list --archive-file <archive> --message-format json`
binary/test pairs before and after pruning and against the non-incremental
archive. This compiles and inventories tests; it does not execute MongoDB tests.

Readers set `CARGO_INCREMENTAL=0` when no graph was restored, including when
the disk guard skipped extraction. Exact and prefix hits both count as a
restore. Only the seven graph-producing warm lanes enable incremental on a miss to
create a reusable graph. The two backend archives stay non-incremental even
when an old graph is available. The selected policy is printed in every cache step.

`retry_incremental.py` retries once after exits 137/143, killed compiler
children, allocation/OOM messages or stale incremental errors. It deletes
both graph roots and sets `CARGO_INCREMENTAL=0` for that retry and subsequent
Actions steps. Thus later plain-Cargo test steps reuse the fallback binary
instead of switching modes and rebuilding it. Ordinary
compiler/test failures are returned unchanged; a second failure is final.
Subprocess fixtures exercise signals, exit codes, graph removal and the retry
environment. This can recover a killed compiler while the job supervisor is
alive; it cannot recover a terminated runner VM or a killed retry process.
Job cancellation terminates the supervisor rather than launching more work.

### Coverage graph transfer investigation

PR coverage job **131505064154** restored main's `b7c67e961` archive and then
used 3,754 s wall / 1,507 s CPU, with about 31 GiB swap. Main warm
**43281997481**, job **131405416421**, and that PR used the same Rust 1.98.1,
llvm-cov 0.9.1, warning flags and line-table settings. The manifest delta is
only workspace dist metadata. However, the intervening main commit
`b7c67e961..66a15dad2` changes **19 backend Rust files: 2,058 insertions and
144 deletions**, including the shared proxy forwarding function signature.
There are no further backend runtime changes from that base to `df20e4869`.

The following controlled local archive builds retain one target directory:

| Successive change from b7 source | Wall | Peak RSS | Native graph directory names |
| --- | ---: | ---: | --- |
| Cold baseline | 543.03 s | 39.71 GiB | Ten directories created |
| Root dist metadata only, plus `--no-clean` | 48.89 s | 21.00 GiB | Same ten |
| Then the two revised build scripts | 48.96 s | 20.84 GiB | Same ten |
| Then the actual 19-file runtime/test delta and changed backend stamp | 429.65 s | 48.22 GiB | Same ten |

Thus neither the metadata edit, archive option nor build-script implementation
created a different local crate identity. The real source delta reproduced a
large incremental rebuild and higher memory use. Unchanged manifest hashes do
not mean unchanged compiler queries in the monolithic crate. The latter build
left two finalized `nyxid_server` sessions of **8,701,074,574** and
**8,855,631,811 bytes** under the same native crate directory; all graphs
totaled **17,956,968,887 logical bytes**. Their compiler/header and option-hash
prefixes matched. This is direct evidence that archive doubling can come from
retained sessions, without a new crate directory.

The old hosted logs contain neither native directory inventories nor rustc
cache-invalidation diagnostics. They cannot establish whether that particular
main graph was rejected outright or loaded and invalidated by source changes.
The local results above do **not** prove a historical hosted cause. This uncertainty no longer determines the archive policy: the ordinary
replay below rejects incremental reuse for those two lanes. Source-change
transfers still need hosted measurement in the remaining graph lanes. New logs expose native crate/session sizes after restore and build, resolving the previously invisible distinction
between new directories and retained sessions.

Reproduce in a disposable directory under `target/`, with pinned nextest
0.9.146 and llvm-cov 0.9.1 on PATH:

```sh
study="$PWD/target/coverage-transfer"
mkdir -p "$study/source"
git archive b7c67e961 | tar -x -C "$study/source"
export RUSTUP_TOOLCHAIN=1.98.1 CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=1
export CARGO_TARGET_DIR="$study/build" RUSTFLAGS=-Dwarnings
export CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_PROFILE_TEST_DEBUG=line-tables-only
export RUST_MIN_STACK=8388608 LLVM_PROFILE_FILE_NAME='nyxid-%4m.profraw'
export NYXID_GIT_HASH=repro
/usr/bin/time -l cargo llvm-cov nextest-archive \
  --manifest-path "$study/source/Cargo.toml" --package nyxid \
  --archive-file "$study/base.tar.zst"
```

Save the names and sizes under `$study/build/llvm-cov-target/debug/incremental`.
Replace only the source copy's root manifest using `git show df20e4869:Cargo.toml`
and repeat with `--no-clean`. Replace its two build scripts using `git show`
and repeat. Finally apply the files listed by
`git diff --name-only b7c67e961 df20e4869 -- backend/src` to that copy, change
`NYXID_GIT_HASH` to `pr-repro`, and repeat. Add
`CARGO_LOG=cargo::core::compiler::fingerprint=info` for the dirty reasons.
Compare directory names and each finalized session's logical size at each step.
These commands compile archives; they do not execute MongoDB tests. Remove
`$study` after recording results. They never change the current source tree.

### Compressed-swap experiment

`NYXID_SWAP_MODE` selects `zswap` or `off`; manual **CI Cache Warm** exposes
`swap-mode` alongside `zswap-pool-percent` (default `20`). The latter maps to
`NYXID_ZSWAP_POOL_PERCENT`, accepts integers 1–100, and leaves settings unchanged
with an explicit log on invalid input. The candidate is zstd with a maximum
compressed pool of 20% of RAM (about 3.2 GiB on a 16 GiB runner); 50% is 8 GiB. This is an
experiment, not a measured winner. Archive warm jobs now compile without
incremental, so compare off/20/50 against each other to isolate zswap; comparing
against run 43310342885 also includes the incremental-policy change. The serial compiler leaves CPU capacity
available, so compression is worth testing against the observed disk wait.
Swappiness and page-cluster remain at runner defaults, and their effective
values are logged. No Rust flags or cache identities depend on this choice.

[Kernel zswap documentation](https://docs.kernel.org/admin-guide/mm/zswap.html)
describes the compressed cache in front of backing swap, dynamic pool growth
and eviction to that backing device. It preserves the mandatory 40 GiB disk
swap capacity. [zram](https://docs.kernel.org/admin-guide/blockdev/zram.html)
would need a separate device and writeback policy to preserve that property;
it is not used. Documentation does not establish which compressor/pool size
wins for this compiler workload, so compare cold `off`, `zswap`/20 and
`zswap`/50 dispatches on separate scratch branches with identical source.

Missing sysfs, rejected writes or counter access never fail the job. Failed
configuration attempts restore prior values where possible and explicitly
log incomplete rollback. Effective parameters and compressed-pool counters
are printed before and after the build. On hosted machines only, debugfs is
mounted if necessary and permitted; unavailable counters are explicitly null.
The existing swap reserve remains mandatory in either mode. Once hosted
measurements choose a winner, remove the experiment selector in that reviewed
change; this document makes no claim that zswap has reduced elapsed time yet.

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

## Eight-commit incremental replay

Main was frozen at `da70dddc9ad9a8de7026b4319909a6927cd2d3e1`. The eight
most recent first-parent commits touching `backend/src` were replayed in
order, starting by building parent `b4b03036ff24d363799adb24f66c396b4350c166`.
Every snapshot used fresh source mtimes and one shared target directory.
Cargo manifests and lockfiles did not change across this window. The parent
includes cold dependencies; subsequent rows reuse dependencies and the
previous graph. The reference deletes the graph and disables incremental
on the last commit, with dependencies warm. There is one sample per commit,
not repeated trials or a Linux memory-pressure simulation.

Wall time includes the entire nextest archive command. Backend rustc time is
Cargo's stable `--timings` duration for `nyxid-server (test)`. Peak RSS is the
maximum child RSS reported by macOS `/usr/bin/time -l`, not the sum of all
concurrent processes. Files and added/deleted lines count `backend/src` changes
from the previous snapshot. No tests are executed by these archive builds.

| Revision / mode | Files | Added / deleted | Wall | Backend rustc | Peak RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Parent `b4b03036f` | — | — | 535.98 s | 456.57 s | 39.52 GiB |
| `0aa535b17` | 2 | 16 / 3 | 127.11 s | 125.01 s | 32.48 GiB |
| `31f3d8969` | 10 | 712 / 59 | 188.24 s | 186.27 s | 37.31 GiB |
| `6470f486c` | 5 | 263 / 20 | 182.57 s | 180.44 s | 36.59 GiB |
| `524d13896` | 2 | 904 / 7 | 350.17 s | 348.19 s | 45.43 GiB |
| `890a2f7bb` | 49 | 2452 / 195 | 400.70 s | 398.56 s | 47.76 GiB |
| `b7c67e961` | 4 | 219 / 22 | 386.08 s | 383.95 s | 47.64 GiB |
| `66a15dad2` | 19 | 2058 / 144 | 421.68 s | 419.53 s | 47.90 GiB |
| `da70dddc9` | 5 | 964 / 11 | 375.21 s | 373.02 s | 46.91 GiB |
| `da70dddc9` / non-incremental reference | — | — | 399.95 s | 395.99 s | 40.60 GiB |

The median incremental wall time is **362.69 s**, 90.7% of the **399.95 s**
non-incremental reference; the mean is **303.97 s**. Three commits benefit
substantially (127.11–188.24 s). The other **five of eight take 350.17–421.68 s**,
87.6–105.4% of the reference, with **45.43–47.90 GiB RSS** versus **40.60 GiB**
without incremental. This distribution does not justify downloading and
retaining the ordinary/instrumented test graphs on a 16 GiB runner. The
instrumented lane follows the same policy; it was not independently replayed.
The earlier instrumented 19-file rebuild (429.65 s / 48.22 GiB) corroborates
its memory risk, but is not substituted into the ordinary replay table.

**Both archive readers and their matching warm jobs now use
`CARGO_INCREMENTAL=0` and do not restore or save graphs.** Warm archive jobs
still execute their full compile commands and save non-incremental dependency
caches using the existing `v0-rust` prefix. Their graph saves are removed
because no archive reader consumes them. Seven remaining graph producers
(Clippy, three KMS variants, CLI tests, CLI coverage and permissions) retain
incremental even on misses; their readers enable it only after a restore.
They keep the original `v1-rust-incremental` dependency prefix. All job names,
nextest packages, eight shards, inventory verification, coverage counters,
73% threshold, base/head comparison and linked KMS validation are unchanged.
The policy trades away the three faster archive cases for lower memory and
no graph transfer cost in the five nearly-cold cases. Linux elapsed improvement
is not yet measured; no hosted speedup is inferred from the macOS ratios.

The worst RSS in this replay is **47.90 GiB**, leaving **8.10 GiB** below the
nominal 16 + 40 = 56 GiB capacity. The non-incremental reference leaves
**15.40 GiB**. These are arithmetic margins for the maximum individual child,
not guarantees for the sum of OS, other processes and compressed-pool memory.
The earlier coverage maximum was **48.22 GiB**, a **7.78 GiB** nominal margin.
A single retry without incremental state handles a killed compiler while the
supervisor survives, as described above.

Reproduce with stable 1.98.1 and nextest 0.9.146 on PATH, using a fresh owned
scratch directory. The following creates an independent disposable clone;
it does not create or change any other worktree:

```sh
git fetch --no-write-fetch-head origin da70dddc9ad9a8de7026b4319909a6927cd2d3e1
python3 - <<'PY'
import os, shutil, subprocess
from pathlib import Path
repo = Path.cwd()
study = repo / 'target/incremental-replay'
source, target = study / 'source', study / 'build'
study.mkdir()
subprocess.run(['git', 'clone', '--quiet', '--shared', '--no-checkout', str(repo), str(source)], check=True)
main = 'da70dddc9ad9a8de7026b4319909a6927cd2d3e1'
def git(*args):
    return subprocess.check_output(['git', *args], cwd=repo, text=True).strip()
commits = git('rev-list', '--first-parent', '-8', main, '--', 'backend/src').splitlines()[::-1]
parent = git('rev-parse', commits[0] + '^')
env = os.environ.copy()
for name in ('RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'RUSTC_BOOTSTRAP',
             'CARGO_LOG', 'CARGO_ENCODED_RUSTFLAGS', 'NYXID_GIT_HASH', 'CARGO_BUILD_TARGET'):
    env.pop(name, None)
env.update(RUSTUP_TOOLCHAIN='1.98.1', CARGO_BUILD_JOBS='4', RUSTFLAGS='-Dwarnings',
           CARGO_PROFILE_DEV_DEBUG='line-tables-only', CARGO_PROFILE_TEST_DEBUG='line-tables-only',
           CARGO_TARGET_DIR=str(target), RUST_MIN_STACK='8388608', CARGO_TERM_COLOR='never', GIT_OPTIONAL_LOCKS='0',
           LLVM_PROFILE_FILE_NAME='nyxid-%4m.profraw',
           NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27017/?replicaSet=rs0&directConnection=true')
for index, (sha, incremental) in enumerate([(parent, True), *[(s, True) for s in commits], (commits[-1], False)]):
    if incremental:
        subprocess.run(['git', '-C', str(source), 'checkout', '--quiet', '--force', '--detach', sha], check=True)
        for path in subprocess.check_output(['git', 'ls-files', '-z'], cwd=source).split(b'\0'):
            if path:
                file = source / os.fsdecode(path)
                if file.is_file() and not file.is_symlink():
                    file.touch()
    else:
        shutil.rmtree(target / 'debug/incremental')
    env['CARGO_INCREMENTAL'] = str(int(incremental))
    with (study / f'{index:02}-{sha[:9]}.log').open('w') as output:
        subprocess.run(['/usr/bin/time', '-l', 'cargo', 'nextest', 'archive',
                        '-p', 'nyxid', '-p', 'nyxid-service-adapters', '--timings',
                        '--archive-file', str(study / 'tests.tar.zst')],
                       cwd=source, env=env, stdout=output, stderr=subprocess.STDOUT, check=True)
    shutil.copyfile(target / 'cargo-timings/cargo-timing.html', study / f'{index:02}-timing.html')
PY
```

Read the per-command time/RSS lines and backend row in each timing report;
`git diff --numstat <previous> <sha> -- backend/src` supplies the change counts.
Remove only this owned scratch directory after recording the measurements.

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

This single small-edit sample made incremental look favorable, but does not
establish its behavior across real PRs. The eight-commit replay above governs
the archive policy; cache misses build without incremental bookkeeping.
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
features matched. Hosted run 43310337092 subsequently passed actual packaging
for Linux arm64 inside Focal; its other three targets skipped on cache hits.
Those targets still need executed hosted builds under the new PR condition.

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
Linux. Hosted CodeQL run 43310337099 subsequently measured the Rust job at
28.3 minutes. Full-scope manual run **43323193873** passed all four
languages; Rust took **65m55s (65.9 minutes)**. This confirms branch full-scope
execution alongside the scoped PR result. Both local runs retained extractor diagnostics: the full run reported five
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

Rounds five through seven change only CI helpers, actions, workflows and documents;
no Rust source or build script changes. The round-six verification
results and replay above supersede the discarded wrapper experiment. The
full MongoDB suites are not rerun locally; the new PR CI run must execute
them after these cache/policy changes. The previous hosted PR CI run
43310337142 was green before those changes.

Round six passed **209 CI helper tests** (24.959 s), **41 script helper tests**
(34.512 s), and **8 dependency-recipe tests** (0.277 s), with no skips. All four
reusable-workflow permission checks, actionlint **1.7.12**, workspace formatting
and `git diff --check` passed. The real Cargo fixtures cover native session
pruning, version changes, fresh checkout mtimes, unchanged builds, multiple
Cargo commands and coverage equality. Retry fixtures cover exit 137/143,
SIGKILL/SIGTERM, OOM output, exactly one non-incremental retry, graph deletion,
persistence to later Actions steps, ordinary-error propagation and cancellation.
The replay and native-graph measurements compile full ordinary archives with
stable **1.98.1** and `RUSTFLAGS=-Dwarnings`; they do not execute the backend
suites. No Rust file changed in this round, so workspace Clippy was not repeated.
The prior accepted real-workspace Clippy results and the newly passing fixtures
remain the applicable evidence. No Docker resources were started this round. The owned 35 GiB scratch tree,
including source snapshots, compiler targets, archives, downloaded tools and
Python environment, was removed after recording these results. `target/`
then measured 4 KiB and no owned benchmark processes remained. The engineer
subsequently committed those edits as `c40a5d95c` before the swap experiment.

Round seven passed **217 CI helper tests** (25.576 s), **41 script helper tests**
(34.385 s) and **8 dependency-recipe tests** (0.306 s), with no skips, using
stable 1.98.1 and the pinned nextest/llvm-cov fixture tools. Actionlint **1.7.12**,
all four reusable-workflow permission checks and `git diff --check` passed.
The new tests cover malformed reader inventories preserving successful and
failed child exits, the same malformed graphs refusing saves, failed `df`,
failed SDK removal, unchanged disk admission and opt-in wiring across every
cache consumer. No Rust source or build script changed, so full-workspace
Clippy and backend suites were not repeated. The helper suites still compile
their real Cargo/build-script/coverage fixtures. No Docker resources were
started. The 83 MiB tool/environment scratch directory and downloaded job log
were removed; `target/` returned to 4 KiB and no verification processes remain.

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
coverage profiles. The initial cleanup left `target/` at 4 KiB with no owned build/test processes,
temporary archive directories, Docker containers or volumes. The
dedicated builders, custom images and known test-only image download were
removed; shared Docker builders and pre-existing base images were preserved.
The second round's scratch tools/logs and newly pulled Focal image were also
removed after verification; its disposable container left no volumes.
The third round's 9.9 GiB of build output and scratch tools/logs were removed
after the final Clippy and CLI checks. Round five removed its 16 GiB scratch
target, source copies, archives and installed measurement tools; `target/`
returned to 4 KiB. No owned compiler/test processes remained, and no Docker
resources were started in that round.
The measurements above and their reproduction commands are self-contained;
the document does not depend on the deleted logs.

## Change map

| Files | Purpose |
| --- | --- |
| `.github/actions/ci-rust-cache/action.yml`, `.github/actions/ci-rust-incremental-save/action.yml` | One save/restore key and path contract, explicit manual-ref save policy, manifest fallback. |
| `.github/workflows/ci-cache-warm.yml`, `.github/scripts/test_ci_cache_warm.py` | Matching producers, resource measurements, owned swap release before saves, contract regression checks. |
| `.github/scripts/cache_budget.py`, `.github/scripts/test_cache_budget.py`, `.github/workflows/cache-maintenance.yml`, `docs/ci/ACTIONS_CACHE_BUDGET.md` | Preserve ref-scoped Rust caches for 48 hours and until a restorable main replacement exists, including cache versions and prefixes. |
| `.github/scripts/prepare-backend-runner.sh`, `backend_disk.py`, `compressed_swap.py` and tests | Restore admission budgets, delayed swap allocation, owned reserve release and optional measured zswap experiment. |
| `.github/scripts/incremental_cache.py`, `retry_incremental.py` and tests | Native session inventory, graph pruning, bounded archives and build/restore size diagnostics. |
| `backend/build.rs`, `cli/build.rs`, `scripts/test_git_build_scripts.py` | Plain Cargo freshness, real per-worktree Git watches and SDK-sensitive macOS linker paths. |
| `.github/workflows/backend-tests.yml`, `.github/scripts/test_coverage_archive_cache.py` | Non-incremental archive policy, isolated counters/current objects, coverage equivalence across compilation modes. |
| `.github/workflows/ci.yml`, `scripts/check-backend-docker-embeds.py`, `scripts/test_backend_docker_embeds.py` | Release-check image inputs, fail-closed current-unit collection, real build/check closure fixture, Git-stamp fixture wiring and updater cache imports. |
| `Cargo.toml`, `scripts/dist-build.sh`, `.github/scripts/dist_cargo.py`, `.github/scripts/test_dist_cargo.py`, `.github/workflows/release.yml`, `.github/workflows/release-cache.yml` | Compile only cargo-dist's distributed binary while retaining workspace feature unification; PR release-cache jobs build even on exact hits. |
| `cli/Dockerfile.machine-updater`, `scripts/check-updater-docker-embeds.py`, `scripts/test_updater_docker_embeds.py`, `.github/workflows/publish-images.yml` | Portable production/test build layers and exact Docker input validation. |
| `docker-compose.yml`, `docker-compose.test.yml`, `CONTRIBUTING.md` | Disposable local test replica set, Git-stamp semantics and current validation contracts. |
| `.github/workflows/codeql.yml`, `.github/codeql/codeql-config.yml` | Uncapped Git change detection, conservative language selection, generated Rust PR scope and historical default categories. |
| `.github/scripts/codeql_cfg_test_files.py`, `codeql_rust_scope.py`, `codeql_compare_sarif.py` and their `test_codeql_*.py` files | Conservative module-tree traversal, language/config contracts and alert-equivalence verification. |
| `docs/CI_PERFORMANCE.md` | Reproducible measurements, rejected alternatives, verification and limits. |

No backend or CLI runtime source is changed. The Git-stamp wrapper and its
workflow/local prefixes were removed; plain Cargo is the supported interface.
There is no crate split, binary rename, KMS feature change or test-ID migration.

## GitHub Actions confirmation checklist

The engineer performed the runs listed above; the agent did not trigger them.
This list separates new required measurements from already-observed results.
Record run IDs, actual save/restore messages, resource samples and check
conclusions. A green warm job alone does not prove a cache was saved.

### Before merge

1. After committing/pushing the round-seven changes, dispatch **CI Cache Warm**
   on the PR branch with `swap-mode=zswap`, `zswap-pool-percent=20`. Verify
   seven graph saves, `after-prune` sizes at most 16 GiB, and one finalized
   session per retained crate. The two archive jobs must log `enabled=0`,
   skip graph restore/save and still compile full archives. Check `df` before
   dependencies in reserve jobs, after graph restore in graph lanes, after
   swap allocation and after reserve release; graph lanes also check disk
   before extraction. Confirm SDK cleanup is skipped in permissions and CLI
   jobs and restored-graph inventory is skipped in archive/image-input jobs.
   Require zswap effective settings/counters or an explicit unavailable result.
2. Dispatch **CI Cache Warm** again at the same commit. The seven graph lanes
   must hit and pass extraction admission; a safe restore skip does not prove
   reuse. Inspect inventories for obsolete crates/sessions. Archive jobs stay
   non-incremental and should hit dependency caches only. Old main maintenance
   does not recognize the new graph prefix, but can still delete branch
   dependency entries; only after merge does the 48-hour grace protect both.
3. Make a small backend Rust edit without manifest/version changes and dispatch
   again. Record Clippy/KMS restore, wall/CPU/RSS/swap and pruned sizes to test
   their cross-commit reuse. Record the ordinary and coverage archive costs
   under the new non-incremental policy. They deliberately do not read main's
   or the branch's old archive graphs.
4. For the compressed-swap comparison, create three scratch branches at the
   same revision and dependency-cache state. Dispatch **CI Cache Warm** in
   parallel with (`swap-mode=off`), (`swap-mode=zswap`, `zswap-pool-percent=20`),
   and (`swap-mode=zswap`, `zswap-pool-percent=50`). The per-ref concurrency
   allows all three. The engineer dispatched runs **43344390346** (off),
   **43344395848** (20%) and **43344400769** (50%) from `c40a5d95c`;
   their build results are not evaluated in this round. The two archive jobs
   must log `enabled=0` and skip graph restoration; no archive-graph cache
   deletion is needed under the final policy. Compare build-phase wall/CPU/RSS/swap and pool statistics
   across these three runs. Run 43310342885 is the previous incremental-policy
   baseline, so a difference from it cannot be attributed to zswap alone.
   Keep the measured winner and remove the experiment inputs in the ensuing
   reviewed change. The workflow itself never deletes remote caches.
5. The new PR **CI** run must keep all ordinary/instrumented shards, inventory
   verification, 73% line gate, base/head comparison, linked KMS builds and
   image proofs green. The previous PR run already passed; the miss policy, pruning
   and restore order need their own hosted validation. Require a cache miss or
   skipped restore to log `enabled=0` in every PR lane. **Release Build Cache**
   must execute **Install dist**, **Install dependencies** and **Build artifacts**
   for all four targets even on exact cache hits, with PR saves disabled.
   No manual release-cache dispatch is needed: this workflow edit triggers the
   PR matrix. A manual dispatch can still skip exact hits by design.
6. CodeQL Rust's 28.3-minute PR and 65.9-minute full-scope branch results
   are established (runs 43310337099 and 43323193873); no repeat dispatch is
   required. Still confirm a non-Rust-only PR omits Rust without a pending
   code-scanning check. The generated list remains inspectable in the scope artifact.
   The local large-diff/error fixtures cover fail-open selection beyond the
   GitHub API diff limits; no workflow-level path filter is used.

Do not use **Publish Images** as a pre-merge dry run: it really publishes.

### After merge

1. Inspect the first normal main **Publish Images** run. Gate reuse must accept
   only successful jobs on the matching tree; every unavailable validation
   must execute. Both backend architectures must complete optimized links,
   smoke checks and manifest publication. Verify the coverage baseline is
   published with the unchanged fingerprint/denominator contract. Cold
   monolithic release compilation still exceeds 16 GiB locally; no hosted
   cold-build speedup is claimed for it.
2. Confirm main **CI Cache Warm** publishes the seven bounded graphs and the
   matching dependency caches, including non-incremental archive dependencies.
   On the next backend PR with a small edit and unchanged dependency closure,
   verify Clippy/KMS main-to-PR reuse and record their compiler/resource profile.
   Both archive jobs must stay non-incremental and complete the full suites.
   Inspect trusted **CI Cache Maintenance** output: compatible main
   entries replace branch generations only after 48 hours from creation, and
   incompatible prefixes/versions never replace them. Check one protected row
   inside the grace period and one eligible row after it.
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

The engineer committed df20e4869 and c40a5d95c, opened PR #2428 and triggered
the hosted runs above. Round-seven edits are left uncommitted on c40a5d95c;
the agent did not push, dispatch workflows or delete remote caches.
