import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";
import {
  renderPage,
  publishedPages,
  publicationInventory,
  publicationPath,
  allDocPages,
  metadataHead,
  markdownPath,
  PUBLIC_ORIGIN,
} from "../.public-render/public-render.js";
import { semanticMarkdown } from "./public-content.mjs";
import { writeRouting } from "./public-routing.mjs";
const root = path.resolve(import.meta.dirname, ".."),
  dist = path.join(root, "dist");
const hash = (value) => createHash("sha256").update(value).digest("hex");
const origin = process.env.PUBLIC_SITE_ORIGIN || PUBLIC_ORIGIN;
if (new URL(origin).origin !== origin)
  throw new Error(
    "PUBLIC_SITE_ORIGIN must be an origin without a trailing slash",
  );
const seen = new Set();
for (const page of publicationInventory) {
  if (
    !/^\/(?:[a-z0-9/-]*)$/.test(page.route) ||
    seen.has(page.route) ||
    !["approved", "pending"].includes(page.approval) ||
    !["published", "candidate"].includes(page.publication)
  )
    throw new Error(`Invalid publication entry: ${page.route}`);
  seen.add(page.route);
  if (page.approval === "approved" && !page.approval_basis)
    throw new Error(`Missing approval basis: ${page.route}`);
  if (page.route.startsWith("/blog/") && page.approval === "approved")
    throw new Error(
      "Formal article source/approval adapter is required before publishing candidate articles",
    );
  if (
    !page.locales.length ||
    new Set(page.locales).size !== page.locales.length ||
    page.locales.some((l) => !["en", "zh"].includes(l))
  )
    throw new Error(`Invalid locales: ${page.route}`);
}
let template = await fs.readFile(path.join(dist, "index.html"), "utf8");
// The private SPA shell is a separate, deliberately non-indexable artifact.
await fs.writeFile(
  path.join(dist, "app.html"),
  template.replace(
    "</head>",
    '<meta name="robots" content="noindex,nofollow"></head>',
  ),
);
template = template
  .replace(/<title>[\s\S]*?<\/title>/, " ")
  .replace(/<meta name="description"[^>]*>/, "");
const docs = allDocPages();
const knownDocs = new Set(docs.map((p) => p.slug));
const readDoc = (slug) => {
  if (!knownDocs.has(slug)) throw new Error("Unmanifested documentation");
  return fs.readFile(path.join(root, "../docs/site", `${slug}.md`), "utf8");
};
const paths = [
  ...publishedPages.flatMap((p) =>
    p.locales.map((locale) => publicationPath(p.route, locale)),
  ),
  "/docs",
  ...docs.map((p) => `/docs/${p.slug}`),
];
const results = [];
// Serial and window-free. No browser, network, full app import or concurrent renderer.
for (const route of paths) {
  const { html, meta } = await renderPage(route, readDoc);
  const revision = hash(semanticMarkdown(html, meta, "", PUBLIC_ORIGIN));
  const file = `__public/${hash(route).slice(0, 20)}.html`;
  const pageHtml = template
    .replace(
      '<html lang="en"',
      `<html lang="${meta.locale === "zh" ? "zh-CN" : "en"}"`,
    )
    .replace("</head>", metadataHead(meta, origin) + "</head>")
    .replace(
      '<div id="root"></div>',
      `<div id="root" data-public-render="v1">${html}</div>`,
    );
  await fs.mkdir(path.dirname(path.join(dist, file)), { recursive: true });
  await fs.writeFile(path.join(dist, file), pageHtml);
  const mdPath = markdownPath(route, meta.locale);
  await fs.mkdir(path.dirname(path.join(dist, mdPath)), { recursive: true });
  await fs.writeFile(
    path.join(dist, mdPath),
    semanticMarkdown(html, meta, revision, PUBLIC_ORIGIN),
  );
  results.push({ ...meta, file, markdown: mdPath, revision });
}
for (const slug of ["privacy", "terms"]) {
  const file = `${slug}.html`,
    html = await fs.readFile(path.join(dist, file), "utf8");
  const meta = {
    path: `/${slug}`,
    route: `/${slug}`,
    locale: "en",
    title: slug === "privacy" ? "Privacy Policy" : "Terms of Service",
    description: `${slug === "privacy" ? "Privacy Policy" : "Terms of Service"} for NyxID`,
    translated: false,
    indexable: true,
  };
  const markdown = markdownPath(meta.path, "en");
  const md = await fs.readFile(
    path.join(root, "public/legal", `${slug}.md`),
    "utf8",
  );
  const revision = hash(md);
  await fs.writeFile(
    path.join(dist, markdown),
    `---\ntitle: ${JSON.stringify(meta.title)}\nlanguage: en\ncanonical: ${PUBLIC_ORIGIN}/${slug}\ncontent_revision: ${revision}\n---\n\n${md}`,
  );
  const enhanced = html
    .replace(/<title>[\s\S]*?<\/title>/, "")
    .replace(/<meta name="description"[^>]*>/, "")
    .replace(/<link rel="canonical"[^>]*>/, "")
    .replace("</head>", metadataHead(meta, origin) + "</head>");
  await fs.writeFile(path.join(dist, file), enhanced);
  results.push({ ...meta, file, markdown, revision });
}
for (const locale of ["en", "zh"]) {
  const entries = results.filter((p) => p.locale === locale);
  await fs.writeFile(
    path.join(dist, `read/${locale}/index.md`),
    `# NyxID — ${locale}\n\n` +
      entries
        .map(
          (p) =>
            `- [${p.title}](${PUBLIC_ORIGIN}${p.markdown}) — ${PUBLIC_ORIGIN}${p.path}`,
        )
        .join("\n") +
      "\n",
  );
}
await fs.writeFile(
  path.join(dist, "llms.txt"),
  `# NyxID\n\n> Public product and documentation reading index. Unavailable solutions are explicitly marked on their pages.\n\n- [English](${PUBLIC_ORIGIN}/read/en/index.md)\n- [中文](${PUBLIC_ORIGIN}/read/zh/index.md)\n- [API developer tool playbook](https://nyx-api.chrono-ai.fun/llms.txt)\n- [API full tool playbook](https://nyx-api.chrono-ai.fun/llms-full.txt)\n\n` +
    results
      .map((p) => `- [${p.title}](${PUBLIC_ORIGIN}${p.markdown})`)
      .join("\n") +
    "\n",
);
const urls = results
  .filter((p) => p.indexable)
  .map((p) => `  <url><loc>${PUBLIC_ORIGIN}${p.path}</loc></url>`)
  .join("\n");
await fs.writeFile(
  path.join(dist, "sitemap.xml"),
  `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls}\n</urlset>\n`,
);
await fs.writeFile(
  path.join(dist, "public-release.json"),
  JSON.stringify(
    {
      schemaVersion: 1,
      sourceCommit: process.env.SOURCE_COMMIT || null,
      contentRevision: hash(JSON.stringify(results)),
      origin,
      pages: results,
    },
    null,
    2,
  ),
);
await writeRouting(root, dist, results);
console.log(
  `Public render: ${results.length} canonical HTML pages; matching Markdown, llms and sitemap written.`,
);
