# Rollup: 2026-09-29 ctkm-1

This rollup was created from `main` commit
`cf02492bbf2ecd70099f3f9afec7013d9f2f8598` and fast-forwarded to `main` at
`e96a50782a97cfe6fe46d357019325de0cc36867` before the change below landed, so
it can merge into `main` without reverting later work. It makes signup
invitation codes an operator choice now that billing is in place, and
redesigns the OAuth consent screen to match the connection flows.

## Included changes and provenance

Each source PR remains the authoritative implementation discussion and merge
record. For both, the source and squash deltas share a stable patch ID and
`git merge-tree` reproduces the landed tree exactly.

| Source PR | Reviewed source head | Landed squash | Intended behavior / scope |
| --- | --- | --- | --- |
| [#1690](https://github.com/ChronoAIProject/NyxID/pull/1690) | `dc5fb1b4be000995027c90412701d66c97766588` | `e7691afe605770e67128376bf87b610c5e8f37a8` | Gate the signup invitation-code requirement behind the global, default-on `auth:invitation-code` feature flag, replacing the `INVITE_CODE_REQUIRED` environment variable. Invitation-code management is retained. |
| [#1701](https://github.com/ChronoAIProject/NyxID/pull/1701) | `fbf21ac1e7dbed35656b777f64526d8d5bf0f867` | `68ad6d8146460ebd721158522c9fabea4256140e` | Redesign `/oauth-consent` to match the connect-link and channel-bot screens: plain-language permissions, collapsed app details, no client-side risk badges, catalog service descriptions, and scrollable service lists. `GET /api/v1/user-services` list rows add optional `catalog_service_description`. The decision form posted to `/oauth/authorize/decision` is unchanged. |

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

## Rollup-only follow-up

- `handlers::nyxbot::tests::the_same_question_is_not_worked_on_twice`
  (inherited from `main` #1697) failed once in the #1700 backend coverage run.
  The test waited a fixed 300 ms for a relayed Lark message that the handler
  accepts (202) before recording; the instrumented coverage build ran slower
  than that. It now polls, bounded to 10 s, until the message is recorded. The
  assertions are unchanged. This is test-only and should also land on `main`.

## Verification

- #1690 CI on `dc5fb1b4be000995027c90412701d66c97766588` (the source branch with `e96a5078` merged):
  24 checks passed, 10 skipped, none failed, including backend
  tests, backend and frontend coverage, Clippy, KMS feature builds, billing smoke
  and all CodeQL languages.
- Locally on the merged tree against a MongoDB replica set: 45 feature-flag,
  57 invitation, 36 auth-handler, 57 social-signup and 3 public-config backend
  tests passed; 874 frontend flag/lib tests passed and TypeScript compiled.
