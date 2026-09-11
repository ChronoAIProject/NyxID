# Nyxbot Onboarding Integration

## Entry and Implemented Behavior

- Implemented inside the existing React frontend at `/onboarding`, outside the dashboard's unrelated AI Services onboarding takeover. TanStack Router search state selects one of four distinct pages:

  | Step | URL |
  | --- | --- |
  | Account | `/onboarding?step=account` |
  | Data source | `/onboarding?step=source` |
  | Channel | `/onboarding?step=channel` |
  | Link chat | `/onboarding?step=link` |

  Missing/invalid steps normalize to Account with history replacement. Page actions push the destination URL; guards and callback cleanup replace it. Browser Back/Forward and refresh use the same route state. No step is inferred from an internal view toggle.
- `/onboarding?channel=telegram` and `?channel=whatsapp` preselect the corresponding channel. This is a frontend entry convention to give the bot team, not an existing bot session protocol. Direct visits have no preselection.
- Authentication uses NyxID's existing social endpoints, registration/login routes, trusted same-origin `return_to`, and NyxID app device login. Email, invite codes and MFA remain in the existing authentication pages.
- The sign-in screen always shows Google, GitHub, Apple and NyxID app in that order. `/public/config` controls whether a social button can begin OAuth, not whether its row exists. An unavailable method explains its state without navigating or creating a fake login. The optional email link follows `email_auth_enabled`.
- Google Workspace uses the real `api-google` catalog entry, `POST /keys`, provider OAuth initiation with the placeholder key ID, and the existing authorization status query. Both Drive file access and Calendar management must actually be granted. A callback URL saying `status=success` is insufficient.
- Connect Google is enabled once its provider route is loaded. Clicking creates/reuses the connection and requests both scopes, then navigates to the returned Google authorization URL. Catalog credential flags and scope lists do not suppress the click; the backend resolves credentials and enforces the request. Initiation failures show the API error and keep the account signed in. A failed attempt does not claim user cancellation or consent.
- Existing personal Google connections with the required scopes can be reused. Organization connections are excluded because this flow has no organization selector or organization ownership contract.
- A bare `/onboarding` visit starts at Account, including for signed-in users. All four sign-in methods remain visible; an existing session additionally offers **Continue as [account name]**. Account continuation and sign-in returns enter Data source. Explicit `step=source` stays there, even with existing authorization; Continue resumes Channel or the saved Link chat page after checking real grants. Back follows Link chat → Channel → Data source → Account and preserves the account session.
- Google service OAuth returns target `step=source` for grant verification. Legacy Channel or step-less callbacks with `provider_status=success|error` or `status` also resolve through Data source. After verifying effective grants, callbacks advance to Channel or the saved Link chat page and remove consumed callback flags; missing permissions/errors remain on Data source. Initial session/grant checks show a neutral loading screen. Unauthenticated downstream URLs replace with Account; Link chat without a stored bot reference replaces with Channel. A URL cannot supply a bot ID, session or completion claim. An explicit Source URL remains there when a pending grant becomes active, until Continue. Data consent is separate from identity sign-in, and OAuth is never initiated on mount.
- Google list (`GET /keys`), catalog (`GET /catalog/api-google`) and stored connection (`GET /keys/{id}`) queries belong exclusively to Data source and unmount when leaving that step. Plain Channel and Link URLs do not check Google permissions or redirect based on Google query errors; visiting those pages does not claim a Google grant. Channel makes its own `GET /user-services` query on entry, selects enabled services with the `api-google-workspace`, `ornn-api` or `chrono-llm-public` slug for registration, and makes no registration request until Connect channel is clicked. Its submission runs Telegram verification, required Aevatar authorization and registration. Link chat retains its own bot/registration status reads. Global NyxID session checks remain in place.
- Telegram uses `useAppForm` and the shared token limits with feature-localized syntax validation. On **Connect channel**, call Telegram `getMe` with the entered token, require a bot identity and `first_name`, replace whitespace with underscores and append one `_nyxid_bot` suffix: `Test01` becomes `Test01_nyxid_bot`. Bound the whole label to NyxID's 200-byte UTF-8 limit. Submit `{ platform: "telegram", bot_token, label, webhook_base_url: "https://aevatar-console-backend-api.aevatar.ai", service_ids }` to the existing Aevatar `POST /api/channels/registrations` through the NyxID `/api/v1/proxy/s/aevatar` transport. `service_ids` contains deduplicated `UserService.id` values for the three exact slugs above, with `is_active: true` and a personal source or an org source with `allowed: true`. Pass the available matching IDs, including an explicit empty array when none exist; do not substitute catalog/key IDs or unrelated services. Block submission while the service list is pending or failed; lookup retry preserves the token. Aevatar owns creation of the NyxID bot and relay route; the frontend does not separately post to `/channel-bots`. Telegram rejection stops before registration. Both requests share one pending state and duplicate-submit guard; errors never echo token-bearing provider messages.
- Aevatar registration requires a Bearer even when the proxy's identity assertion already identifies the account. Read Aevatar's public `/api/auth/nyxid/config` through the existing proxy, then use NyxID's **JSON-mode `/oauth/authorize` + PKCE S256 + `/oauth/token`** with the current browser session and existing Aevatar consent. Use the registered `/auto/callback` URI; validate the returned URI/state without navigating to it. Request `openid profile email proxy` and only the Aevatar resource, verify the resulting subject through `/oauth/userinfo`, and attach Bearer to registration and status requests. The access token stays in memory for at most five minutes (bounded by expiry); logout/account changes invalidate it and any in-flight authorization. No refresh token is persisted or used, no CLI credential is minted, and no production configuration change is needed for the current hosted environment. A first-time `consent_required` offers Aevatar's existing authorization UI, then a manual retry using the unchanged form. Registration POSTs are never automatically replayed.
- Aevatar returns `202` with `status: "accepted"`, `registration_id` and `nyx_channel_bot_id`. Store these as separate resource references. Read back the existing Aevatar `GET /api/channels/registrations/{registration_id}/status` and NyxID bot detail; require matching IDs and active status before showing channel readiness. Aevatar's asynchronous mirror may temporarily return 404, so retry that read three times. Status errors/pending states offer read-only retry and never resubmit registration. Existing saved NyxID-only bots retain their earlier readiness check.
- Per-account, per-tab session storage contains only channel choice and resource IDs. Bot tokens remain in the form/mutation lifecycle and are cleared on successful submission or channel change. Server records remain the source of truth.
- English and Simplified Chinese use a feature-scoped i18next instance. The existing theme store and theme application hook are reused. Inherited NyxID authentication panels keep their existing translations and behavior.

## Design Reference

Inspected Figma file `Ee1gMJqg6DTv02PEzLqbZH`, page `Nyxbot / Prototype aligned / 08 Sep`:

| Page | Figma frame |
| --- | --- |
| Original account step (superseded by the supplied M01 sign-in reference) | `17:117` |
| Original data source (superseded by M02/D02 below) | `17:225` |
| Telegram channel | `17:312` |
| Telegram chat linking | `17:453` |
| WhatsApp channel | `17:536` |
| External Meta reference | `17:667` |
| Monthly cap | `17:737` |
| WhatsApp chat linking | `17:814` |

Help frames: `17:904`, `17:910`, `17:916`, `17:922`, `17:928`, `17:934`, `17:940`.

The implementation renders the web content, excluding the device shell and browser/address-bar illustration. The account, data-source and channel revisions below supersede those original frames. Inter and action/spacing values also follow the linked PRD's web surface. Local assets avoid a runtime font or brand-image CDN dependency.

The Channel screen follows the owner's 10 September screenshot labeled **M03 / Web / 3. Customer channel**. This revision uses that supplied image; its exact Figma node was not independently re-read. It reuses the Data source setup shell: Nyxbot brand header, completed Account/Data source indicators, current Channel indicator and left-aligned heading/subtitle. Channel cards, numbered BotFather instructions, customer-token field, lock note and bottom Back/Connect actions follow the reference. The owner's explicit overrides remain authoritative: omit the Telegram guide-chat note, disable WhatsApp and keep all four cards equal in width/height, including the unavailable row. Existing feature locale/theme state and token validation still apply.

The account screen follows the user's latest `M01 / NyxID / 1. Sign in` screenshot: a NyxID brand header, left-aligned title and continuation subtitle, four provider rows, a sign-up link, and a bottom **Back to Nyxbot** action. The return action remains disabled with an explanatory tooltip until the real guide-chat URL is supplied. This authentication screen has no onboarding progress bar. The optional current-account continuation is an extension for the authenticated state absent from the screenshot. Existing locale and theme state still apply.

The Data source screen follows the subsequently inspected `Nyxbot / Web onboarding / 08 Sep` page in the same file:

| Page | Figma frame | Design size |
| --- | --- | --- |
| M02 / Web / 2. Data source | `30:179` | 390 x 844 |
| D02 / Desktop web / Data source | `30:940` | 1440 x 960 |

It uses the Nyxbot setup header, four-step status strip, signed-in account row, selected Google Workspace and unavailable Notion tiles, separate Drive/Calendar descriptions, combined-consent explanation, and Back / Connect Google actions. The existing shell has a scoped `setup` variant for this page: 24 px mobile side padding, 64 px mobile / 80 px desktop header, 110 px source tiles, 48 px actions, and 560 px desktop inner content. Locale and theme state are retained without adding controls absent from these frames. The header's return-to-Nyxbot control remains disabled until a real guide-chat URL is supplied.

Rules added for states absent from the supplied frames: scrollable content with a sticky action footer on short screens; existing theme tokens for dark mode; explicit loading, failed authorization, missing permissions and unavailable-integration notices. Other onboarding steps retain the earlier compact layout with a 420 px desktop maximum and their existing locale/theme controls. Authentication back-navigation preserves the signed-in account.

## Backend Meeting Checklist

The owner confirmed on 8 September 2026 that the Nyxbot session, chat-pairing and spending-cap APIs do not yet exist. No production mocks or invented endpoints are included.

- [ ] **Bot session and entry link:** establish the real link format, session lookup/create API, authenticated ownership binding, session expiry, resume rules and the Google/channel resource IDs attached to that session. The current frontend query parameter is only a selection hint.
- [ ] **10-minute pairing code:** provide issue/refresh/read-status contracts, server expiry, single-use consumption, chat/sender ownership checks, and the actual Telegram/WhatsApp destination. Clarify what happens to the old code on refresh. Existing NyxID notification and CLI pairing codes have different purposes and are not reused.
- [ ] **WhatsApp monthly budget:** provide server persistence for SGD 120, SGD 250 and unlimited; define billing month, currency, metering source and race-safe enforcement. At the cap, stop automatic replies, continue receiving messages and notify the owner. Define how the budget is committed before activation and how failure/retry is handled.
- [ ] **Meta/payment model:** reconcile the PRD's BSP-held WABA with the repository's Tech Provider/customer-pays-Meta model and existing Business App coexistence support. Confirm number migration and billing responsibility.
- [ ] **Google platform readiness:** confirm the deployed scope policy, platform Client ID/Secret, credential mode, service OAuth callback, enabled Drive/Calendar APIs and Google's consent-screen verification or test-user access. Earlier testing encountered a BYO-only provider; on 9 September the owner subsequently completed Google consent, and the current account's real connection passes both grant checks. This confirms that account's connection, not shared-app readiness for every user. The branch's allowlist extension has not been deployed by this task. Google account-login credentials alone do not configure the service connector; this onboarding does not ask end users to create a Google OAuth application.
- [ ] **End-to-end handoff:** after confirmed pairing, define the return to the Nyxbot guide chat and the server completion event. Connection success alone must not claim that automated replies are enabled.

## Current Integration Limits

- Telegram identity lookup, suffixed bot-name labeling, Aevatar registration and readback use the existing contracts. The browser-session-to-Aevatar Bearer handoff and a real registration using the owner's explicitly supplied bot have been verified against the hosted services. The resulting Aevatar registration and NyxID bot were active, with webhook registration confirmed. This establishes channel provisioning; chat pairing and actual message delivery remain unverified. The current integration pins the supplied production Aevatar/NyxID environment; a different environment needs its matching OAuth client and registered callback. Chat linking displays an unavailable state; code copying and chat opening remain disabled because no real code/destination contract exists.
- WhatsApp is temporarily disabled in the onboarding channel picker at the owner's request. Its tile remains visible with the existing unavailable styling and **Coming soon** label; it cannot be selected by pointer or keyboard. Query and saved WhatsApp preselection do not activate its setup panel, readiness request or spending-cap action. Telegram remains available.
- The existing WhatsApp readiness, Meta and cap implementation is retained for future integration. Before re-enabling, define budget persistence before activation: the existing completion endpoint activates the bot immediately and accepts no budget. No monthly cap is saved by this onboarding.
- The owner's explicit token test performed one real Telegram registration through the existing UI/session. The URL revision only navigates and reads that saved connection; it does not register again. No new OAuth consent or paid Meta activation was performed. Credentials are absent from code and documentation. Automated tests use isolated API responses.

### Channel authentication verification — 9 September 2026

Using the owner's existing local browser tab/session, `/users/me` authenticated successfully. Cookie-only `GET /proxy/s/aevatar/api/channels/me` also succeeded, confirming identity propagation. However, a validation-only `POST /proxy/s/aevatar/api/channels/registrations` containing only `{ "platform": "telegram" }` returned **401**. After the new OAuth flow verified the same NyxID subject, the identical request with Bearer returned **400** with `webhook_base_url is required for Nyx-backed relay provisioning`. This intentionally incomplete payload proves the registration handler passed authentication and reached field validation; no bot token was supplied and no channel was created. Authenticated channel identity lookup also passed. The temporary local verification page was removed after this check. No deployed commit/image was available for server-version pinning; the live request comparison establishes the integration boundary, without claiming a deployed server defect.

## Local Preview Against Hosted NyxID

The owner selected `https://nyx.chrono-ai.fun` for integration. Its API is `https://nyx-api.chrono-ai.fun`; the public configuration enables Google, GitHub and Apple, requires invite codes for registration, and disables email authentication. Run the existing frontend against that environment from `frontend/`:

```sh
BACKEND_URL=https://nyx-api.chrono-ai.fun \
FRONTEND_URL=https://nyx.chrono-ai.fun \
npm run dev -- --host 127.0.0.1 --port 3000 --strictPort
```

`FRONTEND_URL` sets the existing Vite proxy's upstream Origin/Referer. The proxy also uses the repository's existing local cookie rewriting. Neither setting changes the hosted backend's configuration or OAuth return URL policy. This preview uses the selected live environment and its real account data.

On the current development machine, native Node HTTPS connections need the already-running local network proxy. With Node 25, also set `NODE_USE_ENV_PROXY=1`, `HTTPS_PROXY=http://127.0.0.1:7897`, `HTTP_PROXY=http://127.0.0.1:7897` and `NO_PROXY=127.0.0.1,localhost` when starting Vite. Without these, the proxy can fail with `ECONNRESET` before TLS negotiation. Vite's proxy disables the global agent, so `vite.config.ts` explicitly supplies a native HTTPS agent with `proxyEnv` when the flag is enabled for an HTTPS backend. Origin/Referer rewriting runs on the proxy's `start` event, before outgoing request creation, because a proxy agent may send headers before `proxyReq`. TLS certificate verification remains enabled. The opt-in requires a Node version supporting `Agent.proxyEnv` (verified with 25.9.0); machine-specific addresses remain in the local launch environment.

For a local browser session, use the existing **Continue with the NyxID app** flow. Read its displayed code, open the displayed `https://nyx.chrono-ai.fun/login/device` address in a browser already signed in to the same NyxID environment, enter the code, and review and approve that login. The local page polls `/api/v1/auth/device/poll-web`, receives a real browser session cookie through the proxy and returns to `/onboarding?step=source` (preserving any channel hint), where real grants determine whether Google consent is required or Continue is available. No account cookie, access token or OAuth secret needs to be copied between browsers or applications.

Verified on 8 September 2026: proxied public configuration returned HTTP 200 with the three social providers, and this device login flow established the local browser session and reached the data-source step. That step still reported Google Workspace authorization unavailable; account sign-in does not establish Drive/Calendar grants. The Google platform-readiness item above remains open.

Verified on 9 September 2026 after enabling the explicit consent action: Connect Google submits the real connection request. The selected hosted backend rejects connection creation with `Bad request: This provider requires user-provided OAuth client credentials (oauth_client_id + oauth_client_secret, or copy_oauth_client_from an existing connection)`. Its existing Google service setup dialog also asks for a custom OAuth app. No Google authorization URL was returned and no third-party access was granted. The frontend now shows this actual initiation failure, keeps the account signed in and allows retry. Deploying the scope change alone does not resolve the missing shared-app configuration.

The revised Data source page was checked in the existing authenticated browser at 375 x 667, 390 x 844 and 1440 x 960. There was no horizontal overflow; the short-screen notice was fully readable after scrolling and remained above the sticky actions. The desktop inner content measured 560 px. Temporary viewport overrides were reset after verification.

The Account-first revision was checked on 9 September 2026 using the existing local frontend at port 3000 and the same hosted backend. `/health` succeeded; all four provider rows and local logo assets rendered at 375 x 667, 390 x 844 and 1440 x 960. The real current-account action entered Data source, Back returned to Account, and the NyxID app panel generated a real QR code and closed back to the provider list. No new login approval or Google consent was performed in this revision. Related tests cover Account -> Data source -> Channel ordering, authenticated return hints, unauthenticated return protection, and saved-channel restoration after the first two steps.

The owner completed Google consent, and the local page confirmed both Workspace permissions from the hosted connection. Hosted callbacks still return to the configured production frontend, which does not deploy this onboarding route. The current initiation requests `/onboarding?step=source`; older Channel callbacks remain compatible and route through Data source for grant verification. To confirm permissions locally, visit `?step=source` and Continue; an explicit local `?step=channel` opens Channel without loading Google data. The hosted-to-local redirect limitation remains a separate deployment concern.

Direct social login is a separate local-preview limitation: the hosted backend ignores a localhost `return_to`, and its provider callback remains on the hosted API domain. Initiating through a localhost proxy also puts the OAuth state cookie on localhost rather than the hosted callback domain. Changing `BACKEND_URL` alone therefore does not make Google/GitHub/Apple return to localhost. Verify those complete flows with the frontend deployed under the backend's configured frontend origin, or with a dedicated environment whose OAuth callbacks and frontend origin are configured together. Aevatar's `NYXID_CLIENT_ID` belongs to its separate OAuth client flow and is not a substitute for this first-party browser session.

## Assets

- Google glyph: reused from the existing NyxID authentication provider button paths.
- NyxID, Telegram and Facebook marks: existing repository assets/glyphs.
- WhatsApp mark: Simple Icons (`https://github.com/simple-icons/simple-icons`, CC0).
- Inter Latin variable font: Google Fonts distribution, with the Inter SIL Open Font License alongside the font in `frontend/public/nyxbot-inter-LICENSE.txt`.

Local validation follows the personal incremental frontend policy: related Vitest tests and changed-file ESLint/Prettier only. Full frontend tests, typecheck and production build belong to GitHub CI.
