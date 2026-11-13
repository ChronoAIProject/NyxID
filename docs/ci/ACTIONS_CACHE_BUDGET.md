# Actions cache retention

GitHub's repository Actions cache limit is now **100 GB**. The maintenance job
keeps usable cache generations for fast CI and releases, while bounding families
whose restore keys move forward. It does not combine build lanes or reduce the
artifacts used by Rust or release builds. The release cache keys in
`release.yml` and `release-cache.yml` remain unchanged.

The script reads the inventory produced by:

```sh
gh cache list --repo ChronoAIProject/NyxID --limit 10000 \
  --json id,key,ref,sizeInBytes,createdAt,lastAccessedAt,version
```

It is a dry run unless `--apply` is supplied. Only the trusted main-branch
maintenance workflow passes `--apply`; it supplies `CACHE_SOFT_CAP_GB=40`.
The soft cap is an alert, not an eviction target: the script never deletes a
newest usable entry to reach it. A command-line `--soft-cap-gb` overrides the
environment value for local reports.

## Retention rules

- **Release dist** (`v0-rust-dist-<target>-<os>-<arch>-<hash>-<hash>`): group by
  target. Keep the newest main-ref generation for every target. Delete older main
  generations only when that newer main entry exists. Release saves are expected
  to be main-only; any non-main dist copy is removable. A sole newest main entry
  is always protected.
- **Node caches** (`node-cache-<os>-<arch>-<manager>-<lockhash>`): the lockfile
  hash identifies the exact job cache, so main entries are never removed. A
  non-main entry is removable only when an entry with the identical full key
  exists on main. This preserves separate frontend npm, SDK npm and mobile pnpm
  caches; obsolete main lockfile generations are left to GitHub's seven-day
  unused-cache eviction.
- **Backend coverage baselines** (`coverage-backend-base-v2-<sha>-<fingerprint>`):
  remove non-main copies. On main, retain the union of the newest 30 entries and
  every entry created in the last 14 days. This preserves exact-base-SHA lookup
  coverage while bounding old generations.
- **Known Rust lanes** (`v0-rust-<lane>-<os>-<arch>-<8 hex>-<8 hex>` and
  `v1-rust-incremental-<lane>-<os>-<arch>-<8 hex>-<8 hex>`) keep the newest main
  generation and remove older or non-main copies only after that main
  replacement exists with the same key prefix and cache version. Incremental
  jobs keep the `v1-rust-incremental` Swatinem prefix; a leftover `v0-rust`
  entry cannot replace it because the reader cannot restore that prefix.
  Non-incremental lanes stay on `v0-rust`. Billing and wizard lanes share their
  current backend/CLI replacements as configured in the script.
- **Incremental Rust caches** (`v1-rust-graph-<lane>-<os>-<arch>-<manifest>-<commit>`,
  and legacy `v0-rust-incremental-…`) use the same newest-main rule, grouped
  by prefix, lane, runner OS/architecture, and
  the actions cache `version`. That version is the hash of the exact path list.
  The one-path version `a625e3a14900daf5` and the two-path version
  `f98f77cfa579f3b1` do not replace each other. A branch-scoped manual save is
  kept for at least 48 hours from creation, and until a main entry with the
  same prefix and version exists. An inventory row with
  no version is not a replacement for a versioned row.
  The obsolete one-path main entries are deliberately left to GitHub's
  seven-day unused-cache eviction, not retired by this script. They occupy
  about 9 GiB across six lanes in the reviewed inventory. The two-path reader
  cannot restore them, so it cannot refresh their last-access time. Once main
  uses the corrected writers, no new one-path generations are saved; existing
  entries expire after seven unused days from their last save/access. This
  temporary cost fits the 100 GB repository limit. The 40 GB soft cap can warn
  during the transition but does not force eviction. Version-aware retention
  still protects new branch caches from incompatible old main entries.
- **Branch Rust grace period:** both dependency/artifact and incremental
  entries are protected for 48 hours from creation even when main already has
  a compatible entry. This covers two roughly hour-long cold dispatches plus
  engineer review time across multiple six-hour maintenance cycles. Missing,
  malformed and future creation times are preserved. After 48 hours, removal
  still requires a compatible main replacement; age alone never deletes one.
- **Retired Docker BuildKit blobs/indexes** are removable because Docker jobs now
  import and export the per-platform registry cache. No workflow restores these
  GitHub Actions cache records.
- **Unknown families** are left to GitHub's own unused-cache eviction. The script
  never guesses at a new restore key.

The report prints before and retained totals for each recognized family and an
`unknown` bucket. It emits a warning when retained bytes exceed the 40 GB soft
cap (or the selected cap); it still retains every newest entry.

## Current inventory and dry-run procedure

The previous 2026-10-27 inventory had 83 entries and 10.188 GB, before the
repository limit was raised. The new live inventory should be measured before
each review:

```sh
gh cache list --repo ChronoAIProject/NyxID --limit 10000 \
  --json id,key,ref,sizeInBytes,createdAt,lastAccessedAt,version > /tmp/caches.json
python3 .github/scripts/cache_budget.py /tmp/caches.json \
  --repo ChronoAIProject/NyxID --soft-cap-gb 40
```

The 2026-10-28 review snapshot contained 27 entries and 10.265 GB. The dry run
selected six non-main duplicate Node/coverage entries totaling **454.1 MB** and
retained **9.810 GB**, including 2.985 GB of dist caches and 6.550 GB of Rust
lanes. All main Node caches and dist caches were retained; the soft-cap warning
did not fire.

The output lists every planned deletion with its cache id, size, ref, key and
reason.
Review it before enabling `--apply`; no cache deletion is part of normal code
review or local testing.

The maintenance workflow runs after main CI or Publish Images completes, every
six hours, when its trusted configuration changes, or manually. Its concurrency
group prevents overlapping delete passes. A pass is intentionally non-atomic:
all Docker GHA readers were removed before this cleanup was introduced, so an
index cannot be restored while its blobs are being retired.

`ci-cache-warm.yml` is the only writer for ordinary main Rust dependency and
incremental caches. `ci.yml` and the reusable `backend-tests.yml` explicitly pass
`save-if: 'false'` and never save incremental directories. The composite also
defaults to read-only; warm jobs explicitly opt in. Release caches and backend
coverage baseline publication retain their separate writers.

Incremental restore prefers the same lane, runner OS/architecture, and
manifest/lock/toolchain/config hash, then reuses another commit in that prefix.
It also falls back to the same lane and runner without that hash. An unrelated
member manifest or lock change can therefore reuse the lane's graph. The
tradeoff is a wasted download when the change invalidates crate identity: run
43210236259 spent 178 seconds downloading 7.7 GB after a backend version bump,
and a similar miss costs about 3–6 minutes. Cargo still rebuilds crates whose
identity changed. A global version segment would miss on unrelated member bumps
the same way the full manifest hash does, so the key does not add one. The
dependency cache remains separate. See [the measurements](../CI_PERFORMANCE.md)
for the cache evidence and limitations.

`actions/cache` versions `path1|path2|zstd-without-long|1.0`. Readers restore
`target/debug/incremental` and `target/llvm-cov-target/debug/incremental`
together (version `f98f77cfa579f3b1`). A save of only the first path is version
`a625e3a14900daf5` and cannot hit. The restore action defines the key and both
paths once; every warm save passes those outputs through
`ci-rust-incremental-save` and does not restate them.

Push warming writes main only. A manual `workflow_dispatch` also writes the
selected ref. Actions scopes that entry to the branch; the key itself has no
branch segment, so ordinary readers still restore main with the same key.
Ordinary CI passes `save-if: false` and never writes dependency or incremental
caches. Warm compiles run under `ci_resources.py`, which logs elapsed time,
peak RSS, and swap, and each warm job uploads those samples.

Coverage archive producers preserve compiler artifacts with `--no-clean` after
clearing raw counters using `clean --profraw-only`. The pinned
llvm-cov's default archive command otherwise deletes workspace compiler
artifacts. Backend archive CI now disables incremental and restores dependencies
only; `--no-clean` still preserves local repeat-build artifacts. Reports select
objects from the current nextest archive. A real-tool fixture checks deleted
sources, renamed binaries, unchanged-build reuse, exact equality with a clean
incremental report, and equal coverage fields/test IDs without incremental
(only compiler-generated function symbols differ).

The backend warm jobs release only their own compile-time swap reserve after
all compilation finishes and before cache compression. Ordinary cache saves in
runs 43174155488 and 43247779268 failed with disk exhaustion despite successful
job conclusions. This returns roughly 37 GiB on the measured runner while
preserving the compile-time 40 GiB swap capacity and runner-owned swap. A failed
swap release leaves the active file intact and fails the non-blocking warm job.

The warm workflow runs on main changes to Rust workspace sources, manifests,
embedded inputs, build scripts, toolchain/configuration and CI workflows/actions,
including backend-only pushes, and supports manual dispatch. It is non-blocking
and cancels older warm runs on the same ref. Manual branch measurements do not
cancel main warming.
It covers ten separate lanes: ordinary and instrumented backend archives,
clippy, three KMS feature variants, CLI tests (also used by wizard freshness),
CLI coverage, permissions and backend image inputs. No lanes are combined. Saved graphs exclude compilation units not selected
by the successful build; no test selection or compiled proof is removed. Tests compare runner, toolchain, cache-prefix
mode, hashed environment inputs, and incremental save/restore keys for every
reader/writer pair. Feature flags are resolved from a static mapping inside the
build step, after cache restoration, so their environment does not change keys.

Ordinary Rust checks are now event-insensitive and eligible for exact-tree gate
reuse only after every expected job succeeds. Feature job names use the simple
`variant` matrix (`aws-kms`, `gcp-kms`, `aws-and-gcp-kms`); the last name is new,
so an older result under `aws-kms,gcp-kms` cannot satisfy it. CLI head coverage
still enforces 64%; its PR-only base measurement runs separately and the summary
job preserves the `coverage-cli` artifact and PR delta. Claude checks continue
to run on main. The strict reuse parser and its rejection of ambiguous names,
`include`/`exclude` matrices and PR-only jobs are unchanged.
Skipped or missing PR jobs always run on main. For example, a backend-only PR
that did not exercise Machine Container E2E cannot reuse that check, so it may
still determine publication time even when Rust checks are reusable.

Bounded incremental archives use the new `v1-rust-graph` prefix. Old
`v0-rust-incremental` archives have no size bound and contain accumulated
obsolete graphs; readers must not restore them under the new disk budget.
Seven lanes retain graphs: Clippy, three KMS variants, CLI tests, CLI coverage
and permissions. Their dependency/workspace prefix stays `v1-rust-incremental`: native rustc paths
and compiler flags are unchanged. Old main maintenance leaves the unknown new
graph prefix alone but can still remove branch dependency entries until the
48-hour grace rule merges. There is no graph fallback to the old prefix.
Obsolete graph generations are left to GitHub's seven-day unused eviction.
Both backend archive lanes now compile without incremental and save only
`v0-rust` dependency caches. Their old graphs and `v1-rust-incremental`
dependency entries likewise receive no new access and expire unused. The
[eight-commit replay](../CI_PERFORMANCE.md#eight-commit-incremental-replay)
explains this policy; their full archive compilation and validation remain.

The broad manifest fallback remains within the bounded graph prefix. After
restore and before any build, a marker records the start time. Fresh-checkout
mtimes force selected workspace units to compile. Before save, pruning removes
native crate directories with no finalized session newer than the marker.
Retained crates keep only their newest finalized session using rustc's encoded
timestamp and exclusive lock checks; the previously locked input session is
not carried forward. One marker spans all Cargo commands in the job. There is
no compiler wrapper or Cargo debug-log parser. An absent marker, empty graph,
busy lock, unknown layout or graph above 16 GiB refuses the save.

Reader inventories only report diagnostics: filesystem/layout or `df` errors
log warnings without changing the build's exit status. The restored-graph
inventory is skipped when no graph is requested. Pruning and writer preflight
retain strict validation. Only reserve-allocating jobs opt into hosted Linux
SDK reclamation; deletion failures warn, and actual free space still controls
restore admission.

Graph readers build with `CARGO_INCREMENTAL=0` on a miss or disk-guard skip.
The seven graph producers keep it enabled on misses; the two archive jobs
keep it disabled and do not produce unused graphs. A compiler OOM/kill or
stale graph triggers one retry with the graphs removed and incremental off.
See [the disk budget](../CI_PERFORMANCE.md#restore-disk-budget-and-graph-pruning)
for the pre-extraction admission check, swap ordering and size diagnostics.

The tests cover newest-per-target dist retention, non-main dist cleanup, node
family retention, coverage newest-30 plus 14-day retention, known Rust lanes
including the legacy and current prefixes, incremental retention per cache
version/prefix, 48-hour branch grace and boundary cases, Docker retirement, unknown-family protection, malformed-date
conservatism and soft-cap warning behavior.

The [hosted confirmation checklist](../CI_PERFORMANCE.md#github-actions-confirmation-checklist)
separates branch warming checks before merge from main replacement and obsolete
entry eviction checks after merge.
