# PR #1993 conflict resolution

Resolved on 2026-11-18 in the `fix-pr-merge-conflicts` worktree branch.

The original PR head is `791982754991b20a59f917128bf3340e0354f134`
(`aelf-hzz780/NyxID:codex/website-a-delivery`). Its target is
`ChronoAIProject/NyxID:wipnyx-landing`, at
`fbcdb1867d03c7e0f056fe048d31bd77361c0054` when resolved.
The resolution merges that target into the original head, retaining both histories.

## Resolution decisions

- Keep the website's separate public bootstrap and window-free prerenderer.
  Move the current app startup into `app-entry.tsx`, including language readiness,
  assistant error reporting, identity-scoped CMA cache cleanup, and dev harness handles.
- Retain both the website routes and the target's newer app routes. Preserve the
  website's authenticated homepage redirect to `/dashboard`, invitation parameters,
  and trusted solution/auth handoff.
- Preserve the target's response-specific Nginx policies and account-export behavior.
  Generate public locations with complete security headers and explicit app routes;
  unknown paths continue returning an actual 404.
- Include `/data-deletion` in generated HTML, Markdown, sitemap, redirects, legal
  cache policy, and content-derived ETags alongside `/privacy` and `/terms`.
- Share the target's canonical origin helper (`https://nyxid.ai` by default and
  `NYXID_PUBLIC_ORIGIN` at build time) across browser metadata, prerendering,
  sitemap/Markdown output, Nginx indexing policy, and artifact verification.
- Keep the split docs view while carrying forward the target's typography roles
  and interface translation hooks. Public loading/recovery controls use the
  shared common catalog in English, Simplified Chinese, and Traditional Chinese.
- Apply the existing landing-page audit boundary to the new public website,
  which owns approved bilingual page sources and unthemed typography. Docs and
  shared controls remain audited; catalog import restrictions still cover the
  website. Regression tests verify those boundaries.
- Resolve generated CLI artifacts by rebuilding the actual wizard from the merged
  source and lockfile, preserving the existing installer and Rust freshness test.

## Validation

Validation ran against the resolved source in this worktree before creating the
merge commit. The final combined frontend run covered 42 files / 448 tests,
including public rendering/content/visibility, routing and auth handoffs,
login/register/MFA, telemetry/consent, and typography/i18n guard regressions.

- `npm ci --no-audit --no-fund` completed.
- `npm run lint` passed: ESLint reported 0 errors and 59 warnings; the typography
  and i18n checks passed.
- Targeted frontend tests: 448 passed, 0 failed.
- `npm run build` passed, including TypeScript and the 92-page public artifact gate.
- `python3 scripts/test-nginx-policy.py`: 7 passed.
- The packaged Nginx image passed `nginx -t`, 231 HTTP checks, and the log privacy gate.
- `playwright test --config public-playwright.config.ts`: 10 passed, 0 failed.
- An additional browser check loaded `/docs` with a saved Simplified Chinese
  interface preference without console errors or hydration errors.
- `npm run build:wizard` regenerated the bundle and its 216-file source manifest.
- `cargo test -p nyxid-cli --test wizard_bundle_freshness`: 1 passed, 0 failed.
- The resulting diff against `wipnyx-landing` passed `git diff --check`.

Node 22 and Rust 1.98.1 were used. Browser and HTTP checks used an isolated backend
fixture and Nginx container. Full-repository Rust/frontend suites, live provider
flows, production deployment, and the PR merge are outside this validation.

## Updating the original PR

The local worktree name does not need to match the fork branch. A normal push
of the resolution to the PR's exact head branch updates PR #1993:

```sh
git push https://github.com/aelf-hzz780/NyxID.git HEAD:refs/heads/codex/website-a-delivery
```

The resolution is a descendant of the original PR head; it requires no history
rewrite. GitHub reported `maintainerCanModify: true` when the resolution was prepared.
