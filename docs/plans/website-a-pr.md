# Website A PR preparation

Per the owner's latest instruction, delivery consists of implementation commits pushed to `aelf-hzz780/NyxID`. The owner will create the organization PR manually. Connector PR write access and a fork PR are not delivery prerequisites. No PR URL or acceptance result is implied by this document.

Optional fork comparison:

- Repository: `aelf-hzz780/NyxID`
- Base: `codex/website-a-base`
- Head: `codex/website-a-delivery`
- Compare: https://github.com/aelf-hzz780/NyxID/compare/codex%2Fwebsite-a-base...codex%2Fwebsite-a-delivery?expand=1

Organization PR prepared for manual creation by the owner:

- Repository: `ChronoAIProject/NyxID`
- Base: `main`
- Head: `aelf-hzz780:codex/website-a-delivery`
- Compare: https://github.com/ChronoAIProject/NyxID/compare/main...aelf-hzz780:codex%2Fwebsite-a-delivery?expand=1
- Suggested state: draft while independent re-acceptance of A-01–A-03 is pending.

## Title

feat(website): deliver approved NyxID website A

## Body

## Summary

- Implement the approved product HTML as 25 bilingual public routes with its audience showcase, solution examples, integrations, pricing and articles. Keep the v4.1 homepage wall as the original 24-card static grid; no B effects.
- Retain existing AuthFlow/login/register, invitation attribution, query propagation, trusted returns, authenticated root redirect and legacy blog URLs. Replace prototype-only success/trial actions with real registration/community links; unsupported newsletter submission is clearly unavailable.
- Commit the source-migration tool, screenshots, runtime adaptations and review evidence in docs/plans/website-a-delivery.md and website-a-evidence. No auth/billing/backend changes, merge or deployment.

## Test Plan

- [x] 89 relevant frontend tests across 11 files; new regression tests cover the public path boundary, auth context and showcase/playback behavior. Backend tests are outside this frontend-only change.
- [x] Full production build, including TypeScript, legal prerender, credential-accept and mock-footprint checks.
- [x] Changed TypeScript ESLint; source generation reproduced byte-for-byte.
- [x] 25 routes × English/Chinese = 50 desktop browser checks, auth entry/query comparison against production, interaction and original tool-wall geometry checks.
- [x] Actual 390×844 browser checks of both repaired mobile login menus and full query preservation; bilingual custom CTA and all eight sidebar demo/language combinations checked. Initial viewport tooling limitation is resolved.
- [ ] Physical-device touch/performance, Firefox and WebKit validation remain unverified.
- [ ] Real-account OAuth/MFA completion and independent acceptance: pending, not claimed.

## Checklist

- [x] Native React/router/i18n integration with scoped CSS and animation names.
- [x] No new hardcoded secrets or credentials; the existing public referral behavior is preserved.
- [x] No mock login/provisioning, fake contact success or sensitive error disclosure.
- [x] Documentation, production adaptation matrix, commands, screenshots and residual risks recorded.

Review baseline: codex/website-a-base mirrors upstream commit 8280098785b477a9980cc8aed98f43789d6fd3dc. The fork main branch is 12 commits behind; this non-protected base keeps review scoped to the delivery without pushing main. Implementation commit: a0ae2b3f3bf3908387978c3ca2067344f36b6c46. The first independent acceptance found A-01–A-03; fixes are included in this branch and independent re-acceptance is pending.
