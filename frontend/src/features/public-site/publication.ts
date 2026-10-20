import inventory from "../../../content/publication.json";
export type Locale = "en" | "zh";
export const publicationInventory = inventory.pages;
export const publishedPages = publicationInventory.filter(p => p.publication === "published" && p.approval === "approved");
export function publishedRoute(route: string) { return publishedPages.find(p => p.route === route); }
export function publicationPath(route: string, locale: Locale) { return locale === "zh" ? `/zh${route === "/" ? "" : route}` : route; }
export function parsePublicPath(pathname: string) {
  const path = pathname.replace(/\/$/, "") || "/";
  const locale: Locale = path === "/zh" || path.startsWith("/zh/") ? "zh" : "en";
  return { route: locale === "zh" ? path.slice(3) || "/" : path, locale };
}
