# Rollup: 2026-09-25 ctkm-1

This rollup starts from `main` commit
`1b031c77062880e572ec375041a4c029241a86f1` and collects
[PR #1662](https://github.com/ChronoAIProject/NyxID/pull/1662),
`feat: add configurable billing analytics workspace`.

## Purpose

The source branch adds the production admin Usage analytics page at
`/admin/usage`. It replaces the earlier preview-only direction with a persisted
workspace backed by real NyxID usage data. Administrators can use Dashboard and
List views, choose Operations, Overview, or Explorer templates, configure
filters and measures, add and arrange chart panels, resize and drag panels on a
three-column grid, and save named views. Operations is the default template.

The implementation uses the existing Recharts frontend dependency and the
existing usage and billing records. It also preserves the separate user-facing
Billing & Usage page introduced by the base branch, including its Billing and
Usage tabs.

## Rollup acceptance

Before this rollup is merged, PR #1662 must be rebased or merged cleanly onto
this branch and pass its required checks, including frontend tests, backend
tests, Rust checks, CLI checks, wizard bundle freshness, CodeQL, and all
coverage gates. The final branch must retain both `/billing` and `/admin/usage`
routes, contain no unresolved conflict markers, and have a clean working tree.

The rollup should be merged with a squash merge only after the source PR is
green and GitHub reports it as mergeable. The source PR and this record are the
provenance for the analytics workspace integration.
