import { CreditCard } from "lucide-react";
import type { ServiceInsightsState } from "@/hooks/use-service-insights";
import {
  connectionBillingCategory,
  connectionBillingLabels,
  type ConnectionBillingCategory,
} from "@/lib/service-card-summary";
import { insightStatusLabel } from "@/lib/service-insights";
import { plainBilling } from "@/lib/billing-plain";
import type { CatalogEntry, KeyInfo } from "@/types/keys";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";

/** Up to three names, then a count; disabled connections are counted apart. */
function groupNames(connections: readonly KeyInfo[]): string {
  const active = connections.filter((connection) => connection.is_active);
  const disabled = connections.length - active.length;
  const shown = active.slice(0, 3).map((connection) => connection.label);
  const more = active.length - shown.length;
  return [
    shown.join(", ") + (more > 0 ? ` and ${more} more` : ""),
    disabled ? `${disabled} disabled` : "",
  ]
    .filter(Boolean)
    .join(" · ");
}

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
  // Identical entries collapse into one line; a list of 30 equal rows says
  // nothing more than "30 connections".
  const groups = [
    ...rows
      .reduce((all, row) => {
        const detail =
          row.category === "not_billable"
            ? "Not billable by NyxID"
            : row.category === "unknown"
              ? row.billing?.service_billing_configured === true
                ? "Whose key or app is used isn't confirmed"
                : "Billing details unavailable"
              : row.billing
                ? plainBilling(row.connection, row.billing, catalog).short
                : "Billing details unavailable";
        const key = `${row.category}|${detail}`;
        const group = all.get(key) ?? {
          key,
          category: row.category,
          detail,
          rows: [] as typeof rows,
        };
        group.rows.push(row);
        return all.set(key, group);
      }, new Map<string, { key: string; category: ConnectionBillingCategory; detail: string; rows: typeof rows }>())
      .values(),
  ].sort(
    (a, b) =>
      categories.indexOf(a.category) - categories.indexOf(b.category) ||
      b.rows.length - a.rows.length,
  );
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
          className="max-h-[90vh] min-w-56 max-w-[min(22rem,calc(100vw-2rem))] space-y-2 overflow-y-auto break-words [overflow-wrap:anywhere]"
        >
          {notBillable ? (
            <p>Not billable by NyxID</p>
          ) : (
            <>
              <p className="font-medium">Connection billing</p>
              {groups.map((group) => (
                <div key={group.key}>
                  <p>
                    {connectionBillingLabels[group.category]} ·{" "}
                    {group.rows.length}{" "}
                    {group.rows.length === 1 ? "connection" : "connections"}
                  </p>
                  <p className="text-muted-foreground">{group.detail}</p>
                  <p className="text-muted-foreground">
                    {groupNames(group.rows.map((row) => row.connection))}
                  </p>
                </div>
              ))}
              <p className="border-t border-border/60 pt-1.5 text-muted-foreground">
                NyxID: uses NyxID&apos;s key or app and costs NyxID credits.
                BYOK: uses your or your organization&apos;s own key or app. —:
                NyxID doesn&apos;t charge for it.
              </p>
            </>
          )}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
