export const solutionSlugs = ["restaurant-manager", "help-desk", "appointment-booker", "callback-queue", "stock-analyst", "news-aggregator", "desk-researcher", "x-lead-catcher", "back-office-clerk"] as const;

/** No solution has supplied end-to-end activation approval. Browsing stays public. */
export function solutionStartTarget(query: string): string | null {
  const slug = new URLSearchParams(query).get("t");
  if (!slug) return null;
  return (solutionSlugs as readonly string[]).includes(slug) ? `/solutions/${slug}` : "/solutions";
}
