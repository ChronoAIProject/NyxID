import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ArrowLeft } from "lucide-react";
import { publicUrl } from "./public-origin.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dist = path.join(root, "dist");
const css = (await fs.readFile(path.join(root, "src/features/website-a/public-design.css"), "utf8")).replace(/^@import[^;]+;\s*/, "") +
  await fs.readFile(path.join(root, "scripts", "legal-page.css"), "utf8");
const footerGroups = JSON.parse(await fs.readFile(path.join(root, "src/features/public-site/public-footer-links.json"), "utf8"));
const etags = {};
// Standalone legal pages share the public choice without booting the application theme store.
const themeScript = `(()=>{let chosen=null;try{chosen=localStorage.getItem("nyxid-theme")}catch{}const media=matchMedia("(prefers-color-scheme: dark)");const apply=t=>{document.documentElement.dataset.theme=t;document.querySelector('meta[name="theme-color"]')?.setAttribute("content",t==="dark"?"#0E0E10":"#F3F3F0");document.querySelector(".public-theme-toggle")?.setAttribute("aria-label",t==="dark"?"Switch to day mode":"Switch to night mode")};const resolve=()=>chosen==="dark"||chosen==="light"?chosen:media.matches?"dark":"light";apply(resolve());media.addEventListener("change",()=>{if(!chosen)apply(resolve())});addEventListener("storage",e=>{if(e.key==="nyxid-theme"){chosen=e.newValue;apply(resolve())}});document.addEventListener("DOMContentLoaded",()=>{apply(resolve());document.querySelector(".public-theme-toggle")?.addEventListener("click",()=>{chosen=resolve()==="dark"?"light":"dark";try{localStorage.setItem("nyxid-theme",chosen)}catch{}apply(chosen)});document.addEventListener("keydown",e=>{if(e.key==="Escape"){const menu=document.querySelector(".public-more[open]");if(menu){menu.open=false;menu.querySelector("summary")?.focus()}}});for(const a of document.querySelectorAll('a[href^="/"]')){const u=new URL(a.href);const q=new URLSearchParams(location.search);u.searchParams.forEach((v,k)=>q.set(k,v));u.search=q.toString();a.href=u.pathname+u.search+u.hash}})})();`;
const themePath = `/assets/legal-theme-${createHash("sha256").update(themeScript).digest("hex").slice(0, 16)}.js`;
await fs.writeFile(path.join(dist, themePath.slice(1)), themeScript);

function escapeHtml(value) {
  return value.replace(/[&<>"']/g, (char) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[char]);
}

const brand = `<span class="public-brand"><img class="public-brand-mark" src="/nyxid-coloured-icon.svg" alt="" width="24" height="24"><img class="public-brand-day" src="/nyxid-coloured-logo-dark.svg" alt="NyxID" width="88" height="25"><img class="public-brand-night" src="/nyxid-coloured-logo.svg" alt="NyxID" width="88" height="25"></span>`;
const themeButton = `<button class="public-theme-toggle" type="button" aria-label="Switch to night mode"><svg class="public-moon" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="M20 14.5A8 8 0 019.5 4a8 8 0 1010.5 10.5z"/></svg><svg class="public-sun" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5L19 19M5 19l1.5-1.5M17.5 6.5L19 5"/></svg></button>`;
const primaryLinks = [["/nyxbot", "NyxBot"], ["/solutions", "Solutions"], ["/integrations", "Tools"], ["/pricing", "Pricing"]];
const detailLinks = [["/how-it-works", "How it works", "Follow a request through NyxID."], ["/agents", "Your agent", "Connect Claude, Cursor, Codex or your code."], ["/security", "Security", "Credentials, permissions, approvals and audit."], ["/docs", "Docs", "Setup guides and API reference."]];
const header = `<header id="nav" class="public-header"><div class="public-wrap public-nav-bar"><div class="public-nav-box"><a class="public-home-link" href="/" aria-label="NyxID home">${brand}</a></div><div class="public-nav-box public-nav-right"><nav class="public-nav-links" aria-label="Main">${primaryLinks.map(([href,title])=>`<a href="${href}">${title}</a>`).join("")}</nav><details class="public-more"><summary class="list-none">Show more <span aria-hidden="true">⌄</span></summary><nav class="public-more-menu" aria-label="Explore NyxID">${primaryLinks.map(([href,title])=>`<a class="public-more-primary" href="${href}"><b>${title}</b></a>`).join("")}${detailLinks.map(([href,title,detail])=>`<a href="${href}"><span><b>${title}</b><small>${detail}</small></span></a>`).join("")}<div class="public-more-bottom"><a href="/login">Log in</a><a href="/contact">Contact</a></div></nav></details><span class="public-nav-divider" aria-hidden="true"></span><a class="public-login" href="/login">Log in</a>${themeButton}<a class="public-cta" href="/register">Get started</a></div></div></header>`;
const footer = `<footer id="foot" class="public-footer" aria-label="Site footer"><div class="public-wrap"><div class="public-footer-grid"><div class="public-footer-brand"><a href="/" aria-label="NyxID home">${brand}</a><p>An open-source gateway for agents and their tools.</p></div>${footerGroups.map(group=>`<nav aria-label="${escapeHtml(group.title)}"><h2>${escapeHtml(group.title)}</h2>${group.links.map(([href,title])=>`<a href="${escapeHtml(href)}"${href.startsWith("https:")?' target="_blank" rel="noopener noreferrer"':""}>${escapeHtml(title)}</a>`).join("")}</nav>`).join("")}</div><div class="public-footer-legal"><span>© 2026 ChronoAI Pte. Ltd.</span><nav aria-label="Legal"><a href="/terms">Terms</a><a href="/privacy">Privacy</a></nav></div></div></footer>`;

for (const slug of ["privacy", "terms", "data-deletion"]) {
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
<html lang="en" class="nyx-public">
  <head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
    <meta name="description" content="${escapeHtml(displayTitle)} for NyxID">
    <meta name="theme-color" content="#F3F3F0">
    <link rel="canonical" href="${publicUrl(`/${slug}`)}">
    <link rel="icon" type="image/svg+xml" href="/nyxid-coloured-icon.svg">
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Host+Grotesk:wght@300..800&family=JetBrains+Mono:wght@400;500&display=swap">
    <title>${escapeHtml(displayTitle)} | NyxID</title>
    <script src="${themePath}"></script>
    <style>${css}</style>
  </head>
  <body class="legal-page legal-page--${slug}">
    <a class="legal-page__skip" href="#legal-document">Skip to content</a>
    ${header}
    <div class="legal-page__container">
      <header class="legal-page__header">
        <h1>${escapeHtml(displayTitle)}</h1>
        <p>Effective date: ${escapeHtml(effectiveDate)}</p>
      </header>
      <main id="legal-document" class="legal-page__document" data-doc-key="${slug}">${article}</main>
      <div class="legal-page__back"><a href="/">${backIcon}Back to NyxID</a></div>
    </div>
    ${footer}
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
