# Actions cache budget

The release cache keys in `release.yml` and `release-cache.yml` are unchanged.
Their four `v0-rust-dist-*` entries need room in GitHub's **10 GiB** repository
quota (10.737 GB in decimal units). The existing daily release-cache workflow
refreshes their access time. Ordinary CI retains compiled dependency caches;
this change does not switch jobs to registry-only Rust caching.

## Measured inventory, 2026-10-27

Command:

```sh
gh cache list --repo ChronoAIProject/NyxID --limit 100 --sort size_in_bytes \
  --json id,key,ref,sizeInBytes,createdAt,lastAccessedAt
```

The snapshot contained **83 entries, 10.188 GB (9.489 GiB)**:

| Category | GB |
| --- | ---: |
| Ordinary Rust CI | 7.228 |
| Dist, Linux only (both macOS entries evicted) | 1.443 |
| BuildKit blobs/indexes | 1.323 |
| Node caches and coverage reports | 0.195 |

Cache sizes per job, in decimal MB:

| Cache lane | Measured MB | New writer / sharing |
| --- | ---: | --- |
| CLI test | 793.0 | CLI test; wizard restores this cache |
| Backend tests | 781.9 | Ordinary backend build; billing restores this cache |
| Billing smoke | 770.5 + 769.7 | No separate writer |
| Wizard freshness | 716.1 | No separate writer |
| Clippy | 518.3 | Main only, separate analysis cache |
| Features, AWS | 725.3 + 724.9 | Main only, one environment generation |
| Features, GCP | 707.6 | Main only |
| Features, both | 750.9* | Main only; remains separate from individual features |
| CLI coverage | 720.8 | Main only, separate instrumented cache |
| Backend coverage | 786.8* | Main only, separate instrumented cache |
| Backend image inputs | 623.4* | Main only, separate release-profile cache |
| Permissions | 373.7* | Main only; keep both check and test artifacts |

`*` Already evicted at inventory time; sizes come from the restored-cache byte
counts in Publish Images run **40507925449**, jobs **122202039122**,
**122202038988**, **122202039004**, and **122202038594**, respectively. Do not
mistake an evicted cache for zero steady-state storage.

The `index-machine-pr` records on main, tag and PR refs identify Machine
Container E2E as the remaining large GHA writer. Backend Image Inputs uses
rust-cache, not BuildKit. Docker smoke jobs and Publish Images import only the
published per-architecture registry caches. Publish Images exports registry
`mode=max` caches. No GHA import remains: maintenance removes indexes and blobs
in separate operations, so a concurrent GHA reader could otherwise observe an
index whose blobs have already been removed.

Merge PR #2061 first: it owns removal of the backend smoke GHA export (alongside
the Cargo jobs build argument). This branch removes the remaining imports and
the machine smoke export, without duplicating #2061's export-removal change.
After rebasing onto #2061, no workflow imports or exports BuildKit GHA caches.

## Changes and estimated steady state

The ordinary-CI composite removes unused runner toolchains before rust-cache:
that action hashes every installed compiler. Logs show the same pinned 1.98.1
compiler alongside either unused 1.99.0 or 1.99.1, producing duplicate keys.
The cache step also normalizes scheduling/stack environment values; actual
build concurrency is unchanged. Compiler version, RUSTFLAGS, profile values,
manifest requirements and third-party lock contents remain in the cache key.

Only main writes caches. Billing and wizard are read-only consumers of their
compatible backend/CLI cache, so they cannot win an immutable-cache race with
an incomplete dependency subset. Different feature combinations, release,
Clippy, and instrumented builds keep their own caches. All ordinary-CI debug
builds retain file/line information (`line-tables-only`) without full variable
and type debug information; production/release profiles are unchanged. Pinned
taiki-e installs supply nextest/llvm-cov, so their executables are not duplicated
inside every dependency cache (`cache-bin: false`).

At the old per-lane sizes, retaining all ten unique live lanes would cost
**6.782 GB**, plus about **0.2–0.3 GB** for Node and tiny reports. Removing tool
executables and reducing the remaining full-debug builds is expected to save
at least **0.35 GB**: forecast **6.3–6.7 GB total ordinary cache**, leaving
**4.0–4.4 GB** for dist. This is an estimate, not a measured post-change result;
confirm it after the first main cache fills. The four release caches are
expected to fit in that reserve; their current Linux pair uses 1.443 GB.

The maintenance workflow reports actual non-dist bytes and warns below a
**4 GB** reserve. It never evicts a live CI lane to make a forecast appear true.
If the forecast is exceeded, use the reported lane sizes to adjust storage;
do not disable target caching and silently turn regular CI into cold builds.

## Retention and rollout

`cache-maintenance.yml` runs trusted main code when cache configuration changes,
after CI/Publish Images, on a six-hour schedule, or manually. It removes retired
BuildKit entries, redundant
billing/wizard caches once their main replacement exists, non-main Rust copies
once main has a replacement, and older generations of each known CI lane.
It always retains the newest main generation. Unknown cache families and every
`v0-rust-dist-*` entry are protected. No PR artifact or code is executed by the
maintenance workflow. A dry run is the script's default:

```sh
python3 .github/scripts/cache_budget.py caches.json --repo ChronoAIProject/NyxID
```

On the measured snapshot, the dry run selects **71 entries / 4.303 GB** for
removal, leaving **5.885 GB total**, including both surviving dist caches.
That snapshot lacks four live CI lanes, so it is not the steady-state estimate.
No repository caches were deleted during implementation. After merge, the
scheduled maintenance removes retired entries, ordinary CI fills the stable
keys once, and the existing release-cache workflow rebuilds missing macOS
entries. Changing the ordinary debug profile causes one initial dependency
rebuild; subsequent jobs reuse it without extra compile work.
