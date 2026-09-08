# Nyxbot Onboarding Integration

## Entry and Implemented Behavior

- Implemented inside the existing React frontend at `/onboarding`, outside the dashboard's unrelated AI Services onboarding takeover.
- `/onboarding?channel=telegram` and `?channel=whatsapp` preselect the corresponding channel. This is a frontend entry convention to give the bot team, not an existing bot session protocol. Direct visits have no preselection.
- Authentication uses NyxID's existing social endpoints, registration/login routes, trusted same-origin `return_to`, and NyxID app device login. Email, invite codes and MFA remain in the existing authentication pages.
- Google Workspace uses the real `api-google` catalog entry, `POST /keys`, provider OAuth initiation with the placeholder key ID, and the existing authorization status query. Both Drive file access and Calendar management must actually be granted. A callback URL saying `status=success` is insufficient.
- Existing personal Google connections with the required scopes can be reused. Organization connections are excluded because this flow has no organization selector or organization ownership contract.
- Telegram uses the existing channel token schema, `useAppForm` and `POST /channel-bots`. The default label is `Nyxbot Telegram`, under the signed-in user's personal account. The returned channel ID is read back for webhook readiness; a stored bot is not assumed to be live.
- Per-account, per-tab session storage contains only channel choice and resource IDs. Bot tokens remain in the form/mutation lifecycle and are cleared on successful submission or channel change. Server records remain the source of truth.
- English and Simplified Chinese use a feature-scoped i18next instance. The existing theme store and theme application hook are reused. Inherited NyxID authentication panels keep their existing translations and behavior.

## Design Reference

Inspected Figma file `Ee1gMJqg6DTv02PEzLqbZH`, page `Nyxbot / Prototype aligned / 08 Sep`:

| Page | Figma frame |
| --- | --- |
| Account | `17:117` |
| Data source | `17:225` |
| Telegram channel | `17:312` |
| Telegram chat linking | `17:453` |
| WhatsApp channel | `17:536` |
| External Meta reference | `17:667` |
| Monthly cap | `17:737` |
| WhatsApp chat linking | `17:814` |

Help frames: `17:904`, `17:910`, `17:916`, `17:922`, `17:928`, `17:934`, `17:940`.

The implementation renders the web content, excluding the 352 x 706 device shell and browser/address-bar illustration. It uses the inspected white surfaces, compact headings, source/channel grids, contextual help and bottom actions. Inter and action/spacing values also follow the linked PRD's web surface. Local assets avoid a runtime font or brand-image CDN dependency.

Rules added for states absent from the supplied frames: a centered content width up to 420 px on desktop; scrollable content with a sticky action footer on short screens; existing theme tokens for dark mode; explicit loading, failed authorization, missing permissions and unavailable-integration notices. Authentication back-navigation preserves the signed-in account.

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

## Assets

- Google glyph: reused from the existing NyxID authentication provider button paths.
- NyxID, Telegram and Facebook marks: existing repository assets/glyphs.
- WhatsApp mark: Simple Icons (`https://github.com/simple-icons/simple-icons`, CC0).
- Inter Latin variable font: Google Fonts distribution, with the Inter SIL Open Font License alongside the font in `frontend/public/nyxbot-inter-LICENSE.txt`.

Local validation follows the personal incremental frontend policy: related Vitest tests and changed-file ESLint/Prettier only. Full frontend tests, typecheck and production build belong to GitHub CI.
