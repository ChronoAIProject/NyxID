import { CreditCard } from "lucide-react";
import type { ServiceInsightsState } from "@/hooks/use-service-insights";
import {
  connectionBillingCategory,
  connectionBillingLabels,
  type ConnectionBillingCategory,
} from "@/lib/service-card-summary";
import { insightStatusLabel } from "@/lib/service-insights";
import { configuredPlatformPrice } from "@/lib/service-billing-config";
import { lanePriceLabel } from "@/schemas/platform-keys";
import type { CatalogEntry, KeyInfo } from "@/types/keys";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";

export function ServiceBillingSummary({
  connections,
  insights,
  catalog,
  serviceName,
  onOpen,
}: {
  readonly connections: readonly KeyInfo[];
  readonly insights: ServiceInsightsState;
  readonly catalog?: CatalogEntry;
  readonly serviceName: string;
  readonly onOpen: (connectionId: string) => void;
}) {
  const rows = connections.map((connection) => ({
    connection,
    platformPrice: configuredPlatformPrice(connection, catalog),
    category:
      insights.status === "ready"
        ? connectionBillingCategory(
            connection,
            insights.connections.get(connection.id)?.billing,
            catalog,
          )
        : ("unknown" as const),
  }));
  const platformRow = rows.find((row) => row.platformPrice);
  const platformPrice = platformRow?.platformPrice;
  const categories: ConnectionBillingCategory[] = [
    "platform",
    "byok",
    "not_billable",
    "unknown",
  ];
  const countLabels = {
    platform: "NyxID",
    byok: "BYOK",
    not_billable: "not billable",
    unknown: "unverified",
  };
  const label = platformPrice
    ? "NyxID platform billing"
    : insights.status !== "ready"
      ? insightStatusLabel(insights.status, "Billing")
      : rows.length === 1
        ? connectionBillingLabels[rows[0]!.category]
        : categories
            .flatMap((category) => {
              const count = rows.filter((row) => row.category === category).length;
              return count ? [`${count} ${countLabels[category]}`] : [];
            })
            .join(" · ");
  return (
    <TooltipProvider delayDuration={180}>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            aria-label={`Show billing for ${serviceName}`}
            className="flex h-6 w-full min-w-0 items-center gap-2 rounded-sm text-left text-xs focus-visible:outline-2 focus-visible:outline-ring"
            onClick={() => {
              const first =
                rows.find((row) => row.category === "platform") ??
                platformRow ??
                rows[0];
              if (first) onOpen(first.connection.id);
            }}
          >
            <CreditCard
              className="size-3.5 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
            <span className="truncate font-medium">{label}</span>
          </button>
        </TooltipTrigger>
        <TooltipContent
          collisionPadding={12}
          className="max-w-[min(22rem,calc(100vw-2rem))] space-y-1 break-words [overflow-wrap:anywhere]"
        >
          <p className="font-medium">Connection billing</p>
          {platformPrice && (
            <div className="space-y-1 border-b border-border pb-2">
              <p className="font-medium">NyxID platform billing configured</p>
              <p>{lanePriceLabel(platformPrice)}</p>
              <p className="text-muted-foreground">
                From this service's billing configuration. The credential selected
                for each request determines which rate applies.
              </p>
            </div>
          )}
          {rows.map(({ connection, category }) => (
            <p key={connection.id}>
              {connection.label}:{" "}
              {connectionBillingLabels[category]}
              {!connection.is_active ? " · disabled" : ""}
            </p>
          ))}
          <p className="text-muted-foreground">
            NyxID supplies the platform key or developer app. BYOK uses a key
            or app supplied by you or your organization. Not billable means no
            provider credential and no configured NyxID usage charge.
          </p>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
