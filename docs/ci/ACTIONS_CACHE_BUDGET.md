# Actions cache retention

GitHub's repository Actions cache limit is now **100 GB**. The maintenance job
keeps usable cache generations for fast CI and releases, while bounding families
whose restore keys move forward. It does not combine build lanes or reduce the
artifacts used by Rust or release builds. The release cache keys in
`release.yml` and `release-cache.yml` remain unchanged.

The script reads the inventory produced by:

```sh
gh cache list --repo ChronoAIProject/NyxID --limit 10000 \
  --json id,key,ref,sizeInBytes,createdAt,lastAccessedAt
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
- **Known Rust lanes** keep the newest main generation and remove older or
  non-main copies only after that main replacement exists. Billing and wizard
  lanes share their current backend/CLI replacements as configured in the script.
- **Incremental Rust caches** (`v0-rust-incremental-<lane>-<os>-<arch>-...`)
  use the same newest-main rule, grouped by lane and runner OS/architecture.
  A lane with no main replacement is preserved.
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
  --json id,key,ref,sizeInBytes,createdAt,lastAccessedAt > /tmp/caches.json
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

The warm workflow runs on main changes to Rust workspace sources, manifests,
embedded inputs, build scripts, toolchain/configuration and CI workflows/actions,
including backend-only pushes, and supports manual dispatch. It is non-blocking
and cancels older warm runs so only the newest main tree publishes generations.
It covers ten separate lanes: ordinary and instrumented backend archives,
clippy, three KMS feature variants, CLI tests (also used by wizard freshness),
CLI coverage, permissions and backend image inputs. No lanes are combined and
no cache contents are reduced. Tests compare runner, toolchain, cache-prefix
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

The tests cover newest-per-target dist retention, non-main dist cleanup, node
family retention, coverage newest-30 plus 14-day retention, known Rust lanes,
incremental-lane retention, Docker retirement, unknown-family protection,
malformed-date conservatism and soft-cap warning behavior.
