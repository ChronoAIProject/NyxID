export const DEFAULT_PUBLIC_ORIGIN = "https://nyxid.ai";

/** Resolve the public site origin used by static/SEO output at build time. */
export function getPublicOrigin(env = process.env) {
  const configured = typeof env.NYXID_PUBLIC_ORIGIN === "string"
    ? env.NYXID_PUBLIC_ORIGIN.trim()
    : "";
  return (configured || DEFAULT_PUBLIC_ORIGIN).replace(/\/+$/, "");
}

export function publicUrl(pathname, env = process.env) {
  const path = pathname.startsWith("/") ? pathname : `/${pathname}`;
  return `${getPublicOrigin(env)}${path}`;
}

export function buildSitemap(urls, env = process.env) {
  const body = urls
    .map((url) => `  <url><loc>${publicUrl(url, env)}</loc></url>`)
    .join("\n");
  return `<?xml version="1.0" encoding="UTF-8"?>\n` +
    `<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${body}\n</urlset>\n`;
}

export function buildRobots(source, env = process.env) {
  return source.replace(
    /^(Sitemap:\s*)\S+$/m,
    `$1${publicUrl("/sitemap.xml", env)}`,
  );
}

export function buildLegalMarkdown(source, env = process.env) {
  return source.replaceAll(DEFAULT_PUBLIC_ORIGIN, getPublicOrigin(env));
}

