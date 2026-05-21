# Docs Site Plan

Status: **Locked — ready for implementation** · Owner: Calvin · Last updated: 2026-05-20

A public, discoverable documentation site for NyxID, organized by **product surface**
(CLI / Web / AI-assisted) at the top level with doc-type segments inside each, rendered
from markdown that lives in `/docs/site/**`, and reachable from both the marketing site and
the logged-in dashboard.

> Reviewed + fully decided 2026-05-20 (see §9, §11). No open questions remain — this plan is
> meant to be implemented in one pass, starting with the §8 Phase 1a loader spike.

---

## 1. Goals & scope

- A **public** docs site at `/docs` (no login required) — good for SEO, sharing, and
  pre-signup evaluation.
- **Surface-first navigation:** top tabs by how you use NyxID — **CLI · Web · AI-assisted** —
  each with its own sidebar (mirrors the IDE/CLI/Web tab pattern from common dev-tool docs).
- Inside each surface, organize by **doc-type segments**: Get Started → Guides → (surface Reference).
- Surface-agnostic material (**Concepts**, **Developers/API/SDK**) is authored **once** in a
  shared area and linked from every surface — never duplicated.
- **All published content is markdown** in a self-contained **`/docs/site/{cli,web,ai,shared}/`**
  tree (public-worthy files are moved/mirrored here; internal specs stay in `/docs` and are never
  published). Surfaced via the repo's existing pattern: **copy `/docs/site/**` into
  `frontend/public/docs/` at build time and `fetch()` at runtime** (same as today's `public/legal/`).
- A clear boundary between **public docs** (explanation/reference, static) and
  **in-app help** (interactive/personalized, stays in the dashboard).

## 2. Decisions locked

| Decision | Choice |
|---|---|
| Top-level IA | Surface tabs: **CLI / Web / AI-assisted** (how you use NyxID) |
| Within a surface | Doc-type segments: Get Started → Guides → (Reference) |
| Shared material | **Concepts** + **Developers (API/SDK/OAuth)** = pinned shared groups (NOT tabs), authored once |
| Surface | Public route first (`/docs`), outside `DashboardLayout` |
| Markdown layout | All published pages live under self-contained **`/docs/site/{cli,web,ai,shared}/`**; internal specs stay in `/docs`, never published |
| Content loading | `/docs/site/**` copied to `frontend/public/docs/` at build → `fetch` at runtime (matches `public/legal/`); manifest = publish gate **+ lint** |
| Images | Screenshots exist (`connecting-services/img/`, 6 PNGs); move under `/docs/site/**/img/`, copy to `public/docs-assets/`, build rewrites relative `img` srcs (no ban) |
| Search | **Build in v1** — build-time index (tab · slug · title · headings), command-palette `/` UX, scoped to active tab + all-surfaces toggle |
| Render stack | Reuse blog `article-body.tsx`: `react-markdown@10` + `remark-gfm@4` + `rehype-sanitize@6`. **No MDX.** Shiki deferred to Phase 3 |
| SEO | CSR + per-page meta/sitemap/robots in v1; **crawl-prerender (puppeteer/@prerenderer) in Phase 3** — NOT `vite-react-ssg` |
| Lead surface | **AI-assisted first** in Phase 1 (headline; writing-heavy) |
| Docs vs. in-app | Public docs = static explanation/reference; in-app = interactive/personalized |

## 3. Information architecture

Primary nav is **top tabs by product surface** — mirroring the multi-surface docs pattern
(e.g. an IDE / CLI / Web tab bar). Each tab owns its own left sidebar, organized into
doc-type segments. Surface-agnostic material lives in a shared area, authored once.

```
[ CLI ]   [ Web ]   [ AI-assisted ]          (shared, reachable from every tab: Concepts · Developers)
   └─ per-surface sidebar:  Get Started  →  Guides  →  (surface Reference)
```

**Why surface-first:** NyxID has three genuinely different workflows (typing `nyxid`
commands, clicking the dashboard, wiring an agent/MCP). It also fits the existing corpus —
`docs/connecting-services/` *already* splits "connect a service" by surface.

> **The DRY rule (this is what actually prevents duplication — structure alone does not):**
> **one procedure per (task × surface), one explanation in Concepts.** Each surface Guide is a
> *thin, surface-specific procedure* that links to a shared Concept for the "why". Never
> re-explain the model inside a Guide. See the content-map table below (review finding F5).

> **Locked:** the three surfaces (CLI / Web / AI-assisted) are the docs' identity.
> **Concepts** and **Developers** are NOT top tabs — they are shared groups pinned to the
> bottom of every surface sidebar, linking to one canonical set of pages. "Build an app on
> NyxID" (add-login / OAuth / SDK) lives inside **Developers**, not a 4th tab.

> Source legend: **NEW** = curated public markdown to write. **REUSE** = existing `/docs` file
> is already public-appropriate (curate lightly). **All published files (NEW + REUSE) are placed
> under `/docs/site/**`; REUSE files are moved there from their current `/docs` location.**

### Content-map (ownership — prevents the 2–3× rewrite of approvals/nodes/proxy)
| Topic | Concept page (the "why", authored once) | Surface procedure(s) (thin, link to concept) |
|---|---|---|
| Connect a service | endpoints/keys/services + the proxy | CLI · Web · AI (each surface-specific) |
| Approvals | "Approvals / human-in-the-loop" | Web (Telegram/mobile setup) · AI (agent approval flow) — **no 3rd** |
| Credential nodes | "Credential nodes" | CLI (set up node + SSH node). **No Web node story — surfaces need not be symmetric** |
| Agent isolation | "Agent isolation" | CLI (scoped keys) · AI (per-agent setup) |
| OAuth/OIDC | "OAuth/OIDC & identity" | Developers (add login, SDK) |

### Tab: CLI — *driving NyxID from the `nyxid` CLI*
- **Get Started**: Install the CLI · Authenticate (login / pairing / `--profile`) ·
  Your first connection (connect + proxy a call) — *NEW*
- **Guides**: Connect an AI service · Set up a credential node · **Set up an SSH node** ·
  Create scoped agent keys · Manage organizations · Channel bots
  — *REUSE `connecting-services/cli.md`, `quickstarts/node-proxy.md`, `SSH_NODE_KEY_AUTH.md`, `NYXID_NODE.md`, `AGENT_ISOLATION.md`*
- **Command reference**: NyxID's CLI is large — `cli/src/cli.rs` is ~3,300 LOC / **~38 command
  groups** (review F9). v1 hand-authors the **top ~6–8 high-traffic groups** (`service` ·
  `api-key` · `node` · `ssh` · `proxy` · `catalog` · `org` · `mcp`); the rest follow later.
  Source: `cli/src/cli.rs` + playbook §23. **Largest authoring item; do NOT gate Phase 2 on a clap generator** (F10).

### Tab: Web — *the dashboard*
- **Get Started**: Sign up & log in · Dashboard tour · Your first connection (via UI) — *NEW*
- **Guides**: Connect a service (web console) · Manage keys & credentials · Approvals
  (Telegram / mobile) · Organizations & shared credentials · Developer apps (register OAuth
  clients) · Channel bots · Account & security (MFA, sessions)
  — *REUSE `connecting-services/web-ui.md` (ships with its 6 screenshots), `ORG_MODEL.md`, approval docs*

### Tab: AI-assisted — *using NyxID through AI agents* (Phase 1 lead)
- **Get Started**: What AI-assisted access is (the broker model for agents) · Connect your
  agent / install the NyxID skill · Your first agent call (MCP proxy)
  — *mostly **NEW** prose; this Get Started path is the most greenfield writing in the whole site (review F6)*
- **Guides**: Set up Claude Code / Cursor / Codex · MCP proxy & tool discovery · Agent
  isolation · Wrap a REST API as MCP tools · Approvals for agents · The `llms.txt` playbook
  — *REUSE `quickstarts/claude-code.md`, `quickstarts/mcp-wrapping.md`, `AGENT_ISOLATION.md`, `MCP_DELEGATION_FLOW.md`*

### Shared — *surface-agnostic, authored once, linked from every tab*
- **Concepts**: the broker model · endpoints/keys/services · the proxy · MCP proxy ·
  credential nodes · agent isolation · approvals · organizations · OAuth/OIDC & identity ·
  encryption & key management
  — *REUSE `ARCHITECTURE.md`, `ORG_MODEL.md`, `OIDC.md`, `AGENT_ISOLATION.md`, `ENCRYPTION_ARCHITECTURE.md` (trim to concept-only)*
- **Developers** (build an app *on* NyxID): Add login to your app · React SDK · Core SDK ·
  Raw OAuth 2.0 / PKCE · OIDC & `.well-known` · Scopes reference · Service accounts ·
  Token exchange / delegation · **API Reference**
  — *REUSE `integration-guide`, SDK READMEs, `OIDC.md`, `SERVICE_ACCOUNTS.md`, `API.md` (split), `backend/src/errors/mod.rs`*

**Authoring summary:** Guides and most Concepts already exist in `/docs`. The real new
authoring is each surface's **Get Started** path (AI-assisted especially) and the **CLI
command reference**.

## 4. Technical architecture

### Content loading — `/docs/site/**` → `public/`, runtime fetch (matches repo precedent)
All published markdown lives in the self-contained **`/docs/site/{cli,web,ai,shared}/`** tree
(public-worthy files moved here; internal specs stay in `/docs`, never published). The repo's
de-facto pattern is copy-to-`public/` + runtime `fetch` (legal docs are copied into
`frontend/public/legal/`). There is **no** `import.meta.glob`/`?raw` precedent and the frontend
is **Vite 8** (stricter `fs.allow`), so build-time import is **rejected**. Instead:

- A small build/dev step copies `/docs/site/**` → `frontend/public/docs/<tab>/<slug>.md`, and
  `/docs/site/**/img/**` → `frontend/public/docs-assets/`.
- The docs route `fetch()`es the file by slug at runtime and renders it. Out of the JS bundle, cacheable.
- **Image policy (review F3 — confirmed: images exist):** 6 screenshots in `connecting-services/img/`
  are used by `web-ui.md` + `n8n.md` via relative paths. The build copies referenced assets to
  `public/docs-assets/`, and the `img` renderer rewrites relative `src` (both `img/x.png` and
  `../connecting-services/img/x.png` forms) to that public base. A blanket ban is not viable —
  the best Web guide ships *because* of its screenshots.

### Manifest + lint (kills drift/leaks — review F2)
A TypeScript manifest is the publish gate and owns the tab → group → page tree, order, and titles.

```ts
// frontend/src/features/docs/docs.manifest.ts (sketch)
export const DOCS_TABS: DocTab[] = [
  { tab: "CLI", groups: [
    { group: "Get Started", items: [
      { slug: "install",          title: "Install the CLI",      source: "site/cli/install.md" },
      { slug: "first-connection", title: "Your first connection",source: "site/cli/first-connection.md" },
    ]},
    { group: "Command reference", items: [ /* service, api-key, node, ssh, ... */ ] },
  ]},
  { tab: "Web", groups: [ /* ... */ ] },
  { tab: "AI-assisted", groups: [ /* ... */ ] },
];
export const DOCS_SHARED: DocGroup[] = [ /* Concepts, Developers */ ];
```

**Build/CI lint (~50 LOC, required):** every `source` resolves to a real `/docs/site` file; slugs
unique per scope; scan rendered markdown for relative `*.md` links and fail the build unless each
resolves to a *manifested* slug (prevents public dead links and internal-spec leaks).

### Rendering — reuse the proven blog stack (low-risk)
Already ships in `frontend/src/features/blog/components/article-body.tsx`:
**`react-markdown@10` + `remark-gfm@4` + `rehype-sanitize@6`.** Reuse that component map.

- **Do NOT add MDX** — absent and unnecessary for this content.
- Add 3 small standard deps: `remark-directive` (`:::note` → `<Callout>`), `rehype-slug`,
  `rehype-autolink-headings` (anchored headings + right-rail TOC).
- **Shiki not installed and heavy — defer to Phase 3** (paid at prerender time). v1 uses existing `<pre>`/`<code>` styling.
- Component map → DESIGN.md tokens; `a` renderer rewrites internal links (slug-resolved per the lint) and opens external links in a new tab.

### Search (v1)
Build-time index (tab · slug · title · headings · excerpt) generated during the copy step;
client-side fuzzy match reusing the command-palette `/` UX, scoped to the active tab with an
"all surfaces" toggle. No external search service. Ships in Phase 1b.

### SEO — CSR + meta in v1, crawl-prerender in Phase 3 (review F4)
App is `appType:"spa"`, pure CSR, plain `@tanstack/react-router@1.159` (not TanStack Start), with
`window`/auth-store coupling in `beforeLoad` hooks. `vite-react-ssg` is a multi-day retrofit that
fights that coupling — **not a v1 toggle.**

- **v1 (cheap, 70% of the value):** per-page `<title>`/meta/canonical/OG tags + `sitemap.xml`;
  confirm `public/robots.txt` does not `Disallow` `/docs`.
- **Phase 3:** build-time **crawl-prerender** of `/docs/*` via puppeteer/`@prerenderer` (bolt-on,
  no router rewrite; docs are static so crawl output is stable). Not SSG.

### Routing (low-risk)
TanStack Router public route group outside `DashboardLayout`, exactly like `/blog`, `/privacy`,
`/terms`: `/docs` (index landing), `/docs/$tab/$slug`, plus `/docs/concepts/$slug` and
`/docs/developers/$slug`. `DocsLayout` = surface tab bar + per-surface sidebar + content + TOC.

## 5. Layout & components

Public chrome (Playfair wordmark), **surface tab bar** up top (switches the entire left
sidebar), 3-column body on desktop:

```
[Nyx wordmark]        [ CLI | Web | AI-assisted ]        [search] [GitHub] [Log in / Dashboard]
+--------------+---------------------------------+-----------------+
| Per-surface  | Breadcrumb · H1 · lead          | On this page    |
| sidebar:     | body (code, callouts, steps)    | (TOC)           |
|  Get Started | prev / next pager · edit on GH  |                 |
|  Guides      |                                 |                 |
|  Reference   |                                 |                 |
|  ──────────  |                                 |                 |
|  Concepts ◇  |  ◇ shared groups, link to the   |                 |
|  Developers ◇|     single canonical pages      |                 |
+--------------+---------------------------------+-----------------+
```

- Top tabs switch the whole sidebar. Shared **Concepts** / **Developers** groups are pinned to
  the bottom of every surface sidebar and point to one canonical set of pages.
- Right header button is context-aware: "Log in" logged-out, "Go to Dashboard" with a session.
- Search trigger in the header opens the `/` command-palette-style index.
- Mobile: tab bar → segmented control; sidebar → drawer (mirror `MobileNav`).
- Styled to **DESIGN.md**. Reuse markdown components: `<Callout>`, `<CodeBlock>`/`<CodeTabs>`,
  `<Steps>`, `<CardGrid>`, `<ApiEndpoint>`, `<ScopeTable>`.

## 6. Entry points (accessible from both surfaces)

**Marketing** (currently zero docs links):
- Add **"Docs"** to `LandingNavbar`. It uses raw `<a href>` + i18n `t("nav.*")` and the wordmark
  links to `#` (anchor-scroll page) — so add an i18n key **`nav.docs`** and a real
  `<a href="/docs">`, not an in-page anchor (review F8).
- Add a **Developers/Resources** column to `LandingFooter` (Docs · API Reference · CLI · `llms.txt` · GitHub).

**Dashboard:**
- Repoint Quick Links **"Documentation"** from `/guide` → `/docs`.
- Add **"Documentation"** to the command palette (`ALL_ITEMS`).
- Sidebar "Guide" → `/docs`.
- **Contextual deep links** ("Learn more →") on Keys, Nodes, AI Setup, Channel Bots,
  Developer Apps → relevant docs section (default tab: Web, or AI-assisted from AI Setup).

## 7. Public docs vs. in-app help boundary + legacy migration

| | Public `/docs` | In-app help (dashboard) |
|---|---|---|
| Nature | Explanation + reference, static, SEO-able | Interactive + personalized to your account |
| Examples | Concepts, Guides, CLI, API, SDK | `/ai-setup` MCP config generator, copy-your-key snippets |

The 3 legacy guide pages are live and substantial (combined ~1,047 LOC) and referenced in the
sidebar + command palette. **Do not delete paths; do not assume they are thin wrappers (review F7).**
- `/ai-setup` (427 LOC, interactive, `?skill=`/`?tool=`) → **stays in-app**; the AI-assisted docs tab *deep-links into* it.
- `/integration-guide` (232 LOC, `?tab=` deep links) → content moves to shared **Developers**;
  keep the old path as a **permanent `redirect` route** (repo already does this: `/api-keys`→`/keys`, etc.).
- `/guide` (388 LOC) → content moves to **Web → Get Started** + **Concepts**; keep path as a redirect.
- **Before migrating, diff each page's actual copy against the `/docs` files it claims to map to.**
  Capture any hand-written content with no `/docs` equivalent into new `docs/site/*` pages or it is lost.

## 8. Phased rollout

1. **Phase 1a — De-risk spike (do FIRST).** Prove one `/docs/site/` file renders end-to-end via
   the copy-to-`public/` + fetch step (target page: `/docs/cli/install`), wire the manifest + the
   lint, and prove image copy + relative-`src` rewrite using `web-ui.md` + its screenshots. Small,
   but it removes the only real unknowns.
2. **Phase 1b — Spine.** `DocsLayout` + surface tab bar + **AI-assisted** (lead) and **Web** tabs
   (Get Started + core Guides) + shared **Concepts** + **build-time search index** + per-page
   meta/sitemap. Wire marketing nav/footer + dashboard links + legacy redirects.
   *(AI-assisted leads and is writing-heavy, not wiring-heavy.)*
3. **Phase 2 — CLI + Developers.** CLI tab (Get Started + Guides + **curated ~6–8** command-ref
   pages) + shared **Developers** (SDK/OAuth + API Reference, migrating `/integration-guide`).
4. **Phase 3 — Polish.** **crawl-prerender SEO**, Shiki highlighting, "edit on GitHub" /
   "was this helpful", remaining CLI groups + guides.
5. **Later.** Self-host/operator docs — a 4th surface tab or a shared "Operations" area.

## 9. Decisions — all settled (2026-05-20)

*Identity: 3 surfaces (Concepts/Developers are pinned shared groups, not tabs; "build an app on
NyxID" lives in Developers). Loading: `/docs/site/**` copied to `public/` + runtime fetch.
SEO: CSR + meta in v1, crawl-prerender in Phase 3 (not SSG). Render: reuse blog stack, no MDX,
defer Shiki. CLI ref: ~38 groups, v1 curates 6–8 (no clap-generator gate).*

Final four (settled this round):
1. **Markdown layout:** everything published lives under **`/docs/site/{cli,web,ai,shared}/`**
   (self-contained public tree; internal specs stay in `/docs`).
2. **Search:** **build in v1** — build-time index, command-palette `/` UX.
3. **Lead surface:** **AI-assisted first** (headline; budget as writing-heavy).
4. **Images:** screenshots exist; build copies assets to `public/docs-assets/` + rewrites
   relative `img` srcs (no ban).

**No open questions remain.** Ready for one-shot Phase 1 implementation; run the 1a loader spike as the first commit.

## 10. Key file touch-points (for implementation)

- Routing: `frontend/src/router.tsx` (add public `/docs` group; `redirect` routes for `/guide`, `/integration-guide`)
- New: `frontend/src/features/docs/` (`DocsLayout` + tab bar, manifest + lint, search index, fetch+render reusing `features/blog/components/article-body.tsx`)
- Build step: copy `/docs/site/**` → `frontend/public/docs/` + `img/**` → `frontend/public/docs-assets/`; emit search-index JSON
- Marketing: `landing-navbar.tsx` (+ i18n `nav.docs`), `landing-footer.tsx`
- Dashboard wiring: `dashboard.tsx` (Quick Links), `command-palette.tsx` (`ALL_ITEMS`), `sidebar.tsx`
- Content: `/docs/site/{cli,web,ai,shared}/**` (all published pages moved here) + `/docs/site/**/img/**` assets

## 11. Independent review (2026-05-20)

Reviewer: Claude sub-agent, grounded against `frontend/package.json` (Vite 8, `react-markdown@10`,
`remark-gfm@4`, `rehype-sanitize@6`; no MDX/Shiki/SSG), `vite.config.ts` (`appType:"spa"`, no
`fs.allow`), `router.tsx` (CSR `@tanstack/react-router@1.159`), the legacy guide pages, and `cli.rs`.

**Verdict:** architecture sound; **start Phase 1 only after the 1a loader spike** and the
corrected SEO/CLI framing. Must-fix de-risk items (all folded in above):
1. Loader = copy-to-`public/` + fetch (F1), manifest lint (F2), image copy + src-rewrite (F3).
2. SEO = CSR + meta in v1, crawl-prerender (not `vite-react-ssg`) in Phase 3 (F4).
3. IA DRY rule + content-map table so approvals/nodes/proxy aren't written 2–3× (F5).
4. Legacy guides: redirects + diff-before-migrate; `/ai-setup` stays in-app (F7).
5. CLI reference re-baselined (~38 groups, v1 curates 6–8; no clap-generator gate) (F9/F10).

**Confirmed low-risk:** render pipeline (reuse blog stack), routing (`/docs` slots in like
`/blog`), and wiring (targets exist, redirect pattern idiomatic).

> The original Codex review (`/codex`) could not run — its CLI is configured against the aelf LLM
> gateway (`llm.aelf.dev`) which returned `401 INVALID_API_KEY`. Re-auth and re-run for a second
> independent opinion if desired.
