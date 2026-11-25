# Spec maintenance: nightly check

Agents update the governing spec in the same PR as their code changes
([`AGENTS.md`](../AGENTS.md), "Specs: keep them current as you work"). The
main maintainers check every night that this happened and that the specs stay
accurate. Any agent from any provider can draft the check; a maintainer
reviews and merges the result.

## Nightly check

1. **Fetch and list what changed.** For each area in
   [`spec-index.yaml`](spec-index.yaml):

   ```bash
   git fetch origin dev
   git log --oneline <verified_at>..origin/dev -- '<pattern>' '<pattern>' ...  # quote each pattern
   ```

   Patterns use git's default pathspec matching (`*` also matches `/`), so
   each pattern can be passed to `git log` as written. Quote every pattern so the
   shell does not expand it; only quoted patterns also match deleted files. Skip areas with no commits.

2. **Always check the shared registries**, where most spec drift starts:

   ```bash
   git log -p <last check>..origin/dev -- backend/src/routes.rs backend/src/config.rs \
     backend/src/errors/mod.rs backend/src/db.rs
   ```

   New or changed routes go to `docs/API.md`, environment variables and
   defaults to `docs/ENV.md`, error codes to Critical Rule 3 in `AGENTS.md`, and
   collections to `docs/security/PERSONAL_ERASURE_INVENTORY.md`. Feature-flag
   changes go to the area's spec.

3. **Classify each change:**

   | Class | Situation | Action |
   |---|---|---|
   | A | The PR already updated the spec | Spot-check it: accurate against the code, edited in place, one place per fact, layout and size kept |
   | B | Behaviour, a contract or a key decision changed without a spec update | Update the governing spec (and any `site/` page that restates it) |
   | C | Internal change only | Nothing |
   | D | Code contradicts the spec and no PR shows intent to change it | Leave the spec; list it as a candidate NyxID issue |

4. **Record the check.** Set `verified_at` to the checked commit for each area
   you reviewed, and open one docs PR: `docs: nightly spec sync YYYY-MM-DD`,
   listing class B fixes and class D candidates.

## Weekly: index coverage

- Code paths that no `governs` glob matches: list them with `git ls-files backend/src frontend/src mobile/src`
  and compare against the globs in [`spec-index.yaml`](spec-index.yaml). Add important ones to an area, or
  leave them unindexed on purpose.
- Docs that the index or [`README.md`](README.md) should mention but do not.

## Monthly: measure drift

- Sample 30–50 concrete claims (routes, limits, flags, error codes, env vars)
  from indexed specs and check them against the code.
- Ask agents from at least two providers a fixed set of questions using only
  the docs, and grade the answers against the code.
- Record the share of inaccurate claims and wrong answers. If inaccuracy stays
  above about 5%, add a second reviewer to the nightly check.
