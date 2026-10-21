import {
  verifyReleaseConfiguration,
  retainedDisabledFeatureCode,
} from "./public-visibility-contract.mjs";
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { verifyMarkdownContract } from "./public-markdown-contract.mjs";
import { semanticMarkdown } from "./public-content.mjs";
import { Window } from "happy-dom";
const root = path.resolve(import.meta.dirname, ".."),
  dist = path.join(root, "dist");
const release = JSON.parse(
  await fs.readFile(path.join(dist, "public-release.json"), "utf8"),
);
verifyReleaseConfiguration(release);
const inventory = JSON.parse(
  await fs.readFile(path.join(root, "content/publication.json"), "utf8"),
);
const published = JSON.parse(
  await fs.readFile(path.join(root, "content/published.json"), "utf8"),
);
assert.deepEqual(
  published,
  inventory.pages
    .filter((p) => p.approval === "approved" && p.publication === "published")
    .map(({ id, route, locales, indexable }) => ({
      id,
      route,
      locales,
      indexable,
    })),
  "Stale publication projection; regenerate approved pages",
);
const paths = new Set(release.pages.map((p) => p.path));
assert.equal(paths.size, release.pages.length);
const expected = new Set([
  ...published.flatMap((p) =>
    p.locales.map((locale) =>
      locale === "zh" ? `/zh${p.route === "/" ? "" : p.route}` : p.route,
    ),
  ),
  "/docs",
  ...JSON.parse(
    await fs.readFile(path.join(dist, "docs/search-index.json"), "utf8"),
  ).map((p) => `/docs/${p.source.replace(/\.md$/, "")}`),
  "/privacy",
  "/terms",
]);
assert.deepEqual(
  [...paths].sort(),
  [...expected].sort(),
  "Missing/extra canonical HTML",
);
const titles = new Set();
const sitemap = await fs.readFile(path.join(dist, "sitemap.xml"), "utf8");
const llms = await fs.readFile(path.join(dist, "llms.txt"), "utf8");
const failures = [];
assert.equal(
  llms.includes("- [Service status]("),
  release.visibility.footerStatus,
);
const appRoutes = JSON.parse(
  await fs.readFile(path.join(dist, "application-routes.json"), "utf8"),
);
const appPatterns = appRoutes.map(
  (route) =>
    new RegExp(
      "^" +
        route
          .split("/")
          .map((segment) =>
            segment.startsWith("$")
              ? ".+"
              : segment.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"),
          )
          .join("/") +
        "$",
    ),
);
for (const page of release.pages) {
  try {
    const html = await fs.readFile(path.join(dist, page.file), "utf8"),
      md = await fs.readFile(path.join(dist, page.markdown), "utf8");
    verifyMarkdownContract(page.path, md);
    const doc = new Window().document;
    doc.write(html);
    assert.equal(doc.querySelectorAll("h1").length, 1);
    assert.ok(
      (doc.querySelector("main")?.textContent?.trim().length ?? 0) > 40,
    );
    assert.equal(doc.querySelectorAll("title").length, 1);
    assert.equal(doc.title, `${page.title} · NyxID`);
    assert.ok(!titles.has(doc.title), "Duplicate title");
    titles.add(doc.title);
    assert.equal(doc.querySelectorAll('meta[name="description"]').length, 1);
    assert.equal(
      doc.querySelector('meta[name="description"]').content,
      page.description,
    );
    assert.equal(doc.querySelectorAll('link[rel="canonical"]').length, 1);
    assert.equal(
      doc.querySelector('link[rel="canonical"]').href,
      `https://nyx.chrono-ai.fun${page.path}`,
    );
    assert.equal(
      doc.documentElement.lang,
      page.locale === "zh" ? "zh-CN" : "en",
    );
    assert.equal(
      doc.querySelectorAll("link[hreflang]").length,
      page.translated ? 3 : 0,
    );
    for (const link of doc.querySelectorAll("link[hreflang]"))
      assert.ok(paths.has(new URL(link.href).pathname), "Missing alternate");
    for (const [selector, attribute, value] of [
      ['meta[property="og:title"]', "content", `${page.title} · NyxID`],
      ['meta[property="og:description"]', "content", page.description],
      [
        'meta[property="og:url"]',
        "content",
        `https://nyx.chrono-ai.fun${page.path}`,
      ],
      [
        'meta[property="og:image"]',
        "content",
        "https://nyx.chrono-ai.fun/nyxid-coloured-logo.svg",
      ],
      ['meta[name="twitter:card"]', "content", "summary"],
      ['meta[name="twitter:title"]', "content", `${page.title} · NyxID`],
    ]) {
      assert.equal(doc.querySelectorAll(selector).length, 1);
      assert.equal(doc.querySelector(selector).getAttribute(attribute), value);
    }
    if (page.translated) {
      for (const [locale, lang] of [
        ["en", "en"],
        ["zh", "zh-Hans"],
      ]) {
        const target =
          locale === "zh"
            ? `/zh${page.route === "/" ? "" : page.route}`
            : page.route;
        assert.equal(
          doc.querySelector(`link[hreflang="${lang}"]`).href,
          `https://nyx.chrono-ai.fun${target}`,
        );
        assert.ok(
          release.pages.find((p) => p.path === target)?.translated,
          "Alternate must link back",
        );
      }
    }
    if (page.file.startsWith("__public/")) {
      assert.equal(
        semanticMarkdown(
          html,
          page,
          page.revision,
          "https://nyx.chrono-ai.fun",
        ),
        md,
        "HTML/Markdown semantic drift",
      );
      const computed = createHash("sha256")
        .update(semanticMarkdown(html, page, "", "https://nyx.chrono-ai.fun"))
        .digest("hex");
      assert.equal(computed, page.revision, "Content revision drift");
    }
    for (const [feature, enabled] of [
      ["newsletter", release.visibility.newsletter],
      ["contact-chat", release.visibility.contactChat],
      ["footer-status", release.visibility.footerStatus],
    ]) {
      const expected =
        enabled &&
        (feature === "footer-status" ||
          (feature === "newsletter"
            ? /\/blog$/.test(page.path)
            : /\/contact$/.test(page.path))) &&
        !page.path.startsWith("/docs") &&
        !["/privacy", "/terms"].includes(page.path);
      assert.equal(
        Boolean(doc.querySelector(`[data-website-feature="${feature}"]`)),
        Boolean(expected),
        `${page.path}: ${feature} output disagrees with release configuration`,
      );
    }
    const site = doc.querySelector("[data-website-visibility]");
    if (site)
      assert.deepEqual(
        JSON.parse(site.getAttribute("data-website-visibility")),
        release.visibility,
        "SSR visibility snapshot differs from release manifest",
      );
    for (const node of doc.querySelectorAll("[data-website-config]"))
      assert.equal(
        node.getAttribute("data-website-config"),
        encodeURIComponent(JSON.stringify(release.configuration)),
        "HTML configuration differs from manifest",
      );
    const status = doc.querySelector('[data-website-feature="footer-status"]');
    if (status)
      assert.equal(
        status.getAttribute("href"),
        release.configuration.footerStatus,
      );
    const newsletter = doc.querySelector('[data-website-feature="newsletter"]');
    if (newsletter) {
      assert.equal(
        newsletter
          .querySelector("[data-newsletter-privacy]")
          .getAttribute("href"),
        release.configuration.newsletter.privacyUrl,
      );
      assert.equal(
        newsletter
          .querySelector("[data-newsletter-unsubscribe]")
          .getAttribute("href"),
        release.configuration.newsletter.unsubscribeUrl,
      );
    }
    if (site)
      assert.equal(
        Boolean(doc.querySelector('nav [data-nav="blog"]')),
        release.visibility.blogMenu,
        "Blog menu visibility mismatch",
      );
    for (const anchor of doc.querySelectorAll("a[href]")) {
      const href = anchor.getAttribute("href"),
        target = new URL(href, `https://nyx.chrono-ai.fun${page.path}`);
      if (target.origin !== "https://nyx.chrono-ai.fun") continue;
      const known =
        paths.has(target.pathname) ||
        ["/privacy.html", "/terms.html"].includes(target.pathname) ||
        appPatterns.some((pattern) => pattern.test(target.pathname)) ||
        /^\/(?:api|oauth|mcp|health|\.well-known)(?:\/|$)/.test(
          target.pathname,
        );
      if (!known) {
        try {
          await fs.access(path.join(dist, target.pathname));
        } catch {
          throw new Error(`Unknown public link: ${href}`);
        }
      }
      if (target.pathname === page.path && target.hash)
        assert.ok(
          doc.getElementById(decodeURIComponent(target.hash.slice(1))),
          `Missing fragment: ${href}`,
        );
    }
    const schema = JSON.parse(
      doc.querySelector('script[type="application/ld+json"]').textContent,
    );
    assert.equal(schema["@type"], "WebPage");
    assert.equal(schema.url, `https://nyx.chrono-ai.fun${page.path}`);
    assert.ok(md.includes(page.title));
    assert.ok(md.includes(`canonical: https://nyx.chrono-ai.fun${page.path}`));
    assert.ok(llms.includes(page.markdown));
    assert.equal(
      sitemap.includes(`<loc>https://nyx.chrono-ai.fun${page.path}</loc>`),
      page.indexable,
    );
    assert.ok(!html.includes("<!--$!-->"), "Suspended body");
    if (page.route.startsWith("/solutions/")) {
      const slug = page.route.slice(11);
      const enabled = release.visibility.solutions.includes(slug);
      assert.equal(
        Boolean(doc.querySelector(".solution-activation button[disabled]")),
        !enabled,
      );
      if (enabled) {
        const link = doc.querySelector(`[data-solution-activation="${slug}"]`);
        assert.ok(link, "Missing enabled activation CTA");
        assert.equal(
          link.getAttribute("href"),
          `/login?${new URLSearchParams({ return_to: release.configuration.solutions[slug] })}`,
        );
      }
      assert.ok(!doc.querySelector('a[href^="/start"]'));
      assert.ok(
        enabled ||
          md.includes(page.locale === "zh" ? "暂未开放" : "Not available yet"),
      );
    }
    if (page.path.endsWith("contact")) {
      assert.equal(
        doc.querySelectorAll("#cForm input,#cForm select,#cForm textarea")
          .length,
        5,
      );
      assert.equal(
        Boolean(doc.querySelector('[role="log"]')),
        release.visibility.contactChat,
      );
    }
  } catch (error) {
    failures.push(`${page.path}: ${error.message}`);
  }
}
const forbidden = inventory.pages
  .filter((p) => p.approval !== "approved")
  .map((p) => p.route.slice(p.route.lastIndexOf("/") + 1));
async function scan(dir) {
  for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) await scan(file);
    else if (/\.(js|json|map|md|html|xml|txt)$/.test(file)) {
      const text = await fs.readFile(file, "utf8");
      if (/\.js$/.test(file))
        for (const feature of retainedDisabledFeatureCode(
          text,
          release.visibility,
        ))
          failures.push(
            `${path.relative(dist, file)} retains disabled ${feature} code`,
          );
      for (const marker of [
        ...forbidden,
        "A code is a blank cheque",
        "The pitch for push approvals is a single sentence",
        "How a 40-seat bistro stopped missing late-night bookings",
      ]) {
        if (text.includes(marker))
          failures.push(
            `${path.relative(dist, file)} contains unpublished marker: ${marker}`,
          );
      }
    }
  }
}
await scan(dist);
const report = {
  status: failures.length ? "fail" : "pass",
  sourceCommit: release.sourceCommit,
  contentRevision: release.contentRevision,
  checks: [
    "body",
    "locale",
    "head",
    "hreflang",
    "schema",
    "markdown",
    "sitemap",
    "llms",
    "publication-footprint",
    "internal-links",
  ],
  pages: release.pages.length,
  failures,
};
await fs.writeFile(
  path.join(dist, "public-validation.json"),
  JSON.stringify(report, null, 2),
);
assert.equal(failures.length, 0, failures.join("\n"));
console.log(`Public artifact gate passed: ${release.pages.length} pages.`);

console.log(
  "Public release proof:",
  JSON.stringify({
    sourceCommit: release.sourceCommit,
    contentRevision: release.contentRevision,
    pages: release.pages.length,
    status: report.status,
  }),
);
