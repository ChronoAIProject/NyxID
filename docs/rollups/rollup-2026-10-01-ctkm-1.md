# Rollup: 2026-10-01 ctkm-1

This rollup starts from `main` commit `80a153d31200eaa8eafbf005cc13d3e3894181a9`
and integrates [#1721](https://github.com/ChronoAIProject/NyxID/pull/1721) and
[#1725](https://github.com/ChronoAIProject/NyxID/pull/1725).
After the #1721 squash landed, `main` advanced through #1722 to
`479840e8`. The rollup merged that commit as `478b960a`, preserving the latest
main history without conflicts or changes to the Admin Usage files. #1725 was
then raised against the rollup head `e09c7c8e` and squashed on top of it.
Admin Usage needed token counts that users can combine without double counting,
filter options and service details ready when opened, and data tables that can
stay visible or collapse per panel. The OpenAI plugin portal also needed to
verify control of the NyxID MCP host before the Codex/ChatGPT plugin can be
submitted.

## Included changes

| Source PR | Validated source head | Landed squash | Result |
| --- | --- | --- | --- |
| [#1721](https://github.com/ChronoAIProject/NyxID/pull/1721) | `0e24c05eee93101f562c8a165ee5f53991822e71` | `5983b66537b724b057880a774dc45bd01484230f` | Configurable token views, panel data tables, and background loading for Admin Usage. |
| [#1725](https://github.com/ChronoAIProject/NyxID/pull/1725) | `e70ba24bd4ad58735cfa83d3432f1c94d10c2886` | `d85799bfb9a615b87256acd78e9674552051e1fb` | OpenAI apps domain verification challenge served from the MCP host. |

For #1721, the source and landed commits have the same tree
(`a9914949583afb92fee83004879bc7886eaf17e3`), so the squash contains the
validated source exactly. The source PR was rebased onto this rollup's `main`
base before its final CI run; GitHub reported no merge conflicts.

For #1725, the source and landed commits have the same tree
(`ffb7e232c1edba00be67fb368dd67df06f181104`). The PR was based on the current
rollup head, so no rebase was needed; GitHub reported it mergeable and clean.

## Behavior

- The dashboard and list support multi-selected Total, Input, Output,
  Cache-read, and Cache-write token views. Total already includes Input and
  Output, so selecting them together displays those counts without adding
  them twice. Selected cache counts still add. An inline tooltip explains
  the overlap.
- Each analytics panel has a Data table setting: Always open by default, or
  Accordion. The workspace API accepts and validates the persisted setting.
- Services, Billing accounts, and Acting users options load when Usage opens.
  Visible user details preload in the background. React Query keeps these
  results fresh for five minutes and cached for ten minutes.
- `GET /.well-known/openai-apps-challenge` returns the optional
  `OPENAI_APPS_CHALLENGE_TOKEN` as bare `text/plain`, the format OpenAI's
  plugin portal requires for domain verification. When the variable is unset
  the path returns 404, as before. The OIDC, OAuth authorization-server,
  protected-resource, and JWKS discovery documents are unchanged.

## Validation And Rollout

The rebased #1721 source head passed 24 CI checks with no failures, including
the frontend, backend tests, billing smoke, coverage, Rust feature builds, and
CodeQL. Locally, all 18 Admin Usage browser tests, 27 focused frontend tests,
TypeScript, lint, formatting, and backend `cargo check` passed. The landed
source tree is identical to the tested head.

The #1725 source head passed 21 CI checks with no failures, including backend
tests, billing smoke, coverage, Clippy, formatting, Rust feature builds, and
CodeQL; frontend, CLI, and mobile jobs were skipped as unaffected. Locally, the
7 discovery handler tests (5 existing, 2 new) and `cargo clippy -D warnings`
passed. A local server run returned the exact token with `200 text/plain` and no
redirect, returned 404 with the variable unset, and served all four discovery
documents unchanged.

Deploy the backend workspace change before the frontend. An older backend
rejects the new `table_display` field when saving panel settings.

To complete OpenAI domain verification after deploying the backend, set
`OPENAI_APPS_CHALLENGE_TOKEN` to the token shown in the portal, confirm
`https://nyx-api.chrono-ai.fun/.well-known/openai-apps-challenge` prints only
that token, then verify in the portal.
