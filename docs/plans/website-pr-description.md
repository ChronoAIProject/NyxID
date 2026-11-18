## Summary

- Replace the public website prototype with native React pages and bilingual navigation, preserving authentication, trusted return targets, invitation codes, and retained solution/share entry points. Keep Newsletter, public chat, Status, Blog menus, and solution activations independently configurable with conservative defaults.
- Publish canonical HTML and matching Markdown, llms, sitemap, and structured metadata through an explicit publication allowlist. Preserve full approved content and real 404 responses; retain draft source without publishing it.
- Merge upstream main `cefb3e674a2a4364cf5e42c897d9c22659bcbce0`, combining its CSP/proxy/credential policies and legal caching with generated website routes. Preserve the header language-label centering fix and all independent commits.

## Test Plan

- [ ] Existing tests pass (`cargo test`, `npm run test`) — full repository suites were not run for this integration.
- [x] New tests added for changed routing/header behavior, alongside the retained solution/auth and publication contracts.
- [ ] Manually tested the affected live flows — merged-version live account/OAuth/MFA/provider and operational solution acceptance remain unexecuted.
- Local lightweight checks: TypeScript syntax, JavaScript syntax, package/lock consistency, scoped diff checks, six pure nginx-policy input/rendering tests, and a reverse-object audit of the authorized authorship rewrite.
- Isolated CI for `58a45da9735cc6ce442a17ef501f5fea3e484789`: [all four jobs passed](https://github.com/aelf-hzz780/NyxID/actions/runs/39717322160). Six nginx-policy tests, 338 unit tests, the full frontend build, 91-page artifact checks, 227 HTTP checks, 10 default and 2 on-fixture browser tests passed. The actual wizard rebuild matched the committed bundle; the unchanged authoritative Rust freshness test passed (1 test). Exact artifact revisions and verified digests are in [the merge handoff](website-main-sync-handoff.md). A single enabled solution fixture demonstrates wiring only; it is not production enablement or real task acceptance.

## Checklist

- [x] Code follows the project's [architecture rules](../../CONTRIBUTING.md#architecture-rules)
- [x] No hardcoded secrets or credentials
- [x] Error messages do not leak internal details
- [x] Documentation updated (if applicable)

No deployment, PR merge, organization PR creation, live dependency replacement, or service restart was performed. The original checkout and untracked review materials remain intact. The authorized name-only rewrite keeps each tree, message, email and timestamp unchanged and excludes upstream commits; the complete old-to-new mapping and recovery refs are documented in the handoff.
