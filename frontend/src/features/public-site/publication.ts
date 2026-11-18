import published from "../../../content/published.json";
export type Locale = "en" | "zh";
export const publishedPages = published;
export function publishedRoute(route: string) {
  return publishedPages.find((p) => p.route === route);
}
export function publicationPath(route: string, locale: Locale) {
  return locale === "zh" ? `/zh${route === "/" ? "" : route}` : route;
}
export function parsePublicPath(pathname: string) {
  const path = pathname.replace(/\/$/, "") || "/";
  const locale: Locale =
    path === "/zh" || path.startsWith("/zh/") ? "zh" : "en";
  return { route: locale === "zh" ? path.slice(3) || "/" : path, locale };
}
