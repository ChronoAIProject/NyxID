# Rollup: 2026-09-28 ctkm-1

This rollup starts from `main` commit
`567dd3edae1309f6db4cb313928d408752562d3a` and adds optional, token-derived
names for Telegram bot-token channel bots. It was then synchronized with
`main` at `35ff07157bef152c87fc506cf46b8596ad315a3a` so it can land without
reverting later `main` work.

## Summary

- In the Telegram bot-token flow (`/channel-bots/connect/telegram` and
  **Add Bot → Telegram bot token**), the bot name is optional.
- Pasting a complete BotFather token fills a blank name with the bot's Telegram
  name. A name supplied by the user or the `label` link parameter is never
  replaced.
- A blank name at submission is filled server-side from Telegram `getMe`:
  the display name (`first_name`), then the username.
- Other platforms and the `telegram-new` creation flow still require a name.

## Included changes and provenance

The source PR remains the authoritative implementation discussion and merge
record.

| Source PR | Reviewed source head | Landed squash | Intended behavior / scope |
| --- | --- | --- | --- |
| [#1675](https://github.com/ChronoAIProject/NyxID/pull/1675) | `d0ca394831873b26509f5dee10b03d75cfa3f1a6` | `1aab8f1d600c5a6df8ae7be23e1098c69d36f973` | Optional Telegram bot-token labels filled from the bot's Telegram profile, a human-only profile lookup endpoint, a Telegram-only blank-label fallback on create, and the saved `label` in the create response. |

[#1674](https://github.com/ChronoAIProject/NyxID/pull/1674) targets this rollup
but is still open and is not included.

`main` commit `35ff0715` ([#1673](https://github.com/ChronoAIProject/NyxID/pull/1673),
billing) landed after the rollup base. It was merged into the rollup without
conflicts; it is inherited `main` history, not part of this rollup's diff.

## Problem and resulting behavior

Channel setup links such as
`/channel-bots/connect/telegram?label=...` required whoever built the link to
choose a bot name. When omitted, the page used the generic name
"Telegram bot". NyxID already calls Telegram `getMe` to verify the token, but
discarded the bot's name.

Now the name field in the Telegram bot-token flow starts empty with the hint
"Optional. Leave blank to use the name from your bot token." After a complete
token is pasted, the form suggests the bot's Telegram name. It changes the
field only while it is blank or still holds the previous suggestion, so a
typed or linked name is kept. If the field is blank at submission, the backend
applies the same name. `label` remains fully supported and always wins when
provided.

## API changes

- `POST /api/v1/channel-bots/telegram/profile` accepts `{bot_token}` and
  returns `{username, display_name, label}` from Telegram `getMe`. It is on the
  human-only router and limited to 20 requests per user per minute. The token
  is never stored, logged or returned; the request `Debug` output is redacted.
- `POST /api/v1/channel-bots` accepts a blank or omitted `label` only for
  `platform: "telegram"`. A supplied label is stored unchanged. The response
  additionally includes the saved `label`; the change is additive.
- `BotIdentity` gains an optional `display_name`; only the Telegram adapter
  sets it. Derived labels are trimmed and truncated on a UTF-8 boundary to the
  128-byte API limit.

The CLI (`nyxid channel-bot register`) still requires `--label`; relaxing it for
Telegram is a possible follow-up.

## Integration verification

- The rollup's diff against `main` has the same stable patch ID
  (`76f64b1d5e86a770cd3e0969b72ae3fe78b3e80f`) as the #1675 source delta and
  its landed squash. Replaying the source onto the squash parent reproduces the
  landed tree (`93b3cdbe10b913b8b0754acce3abe7d8f28047a5`).
- The `main` sync merge had no conflicts. No #1675 file is an input to the
  CLI wizard bundle, and `wizard_bundle_freshness` passes on the merged tree.

## Validation evidence

Local verification of the #1675 source head:

- 130 backend tests passed against a MongoDB 8.0 replica set: channel-bot
  handlers, the Telegram adapter, `telegram_new` and manager suites, Aurinko
  and `channel_bot_service`. New tests cover blank/omitted-label fallback and
  kept explicit labels through the full create path, rejection of blank labels
  for other platforms, token non-disclosure by the profile lookup, `getMe`
  display-name parsing and byte-limited truncation.
- 1,127 frontend tests passed across channel components, schemas and hooks,
  plus the channel-bot page tests. New tests cover filling from a pasted token,
  never replacing a linked or typed name, and skipping lookups for partial
  tokens.
- `cargo fmt`, `cargo clippy --all-targets -D warnings`, `tsc -b` and ESLint
  were clean for the changed files.

Full CI for the synchronized rollup runs on the rollup PR.

## Rollup acceptance

- [x] The rollup contains the #1675 squash integration, and its diff against
  `main` matches the source patch.
- [x] The rollup is synchronized with current `main` without conflicts.
- [x] The rollup has no unresolved merge entries or conflict markers.
- [x] Local targeted backend/frontend checks and wizard freshness pass.
- [ ] Full CI passes on the synchronized rollup.
- [ ] Complete the required review and merge this rollup into `main`.
