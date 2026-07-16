import type { ReactNode } from "react";
import { useFeature } from "@/hooks/use-feature-flag";
import type { FeatureFlag } from "@/lib/feature-flags";

/**
 * Declarative feature gate: renders `children` only when `flag` is enabled,
 * else `fallback`. Omit `orgId` for the personal (non-org) context; pass it for
 * an org context. Fail-closed via `useFeature`.
 */
export function Feature({
  flag,
  orgId,
  children,
  fallback = null,
}: {
  readonly flag: FeatureFlag;
  readonly orgId?: string;
  readonly children: ReactNode;
  readonly fallback?: ReactNode;
}) {
  const enabled = useFeature(flag, orgId);
  return <>{enabled ? children : fallback}</>;
}
