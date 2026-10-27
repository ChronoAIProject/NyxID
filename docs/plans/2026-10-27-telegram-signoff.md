# Telegram onboarding review status

PR #2057 addresses scoped Telegram creation/recovery and diagnostics for #2045/#1974.
The same Astra PM reviews the persistent Sol implementation.

- Published historical head: `c86a39bd6f490924bcbbaf44d1c4edbe0944c0c9`; its full CI passed.
- Historical combined source: `56ddb0d366c90f791d9e8f119c780958701194b3`; 61 exact tests and local Clippy passed. This is not the newly required published combined/full-suite/Rust 1.98.1 evidence.
- User fixlist T-1/T-2/T-3/T-5/T-6 source corrections are personally reviewed: `SOURCE_REVIEW_COMPLETE` for the 17-file source patch `726d5931f344db668209d1ecb82ebd3f8038ff3b67ea092d89cac4e709f054d1` against the historical published head. Its 30 focused backend regressions, affected frontend checks and required lint remain unexecuted; `LOCAL_VALIDATION_COMPLETE=false`.
- #1984 already supplied the gateway creator authorization change. This PR's #1974 scope is diagnostics, guidance and setup wording. Current-row ranking does not prove exactly one stored row after repeated failed links, nor an actual Telegram owner link or specialist reply. Those claims remain open.
- #2045 native acceptance remains: named-client Create bot, same-account `t.me/newbot` fallback/recovery and actual intended-assistant reply. #1974 still needs actual Personal specialist connection, owner linking and reply.
- Final ticket verdicts: **#2045 BLOCKED; #1974 BLOCKED**. `ASTRA_IMPLEMENTATION_APPROVED` is not issued.

Release requires the coordinated operator runbook: drain old Telegram admission/webhook/creation writers, upgrade every relevant replica, then expose recovery. Rollback must preserve uncertain dispatch, provenance and terminal records; issued native URLs cannot be revoked. See `docs/TELEGRAM_NEW.md` for the Telegram contract.

Historical review/evidence remains available in immutable commit `b67ad9d8116b4f20c0ac0aea5948785cf3a0052c` and the coordinator's preserved evidence packet. Detailed logs and machine manifests are intentionally excluded from the feature diff. The PR body must link the eventual published combined validation and distinguish that result from historical local proof.
