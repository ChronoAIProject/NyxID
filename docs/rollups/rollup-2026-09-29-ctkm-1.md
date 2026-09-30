# Rollup: 2026-09-29 ctkm-1

This rollup was created from `main` commit
`cf02492bbf2ecd70099f3f9afec7013d9f2f8598` and fast-forwarded to `main` at
`e96a50782a97cfe6fe46d357019325de0cc36867` before the changes below landed, so
it can merge into `main` without reverting later work. It makes signup
invitation codes an operator choice now that billing is in place, redesigns
the OAuth consent screen to match the connection flows, and keeps historical
usage priced on the Billing → Usage page after a service price is removed.

## Included changes and provenance

Each source PR remains the authoritative implementation discussion and merge
record. For each, the source and squash deltas share a stable patch ID and
`git merge-tree` reproduces the landed tree exactly.

| Source PR | Reviewed source head | Landed squash | Intended behavior / scope |
| --- | --- | --- | --- |
| [#1690](https://github.com/ChronoAIProject/NyxID/pull/1690) | `dc5fb1b4be000995027c90412701d66c97766588` | `e7691afe605770e67128376bf87b610c5e8f37a8` | Gate the signup invitation-code requirement behind the global, default-on `auth:invitation-code` feature flag, replacing the `INVITE_CODE_REQUIRED` environment variable. Invitation-code management is retained. |
| [#1701](https://github.com/ChronoAIProject/NyxID/pull/1701) | `fbf21ac1e7dbed35656b777f64526d8d5bf0f867` | `68ad6d8146460ebd721158522c9fabea4256140e` | Redesign `/oauth-consent` to match the connect-link and channel-bot screens: plain-language permissions, collapsed app details, no client-side risk badges, catalog service descriptions, and scrollable service lists. `GET /api/v1/user-services` list rows add optional `catalog_service_description`. The decision form posted to `/oauth/authorize/decision` is unchanged. |
| [#1702](https://github.com/ChronoAIProject/NyxID/pull/1702) | `595ed4b3f192aea999def3abf46b2b6dfce9b859` | `c3d49700f98be5bb31f1beacb3700bb25564fbbd` | Keep historical usage priced after a service price is removed: price removal marks the code's `billing_rate_cache` row `retired_at` instead of deleting it, `GET /api/v1/billing/usage` prices historical groups by grant-settled derivation → cached rate → exact per-row reservation rates, and the Usage page shows `≥` lower bounds instead of Unavailable when only some records are unpriced. Patch ID `8a146dbbd181cf376a67c6573838770d2b228a4e`; landed tree `8f684a12f046fc81bf2ade427ca4a17fdb12d7e3` equals the source tree. The source branch was created from this rollup's head `b98949e1`, so no main-sync merge was involved. |

The six `main` commits between the rollup base and `e96a5078` (#1694–#1699,
NyxBot channel/gateway work and service-account catalog skill assignment) are
inherited `main` history, not part of this rollup's diff. The only overlap was
`backend/src/services/feature_flag_service.rs`, where `main` added the
`nyxbot:gateway-*` flags alongside the new `auth:invitation-code` flag; both
definitions and the pinned registry-key test keep every key.

## Problem and resulting behavior

New registration required an invitation code whenever `INVITE_CODE_REQUIRED`
was set, and changing that needed a redeploy. With billing now provisioning a
wallet for every new account, whether signup stays invitation-only should be a
runtime decision.

- `auth:invitation-code` is enabled by default, so behavior is unchanged
  until a platform admin turns it off under Admin > Feature Flags. No restart is
  needed.
- Enabled: email signup requires a valid code; browser social signup can
  redeem one; existing social users still sign in; native social token exchange
  still rejects first-time signup.
- Disabled: email and first-time social signup succeed without a code.
- The flag is global only (a new account has no user or org scope yet); scoped
  overrides are rejected and the admin UI shows only the global control.
- The backend resolves the flag on every signup attempt. `GET
  /api/v1/public/config` reports the effective requirement, and the signup
  screen refreshes it so an open form follows a change; the server stays
  authoritative if the flag flips between display and submit.
- Admin invitation-code management (page, API, CLI) remains; the page states
  when codes are not currently required. Organization membership invitations
  are separate and unchanged.

## Rollout

- Deploy the backend before the frontend.
- `INVITE_CODE_REQUIRED` is no longer read. Deployments that set it to `false`
  will require invitation codes after this lands until a platform admin
  disables `auth:invitation-code`.
- Existing invitation-code data is retained.

## Historical usage pricing after price removal (#1702)

Removing or re-authoring a NyxID-authored service price (a legacy price, a
lane, or a component) deleted the retired Lago code's `billing_rate_cache` row.
Historical `usage_meter` rows recorded under that code, which carry no persisted
gross cost, could then no longer be priced: `GET /api/v1/billing/usage` returned
null costs for the group and the Usage page rendered **Unavailable** for the
service (Chrono LLM) and for the whole Spend total, while grant funding stayed
known from the rows' own consumption records. Credit grants were not the cause.

- Price removal marks the row `retired_at` and never deletes it.
  `fresh_rate` refuses a retired row for new reservations; the usage handler,
  settlement fallbacks and admin usage keep reading it; a full-row re-sync of
  the same code replaces the row.
- Historical groups are priced by the first rule that applies: every row
  settled with a zero wallet debit and zero consumed allowance units → gross
  equals the grant consumption (exact); the cached model-specific, then generic,
  rate; every row carries its reservation rate → exact per-row
  `rate × quantity ÷ 1e12` in Decimal128 credits (allowance = rate × units), as
  exact settlement computes; otherwise gross, wallet and allowance stay null
  with grant credits still reported. Legacy Int64 funding keys are honored.
- The Usage page shows `≥ n` lower bounds with an unpriced-record count when
  only some charged records are unpriced; Unavailable appears only when no
  charged record is priced; free rows never make a total known.
- Admin usage keeps only the cached-rate historical rule; retained rows mean
  future removals stay priced there, while codes deleted before this change
  remain unknown on that page. Documented in `docs/BILLING_UI_GLOSSARY.md`.
- Review: Claude Fable 5.1 (line by line), a Claude Opus 5.5 adversarial pass
  (all findings resolved before merge) and two GPT-6-Astra passes. Astra
  verified the exact-`Credits` valuation, the grant-settled derivation and the
  page rules, and withheld its sign-off on two retention-safety gaps outside
  the pricing fix: settlement of new usage could prefer a retained model-specific
  cache row after re-authoring (no production writer creates such rows), and the
  pre-existing sync/plan-refresh write order can re-activate a retired row after
  a stale operation; partial sums also use the rounding formatter. A follow-up
  source PR closing all three is in progress for this rollup.

### Rollout (#1702)

- Upgrade all backend replicas before removing or re-authoring prices: old
  replicas ignore `retired_at` and could reserve against a removed price until
  the 900 s rate TTL expires, whereas deletion refused immediately.
- `retired_at` is additive; no migration is required.

## Rollup-only follow-up

- `handlers::nyxbot::tests::the_same_question_is_not_worked_on_twice`
  (inherited from `main` #1697) failed once in the #1700 backend coverage run.
  The test waited a fixed 300 ms for a relayed Lark message that the handler
  accepts (202) before recording; the instrumented coverage build ran slower
  than that. It now polls, bounded to 10 s, until the message is recorded. The
  assertions are unchanged. This is test-only and should also land on `main`.
- `AuthFlow — register > allows email registration without a code when the
  flag is disabled` (added by #1690) failed once in the frontend coverage run:
  submit had not reached the register call within the default 1 s `waitFor`.
  It passed in the other three runs on the same head and on rerun. Its wait is
  now 5 s, matching existing `waitFor` timeouts in the frontend tests.
- `Backend Test` and `Backend Billing Smoke` on head `7304e0a7` died five times
  out of seven attempts with exit 143 ("The runner has received a shutdown
  signal") four to five minutes into compiling the `nyxid` test binary, before
  any test ran; the same job passed on the pull_request run of that head and
  the 04:17 pull_request run had hit the identical failure. The runner's memory
  is exhausted while the full-DWARF test binary is codegen'd and linked. Both
  jobs now build with `CARGO_PROFILE_TEST_DEBUG=line-tables-only`, which keeps
  panic locations and backtraces and changes nothing for local development or
  the coverage jobs. CI-only; should also land on `main`.

## Verification

- #1690 CI on `dc5fb1b4be000995027c90412701d66c97766588` (the source branch with `e96a5078` merged):
  24 checks passed, 10 skipped, none failed, including backend
  tests, backend and frontend coverage, Clippy, KMS feature builds, billing smoke
  and all CodeQL languages.
- Locally on the merged tree against a MongoDB replica set: 45 feature-flag,
  57 invitation, 36 auth-handler, 57 social-signup and 3 public-config backend
  tests passed; 874 frontend flag/lib tests passed and TypeScript compiled.
- #1702 CI on `595ed4b3f192aea999def3abf46b2b6dfce9b859` (the source branch,
  already on this rollup's head `b98949e1`): 24 checks passed, 10 skipped, none
  failed — Backend Test, backend and frontend coverage, Frontend, Format,
  Clippy, KMS feature builds, billing smoke, image inputs, wizard freshness and
  all CodeQL languages. The landed tree equals the source tree, so that run
  exercised exactly the code now on the rollup.
- #1702 locally on a MongoDB 8.0 replica set: `billing_integration_tests::usage`,
  `services::billing::{pricing,reservation,tests,funding,reconcile,meter,exact_tests}`
  and `handlers::services::tests` — 141 passed, 0 failed; Clippy `-D warnings`
  clean; frontend eslint 0 errors, vitest 66/66 for the billing and credits
  files, and the type-checking build passed.
