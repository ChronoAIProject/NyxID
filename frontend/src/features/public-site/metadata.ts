import { publicationPath, type Locale } from "./publication";
export const PUBLIC_ORIGIN = "https://nyx.chrono-ai.fun";
export interface PublicMetadata {
  path: string;
  route: string;
  locale: Locale;
  title: string;
  description: string;
  translated: boolean;
  indexable: boolean;
}
export function escapeHtml(value: string) {
  return value.replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ]!,
  );
}
export function markdownPath(path: string, locale: Locale) {
  const route = locale === "zh" ? path.slice(3) || "/" : path;
  return `/read/${locale}/${route === "/" ? "home" : route.slice(1)}.md`;
}
export function metadataHead(meta: PublicMetadata, origin = PUBLIC_ORIGIN) {
  const e = escapeHtml,
    canonical = PUBLIC_ORIGIN + meta.path;
  const title = `${meta.title} · NyxID`;
  const alternates = meta.translated
    ? (["en", "zh"] as const)
        .map(
          (lang) =>
            `<link data-public-head rel="alternate" hreflang="${lang === "zh" ? "zh-Hans" : "en"}" href="${PUBLIC_ORIGIN}${publicationPath(meta.route, lang)}">`,
        )
        .join("") +
      `<link data-public-head rel="alternate" hreflang="x-default" href="${PUBLIC_ORIGIN}${meta.route}">`
    : "";
  return `<title data-public-head>${e(title)}</title>
<meta data-public-head name="description" content="${e(meta.description)}">
<meta data-public-head name="robots" content="${meta.indexable && origin === PUBLIC_ORIGIN ? "index,follow" : "noindex,follow"}">
<link data-public-head rel="canonical" href="${canonical}">${alternates}
<link data-public-head rel="alternate" type="text/markdown" href="${PUBLIC_ORIGIN}${markdownPath(meta.path, meta.locale)}">
<meta data-public-head property="og:type" content="website">
<meta data-public-head property="og:title" content="${e(title)}">
<meta data-public-head property="og:description" content="${e(meta.description)}">
<meta data-public-head property="og:url" content="${canonical}">
<meta data-public-head property="og:locale" content="${meta.locale === "zh" ? "zh_CN" : "en_US"}">
<meta data-public-head property="og:image" content="${PUBLIC_ORIGIN}/nyxid-coloured-logo.svg">
<meta data-public-head name="twitter:card" content="summary">
<meta data-public-head name="twitter:title" content="${e(title)}">
<meta data-public-head name="twitter:description" content="${e(meta.description)}">
<meta data-public-head name="twitter:image" content="${PUBLIC_ORIGIN}/nyxid-coloured-logo.svg">
<script data-public-head type="application/ld+json">${JSON.stringify({ "@context": "https://schema.org", "@type": "WebPage", name: meta.title, description: meta.description, url: canonical, inLanguage: meta.locale === "zh" ? "zh-CN" : "en" }).replace(/</g, "\u003c")}</script>`;
}
export function applyMetadata(meta: PublicMetadata) {
  document.head
    .querySelectorAll(
      '[data-public-head],title,meta[name="description"],meta[name="robots"],link[rel="canonical"],link[hreflang],meta[property^="og:"],meta[name^="twitter:"]',
    )
    .forEach((el) => el.remove());
  const template = document.createElement("template");
  template.innerHTML = metadataHead(meta, window.location.origin);
  document.head.append(template.content);
  document.documentElement.lang = meta.locale === "zh" ? "zh-CN" : "en";
}
