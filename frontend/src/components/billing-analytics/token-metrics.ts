import type { AdminUsageStats } from "@/types/admin";

export const TOKEN_METRICS = [
  "total_tokens",
  "prompt_tokens",
  "completion_tokens",
  "cached_tokens",
  "cache_creation_tokens",
] as const;
export type TokenMetric = (typeof TOKEN_METRICS)[number];

export function sumTokenMetrics(
  usage: AdminUsageStats,
  selected: readonly TokenMetric[],
): number {
  const hasTotal = selected.includes("total_tokens");
  if (hasTotal) return usage.total_tokens;
  return selected.reduce((sum, metric) => sum + usage[metric], 0);
}

export function hasRedundantTokenSelection(
  selected: readonly TokenMetric[],
): boolean {
  return selected.includes("total_tokens") && selected.length > 1;
}
