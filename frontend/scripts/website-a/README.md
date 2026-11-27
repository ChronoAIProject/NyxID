# Website A source migration

The approved product HTML is an external design input with SHA-256
`5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33`.
The original review materials remain untouched and are not a production runtime
or a build dependency. Production builds compile the committed React source.

From `frontend`, using a supported Node runtime and `npm ci`:

```sh
node scripts/website-a/generate.mjs /absolute/path/to/nyxid-site-preview.html
```

The migration checks the exact baseline hash. Its isolated VM evaluates only
baseline template/data declarations at development time; it never runs in the
web app. The generated output uses native JSX, local brand assets, isolated
locale resources, pathname routes and explicitly implemented React interactions.
No HTML iframe, script, `eval` or raw HTML insertion ships to the website.

Files in `src/features/website-a/generated` are separated by locale and route for
lazy loading. The generator contains the explicit production adaptations:
auth/query mapping, disabled newsletter, Discord contact, documentation/legal
links, illustrative story disclosure and excluded internal review guides. Change
the generator when updating those adaptations, then regenerate and run the
checks in `docs/plans/website-a-delivery.md`. The runtime interactions, auth link
helpers, public route policy, legacy blog compatibility and workflow CSS are
maintained normally outside the generated directory.

The homepage tool expansion and English/Chinese Integrations pages share
`nyx-home/production-catalog.json`. This is a reviewed display export from an
authenticated production snapshot. `nyx-home/catalog-curation.json` explicitly
selects public-facing brands, groups auth/region variants and supplies labels
and icons; it does not infer readiness or a roadmap from the old HTML flags.
An explicit `availability: "coming_soon"` retains a release decision even when
the backing service already exists in the production catalog.
`generated/integrations.json` is the legacy design inventory and is not the
source for these current catalog surfaces.

To refresh the display export from a reviewed snapshot containing `checked_at`
and the active catalog `entries`, run from `frontend`:

```sh
node scripts/website-a/sync-home-catalog.mjs /path/to/production-catalog.json
```

Only explicitly curated entries and safe display fields are exported. New
services need a curation entry before they appear; disappeared catalog entries
are omitted. The public website uses the committed export and needs no token.

The pricing route uses the native `PricingSection` adapter and bilingual
`pricing-copy.json`, based on the effective policy in chrono-finance PR #315
(D2/D3/D5). Its 15% markup, own-account exception and GST example replace the
baseline's superseded zero-markup copy. The generator also reads that copy for
pricing titles and descriptions so regeneration preserves the policy update.

The baseline's five-workflow embedded film is migrated to locale JSON and native
React, with accessible selection and playback controls. Its animation timing and
responsive arrangement are implemented in `interactions.tsx` / `workflow-film.css`.
All public CSS selectors and animation names are scoped to avoid changing the app.

The approved A + v4.1 enhancement is generated through native `HeroSection`,
`HeroTitle`, `VisualLink` and precise `RevealFrame` placements. Source adapters,
scoped styles and lifecycle tests live in `visuals/`; the full React Bits license
and pinned-source hashes are retained there. The static 24-card tool wall remains
unchanged. Public pages use an independent dark theme and never change saved app
preferences. See `docs/plans/website-v4.1-delivery.md` for current verification and
the unresolved UnicornStudio branding requirement.

Contact uses a native `ContactSection` adapter while retaining the original
Discord card and scoped layout selectors. Its form and scripted FAQ live in
`src/features/website-a/contact.tsx`; validation is in
`src/schemas/website-contact.ts`. Delivery opens the user's mail client with an encoded draft; it never claims to
send automatically. Never restore the prototype's fake success branch or replace
the whole interface when a backend is unavailable. Contact-specific shrink/error
and disclosure styles are in `contact.css`; the original layout/breakpoint
styles remain in `website-a.css`.

The public contact address is configured once with `VITE_WEBSITE_CONTACT_EMAIL`
(default: `contact@chrono-ai.fun`). It is a public build-time setting; rebuild to
change it. Invalid single-mailbox overrides fall back with a non-sensitive warning.
Native mail client availability and actual sending remain user-controlled.

`primary-button-css.mjs` adapts both base and later mock primary-gradient
shorthands to border-box/no-repeat paint. Preserve this transform when
regenerating CSS so transparent borders cannot tile the gradient edge.

# Public page design

The marketing detail pages use native story layouts. `story-page.tsx` and
`story-page.css` define their shared type and rhythm; `product-story-pages.tsx`
contains NyxBot, How it works, Security and Your agent. Pricing, the tools directory
and Contact have their own content layouts. `solution-story-pages.tsx` uses the
bilingual workflow content in `solution-story-copy.json` for the job details, with `solutions-index.tsx` for the grid and
`solution-workflow-icons.tsx` for explicit, locale-independent workflow marks. The generator preserves these native adapters. Keep the
solution release gate in `SolutionActivation` when changing workflow layouts.

The home page is the design reference for public pages. English and Chinese use
the same `nyx-home-page.tsx` component, artwork, sections and motion. Chinese
homepage copy lives in `nyx-home/home-copy.json`; dynamic motion labels and catalog
controls use the same locale. Switching homepage locale remounts the motion scene
so its transformed nodes and listeners belong to one page lifetime. Shared chrome is in
`src/features/website-a/public-chrome.tsx`, with its bilingual footer directory in
`src/features/public-site/public-footer-links.json`. `public-design.css` holds the
public palette, type and controls; `public-reading.css` adapts docs, auth and legal
surfaces. Their `nyxid-theme` choice stays separate from the application's
`nyxid.theme` preference.

Marketing headers place an English/中文 selector beside the public theme toggle.
Switching language keeps the current published route, visitor query parameters
and explicit section fragment. The selector is omitted on pages without a
published translation. Both header menus use native disclosures and support
keyboard dismissal; the standalone HTML preview uses the same markup.

The Tools introduction includes a compact version of the homepage's channel →
NyxID → apps diagram beside the text. It uses the same brand assets and visible
WhatsApp availability label, and stacks below the introduction on narrow screens.
The searchable directory stays below both columns.

The standard `npm run build` renders these same components into the public HTML
artifacts listed in `dist/public-release.json`. The generated Nginx configuration
serves each route's HTML directly; JavaScript hydrates its controls and motion.
`verify-public-build.mjs` checks both homepage layouts, static language links and
the Tools diagram before packaging. The browser acceptance suite also reads these
pages with JavaScript disabled. The standalone export below is a separate artifact
with a DOM runtime and no React or TanStack in its browser graph.

Standalone legal HTML uses the same public CSS and footer directory during
`scripts/prerender-legal.mjs`, so its first load matches the SPA presentation.

Sign-in uses stills of the landing page's existing halftone artwork. To regenerate
them, start the local preview on port 2972 and follow the capture and `cwebp`
commands at the top of `capture-auth-art.mjs`.

## Standalone HTML homepage preview

`npm run build:landing` exports the approved homepage in English and Chinese to
`dist-landing/index.html` and `dist-landing/zh/index.html`, with its CSS, local
artwork/icons and browser scripts. The header language selector switches between
these two static documents.
The export uses the same approved page source at build time. Its browser entry
contains no React or application router: native HTML links/disclosures and DOM
handlers provide the menu, public theme, catalog search and query handoff, while
the existing GSAP/Lenis module provides the motion. The build rejects React or
TanStack modules in the exported browser graph.

`LANDING_LINK_ORIGIN` sets the origin for detail and sign-in links;
it defaults to the configured public origin. Use the local app origin for local
review:

```sh
LANDING_LINK_ORIGIN=http://127.0.0.1:2972 npm run build:landing
python3 -m http.server 2973 --bind 127.0.0.1 --directory dist-landing
```

The homepage reads the public `nyxid-theme` choice without changing app settings.
A bounded session read redirects a confirmed signed-in visitor to Assistant;
failed or signed-out reads leave the static page available. The export sends no
analytics or diagnostics. Those remain owned by the application reached through
its links. Production routing and the normal public build retain their existing
rendering path; this export is an independent preview artifact.

## Surge preview

Prepare an independent static preview with the full public-page build:

```sh
VITE_STATIC_PREVIEW=true PUBLIC_SITE_ORIGIN=https://ctkm-nyx-landing.surge.sh npm run build
node scripts/prepare-surge-preview.mjs ctkm-nyx-landing.surge.sh
surge .surge-preview ctkm-nyx-landing.surge.sh
```

The packager materializes the generated Nginx routes as static HTML, adds a
fallback and directory URL normalization, and marks the preview as noindex.
Public controls and docs work without an API session. Account routes hand off to
the live public origin, retaining query parameters. `.surge-preview/` is ignored.
Authenticate directly with Surge (`surge login --browser`) before publishing;
credentials do not belong in the repository or preview folder.

## NyxBot app captures

The NyxBot page uses actual development-app captures, with the existing local
HTTP fixtures supplying example data. The workspace, booking hand-off,
specialist memory and one-time confirmation use the production React components;
the capture script supplies fictional booking details and an example.com identity
at the fixture boundary. No real provider requests or personal account data are
involved. The page labels
them as example data. English UI captures are shared by both page languages.

To regenerate, start the local Vite app on port 2972, then run:

```sh
# From the repository root (once):
python3 -m venv .playwright-mcp/media-venv
.playwright-mcp/media-venv/bin/pip install Pillow==11.3.0

# From frontend:
node scripts/website-a/capture-nyxbot-media.mjs
node scripts/website-a/encode-nyxbot-gifs.mjs
```

Capture needs Playwright Chromium and `cwebp` on PATH. Override the dev origin
with `NYXBOT_CAPTURE_ORIGIN`. It writes WebP stills to `public/landing/nyx/app/`
and WebM source recordings to the ignored `.playwright-mcp/app-media/recordings/`.
The encoder decodes the real recording in Chromium and writes GIFs with a shared
palette and changed-frame rectangles. It keeps the native 1200px desktop / 440px
mobile views, eight frames per second. Current GIFs are 635–893 KB on desktop
and 352–418 KB on mobile. The completed state is the first frame, held briefly
before the request and result replay. Only the current theme and viewport version loads.

The page shows the responsive GIF inline, with no player or view tabs. A small
pause control switches to the still; reduced motion always uses the still.
Recordings have no audio. Memory and approval stills have their own mobile
captures. Example content is supplied before the first frame; no personal
identity or live provider effects are captured.

Public page flow is intentionally different per destination. NyxBot owns the
app/channels/specialists; How follows a single request; Your agent owns client
setup and code; Security owns credentials/approval/audit; Pricing owns billing;
Tools is a directory; Solutions uses job-specific headings and setup. Keep
canonical pricing, availability and activation gates when editing those paths.
