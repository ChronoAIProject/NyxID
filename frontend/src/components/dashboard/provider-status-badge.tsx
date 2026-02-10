import { Badge } from "@/components/ui/badge";
import type { UserProviderToken } from "@/types/api";

interface ProviderStatusBadgeProps {
  readonly status: UserProviderToken["status"];
}

const STATUS_CONFIG: Readonly<
  Record<
    UserProviderToken["status"],
    { readonly label: string; readonly variant: "success" | "warning" | "destructive" | "secondary" }
  >
> = {
  active: { label: "Connected", variant: "success" },
  expired: { label: "Expired", variant: "warning" },
  revoked: { label: "Revoked", variant: "destructive" },
  refresh_failed: { label: "Refresh Failed", variant: "destructive" },
};

export function ProviderStatusBadge({ status }: ProviderStatusBadgeProps) {
  const config = STATUS_CONFIG[status] ?? {
    label: status,
    variant: "secondary" as const,
  };

  return <Badge variant={config.variant}>{config.label}</Badge>;
}
