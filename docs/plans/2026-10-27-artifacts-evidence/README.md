# Artifacts #2037 evidence archive

Created from the exact records cited by the standalone sign-off and review ledgers.
The source records were read from `/private/tmp` (the `/tmp` path in the ledgers is
a macOS symlink) and copied byte-for-byte into `raw/`; `source-identity.json` retains
the original source path and SHA-256 for each record.

- Reviewed source tip: `3a2c9c996a756ed4f58673c122a6d4775f0e66b7`
- Source digest: `350bb2497cd79ba222e340ca90874ccc126e74052edff168a70baa62fed1cc33` over `1383` tracked inputs
- Rebuilt binary SHA-256: `2cd44c5d63639d4c315bc3dbe64bac7cd7e7a2f77d8a0d1adfb4fcaf73a8d9ce`
- Archive files: `99` (README, 94 raw records, two redacted derivatives, and two source identity/manifest files; `SHA256SUMS` is the checksum index itself)
- `SHA256SUMS` covers all archived records and derivatives.

The archive includes the final successful compile, malformed-status, current-tip
router/privacy/lifecycle, carried-forward reservation/BSON/retention/lineage, and
warnings-denied Clippy records, plus the historical Clippy failure and bounded
diagnostic/fixture failures referenced during review. Frontend lightweight logs and
runner scripts are included where they were cited or needed to reproduce command
identity.

`redacted/secret-scan.json` records the high-confidence credential scan. It found no
credential-shaped values; fixture marker words are explicitly classified as test data.
`redacted/metadata-redacted.json` is the separate identity-only derivative.

This archive is evidence bookkeeping only. It does not change source validation, the
combined I-01--I-05/I-07 gate, the live Sandbox/Web gate, or the BLOCKED ticket verdict.

The read-only combined integration source was reviewed separately from this
standalone archive. The integration checkpoint is `ed2c6a52ea5a20205b92c1da45d9c84fc1edbc80`
(parent `b1d627eca05d4227f6cd94655f17dccb313fb96e`). The Telegram composition
tip was `427ca78b8`, including Telegram's `b67ad9d81` source/docs; the actual
subsequent current-main merge was `5e8812b6f6b6c8137b23f116a910cc39c303a1cf`
against current main `49f5a61214f6a29485471adea8185763e5ef424d`. The composition
review found only the three expected additive conflicts and no runtime logic
authored by conflict resolution. No Cargo compile or combined test run has been
claimed for those integration commits.

Publication repair: the repository-wide `*.log` ignore rule initially hid 56
indexed raw log/argv records from Git. Before force-adding, every one of the 99
`SHA256SUMS` paths was checked for local existence and matching SHA-256. Only
those exact 56 ignored paths were force-added; no ignore rule changed. The
post-repair audit requires all 99 indexed paths to be tracked with zero missing
or blob/hash mismatches.
