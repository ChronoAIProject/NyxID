# Built-in NyxID login links

NyxID uses its existing API host for native login links. This requires no Dub,
AppsFlyer or other link-service subscription. Existing hosting and app-store
distribution still apply. No new domain, SDK or link-record database is needed.

## Login flow

1. The browser requests a device-login challenge. With an HTTPS origin-only
   `BASE_URL`, a different `FRONTEND_URL` origin, and a configured Apple identity
   (nonblank `APNS_TEAM_ID` + `APNS_TOPIC`) or Android identity, the response
   includes `app_verification_uri`, for example
   `https://api.nyxid.ai/login/device?user_code=ABCD-EFGH`.
2. Only `/login` → Continue with the NyxID app (including its MFA app option)
   opts into native QR payloads. On coarse-pointer/touch devices, a ready, live QR
   is an accessible button. Tapping it opens “Open NyxID app?” confirmation for
   that exact login. Cancel, Escape and Close return focus to the QR. Only the
   affirmative “Open NyxID app” link navigates, using `target="_blank"`,
   `rel="noopener noreferrer"` and `referrerPolicy="no-referrer"`. It opens this
   login without scanning: review and approve, then return to the original browser
   tab. Browser fallback keeps that tab and its in-memory poll secret.
   Fine-pointer `/login` displays a plain native QR for external scanning. Other
   consumers, including `/login/device`, `/login/agent-key` request-bound identity
   verification and onboarding, use plain `verification_uri_complete` QR payloads
   with no app action or dialog. Missing/invalid app URLs and HTTP development
   also use that web payload without losing the challenge.
3. Continue, Generate new, mount, polling and resume never open an app or dialog
   automatically on either platform. The confirmation binds the selected request
   object and URL; both the QR tap and affirmative click recheck the hook's
   original challenge deadline even if background timers were delayed. Back,
   cancel, controlled closing, expiry, terminal state, replacement, eligibility
   loss and unmount retire the confirmation. It never adopts a replacement request.
   iOS Universal Links / Android App Links can open an installed, associated app
   directly at the public preview; no camera is needed for the same-phone link.
   The app extracts only the public code and uses its embedded, configured API.
   A scanned URL never selects the API destination. A configured URL does not
   detect installation or establish live association; actual handoff depends on
   the browser and OS. There is no preopened/blank tab or same-tab redirect.
4. The user reviews the request, signs in or verifies their session if needed,
   and explicitly approves or denies. Opening the link never makes a decision.
5. After approval of a browser request, the app says “Return to your browser to
   continue.” CLI and unknown requests say “Return to where you started signing
   in.” The original browser's private polling completes sign-in when it resumes.
   There is no attempt to select a browser tab or carry credentials through a return URL.

If the app is absent, association fails, or the user chooses the browser, the
API's `GET /login/device` redirects to the fixed frontend approval page. It
forwards only a validated, hyphenated public code and uses `no-store` /
`no-referrer` headers. Missing, invalid or duplicate codes return 400. A configured
frontend with the same origin as `BASE_URL` returns 404 to prevent a redirect loop;
origin comparisons normalize the scheme, host and default port.
The frontend and API link host must be separate origins to avoid redirecting
back to the same route. Extra query parameters cannot change the destination.
The public QR code expires with its underlying request; the private `device_code`
and access/refresh tokens never appear in it.

The approved delivery retains its existing **60-second post-approval collection
window**. Return to the original browser promptly. Visibility-to-visible and
`pageshow` events prompt a poll after the minimum interval, preserving server
`slow_down` and rate-limit backoff. Resume events do not overlap an in-flight poll,
and an old request cannot block or reschedule a newly generated one. After the
window expires, start a new login. This feature does not widen that policy.

The QR, manual code and tap controls retire at the original challenge deadline.
The browser then shows “Checking your request…” and retains only private
collection. No new poll starts at or after the original deadline plus 60 seconds.
Approval at 55 seconds of a 60-second challenge remains
collectable until 115 seconds, so returning at 90 seconds can still finish.
Reaching the original QR deadline does not invalidate an in-flight collection.
An already in-flight read may finish for up to 10 seconds after the poll-start
cutoff. This only waits for an existing response; server approval and delivery
deadlines remain unchanged. For a different request approved just before its
60-second challenge expires, a final poll accepted at 119 seconds (before its
actual server delivery deadline) may return at 120.1 seconds and complete sign-in.
Errors after the cutoff terminate
collection without another poll. At the finite response deadline the browser
discards a hung request and fences its late result; close, regeneration and
unmount fence local adoption immediately. A delivered restricted login code keeps
its own strict expiry and receives no collection or response grace.

Review still asks the user to match the displayed code against the requesting
device or terminal and reject a mismatch. As in RFC 8628 §5.4, an attacker can
send a genuine-domain link for the attacker's pending login. A real-domain link
and the genuine app do not establish the requester's identity. Opening it only
loads a public preview; explicit authenticated human approval remains required.
The existing `skip_mfa_for_approved_sign_ins` setting (default on, including absent
preferences) and live session/MFA step-up policy apply equally to link, QR and
manual-code approval. This change does not alter that policy or CLI behavior.

This feature covers installed-app login. It does not preserve an expiring login
across app-store installation. A user who installs the app can start a fresh
login and scan again. The existing generic app-download URL remains separately
configurable through the frontend's `VITE_MOBILE_APP_URL`; its OneLink fallback
has not been retired by this login feature.

## Server and native identities

Serve these files over public HTTPS, without authentication or redirects, on
every API host embedded in the mobile build:

| File | Backend configuration |
| --- | --- |
| `/.well-known/apple-app-site-association` | Existing `APNS_TEAM_ID` and `APNS_TOPIC`, matching the Apple team and iOS bundle ID. No APNs private key is needed to serve this file. `/login/device` is already included. |
| `/.well-known/assetlinks.json` | `ANDROID_APP_PACKAGE` matching the Android application ID, plus `ANDROID_APP_SHA256_CERT_FINGERPRINTS`. |

Copy the SHA-256 certificate from Google Play Console's **App signing** section,
not the upload-key certificate. Fingerprints contain 32 colon-separated hex pairs;
separate multiple certificates with commas for key rotation. Malformed values
fail server startup. The existing association endpoint returns 404 without both
a package and signing fingerprints; partial configuration does not advertise links.
These are public identities, not signing keys. Configured identity permits the
server to advertise a link; it does not prove that OS association is operational.

`mobile/scripts/lib/native-links.js` preserves the existing custom scheme,
OneLink filter, selected `associatedUrls` host list and the
`shared/native-app-links.json` paths, including `/login/device`. Android HTTPS
filters remain profile-local opt-in through `DEV_ANDROID_HTTPS_APP_LINKS=true`
or `PROD_ANDROID_HTTPS_APP_LINKS=true`, after every selected host serves Asset
Links. iOS retains its configured associated domains. The API-only versus
API-plus-frontend host rollout remains an independent owner decision; this
feature does not select another host set.

### Business links on frontend hosts

To associate frontend business URLs such as `https://nyxid.ai/activity`, set the
active build profile's `DEV_NATIVE_LINK_HOSTS=api+frontend` or
`PROD_NATIVE_LINK_HOSTS=api+frontend`. Unset, empty and `api` keep the existing
API-only host selection. The selector never falls back to the other build
profile. `api+frontend` requires that profile's explicit `*_FRONTEND_URL`; it
cannot claim a host borrowed from the other profile or from `*_LEGAL_BASE_URL`.
Production builds also include `PROD_NEW_FRONTEND_URL` when the complete new
domain set is configured. Both API and frontend host sets share the existing
`shared/native-app-links.json` paths; this includes `/activity`.

For the dual-domain production profile, the intended frontend inputs are:

```dotenv
PROD_FRONTEND_URL=https://nyx.chrono-ai.fun
PROD_NEW_FRONTEND_URL=https://nyxid.ai
PROD_NATIVE_LINK_HOSTS=api+frontend
```

Keep the existing complete `PROD_NEW_*` domain set; the snippet is not a complete
build environment. iOS derives its associated-domain entitlement from the
selected host list. Android additionally needs the existing, profile-local
`*_ANDROID_HTTPS_APP_LINKS=true` opt-in **after** every selected host serves a
valid Digital Asset Links file for that build's package and signing certificate.
The Android and iOS release scripts both use this same profile resolver through
Expo prebuild. Changing these inputs requires a new signed native artifact; an
OTA JavaScript update cannot add host entitlements or intent filters.

The QA package `dev.nyxid.nativecheck` and its Android Debug certificate do not
identify the Play-distributed `fun.chronoai.nyxid` application. Do not publish
that debug identity on production domains to make a sideloaded QA build verify.
Use the Play App Signing identity for Play builds, or an explicitly approved
separate QA host and identity. A successful custom-scheme launch or a forced
`pm set-app-links` state does not prove HTTPS domain verification.

On supported Android versions through Android 11, **every host in the manifest
must verify**. Every selected API/frontend host and the retained OneLink host
must keep serving valid associations for that build. A missing association can
also break other verified links on those versions. Android requires a new native
build to change intent filters; an OTA JavaScript update cannot add them.

## Rollout and verification

1. Confirm the backend `BASE_URL` is a public HTTPS API origin already embedded
   in supported builds. Set the identities above on every replica and expose
   both well-known files on every embedded API host **before releasing a native
   build**. Verify that the retained OneLink host still serves its valid Android
   association, especially for Android 11 and below. Keep the frontend fallback
   reachable.
2. Prepare backend and frontend changes together. The additive field permits
   older frontends to keep using their existing verification URL; newer frontends
   also accept older backend responses.
3. Prepare a mobile build with public-preview handling. Enable that profile's
   Android HTTPS App Links opt-in only after all selected hosts are associated.
   Builds without those filters normally use the browser fallback for external links.
4. Verify on physical iOS and Android devices. OS association caches, user choices,
   Safari behavior and in-app browsers can affect whether an HTTPS link opens
   the app, so a unit test cannot establish the installed-app behavior.

| Check | Expected result |
| --- | --- |
| Android and iOS `/login` same-phone QR, app cold / running | Continue only creates the request; tapping the ready QR opens confirmation. Only affirmative Open attempts new-tab handoff. Verify public preview without camera, separate approval and original-tab preservation. |
| Cancel, Escape or Close confirmation | No navigation; keyboard focus returns to the QR. |
| App absent, browser blocks handoff or association fails | No installed/success claim; verify confirmation-link browser fallback and original poll survive. |
| Create/regenerate request, resume or poll on either platform | No automatic modal, new tab or app opening. |
| `/login/device`, `/login/agent-key`, onboarding and other non-opted-in consumers | Plain web verification QR payload; no QR handoff button, app action or modal. |
| Open modal then expire/deny/replace request, close/back/unmount or lose pointer/prop eligibility | Old confirmation retires without a stale URL or automatic replacement; delayed timers cannot authorize an expired click. |
| Scan the desktop QR with the OS camera or the NyxID scanner | Opens/reviews the same public request. |
| Signed-out app, then intentionally choose Sign in | Preserves the request review; still requires explicit approval. |
| Cold signed-out link, then Back/Done (pending, denied, invalid or expired), then later sign in | Abandoned request does not reappear. |
| Sign out from an open request sheet | Clears the sheet and request state. |
| Approve, then return to the original browser promptly / at about 45 seconds | Original browser resumes polling and completes sign-in using its own poll secret within the delivery window. |
| Approve just before QR expiry, then return after QR expiry but within 60 seconds of approval | QR/tap controls stay retired; private collection can still complete (for example, approve at 55 seconds, return at 90 seconds of a 60-second challenge). |
| Approve, then return after more than 60 seconds | Delivery has expired; a fresh login is required. |
| Browser, CLI and unknown requester approval | Browser instruction only for browser requests; generic return instruction for other kinds, in the current app language. |
| Open another link while the first preview is loading | Only the newest request is displayed. |
| Expired, completed or malformed code | Clear error/terminal state; no decision is sent automatically. |
| App absent, iOS sticky Safari choice, in-app browser, or old Android build without the filter | Verify fallback opens the fixed web approval page in another tab and the original poll survives. |
| Fine-pointer desktop `/login` | Plain native QR for external scanning, with no QR button or web handoff modal/action. |
| Android 11 and below with all manifest hosts present | API hosts and retained OneLink verify and open their supported links. |
| Untrusted host or added `redirect` / `device_code` query | Cannot select a backend, redirect target or credentials. |
| Existing custom scheme, OneLink and push links | Retain their existing routing and trust rules. |

For rollback, revert the frontend preference for `app_verification_uri` to use
`verification_uri_complete`. Keep API fallback and association files available for
distributed links and installed builds. Turning off Android association alone is
not immediate rollback because devices cache verification. Keep both embedded
domain sets serving the same request records during domain migration.

Do not remove the retained OneLink infrastructure as part of this feature. Its
host is still in released Android manifests and remains the generic download
fallback. Before retiring it, first ship a build that removes that intent filter
and account for older supported builds that still require it. Removing the
infrastructure first can break API-host links on Android 11 and below even when
the API association files are healthy. Replacing the generic download destination
requires a separately provisioned HTTPS URL; no replacement is provisioned here.

This matrix records required physical-device checks, not completed results.
Local tests cannot verify signing, store-installed builds, OS caches, new-tab
handoff, or browser-specific app opening. Normal signing, association hosting and
release verification remain prerequisites; this work performs no deployment.
