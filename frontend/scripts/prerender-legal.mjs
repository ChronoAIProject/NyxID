import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ArrowLeft } from "lucide-react";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dist = path.join(root, "dist");
const css = await fs.readFile(path.join(root, "scripts", "legal-page.css"), "utf8");
const etags = {};

function escapeHtml(value) {
  return value.replace(/[&<>"']/g, (char) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[char]);
}

for (const slug of ["privacy", "terms"]) {
  const markdown = await fs.readFile(path.join(root, "public", "legal", `${slug}.md`), "utf8");
  const frontMatter = markdown.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n/);
  if (!frontMatter) throw new Error(`Missing front matter in legal/${slug}.md`);
  const title = frontMatter[1].match(/^title:\s*(.+)$/m)?.[1]?.trim();
  if (!title) throw new Error(`Missing title in legal/${slug}.md`);
  const effectiveDate = frontMatter[1].match(/^effective_date:\s*(.+)$/m)?.[1]?.trim();
  if (!effectiveDate) throw new Error(`Missing effective date in legal/${slug}.md`);
  const displayTitle = slug === "terms" ? "Terms of Service" : title;
  let content = markdown.slice(frontMatter[0].length).replace(/^\s*#\s+[^\n]+\n+/, "");
  if (slug === "privacy") content = content.replace(/^\*\*Effective date:[^\n]+\*\*\s*\n+/, "");
  const article = renderToStaticMarkup(
    React.createElement(ReactMarkdown, { remarkPlugins: [remarkGfm] }, content),
  );
  const backIcon = renderToStaticMarkup(React.createElement(ArrowLeft, { size: 12, "aria-hidden": true }));
  const html = `<!doctype html>
<html lang="en" class="dark">
  <head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
    <meta name="description" content="${escapeHtml(displayTitle)} for NyxID">
    <link rel="canonical" href="https://nyx.chrono-ai.fun/${slug}">
    <link rel="icon" type="image/svg+xml" href="/nyxid-coloured-icon.svg">
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Mona+Sans:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500&display=swap">
    <title>${escapeHtml(displayTitle)} | NyxID</title>
    <script>try{const mode=JSON.parse(localStorage.getItem("nyxid.theme")||"null")?.state?.mode;if(mode==="light"||(mode!=="dark"&&matchMedia("(prefers-color-scheme: light)").matches))document.documentElement.classList.add("theme-light")}catch{}</script>
    <style>${css}</style>
  </head>
  <body class="legal-page legal-page--${slug}">
    <div class="legal-page__container">
      <header class="legal-page__header">
        <a href="/" aria-label="NyxID home"><img class="legal-page__logo legal-page__logo--dark" src="/nyxid-coloured-logo.svg" alt="NyxID"><img class="legal-page__logo legal-page__logo--light" src="/nyxid-coloured-logo-dark.svg" alt=""></a>
        <h1>${escapeHtml(displayTitle)}</h1>
        <p>Effective date: ${escapeHtml(effectiveDate)}</p>
      </header>
      <main class="legal-page__document" data-doc-key="${slug}">${article}</main>
      <footer class="legal-page__footer"><a href="/">${backIcon}Back to NyxID</a></footer>
    </div>
  </body>
</html>
`;
  await fs.writeFile(path.join(dist, `${slug}.html`), html);
  etags[slug] = createHash("sha256").update(html).digest("hex");
}

await fs.writeFile(
  path.join(dist, "legal-cache.conf"),
  `map $uri $nyxid_legal_etag {\n` +
    `    default "";\n` +
    Object.entries(etags).flatMap(([slug, hash]) => [
      `    /${slug} "W/\\"${hash}\\"";\n`,
      `    /${slug}.html "W/\\"${hash}\\"";\n`,
    ]).join("") +
    `}\n`,
);
