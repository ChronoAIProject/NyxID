# Specialist platform actions: verification record

Date: 2026-11-25

Base: 575c5f5962a4ad09ceab3b56fa03d89390a7cf21
Implementation SHA-256: 7f838c6dd0859f59d1c847e675ac7f55e0b2d9f54ad5fb70e68a515997b0d5b0
Files included: 49

The digest includes all changed and new implementation, test and contract files. This verification record and the independent Opus report are excluded so review evidence can be appended without changing the implementation snapshot.

## Validation

- Focused Vitest: 428 tests passed across 16 files, including exact Telegram request recovery without a client continuation and fresh setup with its explicit completion callback.
- Final R1/R2 delta: transport suite passed 171 tests, including a held handoff POST with scoped and full sidebar refreshes. A final focused selection passed 438 tests across 14 files; the selections overlap and counts are not additive.
- Playwright: four scenarios passed, including desktop/phone handoff, source return and reload, embedded Telegram setup, and one-time webhook secret dismissal/navigation/theme protection with the real setup component.
- Production frontend build passed, including the mock footprint check.
- Full frontend lint, typography and translation checks passed (existing repository warnings only).
- TypeScript build passed, including a repeat after the final transport change. Targeted ESLint on the changed modal, transport and tests also passed.
- Final Rust filtered tests passed using the installed rustup toolchain: `handoff_cards_are_offered_only_in_direct_specialist_user_turns` (1/1) and `private_cors_allows_analytics_narrowing_header_only_for_trusted_origin` (1/1). The final assertion rebuild finished successfully in 6m 55s. The linker emitted a non-fatal large unwind-table warning, and Cargo reported a dependency future-compatibility warning.
- `git diff --check` passed. The implementation digest was recomputed after both Rust runs and still matches the snapshot above; no source changes followed the runs.

## Review

Independent reviewer: Claude Opus 5.5, agent 01a21314-ff60-7bd0-a2a7-906636c228c2.
Final verdict: **APPROVED** for the implementation snapshot above. The first pass requested changes; all substantive findings, including the second-pass sidebar and assertion findings, were repaired. Opus's third pass found no remaining code findings, then final approval followed the passing final-tree Rust results. Opus independently recomputed its digest at 15:14Z and confirmed the reviewed implementation had not changed. The review records the accepted scope limits and distinguishes the reviewer's own frontend runs from parent-reported validation.
See [the independent review](specialist-platform-actions-opus-review.md).

## Scope limits

- Handoff rendering is for private personal specialist browser threads. Shared/organization/guest/group/channel/automation replies are inert.
- Browser receipts are bounded and local to this browser; they are not a server-wide or cross-device exactly-once guarantee.
- Native mobile and CLI may show a historical browser artifact as inert fenced text.
- Canonical advanced management links stay in the app using their existing pages; only supported setup journeys use dialogs and the shared completion callback.

## Snapshot files

- backend/src/handlers/assistant_nyxagent.rs
- backend/src/main.rs
- backend/src/services/assistant_nyxagent.rs
- backend/src/services/assistant_nyxagent_runtime.rs
- backend/src/services/assistant_nyxagent_tests.rs
- docs/CHANNEL_BOT_RELAY.md
- docs/chat/05-frontend-ui.md
- docs/chat/09-nyxbot-orchestrator.md
- frontend/e2e/specialist-handoff.spec.ts
- frontend/src/components/assistant/assistant-chat-page.tsx
- frontend/src/components/assistant/assistant-link-modals.test.tsx
- frontend/src/components/assistant/assistant-link-modals.tsx
- frontend/src/components/assistant/blocks/assistant-link.tsx
- frontend/src/components/assistant/blocks/nyxbot-handoff-card.test.tsx
- frontend/src/components/assistant/blocks/nyxbot-handoff-card.tsx
- frontend/src/components/assistant/blocks/text-block.tsx
- frontend/src/components/assistant/chat-message.test.tsx
- frontend/src/components/assistant/chat-message.tsx
- frontend/src/components/channels/channel-bot-connect.tsx
- frontend/src/components/channels/channel-bot-setup.tsx
- frontend/src/components/channels/channel-connect-link-content.tsx
- frontend/src/components/channels/channel-connection-shell.test.tsx
- frontend/src/components/channels/channel-connection-shell.tsx
- frontend/src/components/channels/telegram-new-connect.test.tsx
- frontend/src/components/channels/telegram-new-connect.tsx
- frontend/src/components/dashboard/api-key-create-dialog.test.tsx
- frontend/src/components/dashboard/api-key-create-dialog.tsx
- frontend/src/hooks/use-api-keys.ts
- frontend/src/hooks/use-channel-bots.ts
- frontend/src/i18n/locales/en.json
- frontend/src/i18n/locales/zh-CN.json
- frontend/src/i18n/locales/zh-TW.json
- frontend/src/lib/api-client.ts
- frontend/src/lib/assistant/assistant-link-target.test.ts
- frontend/src/lib/assistant/assistant-link-target.ts
- frontend/src/lib/assistant/mock-setup-journeys.ts
- frontend/src/lib/assistant/nyxagent-http-fixtures.ts
- frontend/src/lib/assistant/nyxagent-transport.test.ts
- frontend/src/lib/assistant/nyxagent-transport.ts
- frontend/src/lib/assistant/nyxbot-handoff-context.ts
- frontend/src/lib/assistant/nyxbot-handoff-receipts.ts
- frontend/src/lib/assistant/nyxbot-handoff.test.ts
- frontend/src/lib/assistant/nyxbot-handoff.ts
- frontend/src/lib/assistant/platform-setup.test.ts
- frontend/src/lib/assistant/platform-setup.ts
- frontend/src/pages/channel-connect-link.test.tsx
- frontend/src/pages/channel-connect-link.tsx
- frontend/src/schemas/assistant-nyxagent.ts
- frontend/src/schemas/nyxbot-handoff.ts
