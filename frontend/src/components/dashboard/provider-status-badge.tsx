import { StatusBadge } from "@/components/shared/status-badge";
import type { UserProviderToken } from "@/types/api";

interface ProviderStatusBadgeProps {
  readonly status: UserProviderToken["status"];
}

/**
 * Thin wrapper around `<StatusBadge domain="provider" />` so existing
 * call sites keep their `<ProviderStatusBadge status={status} />` API.
 * The label/variant/tooltip/remediation all come from
 * `STATUS_REGISTRY.provider` in `@/lib/status-contract`. Wave B item B.4.
 */
export function ProviderStatusBadge({ status }: ProviderStatusBadgeProps) {
  return <StatusBadge domain="provider" statusKey={status} />;
}
