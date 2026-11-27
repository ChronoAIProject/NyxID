# Public website

The English and Chinese homepages share `src/features/website-a/nyx-home/nyx-home-page.tsx`, adapted from Chloe’s light/night design. Chinese homepage copy is in `nyx-home/home-copy.json`. Product pages use the shared story layouts; Solutions uses `solution-story-copy.json` and the activation gate in `SolutionActivation`. Pricing copy is maintained in `pricing-copy.json`.

Shared navigation, footer and theme controls live in `public-chrome.tsx` and `public-design.css`. Docs, auth and legal surfaces use `public-reading.css`. The public `nyxid-theme` preference stays separate from the app’s `nyxid.theme` preference. The header language selector preserves the current route, query values and explicit fragment. New page links start at the top; section anchors and Back/Forward keep their intended positions.

## Build and routing

From `frontend`, use the Node version in `.node-version`:

```sh
npm ci
npm run dev -- --host 127.0.0.1 --port 2972
npm run build
```

The production build renders public HTML from the same React components before shipping. `scripts/prerender-public.mjs` writes the route inventory, HTML, Markdown mirrors, sitemap and generated Nginx routing. Nginx serves each public route’s HTML directly; the browser hydrates its controls and motion. The private app retains its separate shell and existing authentication/session behavior.

`content/publication.json` controls which marketing pages are published; `src/features/docs/manifest.ts` controls published documentation. Candidate articles stay excluded. `scripts/verify-public-build.mjs` checks content, metadata, links and assets before packaging, including homepage parity and the Tools diagram. The public browser suite verifies readable content with JavaScript disabled. Deployment and browser policy are documented in `docs/DEPLOYMENT.md` and `docs/BROWSER_SECURITY_POLICY.md`.

```sh
npm test -- src/features/public-site src/features/website-a test/public-markdown-acceptance.test.ts
npx playwright test --config public-playwright.config.ts
```

Browser acceptance uses the production artifact served at `http://127.0.0.1:4188`; override it with `PUBLIC_TEST_ORIGIN`. `Dockerfile.public-test` and `scripts/public-fixture-server.mjs` provide the isolated server and API fixture.

## Tool catalog

The homepage and Tools pages share `nyx-home/production-catalog.json`, a reviewed display export from production. `catalog-curation.json` selects public brands, groups account variants and supplies labels/icons. Explicit `coming_soon` availability is a release decision. The website needs no catalog credentials.

To refresh from a reviewed snapshot containing `checked_at` and active `entries`:

```sh
node scripts/website-a/sync-home-catalog.mjs /path/to/production-catalog.json
```

## Public controls and assets

Release configuration is in `release-config.ts` and `.env.website.example`. Keep availability and activation gates when changing layouts. Contact opens a mail draft; it does not claim to send a message. Its public address uses `VITE_WEBSITE_CONTACT_EMAIL`, defaulting to `contact@chrono-ai.fun`.

NyxBot media uses real app components with fictional booking data and an `example.com` identity. The responsive GIF has a pause control and a still image for reduced motion. Capture and encoding scripts are `capture-nyxbot-media.mjs` and `encode-nyxbot-gifs.mjs`; `capture-auth-art.mjs` creates the sign-in halftone stills. Generated source recordings stay outside tracked assets. Preserve the artwork/icon credits and third-party licenses beside the production assets and vendored components.
