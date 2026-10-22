# Production browser policy

The frontend image renders an enforcing Content Security Policy at startup.
`frontend/scripts/render-nginx.py` validates deployment origins before writing
configuration; environment values cannot contain nginx or CSP fragments.
Every nginx location includes a complete header policy because nginx 1.27
stops inheriting `add_header` when a child defines one.

## Deployment inputs and response boundaries

| Input | Contract |
|---|---|
| `BACKEND_URL` | Internal HTTP(S) origin, default `http://backend:3001`; no path, credentials, query or fragment. |
| `NYXID_PUBLIC_ORIGIN` | Public frontend HTTP(S) origin, default `http://localhost:8080`. HTTPS enables `Strict-Transport-Security: max-age=31536000`. Compose production supplies `FRONTEND_URL`. |
| `NYXID_BROWSER_CONNECT_ORIGINS` | Up to 16 additional HTTP(S) origins separated by spaces; matching WS(S) origins are derived. Set for custom API origins; analytics uses the same-origin relay. These are destinations, never directive text. |
| Backend `BASE_URL` | Trusted public backend URL. HTTPS enables backend HSTS; request `Host` and forwarded proto headers never enable it. |

HSTS has neither `includeSubDomains` nor `preload`. A TLS terminator should
retain the application response policy; operators choosing a domain-wide
HSTS commitment configure that separately at their own boundary.

SPA HTML, deep links, legal pages, assets, errors and 304 responses share the
SPA policy. API/OAuth/discovery/MCP/credential routes retain their backend CSP,
Referrer-Policy and Permissions-Policy, with exactly one of each header.
Proxy failures receive a restrictive fallback. `^~` backend prefixes prevent
the static `.js` regex from intercepting backend or credential asset routes.
The backend header wrapper encloses rate-limit and routing failures too.

All SPA delivery uses `no-referrer`, before scripts execute. This protects
the bearer claim even when its query name is percent-encoded or duplicated,
or the receiver path is encoded or has a trailing slash. It also prevents
same-origin resource referrers before application capture/URL cleanup.
Access logs keep the original path but omit **all query strings and Referer
values**, including client-supplied Referer headers. The app's existing claim
capture/removal and short-lived login handoff remain in place.

Runtime API configuration accepts HTTP(S) origins; manifest URLs accept
credential-free HTTP(S) URLs without fragments. The telemetry host returned by
public config selects backend ingestion only; it never admits browser vendor
egress. Admission into a runtime config does not widen CSP: custom API origins
must also appear in the explicit deployment connect list.

## Supported resource inventory

| Resource / owner | Policy and execution behavior |
|---|---|
| Vite SPA chunks, prerendered legal theme | Self scripts; legal theme is a hashed external asset. Script inline handlers and eval are forbidden. Zod's interpreter is initialized through an exact Vite alias before any schema executes, including split chunks and standalone/wizard builds. |
| Styles / fonts | Self styles and existing React/Radix inline styles; Google Fonts CSS and `fonts.gstatic.com` font bytes. Font loading uses an ordinary stylesheet without inline `onload`. Style `unsafe-inline` does not admit scripts. |
| Images and previews | Self and HTTPS images plus data/blob previews; self/blob media. No executable blob worker/script source. |
| Landing hero | Exact UnicornStudio 2.1.4 CDN path, SHA-384 SRI, anonymous CORS; scene data from `assets.unicorn.studio` or the precise Google storage path. |
| Managed WhatsApp signup | Facebook SDK at `connect.facebook.net`, Facebook frame origins and graph connection. Existing origin-checked SDK messaging and backend exchange remain. |
| Telegram provider login | Exact Telegram widget loader path and `oauth.telegram.org` frame/connect origin. No string callback/eval: bounded JSON, schema, exact owned frame WindowProxy and exact origin, one accepted callback, cleanup on close. Backend Telegram hash verification remains authoritative. |
| Telegram bot setup | Browser `api.telegram.org/bot…/getMe` validation. Bearer bot tokens are not logged by this policy harness. |
| Aevatar onboarding | Fixed `aevatar-console-backend-api.aevatar.ai` connection origin. |
| API / OAuth popup | Self and explicit runtime API origin; popups retain their own backend response policy. OAuth completion scripts continue to use their handler-specific CSP. |
| Product analytics | Authenticated same-origin NyxID relay only: a fresh `GET /api/v1/users/me/preferences/analytics` gates each `POST /api/v1/users/me/analytics/events`. Account permission, account-bound device opt-in and DNT retain their existing gates. No browser vendor SDK, anonymous events, autocapture, pageviews, replay, remote config, flags, surveys or lazy vendor scripts. No third-party analytics origin in the default CSP; a custom backend ingestion host never widens browser policy. |
| Voice / dictation | Self websocket transports, plus explicit deployment WS(S) origins. Microphone allowed only for self; both PCM paths load the shipped self AudioWorklet through `worker-src 'self'`. Voice WebRTC uses negotiated media, which CSP does not turn into an HTTP allowlist. |
| Standalone credential accept | Backend-packaged HTML, script bytes and release manifest, independently generated strict SRI CSP and `no-referrer`. Both Docker builds use the same Node, npm lockfile, source commit and epoch inputs. Release publication must additionally compare actual shipped manifests (see RELEASE_INTEGRITY.md). |

## Reproducible production harness

This harness executes built production output through the real renderer and
nginx. It never starts Vite, uses `?mock=1`, or imports application dev mocks.
Network fixtures replace backend/provider transport only. The auto fixture
collects real `securitypolicyviolation` events in pages and OAuth popups, and
fails on any unexpected event. Negative runtime/WS cases specify exact expected
directive/destination pairs; other violations still fail. The injection case requires blocked
inline script, event-handler and unlisted connection events.
The Facebook SDK loader and its actual secondary bundle also execute in a
separate probe; managed signup replaces the SDK response at its network
boundary, exercising the application's script loader and the two signup paths
without a real Meta account.

From the repository root (use an external writable evidence directory):

```sh
export NYXID_POLICY_EVIDENCE="$HOME/Library/Caches/nyxid-targets/w5-telemetry-evidence"
export TMPDIR="$NYXID_POLICY_EVIDENCE/tmp"
mkdir -p "$TMPDIR"
mkdir -p "$NYXID_POLICY_EVIDENCE"
python3 frontend/scripts/prepare-production-policy-fixtures.py "$NYXID_POLICY_EVIDENCE/fixtures"
python3 frontend/scripts/test-nginx-policy.py
cd frontend
npm ci
SOURCE_COMMIT=$(git rev-parse HEAD) SOURCE_DATE_EPOCH=$(git show -s --format=%ct HEAD) npm run build
cd ..
python3 frontend/scripts/production-policy-server.py \
  --nginx /path/to/nginx --mime-types /path/to/mime.types \
  --evidence-dir "$NYXID_POLICY_EVIDENCE/nginx"
```

In another terminal, from `frontend/`:

```sh
NYXID_PRODUCTION_TEST_URL=http://127.0.0.1:18089 \
NYXID_NGINX_ACCESS_LOG="$NYXID_POLICY_EVIDENCE/nginx/access.log" \
NYXID_TEST_FONT="$NYXID_POLICY_EVIDENCE/fixtures/jetbrainsmono-v24.ttf" \
NYXID_PRODUCTION_RESULTS="$NYXID_POLICY_EVIDENCE/browser.json" \
NYXID_CHROME_EXECUTABLE=/path/to/chrome \
npm exec playwright test -- --config playwright.production.config.ts
```

The analytics probe restores an authenticated fixture account and submits real
reply feedback through the built UI. It verifies fresh consent admission, the
exact revision-bound relay body, device off/account off/DNT/withdrawal behavior,
and zero vendor data or script egress, including with a custom ingestion host.
Anonymous pages never send analytics. The microphone probe uses Chromium's fake
device while executing actual `getUserMedia`, AudioContext, shipped worklet
module and WebSocket handshake. Successful MFA verification is one-shot: a failed
session refresh retries only the session check, a 401 restarts login with a new
challenge, and uncertain network/server POST results check the session before
any challenge reuse. Successful and failed MFA refreshes, real
Telegram loader/frame messages, actual Unicorn scene, OAuth popup, managed
Meta flow, packaged credential page and legal theme all run under enforcement.
The font transport serves the authenticated real TTF fixture, with its actual
content type and CORS response; it does not claim to be a WOFF2 file.

For the **actual production nginx 1.27 image** on Linux, build with exactly the
same commit/epoch and replace the native server command:

```sh
docker build -f frontend/Dockerfile \
  --build-arg NYXID_GIT_HASH="$(git rev-parse HEAD)" \
  --build-arg SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD)" \
  -t nyxid-policy-frontend .
python3 frontend/scripts/production-policy-server.py \
  --frontend-image nyxid-policy-frontend \
  --evidence-dir "$NYXID_POLICY_EVIDENCE/nginx"
```

Container mode copies the actual image dist out and compares every file's
SHA-256 against the tested dist before serving. It records image inspection,
dist hashes, nginx version and actual access log. The image runs with its
unprivileged user, read-only root, two writable tmpfs paths, default Docker
AppArmor/seccomp and Linux host networking to the local fixture upstream; no
bind mounts are needed. It requires nginx 1.27 and uses the image's installed
renderer/template. Stop with Ctrl-C; the harness stops its own container and
transport. Native nginx evidence is recorded separately and cannot substitute
for this matrix.

The managed WhatsApp tests read the actual runtime-config consumer and complete
the cross-origin SSE POST to the explicitly admitted `api.nyxid.example`.
The negative case supplies an unlisted runtime origin and proves no request
arrives. The additional WebSocket probe performs a real handshake against the
fixture upstream on a distinct origin. The server includes that loopback origin
in its explicit connect list; override `NYXID_TEST_EXTRA_WS_ORIGIN` if changing
the upstream port for the browser tests.

With the server running, execute the response matrix separately:

```sh
python3 frontend/scripts/verify-production-policy.py
```

For the configured HTTPS boundary, start a second server with
`--port 18090 --upstream-port 18490 --public-origin https://app.example.test`
and execute `verify-production-policy.py --base-url http://127.0.0.1:18090
--https-configured`. Both matrices check spoofed Host/forwarded proto, actual
nginx-generated 502/413, nested channel reply budgets, legal 304/405 and exactly
one response policy. The local hop remains HTTP, modeling a TLS terminator;
these checks do not claim a TLS handshake.

The image mode deliberately selects a test port. Separately exercise the
packaged **default** entrypoint, without command or entrypoint overrides:

```sh
python3 frontend/scripts/smoke-frontend-image.py \
  --image nyxid-policy-frontend --evidence-dir "$NYXID_POLICY_EVIDENCE/entrypoint"
```

It checks ordinary startup and three invalid deployment fields failing before
nginx starts. Linux host port 8080 must be free for this smoke test.

Credential HTTP responses in the browser harness are transport fixtures serving
the real built standalone files; they do not execute the Rust credential handler.
Its separate backend tests exercise CSP/SRI/fail-closed behavior. Container
dist extraction checks shipped frontend parity; the release gate additionally
checks the backend-packaged credential output.

Public fixture bytes and source/length/SHA-256 receipts are retained in
`frontend/e2e-production/fixtures/` for offline reruns. The preparation script
verifies and copies them into the external evidence directory; its fallback
download fails if upstream bytes change. Review the vendor source before
updating its pinned test hash. Test transports
do not contact real accounts, publish artifacts or change production state.
