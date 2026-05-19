# Blog Implementation

> **Status (2026-05-04):** Frontend-only first cut. Three public routes are live and rendering against an **in-bundle mock layer that exists purely as a reference implementation**. Both `frontend/src/features/blog/mock-api.ts` and `frontend/src/features/blog/mock-data.ts` are placeholders — they will be **deleted and replaced** when the Directus CMS + CDN go live (Directus serves the article metadata and the markdown body via a CDN-fronted endpoint). See [Mock layer](#mock-layer-placeholder-for-directus--cdn) and the [Production checklist](#production-checklist) for the swap path.

## Overview

The blog is a public-facing reading surface for product writing — engineering deep-dives, security notes, design decisions. It lives entirely on the frontend SPA, alongside the landing page, and reuses the landing's chrome (logo, footer, navbar styling) so it reads as the same product, not a separate microsite.

The data model mirrors a Directus `blog_articles` collection so that, when the Directus CMS + CDN endpoint land, the frontend swap is one line per fetch helper. **Until then, the mock files in `frontend/src/features/blog/` stand in as a reference implementation only.** Nothing in the mock is intended to ship to production.

---

## Routes

| Path | Public? | Component | Behavior |
|---|---|---|---|
| `/blog` | ✅ | `BlogIndexPage` | Lists articles with `status === "published"`, sorted newest first. Shows skeleton while loading, empty state if none, error state if the fetch throws. |
| `/blog/$slug` | ✅ | `BlogDetailPage` | Fetches one article by `slug`, filtered to `status === "published"`. 404-style "not found" card if the slug doesn't resolve. |
| `/preview/$id` | ✅ | `BlogPreviewPage` | Fetches one article by `id` (UUID), **regardless of `status`**. Renders the same layout as `BlogDetailPage` plus a purple "Preview mode" banner with a status pill. The UUID is the preview-URL secret. |

All three are mounted under `rootRoute` in `frontend/src/router.tsx` and added to the `isPublicPath()` allowlist in `frontend/src/main.tsx` so the auth shell doesn't redirect unauthenticated visitors. They have no `beforeLoad` redirect for authenticated users either — signed-in users can read the blog without being bounced to the dashboard.

---

## Data model

The TypeScript interfaces in `frontend/src/features/blog/types.ts` mirror the Directus `blog_articles` collection one-to-one. Many-to-one relations are modelled as embedded objects (matching how Directus returns them with `?fields=*.*`).

### `blog_articles`

| Field | TypeScript | Notes |
|---|---|---|
| `id` | `string` (UUID) | Preview-URL secret. Primary key. |
| `product` | `Product` | M2O — one blog can serve multiple products via the `content_path`. |
| `slug` | `string` | Lowercase, hyphenated. Routes use this for `/blog/$slug`. |
| `title` | `string` | Display title. |
| `description` | `string` | SEO summary, 120-160 chars. Used as the dek/standfirst on detail. |
| `body` | `string` | GFM markdown. No `# Title` line — the title is rendered separately. |
| `tags` | `Tag[]` | M2M — first tag is shown as the category eyebrow. |
| `series` | `Series \| null` | Optional grouping. |
| `author` | `DirectusUser` | Author bio shown at the end of the article. |
| `hero_image` | `DirectusFile \| null` | CDN-hosted image. `url` is pre-resolved. |
| `published_at` | `string \| null` | ISO 8601. `null` for drafts. |
| `status` | `"draft" \| "in_review" \| "published" \| "archived"` | Drives index filter and preview banner. |
| `content_commit_sha` | `string` | Set by the publish Flow. |
| `content_url` | `string` | GitHub file URL. |

### Supporting types

- **`Tag`** — `id`, `slug` (unique), `name`
- **`Series`** — `id`, `slug` (unique), `name`, `description?`
- **`Product`** — `id`, `name`, `site_url`, `site_github_repo`, `site_dispatch_event_type`, `content_path`
- **`DirectusUser`** — `id`, `first_name`, `last_name`, `email`, `title?`, `description?`, `avatar?`
- **`DirectusFile`** — `id`, `filename_disk`, `url`, `width?`, `height?`, `alt?`

### API response envelope

Every fetch helper returns Directus's `{ data: ... }` shape:

```ts
interface DirectusListResponse<T> { data: readonly T[] }
interface DirectusItemResponse<T> { data: T | null }
```

---

## Mock layer (placeholder for Directus + CDN)

> **Reference implementation only.** `mock-api.ts` and `mock-data.ts` exist so the routes, components, loading states, and prose styling can be developed end-to-end before the Directus CMS and CDN endpoint exist. **Both files will be deleted** when the real backend is wired. Treat them as scaffolding, not as code that ships.

### What's here today

`frontend/src/features/blog/mock-api.ts` exposes three async helpers with the same signatures the real client will need:

```ts
fetchPublishedArticles(): Promise<DirectusListResponse<BlogArticle>>
fetchArticleBySlug(slug: string): Promise<DirectusItemResponse<BlogArticle>>
fetchArticleById(id: string): Promise<DirectusItemResponse<BlogArticle>>
```

These read from `mock-data.ts` (5 articles — 4 published, 1 draft for preview testing) with a 250 ms simulated latency. The mock file carries a `SHIP NOTE` comment at the top reiterating that it must not ship.

### What replaces them

When the Directus CMS lands and a CDN bucket is configured, the future state is:

| Today (mock) | Future (Directus + CDN) |
|---|---|
| `mock-data.ts` — markdown bodies bundled into the JS chunk | Directus stores articles; markdown bodies live in the `body` field (or in a content repo committed by a publish Flow) and are served via the CDN-fronted endpoint |
| `mock-api.ts` — `Promise.resolve(MOCK_ARTICLES)` after a `setTimeout` | Real `fetch(import.meta.env.VITE_BLOG_CDN_URL + '/items/blog_articles?...')` calls returning the same `{ data: ... }` envelope |
| Hero images — Unsplash URLs hardcoded | `hero_image.url` resolved by Directus to a CDN asset URL |
| Preview UUID — hardcoded in the bundle, recoverable from JS | Server-enforced lookup at the CDN/Directus layer; the UUID is the only entry point and cannot be enumerated |

### The swap, file by file

When the backend is ready:

1. **Replace** the body of each function in `mock-api.ts` with a `fetch()` call against the configured CDN URL. Keep the function signatures and the `{ data: ... }` envelope identical so no caller changes.
2. **Delete** `mock-data.ts` entirely. Anything still importing it is a leak to fix.
3. **Add** `VITE_BLOG_CDN_URL` (and any read token / preview token) to `frontend/.env.example` and document them in [`docs/ENV.md`](./ENV.md).
4. **Keep** `types.ts`, `utils.ts`, `article-body.tsx`, every page component, every other component, and the routing untouched. They consume the same `BlogArticle` shape regardless of where it comes from.

The contract that protects this is: nothing outside `mock-api.ts` imports `mock-data.ts`. Page components import only the API helpers, not the data. So replacing the data source is a one-file change.

---

## Design language

The blog reuses the **landing page's** visual system — not `DESIGN.md`'s prescribed system. Where the two diverge (typography, primary colour), the **live landing wins** so the blog reads as part of the same site.

### Colour

| Token | Hex | Source |
|---|---|---|
| `--color-landing-bg` | `#0a0a0f` | `frontend/src/app.css` |
| `--color-landing-surface` | `#111118` | `frontend/src/app.css` |
| `--color-landing-surface-raised` | `#18181f` | `frontend/src/app.css` |
| `--color-landing-border-subtle` | `rgba(139, 92, 246, 0.15)` | `frontend/src/app.css` |
| `--color-primary` | `#8b5cf6` | `frontend/src/app.css` (Tailwind violet-500) |
| `--color-success` / `--color-warning` / `--color-info` / `--color-destructive` | `#34d399` / `#f59e0b` / `#60a5fa` / `#f87171` | Status badges (preview banner). |

Note that the landing actually uses Tailwind's `#8b5cf6` despite `DESIGN.md` prescribing `#9775fa`. The blog matches the landing.

### Typography

| Use | Token | Family |
|---|---|---|
| Page hero / article H1 / H2 | `font-serif` | DM Serif Display |
| Card titles, H3, eyebrows, labels | `font-mono` | JetBrains Mono |
| Body text | (default) | Manrope |
| Logo wordmark only | `font-logo` | Playfair Display |

Hierarchy in long-form prose (article body):

- `h1` — `font-serif text-3xl text-white md:text-4xl`
- `h2` — `font-serif text-2xl text-white md:text-3xl`
- `h3` — `font-mono text-lg font-medium text-white`
- `p` — `text-gray-300 leading-relaxed`
- `code` (inline) — `bg-primary/10 text-void-300 font-mono`
- `pre` (block) — `bg-landing-surface border border-landing-border-subtle font-mono`
- `blockquote` — `border-l-2 border-primary/60 font-serif italic text-white`

All mappings live in `frontend/src/features/blog/components/article-body.tsx`.

### Layout primitives

| Element | Class signature |
|---|---|
| Card surface | `rounded-2xl border border-landing-border-subtle bg-landing-surface` (hover `border-primary/30`) |
| Article body container | `mx-auto max-w-3xl` |
| Page container | `mx-auto max-w-6xl px-6` |
| Eyebrow / category | `font-mono text-[10px] tracking-widest text-primary uppercase` |
| Status / tag pill | `rounded-full border px-3 py-1 font-mono text-xs` |

### Restraint rules (inherited from landing)

- **Cover images**: lazy-loaded, `object-cover`, opacity ramps from 80% → 100% on hover. They live on cards and at the top of articles. They are not used decoratively elsewhere.
- **Purple accents**: only on the eyebrow category text, primary CTA, hover borders, and the preview banner. Never as a background tint, gradient, or decoration.
- **No editorial flourishes**: no drop caps, no italic-purple emphasis, no parallax, no Unicorn Studio scenes inside the blog. Those are landing-page chrome.

---

## File map

```
frontend/src/features/blog/
├── types.ts                          # Directus-shape TypeScript interfaces (KEEP)
├── mock-data.ts                      # ⚠ PLACEHOLDER — 5 reference articles. DELETE when Directus + CDN land.
├── mock-api.ts                       # ⚠ PLACEHOLDER — async helpers reading mock-data. REWRITE bodies to fetch() against the CDN; keep signatures.
├── utils.ts                          # estimateReadingMinutes(body) (KEEP)
├── blog-index-page.tsx               # /blog (KEEP)
├── blog-detail-page.tsx              # /blog/$slug (KEEP)
├── blog-preview-page.tsx             # /preview/$id (KEEP)
└── components/                       # All KEEP — consume the BlogArticle shape, not the data source
    ├── blog-shell.tsx                # navbar (matches landing) + LandingFooter wrapper
    ├── article-card.tsx              # grid item: cover + mono category + serif title
    ├── article-meta.tsx              # author chip + date + reading time strip
    ├── article-body.tsx              # react-markdown + remark-gfm + rehype-sanitize
    ├── article-view.tsx              # shared layout for detail + preview
    ├── article-not-found.tsx         # 404 card with "Back to Field Notes"
    └── status-badge.tsx              # for the preview banner

frontend/src/pages/
├── blog-index.tsx                    # thin lazy re-export (KEEP)
├── blog-detail.tsx                   # thin lazy re-export (KEEP)
└── blog-preview.tsx                  # thin lazy re-export (KEEP)
```

**Boundary contract:** only `mock-api.ts` imports `mock-data.ts`. No page or component imports the mock data directly. This is what makes the swap a one-file change rather than a refactor.

Wired into:

- `frontend/src/pages/lazy.ts` — three lazy imports
- `frontend/src/router.tsx` — three public routes under `rootRoute`
- `frontend/src/main.tsx` — `isPublicPath()` allowlist updated for `/blog`, `/blog/`, `/preview/`

Markdown deps in `frontend/package.json`: `react-markdown`, `remark-gfm`, `rehype-sanitize`. They land only on the article-view chunk (~50 KB gzipped), not the landing.

---

## Production checklist

Before shipping the blog publicly:

- [ ] **Replace the mock layer with Directus + CDN.** Rewrite the function bodies in `mock-api.ts` as real `fetch()` calls against `VITE_BLOG_CDN_URL` (Directus `/items/blog_articles` or whatever proxy fronts it), keeping the signatures and `{ data: ... }` envelope identical. **Delete `mock-data.ts` entirely** so reference content (including the draft article body) doesn't ship in the JS bundle. See [Mock layer](#mock-layer-placeholder-for-directus--cdn).
- [ ] **Confirm the preview-URL secret is server-enforced.** With the mock, the draft article and its UUID are bundled into the public JS — anyone who reads the bundle can recover the preview URL. The real Directus + CDN must gate `/preview/<uuid>` behind a server-side lookup that doesn't enumerate.
- [ ] **Add CDN env vars.** `VITE_BLOG_CDN_URL` (and any read/preview tokens) need to land in `frontend/.env.example` and be documented in [`docs/ENV.md`](./ENV.md).
- [ ] **CORS / CDN cache headers.** The CDN endpoint should serve `published` articles with long cache TTLs and `draft`/`in_review` with `no-store`.
- [ ] **Sitemap + RSS.** Neither is implemented yet. `/sitemap.xml` should list all `published` articles; an RSS feed at `/blog/rss.xml` is the smallest meaningful affordance for subscribers.
- [ ] **SEO meta tags.** Detail pages currently set no `<title>` or `<meta description>` on the document. Add an `og:image` fallback to the hero image.
- [ ] **Pagination + filtering.** The index loads all articles in one fetch. Once the list grows past ~20, swap to paginated fetches with category/tag filtering.
- [ ] **Discoverability.** The landing nav has no link to `/blog`. Decide whether to add a "Field Notes" entry to the landing navbar (the blog already has one back to `/`) or leave the blog URL-only.

---

## Related

- [`DESIGN.md`](../DESIGN.md) — prescribed design system. The blog deviates from it where the live landing already does (primary colour, headline font).
- [`docs/CHATBOT_3RD_PARTY_INTEGRATION_SPEC.md`](./CHATBOT_3RD_PARTY_INTEGRATION_SPEC.md) — pattern for similar feature specs in this repo.
