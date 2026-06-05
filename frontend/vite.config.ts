/// <reference types="vitest/config" />
import { defineConfig, type Plugin } from "vite"
import react from "@vitejs/plugin-react"
import tailwindcss from "@tailwindcss/vite"
import path from "path"
import fs from "node:fs"
import { allDocPages } from "./src/features/docs/manifest"

const backendUrl = process.env.BACKEND_URL || "http://localhost:3001"

// Backend CSRF middleware compares the request Origin against FRONTEND_URL.
// When multiple worktrees run in parallel, Vite may pick a port other than
// 3000 and the real Origin won't match — logout (and any other unsafe
// cookie-auth POST) then 403s. Rewrite Origin/Referer at the proxy so the
// backend always sees the expected dev origin.
const expectedOrigin = process.env.FRONTEND_URL || "http://localhost:3000"

function originRewrite(proxyReq: import("http").ClientRequest) {
  if (proxyReq.getHeader("origin")) {
    proxyReq.setHeader("origin", expectedOrigin)
  }
  if (proxyReq.getHeader("referer")) {
    proxyReq.setHeader("referer", `${expectedOrigin}/`)
  }
}

/** Strip Secure / Domain from Set-Cookie so cookies work on http://localhost */
function cookieRewrite(proxyRes: import("http").IncomingMessage) {
  const sc = proxyRes.headers["set-cookie"]
  if (!sc) return
  proxyRes.headers["set-cookie"] = sc.map((c) =>
    c
      .replace(/;\s*Secure/gi, "")
      .replace(/;\s*Domain=[^;]*/gi, "")
      .replace(/;\s*SameSite=None/gi, "; SameSite=Lax"),
  )
}

const proxyTarget = {
  target: backendUrl,
  changeOrigin: true,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  configure: (proxy: any) => {
    proxy.on("proxyReq", originRewrite)
    proxy.on("proxyRes", cookieRewrite)
  },
}

const apiProxy = {
  "/api": proxyTarget,
  "^/oauth(?:/.*)?$": proxyTarget,
  "/mcp": proxyTarget,
  "/.well-known": proxyTarget,
  "/health": proxyTarget,
}

// ── Docs content sync + publish-gate lint ──
// Single source of truth for the public docs site is markdown under
// `/docs/site/**`. This plugin mirrors it (markdown + sibling images) into
// `frontend/public/docs/` so it ships as static assets the docs routes
// `fetch()` at runtime — the same copy-to-public pattern `public/legal/`
// already uses. It also emits `search-index.json` for the in-docs search and
// `public/sitemap.xml` for crawlers, then runs the publish gate: the manifest
// is the only thing that makes a page navigable, so every manifest entry must
// resolve to a real file, every published file must be manifested, and every
// internal `/docs/...` link must point at a manifested slug. A failure aborts
// `vite build`; in dev it logs a warning so authoring isn't blocked.
// Generated output (public/docs, public/sitemap.xml) is gitignored; edit the
// source in `/docs/site/**`.
interface DocSearchEntry {
  source: string
  title: string
  description: string
  headings: string[]
}

interface DocFile {
  slug: string
  raw: string
}

// Canonical origin for sitemap <loc> entries (matches public/robots.txt).
const SITEMAP_BASE = "https://nyx.chrono-ai.fun"
// Public, indexable routes outside the docs tree.
const STATIC_SITEMAP_ROUTES = ["/", "/docs", "/blog", "/privacy", "/terms"]

function docsSync(): Plugin {
  const srcDir = path.resolve(__dirname, "../docs/site")
  const outDir = path.resolve(__dirname, "public/docs")
  const sitemapPath = path.resolve(__dirname, "public/sitemap.xml")
  let isBuild = false

  const parseMeta = (raw: string) => {
    const front = raw.match(/^---\n([\s\S]*?)\n---/)?.[1] ?? ""
    const title = front.match(/title:\s*"?(.+?)"?\s*$/m)?.[1]?.trim() ?? ""
    const description = front.match(/description:\s*"?(.+?)"?\s*$/m)?.[1]?.trim() ?? ""
    const headings = Array.from(
      raw.matchAll(/^##\s+(.+)$/gm),
      (m) => (m[1] ?? "").trim(),
    ).filter(Boolean)
    return { title, description, headings }
  }

  const writeSitemap = () => {
    const urls = [
      ...STATIC_SITEMAP_ROUTES,
      ...allDocPages().map((p) => `/docs/${p.slug}`),
    ]
    const body = urls
      .map((u) => `  <url><loc>${SITEMAP_BASE}${u}</loc></url>`)
      .join("\n")
    fs.writeFileSync(
      sitemapPath,
      `<?xml version="1.0" encoding="UTF-8"?>\n` +
        `<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${body}\n</urlset>\n`,
    )
  }

  // Mirror /docs/site/** -> public/docs/**, emit the search index + sitemap, and
  // return the markdown files (slug + raw body) for the publish-gate lint.
  const sync = (): DocFile[] => {
    if (!fs.existsSync(srcDir)) return []
    fs.rmSync(outDir, { recursive: true, force: true })
    fs.mkdirSync(outDir, { recursive: true })
    const index: DocSearchEntry[] = []
    const docs: DocFile[] = []
    const walk = (dir: string) => {
      for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const abs = path.join(dir, entry.name)
        if (entry.isDirectory()) {
          walk(abs)
          continue
        }
        const rel = path.relative(srcDir, abs).replace(/\\/g, "/")
        const out = path.join(outDir, rel)
        fs.mkdirSync(path.dirname(out), { recursive: true })
        fs.copyFileSync(abs, out)
        if (entry.name.endsWith(".md")) {
          const raw = fs.readFileSync(abs, "utf8")
          index.push({ source: rel, ...parseMeta(raw) })
          docs.push({ slug: rel.replace(/\.md$/, ""), raw })
        }
      }
    }
    walk(srcDir)
    fs.writeFileSync(path.join(outDir, "search-index.json"), JSON.stringify(index))
    writeSitemap()
    return docs
  }

  // Publish gate. Returns human-readable problems; empty array means clean.
  const lint = (docs: DocFile[]): string[] => {
    const errors: string[] = []
    const manifestSlugs = new Set<string>()
    for (const p of allDocPages()) {
      if (manifestSlugs.has(p.slug)) errors.push(`duplicate slug in manifest: ${p.slug}`)
      manifestSlugs.add(p.slug)
    }
    const fileSlugs = new Set(docs.map((d) => d.slug))
    for (const slug of manifestSlugs) {
      if (!fileSlugs.has(slug)) errors.push(`manifest references a missing file: docs/site/${slug}.md`)
    }
    for (const slug of fileSlugs) {
      if (!manifestSlugs.has(slug)) {
        errors.push(`unmanifested doc would publish unlinked: docs/site/${slug}.md`)
      }
    }
    // Scan body links. Internal `/docs/...` links must resolve to a manifested
    // slug; relative links are banned (they break at runtime and can leak
    // private `/docs` spec paths). The `(?<!!)` skips image embeds `![](src)`.
    for (const { slug, raw } of docs) {
      for (const m of raw.matchAll(/(?<!!)\[[^\]]*\]\(([^)\s]+)/g)) {
        const href = (m[1] ?? "").trim()
        if (!href || /^(https?:|mailto:|tel:|#)/.test(href)) continue
        const target = (href.split("#")[0] ?? href).replace(/\/+$/, "")
        if (target.startsWith("/docs/")) {
          if (!manifestSlugs.has(target.slice("/docs/".length))) {
            errors.push(`dead docs link in ${slug}.md: ${href}`)
          }
        } else if (!target.startsWith("/")) {
          errors.push(`relative link in ${slug}.md (use an absolute /docs/<slug>): ${href}`)
        }
        // Other absolute app links (e.g. /keys, /ai-setup) are allowed as-is.
      }
    }
    return errors
  }

  return {
    name: "nyxid-docs-sync",
    configResolved(config) {
      isBuild = config.command === "build"
    },
    buildStart() {
      const errors = lint(sync())
      if (!errors.length) return
      const msg = `docs publish-gate lint failed:\n  - ${errors.join("\n  - ")}`
      if (isBuild) this.error(msg)
      else this.warn(msg)
    },
    configureServer(server) {
      server.watcher.add(srcDir)
      server.watcher.on("all", (_event, file) => {
        if (typeof file !== "string" || !file.startsWith(srcDir)) return
        const errors = lint(sync())
        if (errors.length) {
          server.config.logger.warn(`[docs] publish-gate lint:\n  - ${errors.join("\n  - ")}`)
        }
      })
    },
  }
}

export default defineConfig({
  plugins: [react(), tailwindcss(), docsSync()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes("node_modules/react-dom") || id.includes("node_modules/react/")) {
            return "vendor-react"
          }
          if (id.includes("node_modules/@tanstack/react-router")) {
            return "vendor-router"
          }
          if (id.includes("node_modules/@tanstack/react-query")) {
            return "vendor-query"
          }
          if (id.includes("node_modules/@radix-ui/")) {
            return "vendor-radix"
          }
        },
      },
    },
  },
  server: {
    port: 3000,
    proxy: apiProxy,
  },
  preview: {
    port: 3000,
    proxy: apiProxy,
  },
  appType: "spa",
  test: {
    globals: true,
    environment: "happy-dom",
    setupFiles: ["./src/test-setup.ts"],
    include: ["src/**/*.test.{ts,tsx}", "test/**/*.test.{ts,tsx}"],
    coverage: {
      // CI coverage gate (issue #785). `npm run test:coverage` (vitest run
      // --coverage) enforces the line threshold below and fails the run if FE
      // line coverage drops under it. The CI workflow (.github/workflows/ci.yml
      // → "Coverage (Frontend)") also publishes the summary, uploads the report
      // as an artifact, and feeds the per-component PR-comment delta.
      //
      // Run locally with `npm run test:coverage` and read per-file line % from
      // the text table.
      //
      // Ratchet plan (W21 coverage push, issues #782-#787): raise FE lines to
      // 30% by quarter end as those issues land. Keep this number and the CI
      // PR-comment baseline in sync; bump only — never lower to force a pass.
      provider: "v8",
      // `json-summary` feeds the CI delta comment + threshold gate; `lcov` is
      // the artifact + Codecov-compatible format; `text`/`html` are for humans.
      reporter: ["text", "json-summary", "json", "html", "lcov"],
      reportsDirectory: "./coverage",
      // Fail the run (and therefore CI) when line coverage drops below the
      // enforced gate. Mirrors `cargo llvm-cov --fail-under-lines` for Rust.
      thresholds: {
        lines: 15,
      },
      include: ["src/**/*.{ts,tsx}"],
      exclude: [
        "src/**/*.test.{ts,tsx}",
        "src/**/*.d.ts",
        // Entry points / framework wiring with no branching logic to test.
        "src/main.tsx",
        "src/router.tsx",
        "src/wizard-entry.tsx",
        "src/test-setup.ts",
        // Vendored shadcn/Radix primitives — owned upstream, not our logic.
        "src/components/ui/**",
        "src/types/**",
        // Hand-drawn SVG empty-state icons — presentational, like ui/.
        "src/components/icons/empty-state/**",
        // Marketing landing surface (incl. animation hooks) — presentational.
        "src/features/landing/**",
        // Blog feature: presentational components, fixtures, and types. The
        // one pure helper (utils.ts → estimateReadingMinutes) is kept in and
        // tested; everything else here is markdown rendering / mock content.
        "src/features/blog/**/*.tsx",
        "src/features/blog/mock-data.ts",
        "src/features/blog/mock-api.ts",
        "src/features/blog/types.ts",
        // Lazy route loaders — framework wiring with no branching (cf. router.tsx).
        "src/pages/lazy.ts",
        // Static config / fixtures / type-only modules — no branching logic.
        "src/lib/mock-data.ts",
        "src/lib/navigation.ts",
        "src/lib/telemetry-schema.ts",
        // Static legal / redirect stub pages — no branching logic.
        "src/pages/privacy.tsx",
        "src/pages/terms.tsx",
        "src/pages/connections.tsx",
        "src/pages/services.tsx",
        "src/pages/providers-layout.tsx",
        "src/pages/_legal-document-page.tsx",
      ],
    },
  },
})
