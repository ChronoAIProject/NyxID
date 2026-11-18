import { allDocPages } from "../docs/manifest";
import { parsePublicPath, publishedRoute } from "./publication";
export const docPaths = new Set([
  "/docs",
  ...allDocPages().map((p) => `/docs/${p.slug}`),
]);
export function isContentPath(path: string) {
  const { route, locale } = parsePublicPath(path);
  const page = publishedRoute(route);
  return Boolean(page?.locales.includes(locale)) || docPaths.has(path);
}
