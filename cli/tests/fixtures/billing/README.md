# Billing response fixtures

`config.json`, `rates.json`, `overview.json`, `service.json`,
`catalog-service.json`, `catalog-priced-service.json` and `executions.json`
were captured through read-only production GETs on 2026-10-20. They pin the
deployed response shapes for admin billing, catalog reads and the empty
self-scoped execution list. No production mutation or paid request was made.

Personal names, emails and user identifiers were removed. Remaining identifiers
were replaced with deterministic dummy UUIDs; secret values are absent. Public
catalog slugs and nonsecret metric codes remain so filtering tests exercise the
real field shapes.

`managed-service.json` is synthetic. `set-managed.golden.json` is the canonical
compact JSON request generated from the test's explicit contract, including an
empty operation-price array to clear incompatible previous overrides. Live
tariff freshness tests replace the synthetic timestamp relative to the clock.
