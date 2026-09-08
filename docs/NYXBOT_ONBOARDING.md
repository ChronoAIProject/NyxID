# Nyxbot Onboarding Integration

## Entry and Implemented Behavior

- Implemented inside the existing React frontend at `/onboarding`, outside the dashboard's unrelated AI Services onboarding takeover.
- `/onboarding?channel=telegram` and `?channel=whatsapp` preselect the corresponding channel. This is a frontend entry convention to give the bot team, not an existing bot session protocol. Direct visits have no preselection.
- Authentication uses NyxID's existing social endpoints, registration/login routes, trusted same-origin `return_to`, and NyxID app device login. Email, invite codes and MFA remain in the existing authentication pages.
- The sign-in screen always shows Google, GitHub, Apple and NyxID app in that order. `/public/config` controls whether a social button can begin OAuth, not whether its row exists. An unavailable method explains its state without navigating or creating a fake login. The optional email link follows `email_auth_enabled`.
- Google Workspace uses the real `api-google` catalog entry, `POST /keys`, provider OAuth initiation with the placeholder key ID, and the existing authorization status query. Both Drive file access and Calendar management must actually be granted. A callback URL saying `status=success` is insufficient.
- Existing personal Google connections with the required scopes can be reused. Organization connections are excluded because this flow has no organization selector or organization ownership contract.
- A signed-in visit opens Data source directly and displays the real account name (email fallback). The page explains Drive/Calendar consent separately from NyxID sign-in. It does not initiate OAuth on mount or treat account login as data access.
- Telegram uses the existing channel token schema, `useAppForm` and `POST /channel-bots`. The default label is `Nyxbot Telegram`, under the signed-in user's personal account. The returned channel ID is read back for webhook readiness; a stored bot is not assumed to be live.
- Per-account, per-tab session storage contains only channel choice and resource IDs. Bot tokens remain in the form/mutation lifecycle and are cleared on successful submission or channel change. Server records remain the source of truth.
- English and Simplified Chinese use a feature-scoped i18next instance. The existing theme store and theme application hook are reused. Inherited NyxID authentication panels keep their existing translations and behavior.

## Design Reference

Inspected Figma file `Ee1gMJqg6DTv02PEzLqbZH`, page `Nyxbot / Prototype aligned / 08 Sep`:

| Page | Figma frame |
| --- | --- |
| Original account step (superseded by the supplied X01 sign-in reference) | `17:117` |
| Original data source (superseded by M02/D02 below) | `17:225` |
| Telegram channel | `17:312` |
| Telegram chat linking | `17:453` |
| WhatsApp channel | `17:536` |
| External Meta reference | `17:667` |
| Monthly cap | `17:737` |
| WhatsApp chat linking | `17:814` |

Help frames: `17:904`, `17:910`, `17:916`, `17:922`, `17:928`, `17:934`, `17:940`.

The implementation renders the web content, excluding the 352 x 706 device shell and browser/address-bar illustration. It uses the inspected white surfaces, compact headings, source/channel grids, contextual help and bottom actions. Inter and action/spacing values also follow the linked PRD's web surface. Local assets avoid a runtime font or brand-image CDN dependency.

The account screen follows the user's subsequent `X01 / External / NyxID sign-in` screenshot: a NyxID brand header, left-aligned title and continuation subtitle, four provider rows and a sign-up link. This authentication screen has no onboarding progress bar or sticky Continue/Back footer. Existing locale and theme state still apply.

The Data source screen follows the subsequently inspected `Nyxbot / Web onboarding / 08 Sep` page in the same file:

| Page | Figma frame | Design size |
| --- | --- | --- |
| M02 / Web / 2. Data source | `30:179` | 390 x 844 |
| D02 / Desktop web / Data source | `30:940` | 1440 x 960 |

It uses the Nyxbot setup header, three-step status strip, signed-in account row, selected Google Workspace and unavailable Notion tiles, separate Drive/Calendar descriptions, combined-consent explanation, and Back / Connect Google actions. The existing shell has a scoped `setup` variant for this page: 24 px mobile side padding, 64 px mobile / 80 px desktop header, 110 px source tiles, 48 px actions, and 560 px desktop inner content. Locale and theme state are retained without adding controls absent from these frames. The header's return-to-Nyxbot control remains disabled until a real guide-chat URL is supplied.

Rules added for states absent from the supplied frames: scrollable content with a sticky action footer on short screens; existing theme tokens for dark mode; explicit loading, failed authorization, missing permissions and unavailable-integration notices. Other onboarding steps retain the earlier compact layout with a 420 px desktop maximum and their existing locale/theme controls. Authentication back-navigation preserves the signed-in account.

## Backend Meeting Checklist

The owner confirmed on 8 September 2026 that the Nyxbot session, chat-pairing and spending-cap APIs do not yet exist. No production mocks or invented endpoints are included.

- [ ] **Bot session and entry link:** establish the real link format, session lookup/create API, authenticated ownership binding, session expiry, resume rules and the Google/channel resource IDs attached to that session. The current frontend query parameter is only a selection hint.
- [ ] **10-minute pairing code:** provide issue/refresh/read-status contracts, server expiry, single-use consumption, chat/sender ownership checks, and the actual Telegram/WhatsApp destination. Clarify what happens to the old code on refresh. Existing NyxID notification and CLI pairing codes have different purposes and are not reused.
- [ ] **WhatsApp monthly budget:** provide server persistence for SGD 120, SGD 250 and unlimited; define billing month, currency, metering source and race-safe enforcement. At the cap, stop automatic replies, continue receiving messages and notify the owner. Define how the budget is committed before activation and how failure/retry is handled.
- [ ] **Meta/payment model:** reconcile the PRD's BSP-held WABA with the repository's Tech Provider/customer-pays-Meta model and existing Business App coexistence support. Confirm number migration and billing responsibility.
- [ ] **Google platform readiness:** the repository's `scope_catalog::platform_scope_allowlist("google")` is currently identity-only. Enable and verify the shared OAuth app for `drive.file` plus the agreed Calendar management scope. An existing authorized personal BYO connection can be reused; this onboarding does not ask end users to create a Google OAuth application.
- [ ] **End-to-end handoff:** after confirmed pairing, define the return to the Nyxbot guide chat and the server completion event. Connection success alone must not claim that automated replies are enabled.

## Current Integration Limits

- Telegram registration can be completed when the backend and a properly scoped Google connection are available. Chat linking displays an unavailable state; code copying and chat opening remain disabled because no real code/destination contract exists.
- WhatsApp reads the existing managed-onboarding readiness endpoint. Meta launch and completion are deliberately disabled: the existing completion endpoint activates the bot immediately and accepts no budget. The real Meta SDK and completion implementation remain in `components/channels/managed-whatsapp.tsx` for integration once activation ordering is defined.
- A temporary **Review spending cap** action opens the designed cap choices without implying Meta authorization. **Set cap & connect** remains disabled and nothing is saved. Replace this blocked-state action with the designed Meta-to-cap sequence when the contracts above exist.
- No remote OAuth consent, real Telegram registration, or paid Meta activation was performed during development. Automated tests use isolated API responses; they do not prove a deployed service integration.

## Local Preview Against Hosted NyxID

The owner selected `https://nyx.chrono-ai.fun` for integration. Its API is `https://nyx-api.chrono-ai.fun`; the public configuration enables Google, GitHub and Apple, requires invite codes for registration, and disables email authentication. Run the existing frontend against that environment from `frontend/`:

```sh
BACKEND_URL=https://nyx-api.chrono-ai.fun \
FRONTEND_URL=https://nyx.chrono-ai.fun \
npm run dev -- --host 127.0.0.1 --port 3000 --strictPort
```

`FRONTEND_URL` sets the existing Vite proxy's upstream Origin/Referer. The proxy also uses the repository's existing local cookie rewriting. Neither setting changes the hosted backend's configuration or OAuth return URL policy. This preview uses the selected live environment and its real account data.

On the current development machine, native Node HTTPS connections need the already-running local network proxy. With Node 25, also set `NODE_USE_ENV_PROXY=1`, `HTTPS_PROXY=http://127.0.0.1:7897`, `HTTP_PROXY=http://127.0.0.1:7897` and `NO_PROXY=127.0.0.1,localhost` when starting Vite. Without these, the proxy can fail with `ECONNRESET` before TLS negotiation. Vite's proxy disables the global agent, so `vite.config.ts` explicitly supplies a native HTTPS agent with `proxyEnv` when the flag is enabled for an HTTPS backend. Origin/Referer rewriting runs on the proxy's `start` event, before outgoing request creation, because a proxy agent may send headers before `proxyReq`. TLS certificate verification remains enabled. The opt-in requires a Node version supporting `Agent.proxyEnv` (verified with 25.9.0); machine-specific addresses remain in the local launch environment.

For a local browser session, use the existing **Continue with the NyxID app** flow. Read its displayed code, open the displayed `https://nyx.chrono-ai.fun/login/device` address in a browser already signed in to the same NyxID environment, enter the code, and review and approve that login. The local page polls `/api/v1/auth/device/poll-web`, receives a real browser session cookie through the proxy and returns to `/onboarding`. No account cookie, access token or OAuth secret needs to be copied between browsers or applications.

Verified on 8 September 2026: proxied public configuration returned HTTP 200 with the three social providers, and this device login flow established the local browser session and reached the data-source step. That step still reported Google Workspace authorization unavailable; account sign-in does not establish Drive/Calendar grants. The Google platform-readiness item above remains open.

The revised Data source page was checked in the existing authenticated browser at 375 x 667, 390 x 844 and 1440 x 960. There was no horizontal overflow; the short-screen notice was fully readable after scrolling and remained above the sticky actions. The desktop inner content measured 560 px. Temporary viewport overrides were reset after verification.

Direct social login is a separate local-preview limitation: the hosted backend ignores a localhost `return_to`, and its provider callback remains on the hosted API domain. Initiating through a localhost proxy also puts the OAuth state cookie on localhost rather than the hosted callback domain. Changing `BACKEND_URL` alone therefore does not make Google/GitHub/Apple return to localhost. Verify those complete flows with the frontend deployed under the backend's configured frontend origin, or with a dedicated environment whose OAuth callbacks and frontend origin are configured together. Aevatar's `NYXID_CLIENT_ID` belongs to its separate OAuth client flow and is not a substitute for this first-party browser session.

## Assets

- Google glyph: reused from the existing NyxID authentication provider button paths.
- NyxID, Telegram and Facebook marks: existing repository assets/glyphs.
- WhatsApp mark: Simple Icons (`https://github.com/simple-icons/simple-icons`, CC0).
- Inter Latin variable font: Google Fonts distribution, with the Inter SIL Open Font License alongside the font in `frontend/public/nyxbot-inter-LICENSE.txt`.

Local validation follows the personal incremental frontend policy: related Vitest tests and changed-file ESLint/Prettier only. Full frontend tests, typecheck and production build belong to GitHub CI.
