import { useOrg } from "@/hooks/use-orgs";
import type { FeatureFlag } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";

/**
 * Whether `flag` is enabled for the current user.
 *
 * - `useFeature(flag)` (no `orgId`) → **personal / non-org** context: reads the
 *   server-resolved set from `/users/me` (`capabilities.enabled_features`) via
 *   the auth store. Works for users with zero orgs and on non-org surfaces.
 * - `useFeature(flag, orgId)` → **org** context: reads that org's resolved set
 *   from the `useOrg(orgId)` query.
 *
 * Fail-closed: unknown/loading/missing → `false`, mirroring `isBillingAvailable`.
 * For a declarative gate, use `<Feature>` from `@/components/shared/feature-gate`.
 */
export function useFeature(flag: FeatureFlag, orgId?: string): boolean {
  const personal = useAuthStore((s) => s.user?.capabilities?.enabled_features);
  // `useOrg("")` is disabled (no fetch) when there's no org in context.
  const { data: org } = useOrg(orgId ?? "");
  const enabled = orgId ? org?.enabled_features : personal;
  return enabled?.includes(flag) ?? false;
}
