# Rollup: 2026-10-01 ctkm-1

This rollup starts from `main` commit `80a153d31200eaa8eafbf005cc13d3e3894181a9`
and integrates [#1721](https://github.com/ChronoAIProject/NyxID/pull/1721).
Admin Usage needed token counts that users can combine without double counting,
filter options and service details ready when opened, and data tables that can
stay visible or collapse per panel.

## Included change

| Source PR | Validated source head | Landed squash | Result |
| --- | --- | --- | --- |
| [#1721](https://github.com/ChronoAIProject/NyxID/pull/1721) | `0e24c05eee93101f562c8a165ee5f53991822e71` | `5983b66537b724b057880a774dc45bd01484230f` | Configurable token views, panel data tables, and background loading for Admin Usage. |

The source and landed commits have the same tree
(`a9914949583afb92fee83004879bc7886eaf17e3`), so the squash contains the
validated source exactly. The source PR was rebased onto this rollup's `main`
base before its final CI run; GitHub reported no merge conflicts.

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

## Validation And Rollout

The rebased source head passed 24 CI checks with no failures, including the
frontend, backend tests, billing smoke, coverage, Rust feature builds, and
CodeQL. Locally, all 18 Admin Usage browser tests, 27 focused frontend tests,
TypeScript, lint, formatting, and backend `cargo check` passed. The landed
source tree is identical to the tested head.

Deploy the backend workspace change before the frontend. An older backend
rejects the new `table_display` field when saving panel settings.
