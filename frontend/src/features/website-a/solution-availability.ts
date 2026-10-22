import { websiteRelease } from "./release-config";
import { solutionSlugs } from "./solution-inventory";
export { solutionSlugs } from "./solution-inventory";
/** No solution has supplied end-to-end activation approval. Browsing stays public. */
export function solutionStartTarget(
  query: string,
  targets: Readonly<
    Record<string, string | undefined>
  > = websiteRelease.solutions,
): string | null {
  const slug = new URLSearchParams(query).get("t");
  if (!slug) return null;
  return (solutionSlugs as readonly string[]).includes(slug)
    ? (targets[slug] ?? `/solutions/${slug}`)
    : "/solutions";
}
