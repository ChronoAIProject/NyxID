import { CreditCard } from "lucide-react";
import type { ServiceInsightsState } from "@/hooks/use-service-insights";
import { connectionBillability } from "@/lib/service-card-summary";
import { insightStatusLabel } from "@/lib/service-insights";
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
    billable:
      insights.status === "ready"
        ? connectionBillability(
            connection,
            insights.connections.get(connection.id)?.billing,
            catalog,
          )
        : undefined,
  }));
  const billed = rows.filter((row) => row.billable === true);
  const unknown = rows.filter((row) => row.billable === undefined).length;
  const label =
    insights.status !== "ready"
      ? insightStatusLabel(insights.status, "Billing")
      : billed.length
        ? `${billed.length}${unknown ? "+" : ""} of ${rows.length} ${rows.length === 1 ? "connection" : "connections"} billable`
        : unknown
          ? `Billing unverified · ${unknown} ${unknown === 1 ? "connection" : "connections"}`
          : "No NyxID usage charges";
  return (
    <TooltipProvider delayDuration={180}>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            aria-label={`Show billing for ${serviceName}`}
            className="flex h-6 w-full min-w-0 items-center gap-2 rounded-sm text-left text-xs focus-visible:outline-2 focus-visible:outline-ring"
            onClick={() => {
              const first = billed[0] ?? rows[0];
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
          <p className="font-medium">NyxID usage billing</p>
          {unknown > 0 && (
            <p>
              {unknown} {unknown === 1 ? "connection has" : "connections have"}{" "}
              unverified billing; the billable count may be higher.
            </p>
          )}
          {rows.map(({ connection, billable }) => (
            <p key={connection.id}>
              {connection.label}:{" "}
              {billable === true
                ? "Billable"
                : billable === false
                  ? "No NyxID usage charge"
                  : "Unverified"}
              {!connection.is_active ? " · disabled" : ""}
            </p>
          ))}
          <p className="text-muted-foreground">
            Counts configured charges, including disabled connections.
            Allowances and grants can cover charges. Provider invoices are
            separate.
          </p>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
