import { CreditCard } from "lucide-react";
import type { ServiceInsightsState } from "@/hooks/use-service-insights";
import {
  connectionBillingCategory,
  connectionBillingLabels,
  type ConnectionBillingCategory,
} from "@/lib/service-card-summary";
import { insightStatusLabel, nyxidChargeLabel } from "@/lib/service-insights";
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
    billing: insights.connections.get(connection.id)?.billing,
    category:
      insights.status === "ready"
        ? connectionBillingCategory(
            connection,
            insights.connections.get(connection.id)?.billing,
            catalog,
          )
        : ("unknown" as const),
  }));
  const categories: ConnectionBillingCategory[] = [
    "platform",
    "byok",
    "not_billable",
    "unknown",
  ];
  const countLabels = {
    platform: "NyxID",
    byok: "BYOK",
    not_billable: "—",
    unknown: "unverified",
  };
  const notBillable = rows.every((row) => row.category === "not_billable");
  const label =
    insights.status !== "ready"
      ? insightStatusLabel(insights.status, "Billing")
      : notBillable
        ? "—"
        : rows.length === 1
          ? connectionBillingLabels[rows[0]!.category]
          : categories
              .flatMap((category) => {
                const count = rows.filter(
                  (row) => row.category === category,
                ).length;
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
            aria-description={notBillable ? "Not billable by NyxID" : undefined}
            className="flex min-h-6 w-fit min-w-0 max-w-full items-center gap-2 rounded-sm text-left text-xs focus-visible:outline-2 focus-visible:outline-ring"
            onClick={() => {
              const first =
                rows.find((row) => row.category === "platform") ?? rows[0];
              if (first) onOpen(first.connection.id);
            }}
          >
            <CreditCard
              className="size-3.5 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
            <span className="line-clamp-2 font-medium leading-4">{label}</span>
          </button>
        </TooltipTrigger>
        <TooltipContent
          side={notBillable ? "right" : "bottom"}
          align={notBillable ? "center" : "start"}
          sideOffset={8}
          collisionPadding={12}
          className="max-w-[min(22rem,calc(100vw-2rem))] space-y-1 break-words [overflow-wrap:anywhere]"
        >
          {notBillable ? (
            <p>Not billable by NyxID</p>
          ) : (
            <>
              <p className="font-medium">Connection billing</p>
              {rows.map(({ connection, category, billing }) => (
                <div key={connection.id}>
                  <p>
                    {connection.label}:{" "}
                    {category === "not_billable"
                      ? "Not billable by NyxID"
                      : connectionBillingLabels[category]}
                    {!connection.is_active ? " · disabled" : ""}
                  </p>
                  {category === "unknown" && (
                    <p className="text-muted-foreground">
                      {billing?.service_billing_configured === true
                        ? "Credential supplier unverified"
                        : "Billing configuration unavailable"}
                    </p>
                  )}
                  {billing && category !== "not_billable" && (
                    <p className="text-muted-foreground">
                      {nyxidChargeLabel(billing)}
                    </p>
                  )}
                </div>
              ))}
              <p className="text-muted-foreground">
                NyxID means platform billing is configured. BYOK means the
                selected key or developer app was supplied by you or your
                organization. Free credits and grants do not change these
                labels. A dash means not billable by NyxID; the provider may
                charge separately.
              </p>
            </>
          )}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
