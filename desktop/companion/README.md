# NyxID Companion

NyxID Companion is a local-first desktop pet that stays near the lower-right
edge of the screen and helps with one complete routine: remembering meal times,
offering a few explainable meal ideas, and learning from explicit choices.

## Current scope

- Three configurable local meal reminders with native notifications.
- Snooze, skip, quiet mode, tray controls, close-to-hide, and launch at login.
- Deterministic on-device recommendations based on mood, budget, dietary
  preferences, avoided ingredients, and explicit accept/dislike history.
- Atomic local persistence and reminder recovery after an app restart.
- A fixed external link to the NyxID Assistant for connected services and
  authorization. The local UI never receives a NyxID credential.

Nearby restaurant search, delivery ordering, payment, cloud preference sync,
and embedded Assistant chat are intentionally outside this first slice. The
meal loop keeps working without NyxID login or network access.

## Browser development

```bash
cd desktop/companion
npm ci
npm run dev
```

Open <http://127.0.0.1:1420>. Browser mode uses `localStorage`, simulates the
launch-at-login switch, and keeps the same reminder/recommendation contract as
the native app.

## Native development

Install the [Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/)
for the host OS, then run:

```bash
cd desktop/companion
npm ci
npm run desktop:dev
```

The Tauri app uses a transparent, always-on-top window. It stores
`companion-snapshot.json` in the OS application-data directory for
`dev.nyxid.companion`. The file contains meal settings and compact explicit
feedback only; it contains no passwords, API keys, NyxID tokens, browsing
history, or ambient activity data.

## Focused verification

```bash
npx vitest run \
  src/companion-app.test.tsx \
  src/domain/companion.test.ts \
  src/domain/meal-clock.test.ts \
  src/domain/recommendations.test.ts \
  src/runtime/browser-runtime.test.ts
npm run typecheck
npm run lint
npm run format:check

cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

Production frontend and native release builds run in the scoped GitHub Actions
workflow rather than as part of routine local validation.
