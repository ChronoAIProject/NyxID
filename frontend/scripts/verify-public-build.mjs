import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { semanticMarkdown } from "./public-content.mjs";
import { Window } from "happy-dom";
const root = path.resolve(import.meta.dirname, ".."),
  dist = path.join(root, "dist");
const release = JSON.parse(
  await fs.readFile(path.join(dist, "public-release.json"), "utf8"),
);
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
for (const page of release.pages) {
  try {
    const html = await fs.readFile(path.join(dist, page.file), "utf8"),
      md = await fs.readFile(path.join(dist, page.markdown), "utf8");
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
      assert.ok(doc.querySelector("button[disabled]"));
      assert.ok(!doc.querySelector('a[href^="/start"]'));
      assert.ok(
        md.includes(page.locale === "zh" ? "暂未开放" : "Not available yet"),
      );
    }
    if (page.path.endsWith("contact")) {
      assert.equal(
        doc.querySelectorAll("#cForm input,#cForm select,#cForm textarea")
          .length,
        5,
      );
      assert.equal(doc.querySelector('[role="log"]'), null);
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
